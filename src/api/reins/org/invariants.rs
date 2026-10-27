//! The rules every change to a membership or a role goes through, whoever makes it: a two-person admin change
//! (`admin`), the directory (`scim`, through `deprovision`), or the API. One place, so the paths can't drift apart.
//!
//! - **An org always has an Org owner.** Removing or demoting the last one is refused to admins. When the directory
//!   removes the last one (a SCIM delete or suspend, which nobody in the org can refuse), the owner role and membership
//!   are kept, the account is cut off like any removed member's, and the tenant is flagged `owner_recovery_at` (shown
//!   in the console, recorded in the events). Then, and only then, two Security admins may together make a new owner.
//! - **Org owner and Security admin are people's roles**, never a group's (`AdminAction::check_shape`).

use reins_proto::org::{Role, Subject};

use super::OrgError;
use crate::db::{
    DbConnInner,
    models::reins_org::{self, ACTIVE_STATUS, SUBJECT_MEMBER},
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

/// The memberships that hold Org owner directly and are active.
pub fn owners(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    let mut out = Vec::new();
    for row in reins_org::q_role_rows(c, org)? {
        if row.subject_kind == SUBJECT_MEMBER
            && row.role == Role::OrgOwner.as_str()
            && reins_org::q_membership_by_uuid(c, org, &row.subject_id)?.is_some_and(|m| m.status >= ACTIVE_STATUS)
        {
            out.push(row.subject_id);
        }
    }
    Ok(out)
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

/// Whether the tenant waits for a new owner (the directory removed its last one).
pub fn owner_recovery(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    Ok(reins_org::q_org(c, org)?.is_some_and(|o| o.owner_recovery_at.is_some()))
}

/// Flags or clears owner recovery.
pub fn set_owner_recovery(c: &mut DbConnInner, org: &str, at: Option<i64>) -> Result<(), OrgError> {
    if let Some(mut tenant) = reins_org::q_org(c, org)? {
        tenant.owner_recovery_at = at;
        reins_org::q_save_org(c, &tenant)?;
    }
    Ok(())
}
