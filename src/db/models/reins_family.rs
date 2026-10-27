//! The Family tenant (Decisions v2 · P7, 08 R7): an org of tier `family` and what it adds to O1's tables: its notice,
//! its invitations (bound to one exact email, single use, the secret kept hashed), which members are minors, members'
//! requests to leave, and the accounts a guardian connected for a member (held by the family's executor).

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{reins_families, reins_family_invites, reins_family_leaves, reins_family_members},
};

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_families)]
#[diesel(primary_key(org_uuid))]
pub struct ReinsFamily {
    pub org_uuid: String,
    /// An adult's notice before leaving: 0, 3 or 7 days.
    pub notice_days: i32,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_family_invites)]
#[diesel(primary_key(id))]
pub struct ReinsFamilyInvite {
    pub id: String,
    /// SHA-256 (hex) of the invitation's secret.
    pub token_hash: String,
    pub org_uuid: String,
    /// Normalized (trimmed, lowercase).
    pub email: String,
    pub minor: bool,
    /// The invited person becomes a guardian.
    pub guardian: bool,
    /// A guardian invitation counts once another guardian approved it (`AddGuardian`); every other is approved.
    pub approved: bool,
    pub created_by: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    pub used_by: Option<String>,
}

impl ReinsFamilyInvite {
    #[must_use]
    pub fn open(&self, now: i64) -> bool {
        self.used_at.is_none() && self.expires_at > now
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_family_members)]
#[diesel(primary_key(org_uuid, user_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsFamilyMember {
    pub org_uuid: String,
    pub user_uuid: String,
    pub minor: bool,
    /// A guardian proposed marking this adult a minor; it counts only once they accept on their phone.
    pub minor_proposed_at: Option<i64>,
    pub minor_proposed_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_family_leaves)]
#[diesel(primary_key(org_uuid, user_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsFamilyLeave {
    pub org_uuid: String,
    pub user_uuid: String,
    pub requested_at: i64,
    /// When the member goes: the notice's end (an adult), or the consent's time (a minor). `None`: waiting for consent.
    pub effective_at: Option<i64>,
    pub consented_by: Option<String>,
}

// ---- families --------------------------------------------------------------------------------------------------

pub fn q_family(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsFamily>> {
    reins_families::table.filter(reins_families::org_uuid.eq(org)).first(c).optional()
}

pub fn q_insert_family(c: &mut DbConnInner, row: &ReinsFamily) -> QueryResult<()> {
    diesel::insert_into(reins_families::table).values(row).execute(c).map(drop)
}

pub fn q_set_notice(c: &mut DbConnInner, org: &str, days: i32) -> QueryResult<()> {
    diesel::update(reins_families::table.filter(reins_families::org_uuid.eq(org)))
        .set(reins_families::notice_days.eq(days))
        .execute(c)
        .map(drop)
}

// ---- invitations -----------------------------------------------------------------------------------------------

pub fn q_insert_invite(c: &mut DbConnInner, row: &ReinsFamilyInvite) -> QueryResult<()> {
    diesel::insert_into(reins_family_invites::table).values(row).execute(c).map(drop)
}

pub fn q_invite(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsFamilyInvite>> {
    reins_family_invites::table
        .filter(reins_family_invites::org_uuid.eq(org))
        .filter(reins_family_invites::id.eq(id))
        .first(c)
        .optional()
}

pub fn q_invite_by_hash(c: &mut DbConnInner, token_hash: &str) -> QueryResult<Option<ReinsFamilyInvite>> {
    reins_family_invites::table.filter(reins_family_invites::token_hash.eq(token_hash)).first(c).optional()
}

/// The family's invitations, newest first.
pub fn q_invites(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsFamilyInvite>> {
    reins_family_invites::table
        .filter(reins_family_invites::org_uuid.eq(org))
        .order(reins_family_invites::created_at.desc())
        .load(c)
}

/// Invitations neither used nor expired.
pub fn q_open_invites(c: &mut DbConnInner, org: &str, now: i64) -> QueryResult<i64> {
    reins_family_invites::table
        .filter(reins_family_invites::org_uuid.eq(org))
        .filter(reins_family_invites::used_at.is_null())
        .filter(reins_family_invites::expires_at.gt(now))
        .count()
        .get_result(c)
}

/// Marks an invitation used by `user`, if it is still unused (compare-and-swap: one use only).
pub fn q_use_invite(c: &mut DbConnInner, id: &str, user: &str, now: i64) -> QueryResult<bool> {
    diesel::update(
        reins_family_invites::table
            .filter(reins_family_invites::id.eq(id))
            .filter(reins_family_invites::used_at.is_null()),
    )
    .set((reins_family_invites::used_at.eq(now), reins_family_invites::used_by.eq(user)))
    .execute(c)
    .map(|n| n == 1)
}

pub fn q_approve_invite(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<bool> {
    diesel::update(
        reins_family_invites::table
            .filter(reins_family_invites::org_uuid.eq(org))
            .filter(reins_family_invites::id.eq(id)),
    )
    .set(reins_family_invites::approved.eq(true))
    .execute(c)
    .map(|n| n == 1)
}

/// Withdraws an unused invitation (it expires now).
pub fn q_cancel_invite(c: &mut DbConnInner, org: &str, id: &str, now: i64) -> QueryResult<bool> {
    diesel::update(
        reins_family_invites::table
            .filter(reins_family_invites::org_uuid.eq(org))
            .filter(reins_family_invites::id.eq(id))
            .filter(reins_family_invites::used_at.is_null()),
    )
    .set(reins_family_invites::expires_at.eq(now))
    .execute(c)
    .map(|n| n == 1)
}

// ---- members ---------------------------------------------------------------------------------------------------

pub fn q_member(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Option<ReinsFamilyMember>> {
    reins_family_members::table
        .filter(reins_family_members::org_uuid.eq(org))
        .filter(reins_family_members::user_uuid.eq(user))
        .first(c)
        .optional()
}

pub fn q_save_member(c: &mut DbConnInner, row: &ReinsFamilyMember) -> QueryResult<()> {
    let n = diesel::update(
        reins_family_members::table
            .filter(reins_family_members::org_uuid.eq(&row.org_uuid))
            .filter(reins_family_members::user_uuid.eq(&row.user_uuid)),
    )
    .set(row)
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_family_members::table).values(row).execute(c)?;
    }
    Ok(())
}

// ---- leaving ---------------------------------------------------------------------------------------------------

pub fn q_leave(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Option<ReinsFamilyLeave>> {
    reins_family_leaves::table
        .filter(reins_family_leaves::org_uuid.eq(org))
        .filter(reins_family_leaves::user_uuid.eq(user))
        .first(c)
        .optional()
}

pub fn q_leaves(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsFamilyLeave>> {
    reins_family_leaves::table.filter(reins_family_leaves::org_uuid.eq(org)).load(c)
}

pub fn q_insert_leave(c: &mut DbConnInner, row: &ReinsFamilyLeave) -> QueryResult<()> {
    diesel::insert_into(reins_family_leaves::table).values(row).execute(c).map(drop)
}

/// A guardian's consent: the member goes now. Only a request still waiting for consent.
pub fn q_consent_leave(c: &mut DbConnInner, org: &str, user: &str, guardian: &str, now: i64) -> QueryResult<bool> {
    diesel::update(
        reins_family_leaves::table
            .filter(reins_family_leaves::org_uuid.eq(org))
            .filter(reins_family_leaves::user_uuid.eq(user))
            .filter(reins_family_leaves::effective_at.is_null()),
    )
    .set((reins_family_leaves::effective_at.eq(now), reins_family_leaves::consented_by.eq(guardian)))
    .execute(c)
    .map(|n| n == 1)
}

pub fn q_delete_leave(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<()> {
    diesel::delete(
        reins_family_leaves::table
            .filter(reins_family_leaves::org_uuid.eq(org))
            .filter(reins_family_leaves::user_uuid.eq(user)),
    )
    .execute(c)
    .map(drop)
}

/// Leaves whose time came, of every family.
pub fn q_due_leaves(c: &mut DbConnInner, now: i64) -> QueryResult<Vec<ReinsFamilyLeave>> {
    reins_family_leaves::table.filter(reins_family_leaves::effective_at.le(now)).load(c)
}
