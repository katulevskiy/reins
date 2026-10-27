//! The rules every change to a membership or a role goes through, whoever makes it: a two-person admin change
//! (`admin`), the directory (`scim`, through `deprovision`), or the API. One place, so the paths can't drift apart.
//!
//! - **An org always has an Org owner.** An owner counts when they hold the role directly, their membership is active
//!   and their account is enabled; everything here recomputes that from the database on every call, and no stored flag
//!   decides anything. Removing or demoting the last one is refused to admins. When the directory removes the last one
//!   (a SCIM delete or suspend, which nobody in the org can refuse), the owner role and membership are kept and the
//!   account is cut off like any removed member's. The tenant then has no owner that counts, which is what
//!   [`recovery_mode`] answers: only then may two Security admins together make a new owner. `owner_recovery_at`
//!   only *shows* this (console, events), and is cleared as soon as an owner counts again.
//! - **Org owner and Security admin are people's roles**, never a group's (`AdminAction::check_shape`).

use reins_proto::org::{Role, Subject};

use diesel::prelude::*;

use super::OrgError;
use crate::db::{
    DbConnInner,
    models::reins_org::{self, ACTIVE_STATUS, SUBJECT_MEMBER},
    schema::users,
};

/// What a path that cannot be refused (the directory) does when it would end the last owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnLastOwner {
    /// An admin's change: refuse it.
    Refuse,
    /// The directory's: keep the owner, flag the tenant.
    KeepOwner,
}

/// How a membership may end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndPlan {
    /// End it as asked.
    End,
    /// Keep the membership and its owner role (cut the account off all the same) and flag owner recovery.
    KeepOwner,
}

/// The owners that count right now: memberships holding Org owner directly, active, of an enabled account.
pub fn owners(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    let mut out = Vec::new();
    for row in reins_org::q_role_rows(c, org)? {
        if row.subject_kind != SUBJECT_MEMBER || row.role != Role::OrgOwner.as_str() {
            continue;
        }
        let Some(m) = reins_org::q_membership_by_uuid(c, org, &row.subject_id)? else {
            continue;
        };
        let enabled: bool = users::table
            .filter(users::uuid.eq(m.user_uuid.to_string()))
            .select(users::enabled)
            .first(c)
            .optional()?
            .unwrap_or(false);
        if m.status >= ACTIVE_STATUS && enabled {
            out.push(row.subject_id);
        }
    }
    Ok(out)
}

/// Whether the org has no owner that counts: the only time Security admins may (two together) make one.
pub fn recovery_mode(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    Ok(owners(c, org)?.is_empty())
}

/// Whether `membership` is the org's only active Org owner.
pub fn is_last_owner(c: &mut DbConnInner, org: &str, membership: &str) -> Result<bool, OrgError> {
    let owners = owners(c, org)?;
    Ok(owners.len() == 1 && owners[0] == membership)
}

const KEEP_ONE: OrgError = OrgError::Invalid("An organization keeps at least one Org owner.");

/// Before ending `membership` (removal, SCIM suspend or delete).
pub fn plan_membership_end(
    c: &mut DbConnInner,
    org: &str,
    membership: &str,
    on_last: OnLastOwner,
) -> Result<EndPlan, OrgError> {
    if !is_last_owner(c, org, membership)? {
        return Ok(EndPlan::End);
    }
    match on_last {
        OnLastOwner::Refuse => Err(KEEP_ONE),
        OnLastOwner::KeepOwner => Ok(EndPlan::KeepOwner),
    }
}

/// Before taking `role` from `subject`.
pub fn check_role_removal(c: &mut DbConnInner, org: &str, subject: &Subject, role: Role) -> Result<(), OrgError> {
    if role == Role::OrgOwner
        && let Subject::Member(m) = subject
        && is_last_owner(c, org, m)?
    {
        return Err(KEEP_ONE);
    }
    Ok(())
}

/// The displayed flag (console, events): set when the directory removed the last owner. Never read to decide.
pub fn owner_recovery_shown(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    Ok(reins_org::q_org(c, org)?.is_some_and(|o| o.owner_recovery_at.is_some()))
}

/// Sets the displayed flag.
pub fn show_owner_recovery(c: &mut DbConnInner, org: &str, at: i64) -> Result<(), OrgError> {
    if let Some(mut tenant) = reins_org::q_org(c, org)?
        && tenant.owner_recovery_at.is_none()
    {
        tenant.owner_recovery_at = Some(at);
        reins_org::q_save_org(c, &tenant)?;
    }
    Ok(())
}

/// Clears the displayed flag once an owner counts again; returns whether it was cleared. Called after anything that
/// can bring an owner back: a grant, a directory restore, an account enabled again.
pub fn refresh_owner_recovery(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    if let Some(mut tenant) = reins_org::q_org(c, org)?
        && tenant.owner_recovery_at.is_some()
        && !owners(c, org)?.is_empty()
    {
        tenant.owner_recovery_at = None;
        reins_org::q_save_org(c, &tenant)?;
        return Ok(true);
    }
    Ok(false)
}
