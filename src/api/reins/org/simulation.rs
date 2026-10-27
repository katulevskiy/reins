//! Simulation of a draft on the org's retained history (`docs/design/decisions-v2/02-policy-and-authority.md` §4,
//! `05-organizations.md` §7.3): every decision kept its body-free input ([`SimInput`], decision metadata), and
//! [`run`] replays the draft and the active policy over it with the evaluator the live path runs
//! ([`reins_policy::simulate::simulate_org`]). The report is stored by its hash: it is the evidence an activation
//! names ([`super::activation`]), and only for the draft hash it was made for.
//!
//! The report names request ids only; opening one is an audited read ([`sample`]).

use std::collections::BTreeMap;

use reins_policy::{
    org::{ManagedOutcome, MemberPolicy, OrgPolicy, SimInput},
    simulate::{Retained, SimulationReport, simulate_org},
};
use reins_proto::org::Capability;
use serde_json::json;

use super::{OrgError, audit, require};
use crate::db::{
    DbConnInner,
    models::{
        reins_org, reins_org_keys,
        reins_org_policy_code::{self as code, ReinsOrgSimulation},
        reins_org_request::ReinsOrgRequest,
    },
};

/// History simulated by default, and at most.
pub const DEFAULT_DAYS: i64 = 30;
pub const MAX_DAYS: i64 = 90;
/// Cases replayed at most (the newest are kept).
pub const MAX_CASES: i64 = 100_000;
/// Who runs a simulation for the policy repo.
pub const SYSTEM_ACTOR: &str = "system:policy-repo";

/// Each active member's group names, by user id.
pub fn group_names(c: &mut DbConnInner, org: &str) -> Result<BTreeMap<String, Vec<String>>, OrgError> {
    let names: BTreeMap<String, String> =
        reins_org::q_groups(c, org)?.into_iter().map(|(id, name, _)| (id, name)).collect();
    let mut out = BTreeMap::new();
    for m in reins_org::q_members(c, org)? {
        let mut groups: Vec<String> =
            reins_org::q_groups_of(c, m.uuid.as_ref())?.iter().filter_map(|g| names.get(g).cloned()).collect();
        groups.sort();
        out.insert(m.user_uuid.to_string(), groups);
    }
    Ok(out)
}

/// The members' current own policies, by user id.
pub fn member_policies(c: &mut DbConnInner, org: &str) -> Result<BTreeMap<String, MemberPolicy>, OrgError> {
    let mut out = BTreeMap::new();
    for m in reins_org::q_members(c, org)? {
        let user = m.user_uuid.to_string();
        if let Some(row) = reins_org_keys::q_member_policy(c, org, &user)?
            && let Ok(p) = serde_json::from_str::<MemberPolicy>(&row.body)
        {
            out.insert(user, p);
        }
    }
    Ok(out)
}

/// One stored request as a retained case; `None` when it kept no analysis (decided before O4).
pub fn retained(row: &ReinsOrgRequest, groups: &BTreeMap<String, Vec<String>>) -> Option<Retained> {
    let input: SimInput = serde_json::from_str(row.analysis.as_deref()?).ok()?;
    let outcome = match row.outcome.as_str() {
        "auto" => ManagedOutcome::Auto,
        "review" => ManagedOutcome::Review,
        "deny" => ManagedOutcome::Deny,
        _ => return None,
    };
    Some(Retained {
        id: row.uuid.clone(),
        input,
        groups: groups.get(&row.user_uuid).cloned().unwrap_or_default(),
        outcome,
        snapshot: row.snapshot.clone(),
        member_policy: row.member_policy.clone(),
    })
}

/// The retained cases of `[since, until)`, and how many kept no analysis.
pub fn history(c: &mut DbConnInner, org: &str, since: i64, until: i64) -> Result<(Vec<Retained>, u32), OrgError> {
    let groups = group_names(c, org)?;
    let rows = code::q_history(c, org, since, until, MAX_CASES)?;
    let mut skipped = 0u32;
    let mut cases = Vec::with_capacity(rows.len());
    for row in &rows {
        match retained(row, &groups) {
            Some(r) => cases.push(r),
            None => skipped += 1,
        }
    }
    Ok((cases, skipped))
}

/// A stored policy revision, parsed.
pub fn revision_policy(c: &mut DbConnInner, org: &str, revision: u64) -> Result<(OrgPolicy, String), OrgError> {
    let rev = i64::try_from(revision).map_err(|_| OrgError::NotFound)?;
    let row = reins_org_keys::q_policy(c, org, rev)?.ok_or(OrgError::NotFound)?;
    let p: OrgPolicy = serde_json::from_str(&row.body).map_err(|e| OrgError::Internal(format!("stored draft: {e}")))?;
    Ok((p, row.hash))
}

/// Simulates draft `revision` on `[since, until)` (30 days by default, 90 at most) against the active policy, as
/// `actor` (who needs [`Capability::AuthorPolicy`]; [`SYSTEM_ACTOR`] for the policy repo); stores and records the
/// report.
#[allow(clippy::too_many_arguments, reason = "one simulation's parts")]
pub fn run(
    c: &mut DbConnInner,
    org: &str,
    actor: &str,
    revision: u64,
    since: Option<i64>,
    until: Option<i64>,
    source_ref: Option<&str>,
    now: i64,
) -> Result<SimulationReport, OrgError> {
    if actor != SYSTEM_ACTOR {
        require(c, org, actor, Capability::AuthorPolicy)?;
    }
    let until = until.unwrap_or(now).min(now);
    let since = since.unwrap_or(until - DEFAULT_DAYS * 86_400).max(until - MAX_DAYS * 86_400);
    let (draft, draft_hash) = revision_policy(c, org, revision)?;
    if draft.hash() != draft_hash {
        return Err(OrgError::Internal(format!("draft {revision} of {org} doesn't hash as stored")));
    }
    let base = super::policy::active(c, org)?.map_or_else(|| OrgPolicy::empty(org), |(p, _)| p);
    let (cases, skipped) = history(c, org, since, until)?;
    let mut report = simulate_org(&cases, &member_policies(c, org)?, &base, &draft);
    report.skipped = skipped;
    if report.cases == 0 {
        report.since = since;
        report.until = until;
    }
    let hash = report.hash();
    code::q_insert_simulation(
        c,
        &ReinsOrgSimulation {
            org_uuid: org.to_owned(),
            report_hash: hash.clone(),
            revision: i64::try_from(revision).unwrap_or(0),
            draft_hash: draft_hash.clone(),
            base_hash: report.base_hash.clone(),
            source_ref: source_ref.map(str::to_owned),
            report: serde_json::to_string(&report).map_err(|e| OrgError::Internal(e.to_string()))?,
            ran_at: now,
        },
    )?;
    audit::record(
        c,
        org,
        audit::kinds::POLICY_SIMULATED,
        Some(actor),
        None,
        &json!({"revision": revision, "hash": draft_hash, "report": hash, "line": report.line(),
                "cases": report.cases, "drift": report.drift, "source": source_ref}),
        now,
    )?;
    Ok(report)
}

/// A stored report and when it ran.
pub fn report(
    c: &mut DbConnInner,
    org: &str,
    hash: &str,
) -> Result<Option<(SimulationReport, ReinsOrgSimulation)>, OrgError> {
    match code::q_simulation(c, org, hash)? {
        Some(row) => {
            let r: SimulationReport =
                serde_json::from_str(&row.report).map_err(|e| OrgError::Internal(format!("stored report: {e}")))?;
            Ok(Some((r, row)))
        }
        None => Ok(None),
    }
}

/// One retained case, opened by an Auditor or a Security admin: the read is recorded before anything is returned.
pub fn sample(c: &mut DbConnInner, org: &str, viewer: &str, request: &str, now: i64) -> Result<Retained, OrgError> {
    let auditor = super::roles_of_user(c, org, viewer)?
        .iter()
        .any(|r| matches!(r, reins_proto::org::Role::Auditor | reins_proto::org::Role::SecurityAdmin));
    if !auditor {
        return Err(OrgError::Forbidden);
    }
    let row = crate::db::models::reins_org_request::q_request(c, org, request)?.ok_or(OrgError::NotFound)?;
    audit::record(
        c,
        org,
        audit::kinds::AUDIT_READ,
        Some(viewer),
        Some(request),
        &json!({"request": request, "part": "analysis"}),
        now,
    )?;
    retained(&row, &group_names(c, org)?).ok_or(OrgError::NotFound)
}

/// The retained history of `[since, until)` as `Retained` JSON lines, for `reins-replay policy simulate`: needs
/// [`Capability::ViewAdminEvents`], and is audited.
pub fn export(
    c: &mut DbConnInner,
    org: &str,
    viewer: &str,
    since: i64,
    until: i64,
    now: i64,
) -> Result<String, OrgError> {
    require(c, org, viewer, Capability::ViewAdminEvents)?;
    let (cases, _) = history(c, org, since, until)?;
    audit::record(
        c,
        org,
        audit::kinds::AUDIT_READ,
        Some(viewer),
        None,
        &json!({"history": {"since": since, "until": until, "cases": cases.len()}}),
        now,
    )?;
    let mut out = String::new();
    for case in cases {
        out.push_str(&serde_json::to_string(&case).map_err(|e| OrgError::Internal(e.to_string()))?);
        out.push('\n');
    }
    Ok(out)
}
