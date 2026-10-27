//! The org executors' API (`/reins/org-exec`, the contract in `docs/superpowers/plans/2026-10-26-dv2-o2.md`): an
//! executor claims approved requests of its own tenant, fetches the issuer-signed policy and roster it verifies them
//! against, and returns signed receipts (`docs/design/decisions-v2/05` §5, `08` R5, R12).
//!
//! Every call is signed with the executor's Ed25519 key over (org, executor, method, path, time, SHA-256 of the body);
//! an executor registered (by a two-person, device-signed admin change) to one org reaches only that org's rows.

use reins_proto::managed::{
    DecisionRecord, Envelope, EnvelopeKind, Job, JobKind, LeaseJob, Receipt, ReceiptStatus, SignedMemberPolicy,
    exec_request_message,
};
use rocket::{
    Data, Request, Route,
    http::Status,
    request::{FromRequest, Outcome},
    serde::json::Json,
};
use serde_json::{Value, json};

use super::{OrgError, audit, decisions, enabled, kms, policy, rollout, roster};
use crate::{
    api::reins::{device_api::read_body_limited, now_unix},
    db::{
        DbConn, DbConnInner,
        models::{
            reins_org_exec, reins_org_keys,
            reins_org_request::{self, ReinsOrgBody, ReinsOrgRequest, request_kind, request_state as st},
        },
    },
};

/// Largest body an executor sends (a receipt with a read's result).
const MAX_BODY: u64 = 2 * 1024 * 1024;
/// How far an executor's clock may be from the server's.
pub const SKEW_SECS: i64 = 120;
/// Jobs handed out per claim.
const CLAIM: i64 = 10;

pub fn routes() -> Vec<Route> {
    routes![claim, keys, policy_by_hash, get_roster, get_rollouts, receipt, revocations, revoked]
}

/// The signature headers and the path as the executor signed it.
pub struct ExecHeaders {
    executor: Option<String>,
    at: Option<String>,
    sig: Option<String>,
    path: String,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ExecHeaders {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let h = |n: &str| request.headers().get_one(n).map(str::to_owned);
        Outcome::Success(Self {
            executor: h("X-Reins-Executor"),
            at: h("X-Reins-Time"),
            sig: h("X-Reins-Signature"),
            path: request.uri().to_string(),
        })
    }
}

/// An authenticated executor.
pub struct Executor {
    pub org: String,
    pub id: String,
}

/// Whether the signature headers prove `executor` (registered, not removed, key matches) for this call. Pure over
/// the stored executor row.
pub fn check_signature(
    row: &reins_org_exec::ReinsOrgExecutor,
    method: &str,
    path: &str,
    at: i64,
    sig: &str,
    body: &[u8],
    now: i64,
) -> bool {
    if row.revoked_at.is_some() || (now - at).abs() > SKEW_SECS {
        return false;
    }
    let (Some(key), Some(sig)) = (super::admin::executor_key(&row.public_key), reins_proto::canon::parse_sig(sig))
    else {
        return false;
    };
    let msg = exec_request_message(&row.org_uuid, &row.executor_id, method, path, at, body);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(&msg, &sig).is_ok()
}

async fn authenticate(h: &ExecHeaders, method: &str, body: &[u8], conn: &DbConn) -> Result<Executor, Status> {
    if !enabled() {
        return Err(Status::NotFound);
    }
    let (Some(who), Some(at), Some(sig)) = (&h.executor, &h.at, &h.sig) else {
        return Err(Status::Unauthorized);
    };
    let (org, id) = who.split_once('/').ok_or(Status::Unauthorized)?;
    let at: i64 = at.parse().map_err(|_| Status::Unauthorized)?;
    let (o, i) = (org.to_owned(), id.to_owned());
    let row = conn
        .run(move |c| reins_org_exec::q_executor(c, &o, &i))
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::Unauthorized)?;
    let now = now_unix();
    if !check_signature(&row, method, &h.path, at, sig, body, now) {
        return Err(Status::Unauthorized);
    }
    let (o, i) = (org.to_owned(), id.to_owned());
    conn.run(move |c| reins_org_exec::q_touch_executor(c, &o, &i, now)).await.ok();
    Ok(Executor {
        org: org.to_owned(),
        id: id.to_owned(),
    })
}

fn internal(e: &OrgError) -> Status {
    warn!("Executor API: {e:?}");
    Status::InternalServerError
}

/// The job an executor runs for `row`.
fn job_of(c: &mut DbConnInner, org: &str, row: &ReinsOrgRequest, current: &str) -> Result<Job, OrgError> {
    let body = reins_org_request::q_body(c, org, &row.uuid, "request")?.ok_or(OrgError::NotFound)?;
    let plain = kms::open_data(c, org, &body.key_id, &format!("{}/request", row.uuid), &body.sealed)?;
    let call: Value = serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(e.to_string()))?;
    let decision: Envelope = serde_json::from_str(row.decision.as_deref().unwrap_or_default())
        .map_err(|e| OrgError::Internal(format!("stored decision: {e}")))?;
    let record: DecisionRecord = decision.parse().map_err(|e| OrgError::Internal(format!("stored decision: {e}")))?;
    let member_policy = match &row.member_policy {
        Some(_) => reins_org_keys::q_member_policy(c, org, &row.user_uuid)?.map(|m| SignedMemberPolicy {
            body: m.body,
            version: u64::try_from(m.version).unwrap_or(0),
            at: m.at,
            device: m.device_uuid,
            sig: m.sig,
        }),
        None => None,
    };
    let lease = if row.kind == request_kind::LEASE {
        let l = reins_org_request::q_lease_of_request(c, org, &row.uuid)?.ok_or(OrgError::NotFound)?;
        // A lease that ended (revoked, expired) is never minted.
        if l.state != reins_org_request::lease_state::PENDING {
            return Err(OrgError::Invalid("The lease ended before an executor took it."));
        }
        let bound = record.lease.as_ref().ok_or(OrgError::Internal("a lease decision without its lease".into()))?;
        Some(LeaseJob {
            lease: l.uuid,
            repo: l.repo,
            access: bound.access,
            box_key: l.box_key,
            expires_at: bound.expires_at,
        })
    } else {
        None
    };
    Ok(Job {
        request: row.uuid.clone(),
        kind: if lease.is_some() {
            JobKind::Lease
        } else {
            JobKind::Execute
        },
        tool: call["tool"].as_str().unwrap_or_default().to_owned(),
        args: call["args"].clone(),
        decision,
        current_policy: current.to_owned(),
        member_policy,
        lease,
    })
}

/// Claims up to [`CLAIM`] approved requests of `org` for `executor`. Each job's current policy is its member's
/// effective version now (a rollout's candidate for its cohort, else the active policy).
pub fn q_claim_jobs(c: &mut DbConnInner, org: &str, executor: &str, now: i64) -> Result<Vec<Job>, OrgError> {
    let active = policy::active(c, org)?.map(|(p, _)| p.hash()).unwrap_or_default();
    let steps: Vec<reins_proto::rollout::Rollout> =
        rollout::published_for_org(c, org, now)?.iter().filter_map(|e| e.parse().ok()).collect();
    let mut jobs = Vec::new();
    for row in reins_org_request::q_claimable(c, org, now, CLAIM)? {
        if !reins_org_request::q_claim(c, org, &row.uuid, executor, now)? {
            continue;
        }
        let current = reins_proto::rollout::effective(&steps, &row.user_uuid, &active, now);
        match job_of(c, org, &row, &current) {
            Ok(job) => jobs.push(job),
            Err(e) => {
                // It can't be handed out as decided: it ends, never runs on a guess.
                warn!("Org {org}: request {} can't be handed to an executor: {e:?}", row.uuid);
                reins_org_request::q_set_state(c, org, &row.uuid, &[st::EXECUTING], st::FAILED, now)?;
            }
        }
    }
    Ok(jobs)
}

#[post("/reins/org-exec/claim", data = "<data>")]
async fn claim(h: ExecHeaders, data: Data<'_>, conn: DbConn) -> Result<Json<Vec<Job>>, Status> {
    let body = read_body_limited(data, MAX_BODY).await.map_err(|_| Status::PayloadTooLarge)?;
    let ex = authenticate(&h, "POST", &body, &conn).await?;
    let now = now_unix();
    conn.run(move |c| q_claim_jobs(c, &ex.org, &ex.id, now)).await.map(Json).map_err(|e| internal(&e))
}

#[get("/reins/org-exec/keys")]
async fn keys(h: ExecHeaders, conn: DbConn) -> Result<Json<kms::OrgKeys>, Status> {
    let ex = authenticate(&h, "GET", b"", &conn).await?;
    let now = now_unix();
    conn.run(move |c| kms::public_keys(c, &ex.org, now)).await.map(Json).map_err(|e| internal(&e))
}

#[get("/reins/org-exec/policy/<hash>")]
async fn policy_by_hash(hash: &str, h: ExecHeaders, conn: DbConn) -> Result<Json<Envelope>, Status> {
    let ex = authenticate(&h, "GET", b"", &conn).await?;
    let hash = hash.to_owned();
    conn.run(move |c| policy::by_hash(c, &ex.org, &hash))
        .await
        .map_err(|e| internal(&e))?
        .map(Json)
        .ok_or(Status::NotFound)
}

/// The issuer-signed steps of the org's open and recently finished rollouts (O4): what an executor checks a decision's
/// policy version against.
#[get("/reins/org-exec/rollouts")]
async fn get_rollouts(h: ExecHeaders, conn: DbConn) -> Result<Json<Vec<Envelope>>, Status> {
    let ex = authenticate(&h, "GET", b"", &conn).await?;
    let now = now_unix();
    conn.run(move |c| rollout::published_for_org(c, &ex.org, now)).await.map(Json).map_err(|e| internal(&e))
}

#[get("/reins/org-exec/roster")]
async fn get_roster(h: ExecHeaders, conn: DbConn) -> Result<Json<Envelope>, Status> {
    let ex = authenticate(&h, "GET", b"", &conn).await?;
    let now = now_unix();
    conn.run(move |c| roster::signed(c, &ex.org, now)).await.map(Json).map_err(|e| internal(&e))
}

/// Records an executor's signed receipt for a request it claimed.
pub fn q_receipt(c: &mut DbConnInner, ex: &Executor, env: &Envelope, now: i64) -> Result<(), OrgError> {
    use diesel::Connection;
    let row_key = reins_org_exec::q_executor(c, &ex.org, &ex.id)?.ok_or(OrgError::Forbidden)?;
    let key = super::admin::executor_key(&row_key.public_key).ok_or(OrgError::Forbidden)?;
    let trusted = reins_audit::TrustedKey {
        key_id: ex.id.clone(),
        public: key,
    };
    if env.kind != EnvelopeKind::Receipt || env.org != ex.org || !reins_audit::verify_envelope(env, &[trusted]) {
        return Err(OrgError::Invalid("The receipt isn't signed by this executor."));
    }
    let r: Receipt = env.parse().map_err(|_| OrgError::Invalid("The receipt can't be read."))?;
    if r.org != ex.org || r.executor != ex.id {
        return Err(OrgError::Invalid("The receipt names another executor."));
    }
    c.transaction(|c| {
        let row = reins_org_request::q_request(c, &ex.org, &r.request)?.ok_or(OrgError::NotFound)?;
        if row.claimed_by.as_deref() != Some(ex.id.as_str()) || row.state != st::EXECUTING || row.digest != r.digest {
            return Err(OrgError::Invalid("That request isn't this executor's to finish."));
        }
        let (to, kind) = match r.status {
            ReceiptStatus::Succeeded => (st::DONE, audit::kinds::REQUEST_EXECUTED),
            ReceiptStatus::Failed => (st::FAILED, audit::kinds::REQUEST_FAILED),
            ReceiptStatus::Refused => (st::REFUSED, audit::kinds::REQUEST_REFUSED),
            ReceiptStatus::Stale => (st::STALE, audit::kinds::REQUEST_STALE),
        };
        let keep =
            i64::from(crate::db::models::reins_org::q_org(c, &ex.org)?.map_or(30, |t| t.body_retention_days)) * 86_400;
        let text = serde_json::to_string(env).unwrap_or_default();
        let (key_id, sealed) = kms::seal_data(c, &ex.org, &format!("{}/receipt", r.request), text.as_bytes(), now)?;
        reins_org_request::q_put_body(
            c,
            &ReinsOrgBody {
                request_uuid: r.request.clone(),
                part: "receipt".to_owned(),
                org_uuid: ex.org.clone(),
                key_id,
                sealed,
                created_at: now,
                purge_at: Some(now + keep),
            },
        )?;
        let minimal =
            json!({"status": r.status, "reason": r.reason, "at": r.at, "executor": r.executor, "sig": env.sig});
        if !reins_org_request::q_finish(c, &ex.org, &r.request, &ex.id, to, &minimal.to_string(), now)? {
            return Err(OrgError::Invalid("That request isn't this executor's to finish."));
        }
        reins_org_request::q_set_purge(c, &ex.org, &r.request, now + keep)?;
        audit::record(
            c,
            &ex.org,
            kind,
            None,
            Some(&r.request),
            &json!({"request": {"id": r.request, "digest": r.digest}, "executor": ex.id, "status": r.status,
                    "reason": r.reason, "receipt_sig": env.sig}),
            now,
        )?;
        if row.kind == request_kind::LEASE
            && let Some(lease) = reins_org_request::q_lease_of_request(c, &ex.org, &r.request)?
        {
            let sealed = r.result.get("sealed").and_then(Value::as_str);
            let expires = r.result.get("expires_at").and_then(Value::as_i64).unwrap_or(lease.expires_at);
            match (r.status, sealed) {
                (ReceiptStatus::Succeeded, Some(sealed)) => {
                    // Never past what the decision bound, and never once that has passed (an executor that was
                    // offline minted it late): then it is not handed out, and its token is revoked upstream.
                    let until = expires.min(lease.expires_at);
                    if until <= now {
                        reins_org_request::q_end_lease(
                            c,
                            &ex.org,
                            &lease.uuid,
                            &[reins_org_request::lease_state::PENDING],
                            reins_org_request::lease_state::EXPIRED,
                            now,
                        )?;
                        reins_org_request::q_mark_issued_after_revocation(c, &ex.org, &lease.uuid, now)?;
                    } else if reins_org_request::q_set_lease_issued(c, &ex.org, &lease.uuid, sealed, until, now)? {
                        audit::record(
                            c,
                            &ex.org,
                            audit::kinds::LEASE_ISSUED,
                            Some(&lease.user_uuid),
                            Some(&lease.uuid),
                            &json!({"repo": lease.repo, "access": lease.access, "expires_at": until,
                                        "request": r.request, "executor": ex.id}),
                            now,
                        )?;
                    } else {
                        // Revoked meanwhile (deprovisioning): the minted credential must be revoked upstream.
                        reins_org_request::q_mark_issued_after_revocation(c, &ex.org, &lease.uuid, now)?;
                    }
                }
                _ => {
                    reins_org_request::q_set_lease_state(
                        c,
                        &ex.org,
                        &lease.uuid,
                        &[reins_org_request::lease_state::PENDING],
                        reins_org_request::lease_state::DENIED,
                    )?;
                }
            }
        }
        decisions::wake(&r.request);
        Ok(())
    })
}

#[post("/reins/org-exec/receipt", data = "<data>")]
async fn receipt(h: ExecHeaders, data: Data<'_>, conn: DbConn) -> Status {
    let Ok(body) = read_body_limited(data, MAX_BODY).await else {
        return Status::PayloadTooLarge;
    };
    let ex = match authenticate(&h, "POST", &body, &conn).await {
        Ok(ex) => ex,
        Err(s) => return s,
    };
    let Ok(env) = serde_json::from_slice::<Envelope>(&body) else {
        return Status::BadRequest;
    };
    let now = now_unix();
    match conn.run(move |c| q_receipt(c, &ex, &env, now)).await {
        Ok(()) => Status::NoContent,
        Err(OrgError::Invalid(m)) => {
            info!("Executor receipt refused: {m}");
            Status::Conflict
        }
        Err(OrgError::NotFound) => Status::NotFound,
        Err(OrgError::Forbidden) => Status::Forbidden,
        Err(e) => internal(&e),
    }
}

/// The leases whose token `ex` must revoke upstream: only those it minted (it claimed their request), since only it
/// holds their tokens.
pub fn q_revocations(c: &mut DbConnInner, ex: &Executor) -> Result<Vec<Value>, OrgError> {
    Ok(reins_org_request::q_leases_to_revoke_by(c, &ex.org, &ex.id)?
        .into_iter()
        .map(|l| json!({"lease": l.uuid, "reason": l.state}))
        .collect())
}

/// `ex` revoked lease `id`'s token upstream; false when it isn't one `ex` minted, or was already recorded.
pub fn q_revoked(c: &mut DbConnInner, ex: &Executor, id: &str, now: i64) -> Result<bool, OrgError> {
    if !reins_org_request::q_mark_revoke_done_by(c, &ex.org, id, &ex.id, now)? {
        return Ok(false);
    }
    audit::record(c, &ex.org, audit::kinds::LEASE_REVOKED_UPSTREAM, None, Some(id), &json!({"executor": ex.id}), now)?;
    Ok(true)
}

#[get("/reins/org-exec/revocations")]
async fn revocations(h: ExecHeaders, conn: DbConn) -> Result<Json<Vec<Value>>, Status> {
    let ex = authenticate(&h, "GET", b"", &conn).await?;
    conn.run(move |c| q_revocations(c, &ex)).await.map(Json).map_err(|e| internal(&e))
}

#[post("/reins/org-exec/revoked/<id>", data = "<data>")]
async fn revoked(id: &str, h: ExecHeaders, data: Data<'_>, conn: DbConn) -> Status {
    let Ok(body) = read_body_limited(data, 1024).await else {
        return Status::PayloadTooLarge;
    };
    let ex = match authenticate(&h, "POST", &body, &conn).await {
        Ok(ex) => ex,
        Err(s) => return s,
    };
    let id = id.to_owned();
    let now = now_unix();
    let done = conn.run(move |c| q_revoked(c, &ex, &id, now)).await;
    match done {
        Ok(true) => Status::NoContent,
        Ok(false) => Status::NotFound,
        Err(e) => internal(&e),
    }
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::{canon::encode_sig, org::ExecutorMode};
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };

    use super::*;
    use crate::api::reins::org::{
        decisions::tests::{ask, issue, pilot},
        testing::db,
    };

    struct Exec {
        key: Ed25519KeyPair,
        id: String,
        org: String,
    }

    impl Exec {
        fn register(c: &mut DbConnInner, org: &str, id: &str) -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
            reins_org_exec::q_insert_executor(
                c,
                &reins_org_exec::ReinsOrgExecutor {
                    org_uuid: org.into(),
                    executor_id: id.into(),
                    mode: ExecutorMode::Cloud.as_str().into(),
                    public_key: reins_proto::canon::encode_payload(key.public_key().as_ref()),
                    created_at: 1,
                    last_seen: None,
                    revoked_at: None,
                },
            )
            .unwrap();
            Self {
                key,
                id: id.into(),
                org: org.into(),
            }
        }

        fn sig(&self, method: &str, path: &str, at: i64, body: &[u8]) -> String {
            let msg = exec_request_message(&self.org, &self.id, method, path, at, body);
            encode_sig(self.key.sign(&msg).as_ref().try_into().unwrap())
        }

        fn me(&self) -> Executor {
            Executor {
                org: self.org.clone(),
                id: self.id.clone(),
            }
        }

        fn receipt(&self, job: &Job, status: ReceiptStatus, result: Value) -> Envelope {
            let record: DecisionRecord = job.decision.parse().unwrap();
            let r = Receipt {
                org: self.org.clone(),
                request: job.request.clone(),
                digest: record.digest,
                executor: self.id.clone(),
                status,
                result,
                reason: String::new(),
                at: 200,
            };
            let body = reins_proto::managed::canonical_json(&serde_json::to_value(&r).unwrap());
            let sig = self.key.sign(&reins_proto::managed::envelope_message(
                EnvelopeKind::Receipt,
                &self.org,
                &self.id,
                &body,
            ));
            Envelope {
                kind: EnvelopeKind::Receipt,
                org: self.org.clone(),
                body,
                key_id: self.id.clone(),
                sig: encode_sig(sig.as_ref().try_into().unwrap()),
            }
        }
    }

    #[test]
    fn an_unsigned_or_stale_request_is_refused() {
        let mut c = db();
        let e = Exec::register(&mut c, "o1", "e1");
        let row = reins_org_exec::q_executor(&mut c, "o1", "e1").unwrap().unwrap();
        let good = e.sig("POST", "/reins/org-exec/claim", 1000, b"{}");
        assert!(check_signature(&row, "POST", "/reins/org-exec/claim", 1000, &good, b"{}", 1000));
        assert!(!check_signature(&row, "POST", "/reins/org-exec/claim", 1000, &good, b"{}", 1000 + SKEW_SECS + 1));
        assert!(!check_signature(&row, "POST", "/reins/org-exec/claim", 1000, &good, b"{ }", 1000), "another body");
        assert!(!check_signature(&row, "GET", "/reins/org-exec/claim", 1000, &good, b"{}", 1000), "another method");
        assert!(!check_signature(&row, "POST", "/reins/org-exec/roster", 1000, &good, b"{}", 1000), "another path");
        assert!(!check_signature(&row, "POST", "/reins/org-exec/claim", 1000, "AAAA", b"{}", 1000));
    }

    #[test]
    fn a_revoked_executor_is_refused() {
        let mut c = db();
        let e = Exec::register(&mut c, "o1", "e1");
        reins_org_exec::q_revoke_executor(&mut c, "o1", "e1", 5).unwrap();
        let row = reins_org_exec::q_executor(&mut c, "o1", "e1").unwrap().unwrap();
        let sig = e.sig("POST", "/reins/org-exec/claim", 1000, b"{}");
        assert!(!check_signature(&row, "POST", "/reins/org-exec/claim", 1000, &sig, b"{}", 1000));
    }

    #[test]
    fn an_executor_sees_only_its_own_tenant() {
        let mut c = db();
        pilot(&mut c);
        ask(&mut c, &issue("acme/api"), 100);
        let other = Exec::register(&mut c, "o2", "e1");
        assert!(q_claim_jobs(&mut c, &other.org, &other.id, 101).unwrap().is_empty());
        let mine = Exec::register(&mut c, "o1", "e1");
        let jobs = q_claim_jobs(&mut c, &mine.org, &mine.id, 101).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!((jobs[0].tool.as_str(), &jobs[0].args), ("github_create_issue", &issue("acme/api")));
        assert!(q_claim_jobs(&mut c, &mine.org, &mine.id, 102).unwrap().is_empty(), "claimed once");
        // Another tenant's executor can't finish it.
        let env = other.receipt(&jobs[0], ReceiptStatus::Succeeded, json!({}));
        assert!(q_receipt(&mut c, &other.me(), &env, 103).is_err());
    }

    /// Lee asks for a push lease on `repo` (Auto under the pilot policy); returns the lease id.
    fn lease_for_lee(c: &mut DbConnInner, repo: &str, now: i64) -> String {
        let lee = crate::db::models::ReinsConnection {
            uuid: "conn-lee".into(),
            user_uuid: crate::db::models::UserId::from("lee".to_owned()),
            client_id: "c".into(),
            client_name: "Reins desktop".into(),
            client_host: "h".into(),
            label: "Laptop".into(),
            created_at: 1,
            last_used_at: None,
            sealed_name: None,
        };
        let ask = reins_proto::managed::LeaseRequest {
            repo: repo.into(),
            access: reins_proto::managed::LeaseAccess::Push,
            box_key: reins_proto::canon::encode_payload(&[9u8; 32]),
        };
        super::super::leases::q_request_lease(c, &lee, &ask, now).unwrap().0.id
    }

    /// `ex` claims the one lease job there is and reports it minted at `at`.
    fn issue_lease(c: &mut DbConnInner, ex: &Exec, now: i64, at: i64) {
        let jobs = q_claim_jobs(c, &ex.org, &ex.id, now).unwrap();
        assert_eq!(jobs.len(), 1);
        let expires = jobs[0].lease.as_ref().unwrap().expires_at;
        q_receipt(
            c,
            &ex.me(),
            &ex.receipt(&jobs[0], ReceiptStatus::Succeeded, json!({"sealed": "S", "expires_at": expires})),
            at,
        )
        .unwrap();
    }

    #[test]
    fn revocations_go_only_to_the_executor_that_issued_the_lease() {
        let mut c = db();
        pilot(&mut c);
        let e1 = Exec::register(&mut c, "o1", "e1");
        let e2 = Exec::register(&mut c, "o1", "e2");
        let l1 = lease_for_lee(&mut c, "acme/api", 100);
        issue_lease(&mut c, &e1, 101, 102);
        let l2 = lease_for_lee(&mut c, "acme/web", 103);
        issue_lease(&mut c, &e2, 104, 105);
        super::super::leases::q_revoke_for_deprovision(&mut c, "o1", "lee", 110).unwrap();
        let listed = |c: &mut DbConnInner, ex: &Exec| -> Vec<String> {
            q_revocations(c, &ex.me()).unwrap().into_iter().map(|v| v["lease"].as_str().unwrap().to_owned()).collect()
        };
        assert_eq!(listed(&mut c, &e1), std::slice::from_ref(&l1));
        assert_eq!(listed(&mut c, &e2), std::slice::from_ref(&l2));
        // Another executor can't say a lease it didn't issue was revoked upstream.
        assert!(!q_revoked(&mut c, &e2.me(), &l1, 111).unwrap());
        assert_eq!(listed(&mut c, &e1), std::slice::from_ref(&l1));
        assert!(q_revoked(&mut c, &e1.me(), &l1, 111).unwrap());
        assert!(listed(&mut c, &e1).is_empty());
    }

    #[test]
    fn a_lease_minted_after_its_end_is_not_handed_out_and_is_revoked() {
        let mut c = db();
        pilot(&mut c);
        let e1 = Exec::register(&mut c, "o1", "e1");
        let id = lease_for_lee(&mut c, "acme/api", 100);
        // The executor was offline: it claims after the lease's end.
        let end = reins_org_request::q_lease(&mut c, "o1", &id).unwrap().unwrap().expires_at;
        issue_lease(&mut c, &e1, 101, end + 5);
        let l = reins_org_request::q_lease(&mut c, "o1", &id).unwrap().unwrap();
        assert_eq!((l.state.as_str(), l.sealed.as_deref()), (reins_org_request::lease_state::EXPIRED, None));
        // The token it minted is revoked upstream.
        assert_eq!(q_revocations(&mut c, &e1.me()).unwrap().len(), 1);
    }

    #[test]
    fn the_sweep_ends_leases_past_their_end() {
        let mut c = db();
        pilot(&mut c);
        let e1 = Exec::register(&mut c, "o1", "e1");
        let issued = lease_for_lee(&mut c, "acme/api", 100);
        issue_lease(&mut c, &e1, 101, 102);
        // Approved, never claimed (the executor is offline).
        let pending = lease_for_lee(&mut c, "acme/web", 100);
        let end = reins_org_request::q_lease(&mut c, "o1", &issued).unwrap().unwrap().expires_at;
        super::super::sweeper::sweep_once(&mut c, end).unwrap();
        for id in [&issued, &pending] {
            let l = reins_org_request::q_lease(&mut c, "o1", id).unwrap().unwrap();
            assert_eq!(
                (l.state.as_str(), l.sealed.as_deref()),
                (reins_org_request::lease_state::EXPIRED, None),
                "{id}"
            );
            assert_eq!(l.revoked_at, Some(end), "{id}");
        }
        // The issued one's token is revoked upstream by the executor that minted it; the other was never minted.
        let todo: Vec<Value> = q_revocations(&mut c, &e1.me()).unwrap();
        assert_eq!(todo, [json!({"lease": issued, "reason": "expired"})]);
        // A late claim of the other can't hand it out any more.
        assert!(
            q_claim_jobs(&mut c, "o1", "e1", end + 1)
                .unwrap()
                .iter()
                .all(|j| j.lease.as_ref().is_none_or(|l| l.lease != pending))
        );
    }

    #[test]
    fn a_receipt_must_be_signed_by_the_claiming_executor() {
        let mut c = db();
        pilot(&mut c);
        let s = ask(&mut c, &issue("acme/api"), 100);
        let e1 = Exec::register(&mut c, "o1", "e1");
        let e2 = Exec::register(&mut c, "o1", "e2");
        let jobs = q_claim_jobs(&mut c, "o1", "e1", 101).unwrap();
        // e2 didn't claim it.
        assert!(q_receipt(&mut c, &e2.me(), &e2.receipt(&jobs[0], ReceiptStatus::Succeeded, json!({})), 102).is_err());
        // e1's receipt signed by e2's key.
        let mut forged = e2.receipt(&jobs[0], ReceiptStatus::Succeeded, json!({}));
        forged.key_id = "e1".into();
        assert!(q_receipt(&mut c, &e1.me(), &forged, 102).is_err());
        q_receipt(&mut c, &e1.me(), &e1.receipt(&jobs[0], ReceiptStatus::Succeeded, json!({"number": 7})), 102)
            .unwrap();
        let status = decisions::status(&mut c, "o1", "conn-lee", &s.id, 103).unwrap();
        assert_eq!(status, json!({"status": "done", "ticket": s.id, "result": {"number": 7}}));
        // Once.
        assert!(q_receipt(&mut c, &e1.me(), &e1.receipt(&jobs[0], ReceiptStatus::Failed, json!({})), 104).is_err());
    }
}
