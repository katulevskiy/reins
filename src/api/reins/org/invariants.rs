//! The rules every change to a membership or a role goes through, whoever makes it: a two-person admin change
//! (`admin`), the directory (`scim`, through `deprovision`), or the API. One place, so the paths can't drift apart.
//!
//! - **An org always has an Org owner.** An owner counts when they hold the role directly and their state in this org is
//!   active (`reins_org_members`, the org's own rows; never `users.enabled`); everything here recomputes that from the database on every call, and no stored flag
//!   decides anything. Removing or demoting the last one is refused to admins. When the directory removes the last one
//!   (a SCIM delete or suspend, which nobody in the org can refuse), the owner role and membership are kept and the
//!   account is cut off like any removed member's. The tenant then has no owner that counts because of its own
//!   directory, which is what [`recovery_mode`] answers: only then may two Security admins together make a new owner. `owner_recovery_at`
//!   only *shows* this (console, events), and is cleared as soon as an owner counts again.
//! - **Org owner and Security admin are people's roles**, never a group's (`AdminAction::check_shape`).

use reins_proto::org::{Role, Subject};

use super::OrgError;
use crate::db::{
    DbConnInner,
    models::reins_org::{self, SUBJECT_MEMBER, state},
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

/// The owners that count right now: memberships holding Org owner directly whose state in this org is active. Read
/// from this org's own rows only.
pub fn owners(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    Ok(owner_states(c, org)?.into_iter().filter(|(_, st)| st == state::ACTIVE).map(|(m, _)| m).collect())
}

/// Every membership holding Org owner directly, with its state in this org.
fn owner_states(c: &mut DbConnInner, org: &str) -> Result<Vec<(String, String)>, OrgError> {
    let mut out = Vec::new();
    for row in reins_org::q_role_rows(c, org)? {
        if row.subject_kind != SUBJECT_MEMBER || row.role != Role::OrgOwner.as_str() {
            continue;
        }
        if let Some(m) = reins_org::q_member_state_by_membership(c, org, &row.subject_id)? {
            out.push((row.subject_id, m.state));
        }
    }
    Ok(out)
}

/// Owner recovery: no owner counts **and** this org's own directory is why (an owner it suspended or deprovisioned
/// still holds the role). Nothing outside this org's rows can make it true: not another tenant, not a global account
/// flag, not an admin's removal (refused for the last owner anyway).
pub fn recovery_mode(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    let owners = owner_states(c, org)?;
    Ok(!owners.iter().any(|(_, st)| st == state::ACTIVE)
        && owners.iter().any(|(_, st)| state::BY_DIRECTORY.contains(&st.as_str())))
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
