//! Managed requests, their sealed bodies, reviewers' signed answers and credential leases (Decisions v2 · O2).
//!
//! Every state change is a guarded update (`… WHERE state IN (from)`), so two writers (a reviewer and the sweeper, two
//! executors) can't both move one request.

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{reins_org_approvals, reins_org_bodies, reins_org_leases, reins_org_requests},
};

/// [`ReinsOrgRequest::state`] values (03 §2's state machine, as the decision service keeps it).
pub mod request_state {
    pub const REVIEWING: &str = "reviewing";
    pub const APPROVED: &str = "approved";
    pub const DENIED: &str = "denied";
    pub const EXPIRED: &str = "expired";
    pub const CANCELLED: &str = "cancelled";
    pub const EXECUTING: &str = "executing";
    pub const DONE: &str = "done";
    pub const FAILED: &str = "failed";
    /// The executor refused it (08 R5).
    pub const REFUSED: &str = "refused";
    /// The world changed since the decision (03 §6).
    pub const STALE: &str = "stale";
    /// Claimed and never receipted: never run again (receipt rules).
    pub const UNKNOWN: &str = "unknown";
}

/// [`ReinsOrgRequest::kind`] values.
pub mod request_kind {
    pub const CALL: &str = "call";
    pub const LEASE: &str = "lease";
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_requests)]
#[diesel(primary_key(uuid))]
pub struct ReinsOrgRequest {
    /// `or_` + a uuid.
    pub uuid: String,
    pub org_uuid: String,
    pub user_uuid: String,
    pub connection_uuid: String,
    pub tool: String,
    pub digest: String,
    pub kind: String,
    pub state: String,
    /// `auto`, `review` or `deny`.
    pub outcome: String,
    pub snapshot: String,
    pub member_policy: Option<String>,
    /// The evaluation (zone, level, why) and the effect lines, as JSON.
    pub trace: String,
    /// `Vec<reins_policy::org::Obligation>` as JSON.
    pub obligations: String,
    /// The countersigned decision envelope, once approved.
    pub decision: Option<String>,
    pub created_at: i64,
    /// A review not decided by then expires.
    pub expires_at: i64,
    pub decided_at: Option<i64>,
    pub valid_until: Option<i64>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// The executor's signed receipt envelope.
    pub receipt: Option<String>,
}

pub fn q_insert_request(c: &mut DbConnInner, row: &ReinsOrgRequest) -> QueryResult<()> {
    diesel::insert_into(reins_org_requests::table).values(row).execute(c).map(drop)
}

pub fn q_request(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::uuid.eq(id))
        .first(c)
        .optional()
}

/// Moves `id` of `org` to `to` if it is in one of `from`.
pub fn q_set_state(c: &mut DbConnInner, org: &str, id: &str, from: &[&str], to: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_requests::table
            .filter(reins_org_requests::org_uuid.eq(org))
            .filter(reins_org_requests::uuid.eq(id))
            .filter(reins_org_requests::state.eq_any(from)),
    )
    .set((reins_org_requests::state.eq(to), reins_org_requests::finished_at.eq(Some(at))))
    .execute(c)?;
    Ok(n == 1)
}

/// Records the countersigned decision of a request under review (or just evaluated) and makes it executable.
pub fn q_approve(
    c: &mut DbConnInner,
    org: &str,
    id: &str,
    decision: &str,
    valid_until: i64,
    at: i64,
) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_requests::table
            .filter(reins_org_requests::org_uuid.eq(org))
            .filter(reins_org_requests::uuid.eq(id))
            .filter(reins_org_requests::state.eq(request_state::REVIEWING)),
    )
    .set((
        reins_org_requests::state.eq(request_state::APPROVED),
        reins_org_requests::decision.eq(Some(decision)),
        reins_org_requests::decided_at.eq(Some(at)),
        reins_org_requests::valid_until.eq(Some(valid_until)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

/// Approved requests of `org` an executor may take now, oldest first.
pub fn q_claimable(c: &mut DbConnInner, org: &str, now: i64, limit: i64) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::state.eq(request_state::APPROVED))
        .filter(reins_org_requests::claimed_by.is_null())
        .filter(reins_org_requests::valid_until.gt(now))
        .order(reins_org_requests::created_at.asc())
        .limit(limit)
        .load(c)
}

/// Takes an approved, still valid request for `executor` at `now`; false when someone else took it first or its
/// decision expired.
pub fn q_claim(c: &mut DbConnInner, org: &str, id: &str, executor: &str, now: i64) -> QueryResult<bool> {
    let at = now;
    let n = diesel::update(
        reins_org_requests::table
            .filter(reins_org_requests::org_uuid.eq(org))
            .filter(reins_org_requests::uuid.eq(id))
            .filter(reins_org_requests::state.eq(request_state::APPROVED))
            .filter(reins_org_requests::claimed_by.is_null())
            .filter(reins_org_requests::valid_until.gt(now)),
    )
    .set((
        reins_org_requests::state.eq(request_state::EXECUTING),
        reins_org_requests::claimed_by.eq(Some(executor)),
        reins_org_requests::claimed_at.eq(Some(at)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

/// Records the receipt of the executor that claimed `id`, and its end state.
pub fn q_finish(
    c: &mut DbConnInner,
    org: &str,
    id: &str,
    executor: &str,
    to: &str,
    receipt: &str,
    at: i64,
) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_requests::table
            .filter(reins_org_requests::org_uuid.eq(org))
            .filter(reins_org_requests::uuid.eq(id))
            .filter(reins_org_requests::state.eq(request_state::EXECUTING))
            .filter(reins_org_requests::claimed_by.eq(executor)),
    )
    .set((
        reins_org_requests::state.eq(to),
        reins_org_requests::receipt.eq(Some(receipt)),
        reins_org_requests::finished_at.eq(Some(at)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

/// Newest first, before `before` (created_at) if given.
pub fn q_requests_of_org(
    c: &mut DbConnInner,
    org: &str,
    before: Option<i64>,
    limit: i64,
) -> QueryResult<Vec<ReinsOrgRequest>> {
    let mut q = reins_org_requests::table.filter(reins_org_requests::org_uuid.eq(org)).into_boxed();
    if let Some(b) = before {
        q = q.filter(reins_org_requests::created_at.lt(b));
    }
    q.order(reins_org_requests::created_at.desc()).limit(limit).load(c)
}

/// Requests of `org` under review.
pub fn q_open_reviews(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::state.eq(request_state::REVIEWING))
        .order(reins_org_requests::created_at.asc())
        .limit(500)
        .load(c)
}

/// Every org's reviews past their window (the sweeper).
pub fn q_overdue_reviews(c: &mut DbConnInner, now: i64) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::state.eq(request_state::REVIEWING))
        .filter(reins_org_requests::expires_at.le(now))
        .limit(500)
        .load(c)
}

/// Approved and never claimed past their validity (the sweeper).
pub fn q_lapsed_approvals(c: &mut DbConnInner, now: i64) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::state.eq(request_state::APPROVED))
        .filter(reins_org_requests::claimed_by.is_null())
        .filter(reins_org_requests::valid_until.le(now))
        .limit(500)
        .load(c)
}

/// Claimed before `cutoff` and never receipted (the sweeper).
pub fn q_stuck(c: &mut DbConnInner, cutoff: i64) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::state.eq(request_state::EXECUTING))
        .filter(reins_org_requests::claimed_at.lt(cutoff))
        .limit(500)
        .load(c)
}

/// The open requests `user` made in `org` that no executor has taken yet.
pub fn q_cancellable_of_user(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::user_uuid.eq(user))
        .filter(reins_org_requests::state.eq_any([request_state::REVIEWING, request_state::APPROVED]))
        .filter(reins_org_requests::claimed_by.is_null())
        .load(c)
}

/// How many requests of `connection` in `org` are still open: under review, approved, or running.
pub fn q_open_of_connection(c: &mut DbConnInner, org: &str, connection: &str) -> QueryResult<i64> {
    reins_org_requests::table
        .filter(reins_org_requests::connection_uuid.eq(connection))
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::state.eq_any([
            request_state::REVIEWING,
            request_state::APPROVED,
            request_state::EXECUTING,
        ]))
        .count()
        .get_result(c)
}

/// What the console's reporting line counts (05 §10 basics).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: i64,
    pub auto: i64,
    pub reviewed: i64,
    pub denied: i64,
    pub expired: i64,
    /// Median seconds from request to decision, for reviewed requests that were decided.
    pub median_decision_secs: Option<i64>,
}

pub fn q_counts_since(c: &mut DbConnInner, org: &str, since: i64) -> QueryResult<Counts> {
    let rows: Vec<(String, String, i64, Option<i64>)> = reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::created_at.ge(since))
        .select((
            reins_org_requests::outcome,
            reins_org_requests::state,
            reins_org_requests::created_at,
            reins_org_requests::decided_at,
        ))
        .limit(100_000)
        .load(c)?;
    let mut out = Counts {
        total: i64::try_from(rows.len()).unwrap_or(i64::MAX),
        ..Counts::default()
    };
    let mut times: Vec<i64> = Vec::new();
    for (outcome, state, created, decided) in &rows {
        match outcome.as_str() {
            "auto" => out.auto += 1,
            "review" => {
                out.reviewed += 1;
                if let Some(d) = decided {
                    times.push(d - created);
                }
            }
            _ => {}
        }
        if state == request_state::DENIED {
            out.denied += 1;
        }
        if state == request_state::EXPIRED {
            out.expired += 1;
        }
    }
    times.sort_unstable();
    out.median_decision_secs = times.get(times.len() / 2).copied();
    Ok(out)
}

/// A request's body or result, sealed under the tenant's data key.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_bodies)]
#[diesel(primary_key(request_uuid, part))]
pub struct ReinsOrgBody {
    pub request_uuid: String,
    /// `request` or `result`.
    pub part: String,
    pub org_uuid: String,
    pub key_id: String,
    pub sealed: String,
    pub created_at: i64,
    /// Deleted from then (retention); `None` while it is still needed to execute.
    pub purge_at: Option<i64>,
}

pub fn q_put_body(c: &mut DbConnInner, row: &ReinsOrgBody) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(
            reins_org_bodies::table
                .filter(reins_org_bodies::request_uuid.eq(&row.request_uuid))
                .filter(reins_org_bodies::part.eq(&row.part)),
        )
        .execute(c)?;
        diesel::insert_into(reins_org_bodies::table).values(row).execute(c).map(drop)
    })
}

pub fn q_body(c: &mut DbConnInner, org: &str, request: &str, part: &str) -> QueryResult<Option<ReinsOrgBody>> {
    reins_org_bodies::table
        .filter(reins_org_bodies::org_uuid.eq(org))
        .filter(reins_org_bodies::request_uuid.eq(request))
        .filter(reins_org_bodies::part.eq(part))
        .first(c)
        .optional()
}

/// Sets when a request's bodies go (once it no longer needs them).
pub fn q_set_purge(c: &mut DbConnInner, org: &str, request: &str, at: i64) -> QueryResult<()> {
    diesel::update(
        reins_org_bodies::table
            .filter(reins_org_bodies::org_uuid.eq(org))
            .filter(reins_org_bodies::request_uuid.eq(request)),
    )
    .set(reins_org_bodies::purge_at.eq(Some(at)))
    .execute(c)
    .map(drop)
}

/// Deletes every body past its retention.
pub fn q_purge_bodies(c: &mut DbConnInner, now: i64) -> QueryResult<usize> {
    diesel::delete(reins_org_bodies::table.filter(reins_org_bodies::purge_at.le(now))).execute(c)
}

/// A reviewer's signed answer to a request or an admin change.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_approvals)]
#[diesel(primary_key(subject_uuid, user_uuid, decision))]
pub struct ReinsOrgApproval {
    pub subject_uuid: String,
    pub user_uuid: String,
    /// `approve`, `deny` or `propose`.
    pub decision: String,
    pub org_uuid: String,
    pub device_uuid: String,
    pub at: i64,
    pub sig: String,
    pub obligation: Option<String>,
}

/// Records an answer; false when this person already gave this answer.
pub fn q_insert_approval(c: &mut DbConnInner, row: &ReinsOrgApproval) -> QueryResult<bool> {
    let exists: i64 = reins_org_approvals::table
        .filter(reins_org_approvals::subject_uuid.eq(&row.subject_uuid))
        .filter(reins_org_approvals::user_uuid.eq(&row.user_uuid))
        .filter(reins_org_approvals::decision.eq(&row.decision))
        .count()
        .get_result(c)?;
    if exists > 0 {
        return Ok(false);
    }
    diesel::insert_into(reins_org_approvals::table).values(row).execute(c)?;
    Ok(true)
}

pub fn q_approvals(c: &mut DbConnInner, org: &str, subject: &str) -> QueryResult<Vec<ReinsOrgApproval>> {
    reins_org_approvals::table
        .filter(reins_org_approvals::org_uuid.eq(org))
        .filter(reins_org_approvals::subject_uuid.eq(subject))
        .order(reins_org_approvals::at.asc())
        .load(c)
}

/// Drops `user`'s answers to requests still under review in `org` (a removed reviewer no longer counts).
pub fn q_drop_open_approvals(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<usize> {
    let open: Vec<String> = reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::state.eq(request_state::REVIEWING))
        .select(reins_org_requests::uuid)
        .load(c)?;
    diesel::delete(
        reins_org_approvals::table
            .filter(reins_org_approvals::org_uuid.eq(org))
            .filter(reins_org_approvals::user_uuid.eq(user))
            .filter(reins_org_approvals::subject_uuid.eq_any(open)),
    )
    .execute(c)
}

/// [`ReinsOrgLease::state`] values.
pub mod lease_state {
    pub const PENDING: &str = "pending";
    pub const ISSUED: &str = "issued";
    pub const DENIED: &str = "denied";
    pub const REVOKED: &str = "revoked";
    pub const EXPIRED: &str = "expired";
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_leases)]
#[diesel(primary_key(uuid))]
pub struct ReinsOrgLease {
    pub uuid: String,
    pub org_uuid: String,
    pub user_uuid: String,
    pub connection_uuid: String,
    pub request_uuid: String,
    pub repo: String,
    pub access: String,
    pub box_key: String,
    pub state: String,
    /// The credential sealed to `box_key` (never in clear on the server).
    pub sealed: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub issued_at: Option<i64>,
    pub revoked_at: Option<i64>,
    /// When the executor confirmed the credential was revoked upstream.
    pub revoke_done_at: Option<i64>,
}

pub fn q_insert_lease(c: &mut DbConnInner, row: &ReinsOrgLease) -> QueryResult<()> {
    diesel::insert_into(reins_org_leases::table).values(row).execute(c).map(drop)
}

pub fn q_lease(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsOrgLease>> {
    reins_org_leases::table
        .filter(reins_org_leases::org_uuid.eq(org))
        .filter(reins_org_leases::uuid.eq(id))
        .first(c)
        .optional()
}

pub fn q_lease_of_request(c: &mut DbConnInner, org: &str, request: &str) -> QueryResult<Option<ReinsOrgLease>> {
    reins_org_leases::table
        .filter(reins_org_leases::org_uuid.eq(org))
        .filter(reins_org_leases::request_uuid.eq(request))
        .first(c)
        .optional()
}

/// Sets a lease's state if it is in one of `from`.
pub fn q_set_lease_state(c: &mut DbConnInner, org: &str, id: &str, from: &[&str], to: &str) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::state.eq_any(from)),
    )
    .set(reins_org_leases::state.eq(to))
    .execute(c)?;
    Ok(n == 1)
}

/// The executor minted it: the sealed credential. Only a pending lease is issued (a revoked one stays revoked).
pub fn q_set_lease_issued(
    c: &mut DbConnInner,
    org: &str,
    id: &str,
    sealed: &str,
    expires_at: i64,
    at: i64,
) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::state.eq(lease_state::PENDING)),
    )
    .set((
        reins_org_leases::state.eq(lease_state::ISSUED),
        reins_org_leases::sealed.eq(Some(sealed)),
        reins_org_leases::expires_at.eq(expires_at),
        reins_org_leases::issued_at.eq(Some(at)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

/// Sets a pending lease's expiry (from its decision: the lease lives from then).
pub fn q_set_lease_expiry(c: &mut DbConnInner, org: &str, id: &str, expires_at: i64) -> QueryResult<()> {
    diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::state.eq(lease_state::PENDING)),
    )
    .set(reins_org_leases::expires_at.eq(expires_at))
    .execute(c)
    .map(drop)
}

/// Revokes every pending or issued lease of `user` in `org` and returns them (deprovisioning). The issued ones are
/// then revoked upstream by the executor that minted them ([`q_leases_to_revoke_by`]); the sealed credential is dropped here.
pub fn q_revoke_leases_of_user(c: &mut DbConnInner, org: &str, user: &str, at: i64) -> QueryResult<Vec<ReinsOrgLease>> {
    let live: Vec<ReinsOrgLease> = reins_org_leases::table
        .filter(reins_org_leases::org_uuid.eq(org))
        .filter(reins_org_leases::user_uuid.eq(user))
        .filter(reins_org_leases::state.eq_any([lease_state::PENDING, lease_state::ISSUED]))
        .load(c)?;
    diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::user_uuid.eq(user))
            .filter(reins_org_leases::state.eq_any([lease_state::PENDING, lease_state::ISSUED])),
    )
    .set((
        reins_org_leases::state.eq(lease_state::REVOKED),
        reins_org_leases::revoked_at.eq(Some(at)),
        reins_org_leases::sealed.eq(None::<String>),
    ))
    .execute(c)?;
    Ok(live)
}

/// The executor minted a lease that was revoked meanwhile: the credential exists upstream, so it is queued for
/// revocation ([`q_leases_to_revoke_by`]) while the lease stays revoked.
pub fn q_mark_issued_after_revocation(c: &mut DbConnInner, org: &str, id: &str, at: i64) -> QueryResult<()> {
    diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::state.eq_any([lease_state::REVOKED, lease_state::EXPIRED]))
            .filter(reins_org_leases::issued_at.is_null()),
    )
    .set(reins_org_leases::issued_at.eq(Some(at)))
    .execute(c)
    .map(drop)
}

/// Leases past their end become expired, and their sealed credential goes: issued ones (the executor then revokes the
/// token upstream, [`q_leases_to_revoke_by`]), and pending ones whose request was decided (an executor may be minting
/// one right now; whatever it mints after this is never handed out, and is revoked too). A pending lease of a request
/// still under review waits: its end is set when it is decided.
pub fn q_expire_leases(c: &mut DbConnInner, now: i64) -> QueryResult<usize> {
    let reviewing: Vec<String> = reins_org_requests::table
        .filter(reins_org_requests::state.eq(request_state::REVIEWING))
        .select(reins_org_requests::uuid)
        .load(c)?;
    let issued = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::state.eq(lease_state::ISSUED))
            .filter(reins_org_leases::expires_at.le(now)),
    )
    .set((
        reins_org_leases::state.eq(lease_state::EXPIRED),
        reins_org_leases::sealed.eq(None::<String>),
        reins_org_leases::revoked_at.eq(Some(now)),
    ))
    .execute(c)?;
    let pending = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::state.eq(lease_state::PENDING))
            .filter(reins_org_leases::expires_at.le(now))
            .filter(reins_org_leases::request_uuid.ne_all(reviewing)),
    )
    .set((
        reins_org_leases::state.eq(lease_state::EXPIRED),
        reins_org_leases::sealed.eq(None::<String>),
        reins_org_leases::revoked_at.eq(Some(now)),
    ))
    .execute(c)?;
    Ok(issued + pending)
}

/// Ends a lease that is in one of `from` (its request ended): `to`, at `at`, without its credential.
pub fn q_end_lease(c: &mut DbConnInner, org: &str, id: &str, from: &[&str], to: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::state.eq_any(from)),
    )
    .set((
        reins_org_leases::state.eq(to),
        reins_org_leases::sealed.eq(None::<String>),
        reins_org_leases::revoked_at.eq(Some(at)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

/// Approved lease requests no executor took before their lease's end (the executor was offline): they end too.
pub fn q_lapsed_lease_requests(c: &mut DbConnInner, now: i64) -> QueryResult<Vec<ReinsOrgRequest>> {
    let lapsed: Vec<String> = reins_org_leases::table
        .filter(reins_org_leases::state.eq(lease_state::PENDING))
        .filter(reins_org_leases::expires_at.le(now))
        .select(reins_org_leases::request_uuid)
        .limit(500)
        .load(c)?;
    reins_org_requests::table
        .filter(reins_org_requests::kind.eq(request_kind::LEASE))
        .filter(reins_org_requests::state.eq(request_state::APPROVED))
        .filter(reins_org_requests::claimed_by.is_null())
        .filter(reins_org_requests::uuid.eq_any(lapsed))
        .limit(500)
        .load(c)
}

/// Leases of `org` whose credential an executor must still revoke upstream: revoked or expired after it was issued.
/// The whole queue, for tests; executors read their own part ([`q_leases_to_revoke_by`]).
#[cfg(test)]
pub fn q_leases_to_revoke(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgLease>> {
    reins_org_leases::table
        .filter(reins_org_leases::org_uuid.eq(org))
        .filter(reins_org_leases::state.eq_any([lease_state::REVOKED, lease_state::EXPIRED]))
        .filter(reins_org_leases::issued_at.is_not_null())
        .filter(reins_org_leases::revoke_done_at.is_null())
        .limit(100)
        .load(c)
}

/// Of `requests` (ids in `org`), those `executor` claimed.
fn claimed_by(c: &mut DbConnInner, org: &str, executor: &str, requests: &[String]) -> QueryResult<Vec<String>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::uuid.eq_any(requests))
        .filter(reins_org_requests::claimed_by.eq(executor))
        .select(reins_org_requests::uuid)
        .load(c)
}

/// The revocation queue for the leases `executor` minted (it claimed their request): only it holds their tokens.
pub fn q_leases_to_revoke_by(c: &mut DbConnInner, org: &str, executor: &str) -> QueryResult<Vec<ReinsOrgLease>> {
    let queued: Vec<ReinsOrgLease> = reins_org_leases::table
        .filter(reins_org_leases::org_uuid.eq(org))
        .filter(reins_org_leases::state.eq_any([lease_state::REVOKED, lease_state::EXPIRED]))
        .filter(reins_org_leases::issued_at.is_not_null())
        .filter(reins_org_leases::revoke_done_at.is_null())
        .order(reins_org_leases::created_at.asc())
        .limit(1_000)
        .load(c)?;
    let ids: Vec<String> = queued.iter().map(|l| l.request_uuid.clone()).collect();
    let mine = claimed_by(c, org, executor, &ids)?;
    Ok(queued.into_iter().filter(|l| mine.contains(&l.request_uuid)).take(100).collect())
}

/// Records that `executor` revoked lease `id`'s token upstream; only the executor that minted it may.
pub fn q_mark_revoke_done_by(c: &mut DbConnInner, org: &str, id: &str, executor: &str, at: i64) -> QueryResult<bool> {
    let Some(lease) = q_lease(c, org, id)? else {
        return Ok(false);
    };
    if claimed_by(c, org, executor, std::slice::from_ref(&lease.request_uuid))?.is_empty() {
        return Ok(false);
    }
    let n = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::revoke_done_at.is_null()),
    )
    .set(reins_org_leases::revoke_done_at.eq(Some(at)))
    .execute(c)?;
    Ok(n == 1)
}

#[cfg(test)]
pub fn q_mark_revoke_done(c: &mut DbConnInner, org: &str, id: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_leases::table
            .filter(reins_org_leases::org_uuid.eq(org))
            .filter(reins_org_leases::uuid.eq(id))
            .filter(reins_org_leases::revoke_done_at.is_null()),
    )
    .set(reins_org_leases::revoke_done_at.eq(Some(at)))
    .execute(c)?;
    Ok(n == 1)
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    pub fn request(org: &str, id: &str, state: &str) -> ReinsOrgRequest {
        ReinsOrgRequest {
            uuid: id.to_owned(),
            org_uuid: org.to_owned(),
            user_uuid: "u1".to_owned(),
            connection_uuid: "c1".to_owned(),
            tool: "github_create_issue".to_owned(),
            digest: "d".to_owned(),
            kind: request_kind::CALL.to_owned(),
            state: state.to_owned(),
            outcome: "review".to_owned(),
            snapshot: "s".to_owned(),
            member_policy: None,
            trace: "{}".to_owned(),
            obligations: "[]".to_owned(),
            decision: None,
            created_at: 10,
            expires_at: 100,
            decided_at: None,
            valid_until: None,
            claimed_by: None,
            claimed_at: None,
            finished_at: None,
            receipt: None,
        }
    }

    fn lease(id: &str, user: &str, state: &str, issued: Option<i64>) -> ReinsOrgLease {
        ReinsOrgLease {
            uuid: id.to_owned(),
            org_uuid: "o1".to_owned(),
            user_uuid: user.to_owned(),
            connection_uuid: "c1".to_owned(),
            request_uuid: format!("or_{id}"),
            repo: "acme/api".to_owned(),
            access: "push".to_owned(),
            box_key: "k".to_owned(),
            state: state.to_owned(),
            sealed: issued.map(|_| "sealed".to_owned()),
            created_at: 1,
            expires_at: 1_000,
            issued_at: issued,
            revoked_at: None,
            revoke_done_at: None,
        }
    }

    #[test]
    fn state_moves_are_guarded() {
        let mut c = test_db();
        q_insert_request(&mut c, &request("o1", "r1", request_state::REVIEWING)).unwrap();
        assert!(!q_set_state(&mut c, "o2", "r1", &[request_state::REVIEWING], request_state::DENIED, 5).unwrap());
        assert!(q_approve(&mut c, "o1", "r1", "env", 500, 5).unwrap());
        assert!(!q_approve(&mut c, "o1", "r1", "env2", 500, 6).unwrap(), "approved once");
        assert!(!q_set_state(&mut c, "o1", "r1", &[request_state::REVIEWING], request_state::EXPIRED, 7).unwrap());
        let r = q_request(&mut c, "o1", "r1").unwrap().unwrap();
        assert_eq!((r.state.as_str(), r.decision.as_deref()), (request_state::APPROVED, Some("env")));
    }

    #[test]
    fn claims_are_exclusive() {
        let mut c = test_db();
        q_insert_request(&mut c, &request("o1", "r1", request_state::REVIEWING)).unwrap();
        q_approve(&mut c, "o1", "r1", "env", 500, 5).unwrap();
        assert_eq!(q_claimable(&mut c, "o2", 6, 10).unwrap().len(), 0, "another tenant");
        assert_eq!(q_claimable(&mut c, "o1", 600, 10).unwrap().len(), 0, "past its validity");
        assert_eq!(q_claimable(&mut c, "o1", 6, 10).unwrap().len(), 1);
        assert!(!q_claim(&mut c, "o1", "r1", "e1", 500).unwrap(), "its decision expired");
        assert!(q_claim(&mut c, "o1", "r1", "e1", 6).unwrap());
        assert!(!q_claim(&mut c, "o1", "r1", "e2", 6).unwrap());
        assert!(!q_finish(&mut c, "o1", "r1", "e2", request_state::DONE, "rc", 7).unwrap(), "not its claim");
        assert!(q_finish(&mut c, "o1", "r1", "e1", request_state::DONE, "rc", 7).unwrap());
        assert!(!q_finish(&mut c, "o1", "r1", "e1", request_state::FAILED, "rc", 8).unwrap(), "finished once");
    }

    #[test]
    fn leases_of_a_user_are_revoked_and_queued_for_upstream_revocation() {
        let mut c = test_db();
        q_insert_lease(&mut c, &lease("l1", "u1", lease_state::ISSUED, Some(2))).unwrap();
        q_insert_lease(&mut c, &lease("l2", "u1", lease_state::PENDING, None)).unwrap();
        q_insert_lease(&mut c, &lease("l3", "u2", lease_state::ISSUED, Some(2))).unwrap();
        let revoked = q_revoke_leases_of_user(&mut c, "o1", "u1", 9).unwrap();
        assert_eq!(revoked.len(), 2);
        let l1 = q_lease(&mut c, "o1", "l1").unwrap().unwrap();
        assert_eq!((l1.state.as_str(), l1.sealed), (lease_state::REVOKED, None));
        // Only the issued one needs revoking upstream; a pending one can no longer be issued.
        let todo: Vec<String> = q_leases_to_revoke(&mut c, "o1").unwrap().into_iter().map(|l| l.uuid).collect();
        assert_eq!(todo, ["l1"]);
        assert!(!q_set_lease_issued(&mut c, "o1", "l2", "s", 900, 10).unwrap());
        assert!(q_mark_revoke_done(&mut c, "o1", "l1", 11).unwrap());
        assert!(q_leases_to_revoke(&mut c, "o1").unwrap().is_empty());
        assert_eq!(q_lease(&mut c, "o1", "l3").unwrap().unwrap().state, lease_state::ISSUED, "others untouched");
        // Expired leases are revoked upstream as well.
        assert_eq!(q_expire_leases(&mut c, 1_000).unwrap(), 1);
        assert_eq!(q_leases_to_revoke(&mut c, "o1").unwrap().len(), 1);
    }

    #[test]
    fn bodies_purge_after_retention() {
        let mut c = test_db();
        let body = |id: &str, purge: Option<i64>| ReinsOrgBody {
            request_uuid: id.to_owned(),
            part: "request".to_owned(),
            org_uuid: "o1".to_owned(),
            key_id: "k".to_owned(),
            sealed: "x".to_owned(),
            created_at: 1,
            purge_at: purge,
        };
        q_put_body(&mut c, &body("r1", Some(10))).unwrap();
        q_put_body(&mut c, &body("r2", None)).unwrap();
        assert_eq!(q_purge_bodies(&mut c, 10).unwrap(), 1);
        assert!(q_body(&mut c, "o1", "r1", "request").unwrap().is_none());
        assert!(q_body(&mut c, "o2", "r2", "request").unwrap().is_none(), "another tenant");
        q_set_purge(&mut c, "o1", "r2", 20).unwrap();
        assert_eq!(q_purge_bodies(&mut c, 19).unwrap(), 0);
        assert_eq!(q_purge_bodies(&mut c, 20).unwrap(), 1);
    }

    #[test]
    fn approvals_are_once_per_person_and_answer() {
        let mut c = test_db();
        q_insert_request(&mut c, &request("o1", "r1", request_state::REVIEWING)).unwrap();
        let a = ReinsOrgApproval {
            subject_uuid: "r1".to_owned(),
            user_uuid: "u2".to_owned(),
            decision: "approve".to_owned(),
            org_uuid: "o1".to_owned(),
            device_uuid: "d".to_owned(),
            at: 1,
            sig: "s".to_owned(),
            obligation: None,
        };
        assert!(q_insert_approval(&mut c, &a).unwrap());
        assert!(!q_insert_approval(&mut c, &a).unwrap());
        assert_eq!(q_approvals(&mut c, "o1", "r1").unwrap().len(), 1);
        assert_eq!(q_drop_open_approvals(&mut c, "o1", "u2").unwrap(), 1);
        assert!(q_approvals(&mut c, "o1", "r1").unwrap().is_empty());
    }
}
