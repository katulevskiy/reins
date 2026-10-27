//! The org decision service (`docs/design/decisions-v2/05-organizations.md` §5–§6, `06` §1, §3, §5; `08` R5): it
//! evaluates every Managed request with [`reins_policy::org::evaluate_managed`] against the active, issuer-signed org
//! policy and the member's own policy, records the decision and its trace in the audit chain, opens reviews, and
//! countersigns what an executor may run.
//!
//! - **Auto:** countersigned at once; an executor runs it.
//! - **Review:** the obligations wait for eligible reviewers' device-signed answers ([`super::reviews`]); once all are
//!   met it is countersigned with their signatures.
//! - **Deny:** recorded; the agent learns only the deciding rule's explanation (02 §9).
//!
//! Every request names the policy snapshot it was decided against; its body is sealed under the tenant's data key and
//! kept per the org's retention. No active policy means no Managed request is decided (fails closed).

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use diesel::Connection;
use reins_policy::org::{
    ManagedOutcome, ManagedRequest, ManagedVerdict, MemberPolicy, Obligation, ReviewClass, evaluate_managed,
};
use reins_proto::managed::{
    DecisionRecord, EnvelopeKind, LeaseBinding, LeaseJob, ReviewSignature, Verdict, canonical_json, request_digest,
};
use serde_json::{Value, json};
use tokio::sync::watch;

use super::{
    OrgError, audit,
    kms::{self, KeyPurpose},
    routing,
};
use crate::db::{
    DbConnInner,
    models::{
        reins_org, reins_org_keys,
        reins_org_request::{self, ReinsOrgBody, ReinsOrgRequest, lease_state, request_kind, request_state as st},
    },
};

/// How long an approved decision may wait for its executor (03 §6: 24 h for executor-run actions).
pub const VALID_SECS: i64 = 24 * 3600;
/// A claimed request not receipted by then is `unknown` and never run again.
pub const STUCK_SECS: i64 = 300;
/// How long an issued lease lives.
pub const LEASE_SECS: i64 = 900;

/// What [`submit`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submitted {
    /// `or_…`
    pub id: String,
    pub state: String,
    pub outcome: ManagedOutcome,
    /// A deny's reason, in the issuer's words.
    pub explain: String,
    /// When a review must be decided by.
    pub expires_at: i64,
    /// Every obligation is Instant: the agent waits for it.
    pub instant: bool,
    /// Whom to tell about the review (user ids with a device that may decide).
    pub notify: Vec<String>,
}

/// A request's lease part (a `git_lease` request).
#[derive(Clone, Debug)]
pub struct LeaseAsk {
    pub id: String,
    pub repo: String,
    pub access: reins_proto::managed::LeaseAccess,
    pub box_key: String,
}

/// The member's own stricter policy, when they set one.
pub fn member_policy_of(c: &mut DbConnInner, org: &str, user: &str) -> Result<Option<MemberPolicy>, OrgError> {
    Ok(reins_org_keys::q_member_policy(c, org, user)?.and_then(|row| serde_json::from_str(&row.body).ok()))
}

fn email_of(c: &mut DbConnInner, user: &str) -> Result<String, OrgError> {
    use crate::db::schema::users;
    use diesel::prelude::*;
    Ok(users::table
        .filter(users::uuid.eq(user))
        .select(users::email)
        .first::<String>(c)
        .optional()?
        .unwrap_or_default())
}

/// The trace kept with a request (02 §4): zone, level, certainty, why, the effect lines and the composed snapshot.
fn trace_of(v: &ManagedVerdict) -> Value {
    json!({
        "zone": v.evaluation.zone, "level": v.evaluation.level, "certainty": v.evaluation.certainty,
        "why": v.evaluation.why, "explain": v.explain, "lines": v.lines, "composed": v.evaluation.snapshot,
        "member_policy": v.member_policy,
    })
}

/// The `request` part of an audit event (06 §5.1).
fn request_detail(row: &ReinsOrgRequest, member_email: &str, agent: &str, trace: &Value) -> Value {
    json!({"id": row.uuid, "digest": row.digest, "agent": agent, "member": member_email, "tool": row.tool,
           "effects": trace["lines"], "level": trace["level"], "zone": trace["zone"]})
}

fn policy_detail(row: &ReinsOrgRequest, trace: &Value, obligations: &[Obligation]) -> Value {
    json!({"snapshot": row.snapshot, "matched": trace["why"],
           "obligations": obligations.iter().map(|o| &o.id).collect::<Vec<_>>()})
}

fn retention_secs(c: &mut DbConnInner, org: &str) -> Result<i64, OrgError> {
    Ok(reins_org::q_org(c, org)?.map_or(30, |t| i64::from(t.body_retention_days)) * 86_400)
}

/// Whoever can satisfy one of `obligations` now from their standing and has a device that may decide.
#[cfg(test)]
pub fn eligible_reviewers(
    c: &mut DbConnInner,
    org: &str,
    requester: &str,
    obligations: &[Obligation],
    now: i64,
) -> Result<Vec<String>, OrgError> {
    let policy = super::policy::active(c, org)?.map(|(p, _)| p);
    let roster = super::roster::build(c, org, policy.as_ref(), now)?;
    Ok(roster
        .members
        .iter()
        .filter(|m| m.devices.iter().any(|d| d.may_decide))
        .filter(|m| obligations.iter().any(|o| o.spec.eligible(&m.user, &m.groups, &m.roles, requester)))
        .map(|m| m.user.clone())
        .collect())
}

/// What a request brings besides its call (O3): the agent's reason, and what the dynamic routing sources said
/// ([`routing::prefetch`]).
#[derive(Clone, Copy)]
pub struct Extra<'a> {
    pub reason: Option<&'a str>,
    pub touches: &'a routing::Touches,
    pub dynamic: &'a routing::Dynamic,
}

static NO_TOUCHES: LazyLock<routing::Touches> = LazyLock::new(routing::Touches::default);
static NO_DYNAMIC: LazyLock<routing::Dynamic> = LazyLock::new(routing::Dynamic::new);

impl Default for Extra<'_> {
    fn default() -> Self {
        Self {
            reason: None,
            touches: &NO_TOUCHES,
            dynamic: &NO_DYNAMIC,
        }
    }
}

/// The longest reason kept.
pub const MAX_REASON: usize = reins_policy::org::MAX_REASON;

/// Evaluates and records one Managed request of `user` through `connection` (its id and label).
#[cfg(test)]
#[allow(clippy::too_many_arguments, reason = "one request's parts")]
pub fn submit(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    connection: &str,
    agent_label: &str,
    tool: &str,
    args: &Value,
    lease: Option<&LeaseAsk>,
    now: i64,
) -> Result<Submitted, OrgError> {
    submit_with(c, org, user, connection, agent_label, tool, args, lease, &Extra::default(), now)
}

/// [`submit`] with a reason and the routing sources' answers.
#[allow(clippy::too_many_arguments, reason = "one request's parts")]
pub fn submit_with(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    connection: &str,
    agent_label: &str,
    tool: &str,
    args: &Value,
    lease: Option<&LeaseAsk>,
    extra: &Extra<'_>,
    now: i64,
) -> Result<Submitted, OrgError> {
    // A reason beyond what a permit accepts isn't kept (and can't satisfy one).
    let reason = extra.reason.map(str::trim).filter(|r| !r.is_empty() && r.chars().count() <= MAX_REASON);
    if !reins_proto::managed::ids_ok(&[org, user, connection, tool]) {
        return Err(OrgError::Invalid("That request can't be read."));
    }
    // A lease is judged, digested and bound by exactly what it would mint: the arguments must be the lease's.
    match lease {
        Some(l) => {
            if tool != reins_proto::managed::LEASE_TOOL
                || *args != reins_proto::managed::lease_args(&l.repo, l.access, &l.box_key)
            {
                return Err(OrgError::Invalid("That lease request doesn't match what it asks for."));
            }
        }
        None if tool == reins_proto::managed::LEASE_TOOL => {
            return Err(OrgError::Invalid("That request can't be read."));
        }
        None => {}
    }
    c.transaction(|c| {
        let (policy, _) = super::policy::active(c, org)?
            .ok_or(OrgError::Invalid("Your organization hasn't activated a policy yet."))?;
        let member = member_policy_of(c, org, user)?;
        let managed = ManagedRequest {
            org,
            member: user,
            connection,
            tool,
            args,
            now,
            reason,
        };
        let verdict = evaluate_managed(&managed, &policy, member.as_ref());
        let id = format!("or_{}", crate::util::get_uuid());
        let trace = trace_of(&verdict);
        let window = verdict.obligations.iter().map(|o| i64::try_from(o.spec.window()).unwrap_or(0)).max().unwrap_or(0);
        let row = ReinsOrgRequest {
            uuid: id.clone(),
            org_uuid: org.to_owned(),
            user_uuid: user.to_owned(),
            connection_uuid: connection.to_owned(),
            tool: tool.to_owned(),
            digest: request_digest(org, user, connection, tool, args),
            kind: if lease.is_some() {
                request_kind::LEASE
            } else {
                request_kind::CALL
            }
            .to_owned(),
            state: st::REVIEWING.to_owned(),
            outcome: verdict.outcome.as_str().to_owned(),
            snapshot: verdict.snapshot.clone(),
            member_policy: verdict.member_policy.clone(),
            trace: trace.to_string(),
            obligations: serde_json::to_string(&verdict.obligations).unwrap_or_else(|_| "[]".to_owned()),
            decision: None,
            created_at: now,
            expires_at: now + window,
            decided_at: None,
            valid_until: None,
            claimed_by: None,
            claimed_at: None,
            finished_at: None,
            receipt: None,
            resolved: None,
        };
        reins_org_request::q_insert_request(c, &row)?;
        let mut body = json!({"tool": tool, "args": args});
        if let Some(r) = reason {
            body["reason"] = json!(r);
        }
        let body = canonical_json(&body);
        let (key_id, sealed) = kms::seal_data(c, org, &format!("{id}/request"), body.as_bytes(), now)?;
        reins_org_request::q_put_body(
            c,
            &ReinsOrgBody {
                request_uuid: id.clone(),
                part: "request".to_owned(),
                org_uuid: org.to_owned(),
                key_id,
                sealed,
                created_at: now,
                purge_at: None,
            },
        )?;
        if let Some(l) = lease {
            reins_org_request::q_insert_lease(
                c,
                &reins_org_request::ReinsOrgLease {
                    uuid: l.id.clone(),
                    org_uuid: org.to_owned(),
                    user_uuid: user.to_owned(),
                    connection_uuid: connection.to_owned(),
                    request_uuid: id.clone(),
                    repo: l.repo.clone(),
                    access: l.access.as_str().to_owned(),
                    box_key: l.box_key.clone(),
                    state: lease_state::PENDING.to_owned(),
                    sealed: None,
                    created_at: now,
                    expires_at: now + LEASE_SECS,
                    issued_at: None,
                    revoked_at: None,
                    revoke_done_at: None,
                },
            )?;
        }
        let email = email_of(c, user)?;
        let request = request_detail(&row, &email, agent_label, &trace);
        let pol = policy_detail(&row, &trace, &verdict.obligations);
        audit::record(
            c,
            org,
            audit::kinds::REQUEST_RECEIVED,
            Some(user),
            Some(&id),
            &json!({"request": request, "policy": pol}),
            now,
        )?;
        let mut notify = Vec::new();
        let mut expires_at = row.expires_at;
        let state = match verdict.outcome {
            ManagedOutcome::Auto => {
                finalize(c, org, &row, Vec::new(), now)?;
                st::APPROVED
            }
            ManagedOutcome::Deny => {
                end(c, org, &row, st::DENIED, now)?;
                audit::record(
                    c,
                    org,
                    audit::kinds::DECISION_DENIED,
                    None,
                    Some(&id),
                    &json!({"request": request, "policy": pol, "explain": verdict.explain}),
                    now,
                )?;
                st::DENIED
            }
            ManagedOutcome::Review => {
                audit::record(
                    c,
                    org,
                    audit::kinds::REVIEW_OPENED,
                    None,
                    Some(&id),
                    &json!({"request": request, "policy": pol, "expires_at": row.expires_at}),
                    now,
                )?;
                let resources = routing::resources_of(&policy, &managed);
                let ctx = routing::Ctx {
                    org,
                    requester: user,
                    policy: &policy,
                    resources: &resources,
                    touches: extra.touches,
                    dynamic: extra.dynamic,
                };
                let routed = routing::open(c, &id, &ctx, &verdict.obligations, now)?;
                if !routed.resolved.is_empty() {
                    let text = serde_json::to_string(&routed.resolved).unwrap_or_default();
                    reins_org_request::q_set_resolved(c, org, &id, &text)?;
                }
                if routed.expires_at != row.expires_at {
                    reins_org_request::q_set_expiry(c, org, &id, routed.expires_at)?;
                    expires_at = routed.expires_at;
                }
                notify = routed.notify.into_iter().collect();
                st::REVIEWING
            }
        };
        Ok(Submitted {
            id,
            state: state.to_owned(),
            outcome: verdict.outcome,
            explain: verdict.explain.clone(),
            expires_at,
            instant: verdict.outcome == ManagedOutcome::Review
                && verdict.obligations.iter().all(|o| o.spec.class == ReviewClass::Instant),
            notify,
        })
    })
}

/// Ends a request that won't run: its state, its pending lease, and its bodies' retention from now.
fn end(c: &mut DbConnInner, org: &str, row: &ReinsOrgRequest, to: &str, now: i64) -> Result<bool, OrgError> {
    let moved = reins_org_request::q_set_state(c, org, &row.uuid, &[st::REVIEWING, st::APPROVED], to, now)?;
    if moved {
        if let Some(l) = reins_org_request::q_lease_of_request(c, org, &row.uuid)? {
            let lease_to = if to == st::DENIED {
                lease_state::DENIED
            } else {
                lease_state::EXPIRED
            };
            reins_org_request::q_set_lease_state(c, org, &l.uuid, &[lease_state::PENDING], lease_to)?;
        }
        let keep = retention_secs(c, org)?;
        reins_org_request::q_set_purge(c, org, &row.uuid, now + keep)?;
    }
    wake(&row.uuid);
    Ok(moved)
}

/// Countersigns `row`'s decision with `approvals` (none for Auto) and makes it executable.
pub fn finalize(
    c: &mut DbConnInner,
    org: &str,
    row: &ReinsOrgRequest,
    approvals: Vec<ReviewSignature>,
    now: i64,
) -> Result<(), OrgError> {
    let lease = match reins_org_request::q_lease_of_request(c, org, &row.uuid)? {
        Some(l) => {
            let job = LeaseJob {
                lease: l.uuid.clone(),
                repo: l.repo.clone(),
                access: match l.access.as_str() {
                    "read" => reins_proto::managed::LeaseAccess::Read,
                    "push" => reins_proto::managed::LeaseAccess::Push,
                    other => return Err(OrgError::Internal(format!("lease {} has access {other:?}", l.uuid))),
                },
                box_key: l.box_key.clone(),
                expires_at: now + LEASE_SECS,
            };
            reins_org_request::q_set_lease_expiry(c, org, &l.uuid, job.expires_at)?;
            Some(LeaseBinding::of(&job))
        }
        None => None,
    };
    let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    let resolved =
        routing::stored(row.resolved.as_deref()).into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect();
    let reason = match reins_org_request::q_body(c, org, &row.uuid, "request")? {
        Some(body) => {
            let plain = kms::open_data(c, org, &body.key_id, &format!("{}/request", row.uuid), &body.sealed)?;
            serde_json::from_slice::<Value>(&plain)
                .ok()
                .and_then(|v| v.get("reason").and_then(Value::as_str).map(str::to_owned))
        }
        None => None,
    };
    let valid = obligations
        .iter()
        .filter_map(|o| o.spec.approval_valid_secs)
        .min()
        .map_or(VALID_SECS, |s| i64::try_from(s).unwrap_or(VALID_SECS).min(VALID_SECS));
    let record = DecisionRecord {
        v: reins_proto::managed::DECISION_VERSION,
        org: org.to_owned(),
        request: row.uuid.clone(),
        digest: row.digest.clone(),
        member: row.user_uuid.clone(),
        connection: row.connection_uuid.clone(),
        tool: row.tool.clone(),
        verdict: if obligations.is_empty() {
            Verdict::Auto
        } else {
            Verdict::Approved
        },
        snapshot: row.snapshot.clone(),
        member_policy: row.member_policy.clone(),
        obligations: obligations.iter().map(|o| o.id.clone()).collect(),
        approvals,
        lease,
        resolved,
        reason,
        evaluated_at: row.created_at,
        decided_at: now,
        valid_until: now + valid,
    };
    let env = kms::envelope(
        c,
        org,
        KeyPurpose::Decision,
        EnvelopeKind::Decision,
        &serde_json::to_value(&record).map_err(|e| OrgError::Internal(e.to_string()))?,
        now,
    )?;
    if !reins_org_request::q_approve(
        c,
        org,
        &row.uuid,
        &serde_json::to_string(&env).unwrap_or_default(),
        record.valid_until,
        now,
    )? {
        return Err(OrgError::Invalid("This request was already decided."));
    }
    let kind = if record.verdict == Verdict::Auto {
        audit::kinds::DECISION_AUTO
    } else {
        audit::kinds::DECISION_APPROVED
    };
    let decided_by: Vec<Value> =
        record.approvals.iter().map(|a| json!({"member": a.member, "device": a.device, "sig": a.sig})).collect();
    audit::record(
        c,
        org,
        kind,
        None,
        Some(&row.uuid),
        &json!({"request": {"id": row.uuid, "digest": row.digest, "tool": row.tool},
                "policy": {"snapshot": row.snapshot, "obligations": record.obligations},
                "decided_by": decided_by, "countersig": env.sig, "key": env.key_id,
                "valid_until": record.valid_until}),
        now,
    )?;
    wake(&row.uuid);
    Ok(())
}

/// Denies a request under review (an eligible reviewer said no).
pub fn deny(c: &mut DbConnInner, org: &str, row: &ReinsOrgRequest, by: &str, now: i64) -> Result<(), OrgError> {
    if end(c, org, row, st::DENIED, now)? {
        audit::record(
            c,
            org,
            audit::kinds::DECISION_DENIED,
            Some(by),
            Some(&row.uuid),
            &json!({"request": {"id": row.uuid, "digest": row.digest}, "policy": {"snapshot": row.snapshot}}),
            now,
        )?;
    }
    Ok(())
}

fn rfc3339(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// The receipt of a finished request, if any: (status, result, reason).
fn receipt_of(
    c: &mut DbConnInner,
    org: &str,
    row: &ReinsOrgRequest,
) -> Result<Option<reins_proto::managed::Receipt>, OrgError> {
    let Some(body) = reins_org_request::q_body(c, org, &row.uuid, "receipt")? else {
        return Ok(None);
    };
    let plain = kms::open_data(c, org, &body.key_id, &format!("{}/receipt", row.uuid), &body.sealed)?;
    let env: reins_proto::managed::Envelope =
        serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(e.to_string()))?;
    Ok(env.parse().ok())
}

/// A request's status for the connection that asked (03 §4.1's shape while open); `NotFound` for anyone else.
pub fn status(c: &mut DbConnInner, org: &str, connection: &str, id: &str, now: i64) -> Result<Value, OrgError> {
    let row = reins_org_request::q_request(c, org, id)?.ok_or(OrgError::NotFound)?;
    if !crate::crypto::ct_eq(&row.connection_uuid, connection) {
        return Err(OrgError::NotFound);
    }
    let trace: Value = serde_json::from_str(&row.trace).unwrap_or_default();
    let summary =
        trace["lines"].as_array().and_then(|l| l.first()).and_then(Value::as_str).unwrap_or(&row.tool).to_owned();
    Ok(match row.state.as_str() {
        st::REVIEWING => json!({
            "status": "pending_review", "ticket": row.uuid, "summary": summary,
            "expected_by": rfc3339(row.expires_at.max(now)),
            "next": "Continue with other work. Call reins_get_result with this ticket later."}),
        st::APPROVED | st::EXECUTING => json!({
            "status": "approved", "ticket": row.uuid, "summary": summary,
            "next": "Approved; your organization's executor is running it. Call reins_get_result again in a few seconds."}),
        st::DONE => {
            let r = receipt_of(c, org, &row)?;
            json!({"status": "done", "ticket": row.uuid, "result": r.map_or(Value::Null, |r| r.result)})
        }
        st::DENIED => json!({"status": "denied", "ticket": row.uuid,
                             "reason": trace["explain"].as_str().filter(|s| !s.is_empty()).unwrap_or("Not approved.")}),
        other => {
            let reason = receipt_of(c, org, &row)?.map(|r| r.reason).unwrap_or_default();
            let words = match other {
                st::EXPIRED => "Expired: not approved in time.",
                st::CANCELLED => "Cancelled.",
                st::STALE => "Your organization's policy changed since it was approved. Ask again.",
                st::REFUSED => "Your organization's executor refused to run it.",
                st::UNKNOWN => "It may or may not have run. Check before trying again.",
                _ => "It failed.",
            };
            json!({"status": other, "ticket": row.uuid, "reason": if reason.is_empty() { words.to_owned() } else { format!("{words} {reason}") }})
        }
    })
}

/// Whether a status is final (the agent has its answer).
#[must_use]
pub fn is_final(status: &Value) -> bool {
    !matches!(status["status"].as_str(), Some("pending_review" | "approved"))
}

static WAITERS: LazyLock<Mutex<HashMap<String, watch::Sender<u64>>>> = LazyLock::new(Mutex::default);

/// Tells whoever waits on `id` that it changed.
pub fn wake(id: &str) {
    let map = WAITERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(tx) = map.get(id) {
        tx.send_modify(|n| *n += 1);
    }
}

/// Waits up to `max` for `id` to change (or one second at most, so callers re-read the database).
pub async fn wait_change(id: &str, max: Duration) {
    let mut rx = {
        let mut map = WAITERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, tx| tx.receiver_count() > 0);
        map.entry(id.to_owned()).or_insert_with(|| watch::channel(0).0).subscribe()
    };
    let _ = tokio::time::timeout(max.min(Duration::from_secs(1)), rx.changed()).await;
}

/// Expires a request under review that nobody approved in time (`why`: what ran out).
pub fn expire(c: &mut DbConnInner, org: &str, row: &ReinsOrgRequest, why: &str, now: i64) -> Result<bool, OrgError> {
    let moved = end(c, org, row, st::EXPIRED, now)?;
    if moved {
        audit::record(
            c,
            org,
            audit::kinds::REVIEW_EXPIRED,
            None,
            Some(&row.uuid),
            &json!({"on_expiry": "deny", "why": why}),
            now,
        )?;
    }
    Ok(moved)
}

/// The sweeper's part: expires overdue reviews and lapsed approvals, and marks stuck executions unknown.
#[cfg(test)]
pub fn sweep(c: &mut DbConnInner, now: i64) -> Result<usize, OrgError> {
    Ok(sweep_routed(c, now, &std::collections::BTreeMap::new())?.0)
}

/// [`sweep`], escalating first (O3) with what the dynamic sources said for each request; returns the escalations to
/// notify. Every part runs whatever another did, and every row on its own: a row that fails is logged and skipped,
/// never stopping deadline expiry for the others.
pub fn sweep_routed(
    c: &mut DbConnInner,
    now: i64,
    prefetched: &routing::escalation::Prefetched,
) -> Result<(usize, Vec<routing::escalation::Swept>), OrgError> {
    use diesel::Connection;
    let swept = routing::escalation::escalate_due(c, now, prefetched).unwrap_or_else(|e| {
        warn!("Org sweep: escalation: {e:?}");
        Vec::new()
    });
    let mut n = swept.len();
    let mut each = |c: &mut DbConnInner,
                    what: &str,
                    row: &ReinsOrgRequest,
                    f: &dyn Fn(&mut DbConnInner) -> Result<bool, OrgError>| {
        match c.transaction(|c| f(c)) {
            Ok(true) => n += 1,
            Ok(false) => {}
            Err(e) => warn!("Org {}: {what} of {} failed: {e:?}", row.org_uuid, row.uuid),
        }
    };
    for row in listed(reins_org_request::q_overdue_reviews(c, now), "q_overdue_reviews") {
        each(c, "expiry", &row, &|c| expire(c, &row.org_uuid, &row, "window", now));
    }
    for row in listed(reins_org_request::q_lapsed_approvals(c, now), "q_lapsed_approvals") {
        each(c, "lapse", &row, &|c| {
            let moved = end(c, &row.org_uuid, &row, st::EXPIRED, now)?;
            if moved {
                audit::record(c, &row.org_uuid, audit::kinds::REQUEST_EXPIRED, None, Some(&row.uuid), &json!({}), now)?;
            }
            Ok(moved)
        });
    }
    for row in listed(reins_org_request::q_stuck(c, now - STUCK_SECS), "q_stuck") {
        each(c, "stuck check", &row, &|c| {
            if !reins_org_request::q_set_state(c, &row.org_uuid, &row.uuid, &[st::EXECUTING], st::UNKNOWN, now)? {
                return Ok(false);
            }
            let keep = retention_secs(c, &row.org_uuid)?;
            reins_org_request::q_set_purge(c, &row.org_uuid, &row.uuid, now + keep)?;
            audit::record(
                c,
                &row.org_uuid,
                audit::kinds::REQUEST_UNKNOWN,
                None,
                Some(&row.uuid),
                &json!({"executor": row.claimed_by}),
                now,
            )?;
            wake(&row.uuid);
            Ok(true)
        });
    }
    Ok((n, swept))
}

/// Rows a sweep part works on; a query that fails is logged and the part skipped this pass.
fn listed(rows: diesel::QueryResult<Vec<ReinsOrgRequest>>, what: &str) -> Vec<ReinsOrgRequest> {
    rows.unwrap_or_else(|e| {
        warn!("Org sweep: {what}: {e:?}");
        Vec::new()
    })
}

/// A member is gone (deprovisioning): their requests no executor took yet are cancelled, and their answers to reviews
/// still open no longer count (the reviews route to the remaining reviewers).
pub fn on_member_removed(c: &mut DbConnInner, org: &str, user: &str, now: i64) -> Result<usize, OrgError> {
    let mut n = 0;
    for row in reins_org_request::q_cancellable_of_user(c, org, user)? {
        if end(c, org, &row, st::CANCELLED, now)? {
            audit::record(
                c,
                org,
                audit::kinds::REQUEST_CANCELLED,
                None,
                Some(&row.uuid),
                &json!({"reason": "member_removed"}),
                now,
            )?;
            n += 1;
        }
    }
    reins_org_request::q_drop_open_approvals(c, org, user)?;
    Ok(n)
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use reins_audit::verify_envelope;
    use reins_proto::org::Role;

    use super::*;
    use crate::api::reins::org::{
        admin,
        kms::tests::{trusted, use_test_kms},
        policy,
        testing::{add_group, add_org, db, member},
    };
    use crate::db::models::reins_org_event;

    /// An org `o1` with Dana (Security admin), Kim (Security admin, group `sec`) and Lee (member), and the Developer
    /// baseline plus `prod-review` (writes on `acme/prod-*` reviewed by `sec`) and `prod-deny` (`acme/forbidden`)
    /// active, with `acme` and `acme.com` as Work.
    pub fn pilot(c: &mut DbConnInner) -> String {
        use_test_kms();
        add_org(c, "o1");
        member(c, "o1", "dana", &[Role::SecurityAdmin]);
        let kim = member(c, "o1", "kim", &[Role::SecurityAdmin]);
        member(c, "o1", "lee", &[]);
        add_group(c, "o1", "sec", &[&kim]);
        let body = policy::DEVELOPER_BASELINE.replace(
            "\"rules\": []",
            r#""rules": [
              {"id": "prod-review", "issuer": "owner", "when": {"resource": "acme/prod-*"}, "then": "review",
               "explain": "Production needs Security", "review": {"reviewers": {"group": "sec"}, "class": "deferred"}},
              {"id": "prod-deny", "issuer": "owner", "when": {"resource": "acme/forbidden"}, "then": "deny",
               "explain": "Not through agents"}]"#,
        );
        let mut p: reins_policy::org::OrgPolicy = serde_json::from_str(&body).unwrap();
        p.facts.set(reins_policy::context::FactKind::Owner, "acme", reins_policy::context::Zone::Work);
        let (revision, hash) = policy::save_draft(c, "o1", "dana", &serde_json::to_string(&p).unwrap(), 10).unwrap();
        let action = reins_proto::org::AdminAction::ActivatePolicy {
            revision,
            hash: hash.clone(),
        };
        let (admin::Proposed::Pending(ch), _) = admin::propose(c, "o1", "dana", &action, None, 11).unwrap() else {
            panic!("pending")
        };
        admin::approve(c, "o1", &ch, "kim", None, 12).unwrap();
        hash
    }

    pub fn issue(repo: &str) -> Value {
        json!({"repo": repo, "title": "Flaky test"})
    }

    pub fn ask(c: &mut DbConnInner, args: &Value, now: i64) -> Submitted {
        submit(c, "o1", "lee", "conn-lee", "Laptop · Claude Code", "github_create_issue", args, None, now).unwrap()
    }

    fn kinds(c: &mut DbConnInner) -> Vec<String> {
        let mut e = reins_org_event::q_events(c, "o1", None, 500).unwrap();
        e.reverse();
        e.into_iter().map(|e| e.kind).filter(|k| k != "key.created").collect()
    }

    #[test]
    fn an_auto_request_is_countersigned_and_queued() {
        let mut c = db();
        let snapshot = pilot(&mut c);
        let s = ask(&mut c, &issue("acme/api"), 100);
        assert_eq!((s.outcome, s.state.as_str()), (ManagedOutcome::Auto, st::APPROVED));
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let env: reins_proto::managed::Envelope = serde_json::from_str(row.decision.as_deref().unwrap()).unwrap();
        let dec = kms::signing_key(&mut c, "o1", KeyPurpose::Decision, 1).unwrap();
        assert!(verify_envelope(&env, &[trusted(&dec)]));
        let record: DecisionRecord = env.parse().unwrap();
        assert_eq!((record.verdict, record.snapshot.as_str()), (Verdict::Auto, snapshot.as_str()));
        assert_eq!(record.digest, request_digest("o1", "lee", "conn-lee", "github_create_issue", &issue("acme/api")));
        assert_eq!(reins_org_request::q_claimable(&mut c, "o1", 101, 10).unwrap().len(), 1);
        assert!(kinds(&mut c).ends_with(&["request.received".to_owned(), "decision.auto".to_owned()]));
    }

    #[test]
    fn a_deny_tells_only_the_explanation() {
        let mut c = db();
        pilot(&mut c);
        let s = ask(&mut c, &issue("acme/forbidden"), 100);
        assert_eq!((s.outcome, s.explain.as_str()), (ManagedOutcome::Deny, "Not through agents"));
        let status = status(&mut c, "o1", "conn-lee", &s.id, 101).unwrap();
        assert_eq!(status, json!({"status": "denied", "ticket": s.id, "reason": "Not through agents"}));
        assert!(reins_org_request::q_claimable(&mut c, "o1", 101, 10).unwrap().is_empty());
    }

    #[test]
    fn a_review_waits_for_its_obligations() {
        let mut c = db();
        pilot(&mut c);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        assert_eq!((s.outcome, s.state.as_str(), s.instant), (ManagedOutcome::Review, st::REVIEWING, false));
        assert_eq!(s.expires_at, 100 + 4 * 3600);
        let st = status(&mut c, "o1", "conn-lee", &s.id, 101).unwrap();
        assert_eq!(st["status"], "pending_review");
        assert!(reins_org_request::q_claimable(&mut c, "o1", 101, 10).unwrap().is_empty());
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap();
        assert_eq!(obligations[0].id, "prod-review");
    }

    fn lease(id: &str, repo: &str, access: reins_proto::managed::LeaseAccess) -> LeaseAsk {
        LeaseAsk {
            id: id.to_owned(),
            repo: repo.to_owned(),
            access,
            box_key: "BOXKEY".to_owned(),
        }
    }

    #[test]
    fn a_lease_is_judged_by_what_it_mints() {
        use reins_proto::managed::{LEASE_TOOL, LeaseAccess, lease_args};
        let mut c = db();
        pilot(&mut c);
        let ask_lease = |c: &mut DbConnInner, l: &LeaseAsk, args: &Value| {
            submit(c, "o1", "lee", "conn-lee", "Laptop", LEASE_TOOL, args, Some(l), 100)
        };
        // Harmless arguments with a lease for another repository, access or recipient: refused.
        let prod = lease("l1", "acme/forbidden", LeaseAccess::Push);
        for args in [
            lease_args("acme/api", LeaseAccess::Push, "BOXKEY"),
            lease_args("acme/forbidden", LeaseAccess::Read, "BOXKEY"),
            lease_args("acme/forbidden", LeaseAccess::Push, "OTHER"),
            json!({"repo": "acme/forbidden", "access": "push"}),
        ] {
            assert!(ask_lease(&mut c, &prod, &args).is_err(), "{args}");
        }
        // A lease without its lease part, or a call named as a lease: refused.
        assert!(
            submit(
                &mut c,
                "o1",
                "lee",
                "conn-lee",
                "x",
                LEASE_TOOL,
                &lease_args("acme/api", LeaseAccess::Read, "B"),
                None,
                100
            )
            .is_err()
        );
        assert!(
            submit(&mut c, "o1", "lee", "conn-lee", "x", "github_create_issue", &issue("acme/api"), Some(&prod), 100)
                .is_err()
        );
        // The org's deny on the lease's repository blocks it.
        let s = ask_lease(&mut c, &prod, &lease_args("acme/forbidden", LeaseAccess::Push, "BOXKEY")).unwrap();
        assert_eq!(s.outcome, ManagedOutcome::Deny);
        let l = reins_org_request::q_lease(&mut c, "o1", "l1").unwrap().unwrap();
        assert_eq!(l.state, lease_state::DENIED);
        // An Auto lease binds what it mints, and its digest covers the box key.
        let ok = lease("l2", "acme/api", LeaseAccess::Push);
        let args = lease_args("acme/api", LeaseAccess::Push, "BOXKEY");
        let s = ask_lease(&mut c, &ok, &args).unwrap();
        assert_eq!(s.outcome, ManagedOutcome::Auto);
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        assert_eq!(row.digest, request_digest("o1", "lee", "conn-lee", LEASE_TOOL, &args));
        let record: DecisionRecord =
            serde_json::from_str::<reins_proto::managed::Envelope>(row.decision.as_deref().unwrap())
                .unwrap()
                .parse()
                .unwrap();
        let bound = record.lease.unwrap();
        assert_eq!((bound.repo.as_str(), bound.access, bound.lease.as_str()), ("acme/api", LeaseAccess::Push, "l2"));
    }

    #[test]
    fn no_active_policy_fails_closed() {
        let mut c = db();
        use_test_kms();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "lee", &[]);
        let r = submit(&mut c, "o1", "lee", "conn", "x", "github_create_issue", &issue("acme/api"), None, 1);
        assert_eq!(r, Err(OrgError::Invalid("Your organization hasn't activated a policy yet.")));
    }

    #[test]
    fn every_decision_records_snapshot_and_trace() {
        let mut c = db();
        let snapshot = pilot(&mut c);
        let s = ask(&mut c, &issue("acme/api"), 100);
        let events = reins_org_event::q_events(&mut c, "o1", None, 10).unwrap();
        let received =
            events.iter().find(|e| e.kind == "request.received" && e.subject.as_deref() == Some(&s.id)).unwrap();
        let d: Value = serde_json::from_str(&received.detail).unwrap();
        assert_eq!(d["policy"]["snapshot"], snapshot);
        assert_eq!(d["request"]["zone"], "work");
        assert!(d["policy"]["matched"].is_array());
        assert_eq!(d["request"]["member"], "lee@member.example");
    }

    #[test]
    fn status_is_visible_only_to_the_asking_connection() {
        let mut c = db();
        pilot(&mut c);
        let s = ask(&mut c, &issue("acme/api"), 100);
        assert_eq!(status(&mut c, "o1", "conn-other", &s.id, 101), Err(OrgError::NotFound));
        assert_eq!(status(&mut c, "o2", "conn-lee", &s.id, 101), Err(OrgError::NotFound));
    }

    #[test]
    fn expired_reviews_expire_and_stuck_executions_become_unknown() {
        let mut c = db();
        pilot(&mut c);
        let review = ask(&mut c, &issue("acme/prod-db"), 100);
        let auto = ask(&mut c, &issue("acme/api"), 100);
        assert!(reins_org_request::q_claim(&mut c, "o1", &auto.id, "e1", 101).unwrap());
        sweep(&mut c, 100 + 4 * 3600).unwrap();
        assert_eq!(reins_org_request::q_request(&mut c, "o1", &review.id).unwrap().unwrap().state, st::EXPIRED);
        assert_eq!(reins_org_request::q_request(&mut c, "o1", &auto.id).unwrap().unwrap().state, st::UNKNOWN);
        // Never claimable again.
        assert!(reins_org_request::q_claimable(&mut c, "o1", 100 + 4 * 3600, 10).unwrap().is_empty());
    }

    #[test]
    fn a_removed_members_requests_are_cancelled() {
        let mut c = db();
        pilot(&mut c);
        let review = ask(&mut c, &issue("acme/prod-db"), 100);
        let auto = ask(&mut c, &issue("acme/api"), 100);
        assert_eq!(on_member_removed(&mut c, "o1", "lee", 110).unwrap(), 2);
        for id in [review.id, auto.id] {
            assert_eq!(reins_org_request::q_request(&mut c, "o1", &id).unwrap().unwrap().state, st::CANCELLED);
        }
        assert!(reins_org_request::q_claimable(&mut c, "o1", 111, 10).unwrap().is_empty());
    }
}
