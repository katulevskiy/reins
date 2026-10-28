//! Family connections (Decisions v2 · P7): the accounts a guardian connected for a family member, and the jobs the
//! server queues for the family's executor (which alone holds the tokens).

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{reins_family_connect_jobs, reins_family_connections},
};

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_family_connections)]
#[diesel(primary_key(org_uuid, user_uuid, provider))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsFamilyConnection {
    pub org_uuid: String,
    pub user_uuid: String,
    /// `google` or `github`.
    pub provider: String,
    /// The connected account's address or login, as the executor reported it.
    pub account: Option<String>,
    /// `pending`, `connected` or `revoked`.
    pub state: String,
    pub connected_at: Option<i64>,
    pub updated_at: i64,
}

/// [`ReinsFamilyConnection::state`] values.
pub mod connection_state {
    pub const PENDING: &str = "pending";
    pub const CONNECTED: &str = "connected";
    pub const REVOKED: &str = "revoked";
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_family_connect_jobs)]
#[diesel(primary_key(id))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsFamilyConnectJob {
    pub id: String,
    pub org_uuid: String,
    pub user_uuid: String,
    pub provider: String,
    /// `begin`, `finish` or `revoke`.
    pub phase: String,
    /// The OAuth `state` the executor chose (a begin's receipt), which the provider's redirect carries back.
    pub oauth_state: Option<String>,
    pub code_key_id: Option<String>,
    /// A finishing job's one-time code, sealed at rest; cleared when the executor claims it.
    pub sealed_code: Option<String>,
    /// A begin's authorization URL (the executor's receipt).
    pub auth_url: Option<String>,
    /// `queued`, `claimed`, `done` or `failed`.
    pub state: String,
    pub claimed_by: Option<String>,
    pub requested_by: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub done_at: Option<i64>,
}

/// [`ReinsFamilyConnectJob::state`] values.
pub mod job_state {
    pub const QUEUED: &str = "queued";
    pub const CLAIMED: &str = "claimed";
    pub const DONE: &str = "done";
    pub const FAILED: &str = "failed";
}

// ---- connections -----------------------------------------------------------------------------------------------

pub fn q_connections(c: &mut DbConnInner, org: &str, user: Option<&str>) -> QueryResult<Vec<ReinsFamilyConnection>> {
    let mut q = reins_family_connections::table.filter(reins_family_connections::org_uuid.eq(org)).into_boxed();
    if let Some(u) = user {
        q = q.filter(reins_family_connections::user_uuid.eq(u.to_owned()));
    }
    q.load(c)
}

pub fn q_save_connection(c: &mut DbConnInner, row: &ReinsFamilyConnection) -> QueryResult<()> {
    let n = diesel::update(
        reins_family_connections::table
            .filter(reins_family_connections::org_uuid.eq(&row.org_uuid))
            .filter(reins_family_connections::user_uuid.eq(&row.user_uuid))
            .filter(reins_family_connections::provider.eq(&row.provider)),
    )
    .set(row)
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_family_connections::table).values(row).execute(c)?;
    }
    Ok(())
}

pub fn q_insert_job(c: &mut DbConnInner, row: &ReinsFamilyConnectJob) -> QueryResult<()> {
    diesel::insert_into(reins_family_connect_jobs::table).values(row).execute(c).map(drop)
}

pub fn q_job(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsFamilyConnectJob>> {
    reins_family_connect_jobs::table
        .filter(reins_family_connect_jobs::org_uuid.eq(org))
        .filter(reins_family_connect_jobs::id.eq(id))
        .first(c)
        .optional()
}

pub fn q_job_by_oauth_state(c: &mut DbConnInner, state: &str) -> QueryResult<Option<ReinsFamilyConnectJob>> {
    reins_family_connect_jobs::table.filter(reins_family_connect_jobs::oauth_state.eq(state)).first(c).optional()
}

pub fn q_save_job(c: &mut DbConnInner, row: &ReinsFamilyConnectJob) -> QueryResult<()> {
    diesel::update(reins_family_connect_jobs::table.filter(reins_family_connect_jobs::id.eq(&row.id)))
        .set(row)
        .execute(c)
        .map(drop)
}

/// Queued, unexpired jobs of `org`, oldest first.
pub fn q_queued_jobs(c: &mut DbConnInner, org: &str, now: i64, limit: i64) -> QueryResult<Vec<ReinsFamilyConnectJob>> {
    reins_family_connect_jobs::table
        .filter(reins_family_connect_jobs::org_uuid.eq(org))
        .filter(reins_family_connect_jobs::state.eq(job_state::QUEUED))
        .filter(reins_family_connect_jobs::expires_at.gt(now))
        .order(reins_family_connect_jobs::created_at.asc())
        .limit(limit)
        .load(c)
}

/// Drops every job whose time ran out, with whatever it held.
pub fn q_expire_jobs(c: &mut DbConnInner, now: i64) -> QueryResult<usize> {
    diesel::delete(reins_family_connect_jobs::table.filter(reins_family_connect_jobs::expires_at.le(now))).execute(c)
}
