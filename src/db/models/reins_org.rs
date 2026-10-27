//! Reins organization tenants (Decisions v2 · O1) on top of Vaultwarden's organizations: the tenant row is an
//! `organizations` row, members are `users_organizations` rows and SCIM groups are `groups`. The tables here add the
//! tier and sign-in, domains, roles, two-person changes and enrolled devices.
//!
//! The `q_*` functions take a bare connection so that one service call can run several of them in one transaction
//! (`conn.run(|c| c.transaction(|c| …))`), and so that the model tests run against an in-memory SQLite.

#![allow(dead_code)] // O1 wiring in progress: the org services use these.

use diesel::prelude::*;
use reins_proto::org::{Capability, IdentityKind, Role, Tier, role_grants};

use crate::db::{
    DbConnInner,
    schema::{
        groups, groups_users, reins_org_changes, reins_org_devices, reins_org_domains, reins_org_roles,
        reins_org_verified_domains, reins_orgs, users_organizations,
    },
};

use super::{Membership, MembershipStatus};

/// A membership counts (signs in, holds roles) when it was accepted or confirmed and is not revoked.
pub const ACTIVE_STATUS: i32 = MembershipStatus::Accepted as i32;

#[derive(Clone, Debug, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_orgs)]
#[diesel(primary_key(org_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrg {
    pub org_uuid: String,
    pub tier: String,
    pub identity: String,
    pub workos_org_id: Option<String>,
    pub directory_id: Option<String>,
    pub github_org: Option<String>,
    pub two_person: bool,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ReinsOrg {
    #[must_use]
    pub fn tier(&self) -> Tier {
        Tier::parse(&self.tier).unwrap_or(Tier::Team)
    }

    #[must_use]
    pub fn identity(&self) -> IdentityKind {
        IdentityKind::parse(&self.identity).unwrap_or(IdentityKind::None)
    }
}

#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_domains)]
#[diesel(primary_key(org_uuid, domain))]
pub struct ReinsOrgDomain {
    pub org_uuid: String,
    pub domain: String,
    pub token: String,
    pub verified_at: Option<i64>,
    pub checked_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_roles)]
#[diesel(primary_key(org_uuid, subject_kind, subject_id, role))]
pub struct ReinsOrgRole {
    pub org_uuid: String,
    /// [`SUBJECT_MEMBER`] or [`SUBJECT_GROUP`].
    pub subject_kind: String,
    pub subject_id: String,
    pub role: String,
    pub granted_at: i64,
    pub granted_by: String,
}

pub const SUBJECT_MEMBER: &str = "member";
pub const SUBJECT_GROUP: &str = "group";

#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_changes)]
#[diesel(primary_key(uuid))]
pub struct ReinsOrgChange {
    pub uuid: String,
    pub org_uuid: String,
    pub action: String,
    pub proposed_by: String,
    pub proposed_at: i64,
    pub expires_at: i64,
    pub state: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<i64>,
}

pub const CHANGE_PENDING: &str = "pending";
pub const CHANGE_APPLIED: &str = "applied";
pub const CHANGE_REJECTED: &str = "rejected";
pub const CHANGE_EXPIRED: &str = "expired";

#[derive(Clone, Debug, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_devices)]
#[diesel(primary_key(org_uuid, user_uuid, device_uuid))]
pub struct ReinsOrgDevice {
    pub org_uuid: String,
    pub user_uuid: String,
    pub device_uuid: String,
    pub key_algorithm: String,
    pub public_key: String,
    pub hardware: bool,
    /// `Vec<reins_proto::org::Attestation>` as JSON.
    pub attestations: String,
    /// `reins_proto::org::Posture` as JSON.
    pub posture: String,
    pub posture_at: i64,
    pub enrolled_at: i64,
    pub revoked_at: Option<i64>,
}

// ---------------------------------------------------------------------------------------------------------------
// Tenants and domains
// ---------------------------------------------------------------------------------------------------------------

pub fn q_org(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrg>> {
    reins_orgs::table.filter(reins_orgs::org_uuid.eq(org)).first(c).optional()
}

pub fn q_org_by_workos(c: &mut DbConnInner, workos_org_id: &str) -> QueryResult<Option<ReinsOrg>> {
    reins_orgs::table.filter(reins_orgs::workos_org_id.eq(workos_org_id)).first(c).optional()
}

pub fn q_insert_org(c: &mut DbConnInner, org: &ReinsOrg) -> QueryResult<()> {
    diesel::insert_into(reins_orgs::table).values(org).execute(c).map(|_| ())
}

pub fn q_save_org(c: &mut DbConnInner, org: &ReinsOrg) -> QueryResult<()> {
    diesel::update(reins_orgs::table.filter(reins_orgs::org_uuid.eq(&org.org_uuid))).set(org).execute(c).map(|_| ())
}

/// The tenants `user` is an active member of.
pub fn q_orgs_of_user(c: &mut DbConnInner, user: &str) -> QueryResult<Vec<ReinsOrg>> {
    let orgs: Vec<String> = users_organizations::table
        .filter(users_organizations::user_uuid.eq(user))
        .filter(users_organizations::status.ge(ACTIVE_STATUS))
        .select(users_organizations::org_uuid)
        .load(c)?;
    reins_orgs::table.filter(reins_orgs::org_uuid.eq_any(orgs)).order(reins_orgs::created_at.asc()).load(c)
}

/// The tenants `user` has a membership in, whatever its status (a removed member is still the org's to refuse).
pub fn q_orgs_of_user_any(c: &mut DbConnInner, user: &str) -> QueryResult<Vec<String>> {
    let orgs: Vec<String> = users_organizations::table
        .filter(users_organizations::user_uuid.eq(user))
        .select(users_organizations::org_uuid)
        .load(c)?;
    reins_orgs::table
        .filter(reins_orgs::org_uuid.eq_any(orgs))
        .order(reins_orgs::created_at.asc())
        .select(reins_orgs::org_uuid)
        .load(c)
}

/// The org's display name (`organizations.name`).
pub fn q_org_name(c: &mut DbConnInner, org: &str) -> QueryResult<Option<String>> {
    crate::db::schema::organizations::table
        .filter(crate::db::schema::organizations::uuid.eq(org))
        .select(crate::db::schema::organizations::name)
        .first(c)
        .optional()
}

pub fn q_domains(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgDomain>> {
    reins_org_domains::table.filter(reins_org_domains::org_uuid.eq(org)).order(reins_org_domains::domain.asc()).load(c)
}

pub fn q_domain(c: &mut DbConnInner, org: &str, domain: &str) -> QueryResult<Option<ReinsOrgDomain>> {
    reins_org_domains::table
        .filter(reins_org_domains::org_uuid.eq(org))
        .filter(reins_org_domains::domain.eq(domain))
        .first(c)
        .optional()
}

pub fn q_insert_domain(c: &mut DbConnInner, row: &ReinsOrgDomain) -> QueryResult<()> {
    diesel::insert_into(reins_org_domains::table).values(row).execute(c).map(|_| ())
}

pub fn q_delete_domain(c: &mut DbConnInner, org: &str, domain: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(
            reins_org_verified_domains::table
                .filter(reins_org_verified_domains::org_uuid.eq(org))
                .filter(reins_org_verified_domains::domain.eq(domain)),
        )
        .execute(c)?;
        diesel::delete(
            reins_org_domains::table
                .filter(reins_org_domains::org_uuid.eq(org))
                .filter(reins_org_domains::domain.eq(domain)),
        )
        .execute(c)
        .map(|_| ())
    })
}

/// The org that verified `domain`, if one did.
pub fn q_verified_owner(c: &mut DbConnInner, domain: &str) -> QueryResult<Option<String>> {
    reins_org_verified_domains::table
        .filter(reins_org_verified_domains::domain.eq(domain))
        .select(reins_org_verified_domains::org_uuid)
        .first(c)
        .optional()
}

/// Marks `org`'s pending `domain` verified; `false`, writing nothing, when another org has it verified. The
/// `reins_org_verified_domains` primary key decides between two orgs verifying at the same moment: the second insert
/// fails on it, whatever both read before.
pub fn q_mark_verified(c: &mut DbConnInner, org: &str, domain: &str, now: i64) -> QueryResult<bool> {
    let claimed = c.transaction(|c| {
        if q_domain(c, org, domain)?.is_none() {
            return Ok(false);
        }
        match q_verified_owner(c, domain)? {
            Some(owner) if owner == org => {}
            Some(_) => return Ok(false),
            None => {
                diesel::insert_into(reins_org_verified_domains::table)
                    .values((
                        reins_org_verified_domains::domain.eq(domain),
                        reins_org_verified_domains::org_uuid.eq(org),
                        reins_org_verified_domains::verified_at.eq(now),
                    ))
                    .execute(c)?;
            }
        }
        diesel::update(
            reins_org_domains::table
                .filter(reins_org_domains::org_uuid.eq(org))
                .filter(reins_org_domains::domain.eq(domain)),
        )
        .set((reins_org_domains::verified_at.eq(now), reins_org_domains::checked_at.eq(now)))
        .execute(c)?;
        Ok(true)
    });
    match claimed {
        Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)) => Ok(false),
        other => other,
    }
}

pub fn q_mark_checked(c: &mut DbConnInner, org: &str, domain: &str, now: i64) -> QueryResult<()> {
    diesel::update(
        reins_org_domains::table
            .filter(reins_org_domains::org_uuid.eq(org))
            .filter(reins_org_domains::domain.eq(domain)),
    )
    .set(reins_org_domains::checked_at.eq(now))
    .execute(c)
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------------------------
// Members, groups and roles
// ---------------------------------------------------------------------------------------------------------------

/// `user`'s membership in `org`, whatever its status.
pub fn q_membership(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Option<Membership>> {
    users_organizations::table
        .filter(users_organizations::org_uuid.eq(org))
        .filter(users_organizations::user_uuid.eq(user))
        .first(c)
        .optional()
}

pub fn q_membership_by_uuid(c: &mut DbConnInner, org: &str, membership: &str) -> QueryResult<Option<Membership>> {
    users_organizations::table
        .filter(users_organizations::org_uuid.eq(org))
        .filter(users_organizations::uuid.eq(membership))
        .first(c)
        .optional()
}

pub fn q_members(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<Membership>> {
    users_organizations::table.filter(users_organizations::org_uuid.eq(org)).load(c)
}

/// The groups `membership` is in.
pub fn q_groups_of(c: &mut DbConnInner, membership: &str) -> QueryResult<Vec<String>> {
    groups_users::table
        .filter(groups_users::users_organizations_uuid.eq(membership))
        .select(groups_users::groups_uuid)
        .load(c)
}

/// The roles of `membership`: its own and those of every group it is in. Empty when the membership is not active.
pub fn q_roles_of(c: &mut DbConnInner, org: &str, membership: &str) -> QueryResult<Vec<Role>> {
    let Some(m) = q_membership_by_uuid(c, org, membership)? else {
        return Ok(Vec::new());
    };
    if m.status < ACTIVE_STATUS {
        return Ok(Vec::new());
    }
    let groups = q_groups_of(c, membership)?;
    let direct = reins_org_roles::table
        .filter(reins_org_roles::org_uuid.eq(org))
        .filter(reins_org_roles::subject_kind.eq(SUBJECT_MEMBER))
        .filter(reins_org_roles::subject_id.eq(membership))
        .select(reins_org_roles::role);
    let mut names: Vec<String> = direct.load(c)?;
    names.extend(
        reins_org_roles::table
            .filter(reins_org_roles::org_uuid.eq(org))
            .filter(reins_org_roles::subject_kind.eq(SUBJECT_GROUP))
            .filter(reins_org_roles::subject_id.eq_any(&groups))
            .select(reins_org_roles::role)
            .load::<String>(c)?,
    );
    let mut roles: Vec<Role> = names.iter().filter_map(|n| Role::parse(n)).collect();
    roles.sort();
    roles.dedup();
    Ok(roles)
}

/// The roles granted to a subject itself (not through groups).
pub fn q_subject_roles(c: &mut DbConnInner, org: &str, kind: &str, id: &str) -> QueryResult<Vec<Role>> {
    let names: Vec<String> = reins_org_roles::table
        .filter(reins_org_roles::org_uuid.eq(org))
        .filter(reins_org_roles::subject_kind.eq(kind))
        .filter(reins_org_roles::subject_id.eq(id))
        .select(reins_org_roles::role)
        .load(c)?;
    Ok(names.iter().filter_map(|n| Role::parse(n)).collect())
}

pub fn q_role_rows(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgRole>> {
    reins_org_roles::table.filter(reins_org_roles::org_uuid.eq(org)).load(c)
}

pub fn q_grant(c: &mut DbConnInner, row: &ReinsOrgRole) -> QueryResult<()> {
    c.transaction(|c| {
        q_revoke(c, &row.org_uuid, &row.subject_kind, &row.subject_id, &row.role)?;
        diesel::insert_into(reins_org_roles::table).values(row).execute(c).map(|_| ())
    })
}

pub fn q_revoke(c: &mut DbConnInner, org: &str, kind: &str, id: &str, role: &str) -> QueryResult<()> {
    diesel::delete(
        reins_org_roles::table
            .filter(reins_org_roles::org_uuid.eq(org))
            .filter(reins_org_roles::subject_kind.eq(kind))
            .filter(reins_org_roles::subject_id.eq(id))
            .filter(reins_org_roles::role.eq(role)),
    )
    .execute(c)
    .map(|_| ())
}

/// Every role grant to `id` (a member or a group that goes away).
pub fn q_revoke_all(c: &mut DbConnInner, org: &str, kind: &str, id: &str) -> QueryResult<()> {
    diesel::delete(
        reins_org_roles::table
            .filter(reins_org_roles::org_uuid.eq(org))
            .filter(reins_org_roles::subject_kind.eq(kind))
            .filter(reins_org_roles::subject_id.eq(id)),
    )
    .execute(c)
    .map(|_| ())
}

/// The users (uuids) whose active membership holds a role granting `cap`, directly or through a group.
pub fn q_holders(c: &mut DbConnInner, org: &str, cap: Capability) -> QueryResult<Vec<String>> {
    let roles: Vec<&str> = Role::ALL.iter().filter(|r| role_grants(**r, cap)).map(|r| r.as_str()).collect();
    let rows: Vec<(String, String)> = reins_org_roles::table
        .filter(reins_org_roles::org_uuid.eq(org))
        .filter(reins_org_roles::role.eq_any(&roles))
        .select((reins_org_roles::subject_kind, reins_org_roles::subject_id))
        .load(c)?;
    let mut memberships: Vec<String> =
        rows.iter().filter(|(k, _)| k == SUBJECT_MEMBER).map(|(_, id)| id.clone()).collect();
    let group_ids: Vec<&String> = rows.iter().filter(|(k, _)| k == SUBJECT_GROUP).map(|(_, id)| id).collect();
    if !group_ids.is_empty() {
        memberships.extend(
            groups_users::table
                .filter(groups_users::groups_uuid.eq_any(group_ids))
                .select(groups_users::users_organizations_uuid)
                .load::<String>(c)?,
        );
    }
    let mut users: Vec<String> = users_organizations::table
        .filter(users_organizations::org_uuid.eq(org))
        .filter(users_organizations::uuid.eq_any(&memberships))
        .filter(users_organizations::status.ge(ACTIVE_STATUS))
        .select(users_organizations::user_uuid)
        .load(c)?;
    users.sort();
    users.dedup();
    Ok(users)
}

/// The org's groups: (uuid, name, external id).
pub fn q_groups(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<(String, String, Option<String>)>> {
    groups::table
        .filter(groups::organizations_uuid.eq(org))
        .select((groups::uuid, groups::name, groups::external_id))
        .order(groups::name.asc())
        .load(c)
}

pub fn q_group_members(c: &mut DbConnInner, group: &str) -> QueryResult<Vec<String>> {
    groups_users::table
        .filter(groups_users::groups_uuid.eq(group))
        .select(groups_users::users_organizations_uuid)
        .load(c)
}

// ---------------------------------------------------------------------------------------------------------------
// Two-person changes
// ---------------------------------------------------------------------------------------------------------------

pub fn q_insert_change(c: &mut DbConnInner, row: &ReinsOrgChange) -> QueryResult<()> {
    diesel::insert_into(reins_org_changes::table).values(row).execute(c).map(|_| ())
}

pub fn q_change(c: &mut DbConnInner, org: &str, uuid: &str) -> QueryResult<Option<ReinsOrgChange>> {
    reins_org_changes::table
        .filter(reins_org_changes::org_uuid.eq(org))
        .filter(reins_org_changes::uuid.eq(uuid))
        .first(c)
        .optional()
}

pub fn q_pending_changes(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgChange>> {
    reins_org_changes::table
        .filter(reins_org_changes::org_uuid.eq(org))
        .filter(reins_org_changes::state.eq(CHANGE_PENDING))
        .order(reins_org_changes::proposed_at.asc())
        .load(c)
}

/// Moves a pending change to `state`; `false` when it was no longer pending (decided by someone else meanwhile).
pub fn q_decide(c: &mut DbConnInner, uuid: &str, state: &str, by: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_changes::table
            .filter(reins_org_changes::uuid.eq(uuid))
            .filter(reins_org_changes::state.eq(CHANGE_PENDING)),
    )
    .set((
        reins_org_changes::state.eq(state),
        reins_org_changes::decided_by.eq(by),
        reins_org_changes::decided_at.eq(at),
    ))
    .execute(c)?;
    Ok(n == 1)
}

// ---------------------------------------------------------------------------------------------------------------
// Enrolled devices
// ---------------------------------------------------------------------------------------------------------------

/// Enrolls (or re-enrolls) a device: a new key replaces the old one.
pub fn q_upsert_device(c: &mut DbConnInner, row: &ReinsOrgDevice) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(
            reins_org_devices::table
                .filter(reins_org_devices::org_uuid.eq(&row.org_uuid))
                .filter(reins_org_devices::user_uuid.eq(&row.user_uuid))
                .filter(reins_org_devices::device_uuid.eq(&row.device_uuid)),
        )
        .execute(c)?;
        diesel::insert_into(reins_org_devices::table).values(row).execute(c).map(|_| ())
    })
}

pub fn q_device(c: &mut DbConnInner, org: &str, user: &str, device: &str) -> QueryResult<Option<ReinsOrgDevice>> {
    reins_org_devices::table
        .filter(reins_org_devices::org_uuid.eq(org))
        .filter(reins_org_devices::user_uuid.eq(user))
        .filter(reins_org_devices::device_uuid.eq(device))
        .first(c)
        .optional()
}

pub fn q_set_posture(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    device: &str,
    posture: &str,
    at: i64,
) -> QueryResult<()> {
    diesel::update(
        reins_org_devices::table
            .filter(reins_org_devices::org_uuid.eq(org))
            .filter(reins_org_devices::user_uuid.eq(user))
            .filter(reins_org_devices::device_uuid.eq(device)),
    )
    .set((reins_org_devices::posture.eq(posture), reins_org_devices::posture_at.eq(at)))
    .execute(c)
    .map(|_| ())
}

/// Revokes every device `user` enrolled to `org`; returns how many were active.
pub fn q_revoke_devices(c: &mut DbConnInner, org: &str, user: &str, at: i64) -> QueryResult<usize> {
    diesel::update(
        reins_org_devices::table
            .filter(reins_org_devices::org_uuid.eq(org))
            .filter(reins_org_devices::user_uuid.eq(user))
            .filter(reins_org_devices::revoked_at.is_null()),
    )
    .set(reins_org_devices::revoked_at.eq(at))
    .execute(c)
}

pub fn q_devices_of_org(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgDevice>> {
    reins_org_devices::table.filter(reins_org_devices::org_uuid.eq(org)).load(c)
}

/// Everything enrolled by `user` (account deletion).
pub fn q_delete_user_devices(c: &mut DbConnInner, user: &str) -> QueryResult<()> {
    diesel::delete(reins_org_devices::table.filter(reins_org_devices::user_uuid.eq(user))).execute(c).map(|_| ())
}

// ---------------------------------------------------------------------------------------------------------------
// Cutting a member off
// ---------------------------------------------------------------------------------------------------------------

/// What [`q_cut_off_user`] removed, and what the caller still has to do outside the database.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CutOff {
    /// Signed-in devices (their tokens stop working at once).
    pub devices: usize,
    /// AI and desktop connections, with their refresh tokens.
    pub connections: usize,
    /// The SSO sessions the devices signed in with, to end at the provider.
    pub sso_sessions: Vec<String>,
    /// The approval device's push token, to tell it once more.
    pub push_token: Option<String>,
}

/// Signs `user` out everywhere and disables the account: every Vaultwarden device, the approval device and the phone
/// roster, the SSO session mappings, every AI and desktop connection with its refresh tokens. One transaction, the
/// caller's.
pub fn q_cut_off_user(c: &mut DbConnInner, user: &str) -> QueryResult<CutOff> {
    use crate::db::schema::{
        devices, reins_connections, reins_devices, reins_refresh_tokens, reins_sso_sessions, users,
    };
    let push_token: Option<Option<String>> = reins_devices::table
        .filter(reins_devices::user_uuid.eq(user))
        .select(reins_devices::fcm_token)
        .first(c)
        .optional()?;
    let sso_sessions: Vec<String> = reins_sso_sessions::table
        .filter(reins_sso_sessions::user_uuid.eq(user))
        .select(reins_sso_sessions::session_id)
        .load(c)?;
    let connections: Vec<String> = reins_connections::table
        .filter(reins_connections::user_uuid.eq(user))
        .select(reins_connections::uuid)
        .load(c)?;
    diesel::delete(reins_refresh_tokens::table.filter(reins_refresh_tokens::connection_uuid.eq_any(&connections)))
        .execute(c)?;
    diesel::delete(reins_connections::table.filter(reins_connections::user_uuid.eq(user))).execute(c)?;
    diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(user))).execute(c)?;
    diesel::delete(reins_sso_sessions::table.filter(reins_sso_sessions::user_uuid.eq(user))).execute(c)?;
    super::reins_roster::q_delete_all(c, user)?;
    let devices = diesel::delete(devices::table.filter(devices::user_uuid.eq(user))).execute(c)?;
    diesel::update(users::table.filter(users::uuid.eq(user))).set(users::enabled.eq(false)).execute(c)?;
    Ok(CutOff {
        devices,
        connections: connections.len(),
        sso_sessions,
        push_token: push_token.flatten(),
    })
}

/// Ends a membership: revoked (it can be restored) or deleted, and in both cases out of every group and without its
/// own role grants.
pub fn q_end_membership(c: &mut DbConnInner, org: &str, membership: &str, delete: bool) -> QueryResult<()> {
    diesel::delete(groups_users::table.filter(groups_users::users_organizations_uuid.eq(membership))).execute(c)?;
    q_revoke_all(c, org, SUBJECT_MEMBER, membership)?;
    let target = users_organizations::table
        .filter(users_organizations::org_uuid.eq(org))
        .filter(users_organizations::uuid.eq(membership));
    if delete {
        diesel::delete(target).execute(c)?;
    } else {
        // Vaultwarden's revocation: the status moves below `Revoked` and can move back.
        diesel::update(target.filter(users_organizations::status.gt(MembershipStatus::Revoked as i32)))
            .set(users_organizations::status.eq(users_organizations::status - 128))
            .execute(c)?;
    }
    Ok(())
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use super::super::reins_device::test_db;
    use super::*;
    use crate::db::schema::{groups, groups_users, organizations, users_organizations};

    pub fn org_row(org: &str) -> ReinsOrg {
        ReinsOrg {
            org_uuid: org.to_owned(),
            tier: "business".to_owned(),
            identity: "none".to_owned(),
            workos_org_id: None,
            directory_id: None,
            github_org: None,
            two_person: true,
            created_by: "u1".to_owned(),
            created_at: 1,
            updated_at: 1,
        }
    }

    pub fn add_org(c: &mut DbConnInner, org: &str) {
        diesel::insert_into(organizations::table)
            .values((
                organizations::uuid.eq(org),
                organizations::name.eq(format!("Org {org}")),
                organizations::billing_email.eq("owner@acme.com"),
            ))
            .execute(c)
            .unwrap();
        q_insert_org(c, &org_row(org)).unwrap();
    }

    /// A membership `m` of `user` in `org` with `status`.
    pub fn add_member(c: &mut DbConnInner, org: &str, m: &str, user: &str, status: i32) {
        diesel::insert_into(users_organizations::table)
            .values((
                users_organizations::uuid.eq(m),
                users_organizations::user_uuid.eq(user),
                users_organizations::org_uuid.eq(org),
                users_organizations::access_all.eq(false),
                users_organizations::akey.eq(""),
                users_organizations::status.eq(status),
                users_organizations::atype.eq(2),
            ))
            .execute(c)
            .unwrap();
    }

    pub fn add_group(c: &mut DbConnInner, org: &str, g: &str, members: &[&str]) {
        let now = chrono::Utc::now().naive_utc();
        diesel::insert_into(groups::table)
            .values((
                groups::uuid.eq(g),
                groups::organizations_uuid.eq(org),
                groups::name.eq(format!("Group {g}")),
                groups::access_all.eq(false),
                groups::creation_date.eq(now),
                groups::revision_date.eq(now),
            ))
            .execute(c)
            .unwrap();
        for m in members {
            diesel::insert_into(groups_users::table)
                .values((groups_users::groups_uuid.eq(g), groups_users::users_organizations_uuid.eq(*m)))
                .execute(c)
                .unwrap();
        }
    }

    pub fn grant(c: &mut DbConnInner, org: &str, kind: &str, id: &str, role: Role) {
        q_grant(
            c,
            &ReinsOrgRole {
                org_uuid: org.to_owned(),
                subject_kind: kind.to_owned(),
                subject_id: id.to_owned(),
                role: role.as_str().to_owned(),
                granted_at: 1,
                granted_by: "u1".to_owned(),
            },
        )
        .unwrap();
    }

    fn domain(org: &str, d: &str) -> ReinsOrgDomain {
        ReinsOrgDomain {
            org_uuid: org.to_owned(),
            domain: d.to_owned(),
            token: "t".to_owned(),
            verified_at: None,
            checked_at: None,
            created_at: 1,
        }
    }

    #[test]
    fn a_domain_is_verified_by_one_org_only() {
        let mut c = test_db();
        add_org(&mut c, "o1");
        add_org(&mut c, "o2");
        q_insert_domain(&mut c, &domain("o1", "acme.com")).unwrap();
        q_insert_domain(&mut c, &domain("o2", "acme.com")).unwrap();
        assert_eq!(q_verified_owner(&mut c, "acme.com").unwrap(), None);
        assert!(q_mark_verified(&mut c, "o2", "acme.com", 5).unwrap());
        assert!(!q_mark_verified(&mut c, "o1", "acme.com", 6).unwrap());
        assert_eq!(q_verified_owner(&mut c, "acme.com").unwrap().as_deref(), Some("o2"));
        assert_eq!(q_domain(&mut c, "o1", "acme.com").unwrap().unwrap().verified_at, None);
        // Verifying again is harmless; an org that never added the domain verifies nothing.
        assert!(q_mark_verified(&mut c, "o2", "acme.com", 7).unwrap());
        assert!(!q_mark_verified(&mut c, "o3", "other.com", 7).unwrap());
        // Removing it frees it for another org.
        q_delete_domain(&mut c, "o2", "acme.com").unwrap();
        assert!(q_mark_verified(&mut c, "o1", "acme.com", 8).unwrap());
    }

    /// Two orgs verify the same domain at the same moment, on two connections to one database file: exactly one wins,
    /// and the loser's row stays pending.
    #[test]
    fn concurrent_verifications_of_one_domain_have_one_winner() {
        use diesel::Connection;
        let dir = std::env::temp_dir().join(format!("reins-org-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = dir.join("db.sqlite3").to_string_lossy().into_owned();
        let open = || {
            let mut c = SqliteConnection::establish(&url).unwrap();
            diesel::sql_query("PRAGMA foreign_keys = OFF").execute(&mut c).unwrap();
            diesel::sql_query("PRAGMA busy_timeout = 5000").execute(&mut c).unwrap();
            DbConnInner::Sqlite(c)
        };
        {
            use diesel_migrations::MigrationHarness;
            let mut c = open();
            let DbConnInner::Sqlite(s) = &mut c;
            s.run_pending_migrations(crate::db::sqlite_migrations::MIGRATIONS).unwrap();
            for org in ["o1", "o2"] {
                add_org(&mut c, org);
                q_insert_domain(&mut c, &domain(org, "acme.com")).unwrap();
            }
        }
        for _ in 0..10 {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let wins: Vec<bool> = ["o1", "o2"]
                .map(|org| {
                    let barrier = std::sync::Arc::clone(&barrier);
                    let mut c = open();
                    std::thread::spawn(move || {
                        barrier.wait();
                        loop {
                            match q_mark_verified(&mut c, org, "acme.com", 5) {
                                Ok(won) => return won,
                                // SQLite's writer lock, not the claim: try again.
                                Err(e) if e.to_string().contains("locked") || e.to_string().contains("busy") => {}
                                Err(e) => panic!("{e}"),
                            }
                        }
                    })
                })
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect();
            assert_eq!(wins.iter().filter(|w| **w).count(), 1, "{wins:?}");
            let mut c = open();
            let owner = q_verified_owner(&mut c, "acme.com").unwrap().unwrap();
            let loser = if owner == "o1" {
                "o2"
            } else {
                "o1"
            };
            assert_eq!(q_domain(&mut c, loser, "acme.com").unwrap().unwrap().verified_at, None);
            q_delete_domain(&mut c, &owner, "acme.com").unwrap();
            q_insert_domain(&mut c, &domain(&owner, "acme.com")).unwrap();
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn roles_come_from_the_member_and_their_groups() {
        let mut c = test_db();
        add_org(&mut c, "o1");
        add_member(&mut c, "o1", "m1", "u1", ACTIVE_STATUS);
        add_member(&mut c, "o1", "m2", "u2", ACTIVE_STATUS);
        add_group(&mut c, "o1", "g1", &["m1"]);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", Role::Auditor);
        grant(&mut c, "o1", SUBJECT_GROUP, "g1", Role::Approver);
        grant(&mut c, "o1", SUBJECT_GROUP, "g1", Role::Auditor);
        assert_eq!(q_roles_of(&mut c, "o1", "m1").unwrap(), [Role::Approver, Role::Auditor]);
        assert!(q_roles_of(&mut c, "o1", "m2").unwrap().is_empty());
        assert_eq!(q_holders(&mut c, "o1", Capability::Approve).unwrap(), ["u1"]);
        assert_eq!(q_holders(&mut c, "o1", Capability::ViewAdminEvents).unwrap(), ["u1"]);
        assert!(q_holders(&mut c, "o1", Capability::SetUpOrg).unwrap().is_empty());
        // Another org's grants never count.
        add_org(&mut c, "o2");
        grant(&mut c, "o2", SUBJECT_MEMBER, "m2", Role::OrgOwner);
        assert!(q_roles_of(&mut c, "o1", "m2").unwrap().is_empty());
        assert!(q_holders(&mut c, "o1", Capability::SetUpOrg).unwrap().is_empty());
    }

    #[test]
    fn revoked_memberships_hold_no_capability() {
        let mut c = test_db();
        add_org(&mut c, "o1");
        add_member(&mut c, "o1", "m1", "u1", ACTIVE_STATUS - 128);
        add_member(&mut c, "o1", "m2", "u2", MembershipStatus::Invited as i32);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", Role::OrgOwner);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m2", Role::OrgOwner);
        assert!(q_roles_of(&mut c, "o1", "m1").unwrap().is_empty());
        assert!(q_holders(&mut c, "o1", Capability::SetUpOrg).unwrap().is_empty());
        assert_eq!(q_orgs_of_user(&mut c, "u1").unwrap().len(), 0);
    }

    #[test]
    fn a_decided_change_cannot_be_decided_again() {
        let mut c = test_db();
        add_org(&mut c, "o1");
        q_insert_change(
            &mut c,
            &ReinsOrgChange {
                uuid: "ch1".to_owned(),
                org_uuid: "o1".to_owned(),
                action: "{}".to_owned(),
                proposed_by: "u1".to_owned(),
                proposed_at: 1,
                expires_at: 100,
                state: CHANGE_PENDING.to_owned(),
                decided_by: None,
                decided_at: None,
            },
        )
        .unwrap();
        assert_eq!(q_pending_changes(&mut c, "o1").unwrap().len(), 1);
        assert!(q_decide(&mut c, "ch1", CHANGE_APPLIED, "u2", 5).unwrap());
        assert!(!q_decide(&mut c, "ch1", CHANGE_REJECTED, "u3", 6).unwrap());
        let ch = q_change(&mut c, "o1", "ch1").unwrap().unwrap();
        assert_eq!((ch.state.as_str(), ch.decided_by.as_deref()), (CHANGE_APPLIED, Some("u2")));
        assert!(q_change(&mut c, "o2", "ch1").unwrap().is_none());
    }

    #[test]
    fn devices_are_revoked_per_user() {
        let mut c = test_db();
        add_org(&mut c, "o1");
        let dev = |user: &str, d: &str| ReinsOrgDevice {
            org_uuid: "o1".to_owned(),
            user_uuid: user.to_owned(),
            device_uuid: d.to_owned(),
            key_algorithm: "p256".to_owned(),
            public_key: "k".to_owned(),
            hardware: true,
            attestations: "[]".to_owned(),
            posture: "{}".to_owned(),
            posture_at: 1,
            enrolled_at: 1,
            revoked_at: None,
        };
        q_upsert_device(&mut c, &dev("u1", "d1")).unwrap();
        q_upsert_device(&mut c, &dev("u1", "d1")).unwrap();
        q_upsert_device(&mut c, &dev("u2", "d2")).unwrap();
        assert_eq!(q_revoke_devices(&mut c, "o1", "u1", 9).unwrap(), 1);
        assert_eq!(q_revoke_devices(&mut c, "o1", "u1", 10).unwrap(), 0);
        assert_eq!(q_device(&mut c, "o1", "u1", "d1").unwrap().unwrap().revoked_at, Some(9));
        assert_eq!(q_device(&mut c, "o1", "u2", "d2").unwrap().unwrap().revoked_at, None);
    }
}
