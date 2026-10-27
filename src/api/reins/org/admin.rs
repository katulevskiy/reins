//! Roles and two-person admin changes (05 §3: "Admin actions use two-person approval by default … An admin can't
//! approve their own change").
//!
//! - Every [`AdminAction`] needs a capability, judged by its target ([`AdminAction::capability`]): removing someone who
//!   holds a privileged role takes what making them one takes.
//! - [`propose`] applies at once only when two-person approval is off, or nobody *else* holds the capability (an org's
//!   first owner setting it up; the event says so). Otherwise the change waits, up to [`CHANGE_TTL_SECS`].
//! - [`approve`] refuses the proposer, whatever roles or groups they approve through, and re-checks everything as of
//!   now: the proposer must still hold the capability, the approver must hold it against the target's *current* roles,
//!   and the action must still be valid (the last Org owner is never removed or demoted).

use diesel::prelude::*;
use reins_proto::org::{AdminAction, Capability, IdentityKind, Role, Subject};
use serde_json::json;

use super::{
    OrgError, can, deprovision, event,
    invariants::{self, OnLastOwner},
    workos_api::VerifiedBinding,
};
use crate::db::{
    DbConnInner,
    models::reins_org::{
        self, ACTIVE_STATUS, CHANGE_APPLIED, CHANGE_EXPIRED, CHANGE_PENDING, CHANGE_REJECTED, ReinsOrgChange,
        ReinsOrgRole, SUBJECT_GROUP, SUBJECT_MEMBER,
    },
    schema::groups,
};

/// How long a change waits for its second person.
pub const CHANGE_TTL_SECS: i64 = 7 * 24 * 3600;

/// What [`propose`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Proposed {
    Applied,
    /// Waiting for a second person: the change's id.
    Pending(String),
}

/// What applying an action left to do after the transaction commits (a removed member's [`deprovision::finish`]).
pub type Followups = Vec<deprovision::Removal>;

/// The roles the action's subject holds now: a member's own and group roles, a group's roles.
pub fn target_roles(c: &mut DbConnInner, org: &str, action: &AdminAction) -> Result<Vec<Role>, OrgError> {
    Ok(match action {
        AdminAction::GrantRole {
            subject,
            ..
        }
        | AdminAction::RevokeRole {
            subject,
            ..
        } => match subject {
            Subject::Member(m) => reins_org::q_roles_of(c, org, m)?,
            Subject::Group(g) => reins_org::q_subject_roles(c, org, SUBJECT_GROUP, g)?,
        },
        AdminAction::RemoveMember {
            member,
        } => reins_org::q_roles_of(c, org, member)?,
        _ => Vec::new(),
    })
}

/// The capability `action` needs right now.
pub fn needed(c: &mut DbConnInner, org: &str, action: &AdminAction) -> Result<Capability, OrgError> {
    if is_owner_recovery(c, org, action)? {
        return Ok(Capability::AssignRoles);
    }
    Ok(action.capability(&target_roles(c, org, action)?))
}

/// Making a new Org owner while the tenant waits for one (the directory removed the last): Security admins may, two of
/// them together ([`invariants`]).
fn is_owner_recovery(c: &mut DbConnInner, org: &str, action: &AdminAction) -> Result<bool, OrgError> {
    Ok(matches!(
        action,
        AdminAction::GrantRole {
            subject: Subject::Member(_),
            role: Role::OrgOwner
        }
    ) && invariants::owner_recovery(c, org)?)
}

fn other_holders(c: &mut DbConnInner, org: &str, cap: Capability, user: &str) -> Result<usize, OrgError> {
    Ok(reins_org::q_holders(c, org, cap)?.into_iter().filter(|u| u != user).count())
}

/// Proposes `action`; applies it at once when nobody else could approve it or two-person approval is off.
pub fn propose(
    c: &mut DbConnInner,
    org: &str,
    proposer: &str,
    action: &AdminAction,
    binding: Option<&VerifiedBinding>,
    now: i64,
) -> Result<(Proposed, Followups), OrgError> {
    action.check_shape().map_err(OrgError::Invalid)?;
    c.transaction(|c| {
        let tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
        let cap = needed(c, org, action)?;
        if !can(c, org, proposer, cap)? {
            return Err(OrgError::Forbidden);
        }
        validate(c, org, action, binding)?;
        let detail = serde_json::to_value(action).unwrap_or_default();
        let others = other_holders(c, org, cap, proposer)?;
        if is_owner_recovery(c, org, action)? && others == 0 {
            return Err(OrgError::Invalid("A new Org owner takes two Security admins. Ask Reins support."));
        }
        let alone = !tenant.two_person && !is_owner_recovery(c, org, action)?;
        if alone || others == 0 {
            let followups = apply(c, org, action, proposer, binding, now)?;
            event(
                c,
                org,
                "change.applied",
                Some(proposer),
                None,
                &json!({"action": detail, "sole_holder": tenant.two_person}),
                now,
            )?;
            return Ok((Proposed::Applied, followups));
        }
        let uuid = crate::util::get_uuid();
        reins_org::q_insert_change(
            c,
            &ReinsOrgChange {
                uuid: uuid.clone(),
                org_uuid: org.to_owned(),
                action: detail.to_string(),
                proposed_by: proposer.to_owned(),
                proposed_at: now,
                expires_at: now + CHANGE_TTL_SECS,
                state: CHANGE_PENDING.to_owned(),
                decided_by: None,
                decided_at: None,
            },
        )?;
        event(c, org, "change.proposed", Some(proposer), Some(&uuid), &json!({"action": detail}), now)?;
        Ok((Proposed::Pending(uuid), Vec::new()))
    })
}

fn load_pending(c: &mut DbConnInner, org: &str, change: &str) -> Result<(ReinsOrgChange, AdminAction), OrgError> {
    let row = reins_org::q_change(c, org, change)?.ok_or(OrgError::NotFound)?;
    if row.state != CHANGE_PENDING {
        return Err(OrgError::Invalid("This change was already decided."));
    }
    let action: AdminAction = serde_json::from_str(&row.action)
        .map_err(|e| OrgError::Internal(format!("stored change {change} does not parse: {e}")))?;
    Ok((row, action))
}

/// Approves and applies a pending change, as a second person.
pub fn approve(
    c: &mut DbConnInner,
    org: &str,
    change: &str,
    approver: &str,
    binding: Option<&VerifiedBinding>,
    now: i64,
) -> Result<Followups, OrgError> {
    let expired = c.transaction(|c| {
        let (row, action) = load_pending(c, org, change)?;
        if crate::crypto::ct_eq(&row.proposed_by, approver) {
            return Err(OrgError::Invalid("You can't approve your own change."));
        }
        let cap = needed(c, org, &action)?;
        if !can(c, org, approver, cap)? {
            return Err(OrgError::Forbidden);
        }
        if now >= row.expires_at || !can(c, org, &row.proposed_by, cap)? {
            // Too old, or its proposer may no longer make it: it lapses instead of applying.
            reins_org::q_decide(c, change, CHANGE_EXPIRED, approver, now)?;
            event(c, org, "change.expired", Some(approver), Some(change), &json!({}), now)?;
            return Ok(None);
        }
        validate(c, org, &action, binding)?;
        if !reins_org::q_decide(c, change, CHANGE_APPLIED, approver, now)? {
            return Err(OrgError::Invalid("This change was already decided."));
        }
        let followups = apply(c, org, &action, approver, binding, now)?;
        let detail = json!({"action": action, "proposed_by": row.proposed_by});
        event(c, org, "change.approved", Some(approver), Some(change), &detail, now)?;
        event(c, org, "change.applied", Some(approver), Some(change), &detail, now)?;
        Ok(Some(followups))
    })?;
    expired.ok_or(OrgError::Invalid("This change expired before it was approved."))
}

/// Rejects a pending change (its proposer withdrawing it, or another holder of the capability).
pub fn reject(c: &mut DbConnInner, org: &str, change: &str, actor: &str, now: i64) -> Result<(), OrgError> {
    c.transaction(|c| {
        let (row, action) = load_pending(c, org, change)?;
        let cap = needed(c, org, &action)?;
        if row.proposed_by != actor && !can(c, org, actor, cap)? {
            return Err(OrgError::Forbidden);
        }
        reins_org::q_decide(c, change, CHANGE_REJECTED, actor, now)?;
        event(c, org, "change.rejected", Some(actor), Some(change), &json!({}), now)?;
        Ok(())
    })
}

/// A pending change as one viewer sees it.
#[derive(Debug)]
pub struct PendingView {
    pub change: ReinsOrgChange,
    pub action: AdminAction,
    /// The viewer may approve it: not the proposer, and holds the capability.
    pub can_approve: bool,
    pub can_reject: bool,
}

pub fn pending_for(c: &mut DbConnInner, org: &str, viewer: &str) -> Result<Vec<PendingView>, OrgError> {
    let mut out = Vec::new();
    for change in reins_org::q_pending_changes(c, org)? {
        let Ok(action) = serde_json::from_str::<AdminAction>(&change.action) else {
            continue;
        };
        let cap = needed(c, org, &action)?;
        let holds = can(c, org, viewer, cap)?;
        out.push(PendingView {
            can_approve: holds && change.proposed_by != viewer,
            can_reject: holds || change.proposed_by == viewer,
            change,
            action,
        });
    }
    Ok(out)
}

fn active_member(c: &mut DbConnInner, org: &str, membership: &str) -> Result<(), OrgError> {
    match reins_org::q_membership_by_uuid(c, org, membership)? {
        Some(m) if m.status >= ACTIVE_STATUS => Ok(()),
        _ => Err(OrgError::Invalid("That person isn't a member of this organization.")),
    }
}

fn group_exists(c: &mut DbConnInner, org: &str, group: &str) -> Result<(), OrgError> {
    let n: i64 = groups::table
        .filter(groups::organizations_uuid.eq(org))
        .filter(groups::uuid.eq(group))
        .count()
        .get_result(c)?;
    if n == 0 {
        return Err(OrgError::Invalid("That group isn't in this organization."));
    }
    Ok(())
}

fn github_login_ok(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && !login.starts_with('-')
        && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn workos_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `action` can be applied to the org as it is now. An SSO identity needs `binding`, WorkOS's own word that the
/// WorkOS organization and directory are this tenant's ([`super::workos_api`]).
fn validate(
    c: &mut DbConnInner,
    org: &str,
    action: &AdminAction,
    binding: Option<&VerifiedBinding>,
) -> Result<(), OrgError> {
    let tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
    match action {
        AdminAction::GrantRole {
            subject,
            ..
        } => match subject {
            Subject::Member(m) => active_member(c, org, m),
            Subject::Group(g) => group_exists(c, org, g),
        },
        AdminAction::RevokeRole {
            subject,
            role,
        } => invariants::check_role_removal(c, org, subject, *role),
        AdminAction::RemoveMember {
            member,
        } => {
            active_member(c, org, member)?;
            invariants::plan_membership_end(c, org, member, OnLastOwner::Refuse).map(drop)
        }
        AdminAction::SetIdentity {
            identity,
            workos_org_id,
            directory_id,
            github_org,
        } => {
            if !tenant.tier().allows(*identity) {
                return Err(OrgError::Invalid("This plan signs in with Google Workspace or a GitHub organization."));
            }
            if *identity == IdentityKind::None {
                return Ok(());
            }
            // Every identity goes through the tenant's own WorkOS organization, confirmed by WorkOS.
            let Some(id) = workos_org_id.as_deref().filter(|id| workos_id_ok(id)) else {
                return Err(OrgError::Invalid("Name the WorkOS organization (org_…)."));
            };
            if reins_org::q_org_by_workos(c, id)?.is_some_and(|other| other.org_uuid != org) {
                return Err(OrgError::Taken("That WorkOS organization belongs to another tenant."));
            }
            let directory = directory_id.as_deref().filter(|_| *identity == IdentityKind::Sso && tenant.tier().scim());
            if let Some(d) = directory {
                if !workos_id_ok(d) {
                    return Err(OrgError::Invalid("That isn't a WorkOS directory id (directory_…)."));
                }
                if reins_org::q_org_by_directory(c, d)?.is_some_and(|other| other.org_uuid != org) {
                    return Err(OrgError::Taken("That directory belongs to another tenant."));
                }
            }
            if *identity == IdentityKind::GithubOrg && !github_org.as_deref().is_some_and(github_login_ok) {
                return Err(OrgError::Invalid("Name the GitHub organization (its login)."));
            }
            if !binding.is_some_and(|b| b.proves(id, directory)) {
                return Err(OrgError::Invalid("WorkOS hasn't confirmed this organization and directory."));
            }
            Ok(())
        }
        AdminAction::SetTier {
            tier,
        } => {
            if !tier.allows(tenant.identity()) {
                return Err(OrgError::Invalid("Change how members sign in first: this plan has no SSO."));
            }
            Ok(())
        }
        AdminAction::RemoveDomain {
            domain,
        } => {
            reins_org::q_domain(c, org, domain)?.ok_or(OrgError::NotFound)?;
            Ok(())
        }
        AdminAction::SetTwoPerson {
            ..
        } => Ok(()),
    }
}

/// Applies a validated action inside the caller's transaction.
fn apply(
    c: &mut DbConnInner,
    org: &str,
    action: &AdminAction,
    actor: &str,
    binding: Option<&VerifiedBinding>,
    now: i64,
) -> Result<Followups, OrgError> {
    validate(c, org, action, binding)?;
    let recovery = is_owner_recovery(c, org, action)?;
    let subject_row = |subject: &Subject| match subject {
        Subject::Member(m) => (SUBJECT_MEMBER, m.clone()),
        Subject::Group(g) => (SUBJECT_GROUP, g.clone()),
    };
    match action {
        AdminAction::GrantRole {
            subject,
            role,
        } => {
            let (kind, id) = subject_row(subject);
            reins_org::q_grant(
                c,
                &ReinsOrgRole {
                    org_uuid: org.to_owned(),
                    subject_kind: kind.to_owned(),
                    subject_id: id,
                    role: role.as_str().to_owned(),
                    granted_at: now,
                    granted_by: actor.to_owned(),
                },
            )?;
            if recovery {
                invariants::set_owner_recovery(c, org, None)?;
                event(c, org, "org.owner_recovered", Some(actor), None, &serde_json::json!({}), now)?;
            }
        }
        AdminAction::RevokeRole {
            subject,
            role,
        } => {
            let (kind, id) = subject_row(subject);
            reins_org::q_revoke(c, org, kind, &id, role.as_str())?;
        }
        AdminAction::SetIdentity {
            identity,
            workos_org_id,
            directory_id,
            github_org,
        } => {
            let mut tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
            tenant.identity = identity.as_str().to_owned();
            let sso = *identity == IdentityKind::Sso;
            tenant.workos_org_id = workos_org_id.clone().filter(|_| *identity != IdentityKind::None);
            tenant.directory_id = directory_id.clone().filter(|_| sso && tenant.tier().scim());
            tenant.github_org = github_org.clone().filter(|_| *identity == IdentityKind::GithubOrg);
            tenant.updated_at = now;
            reins_org::q_save_org(c, &tenant)?;
        }
        AdminAction::SetTier {
            tier,
        } => {
            let mut tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
            tenant.tier = tier.as_str().to_owned();
            if !tier.scim() {
                tenant.directory_id = None;
            }
            tenant.updated_at = now;
            reins_org::q_save_org(c, &tenant)?;
        }
        AdminAction::RemoveDomain {
            domain,
        } => reins_org::q_delete_domain(c, org, domain)?,
        AdminAction::RemoveMember {
            member,
        } => {
            let m = reins_org::q_membership_by_uuid(c, org, member)?.ok_or(OrgError::NotFound)?;
            let removal = deprovision::q_deprovision(
                c,
                org,
                m.user_uuid.as_ref(),
                deprovision::Reason::AdminRemoval,
                Some(actor),
                now,
            )?;
            return Ok(removal.into_iter().collect());
        }
        AdminAction::SetTwoPerson {
            on,
        } => {
            let mut tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
            tenant.two_person = *on;
            tenant.updated_at = now;
            reins_org::q_save_org(c, &tenant)?;
        }
    }
    Ok(Vec::new())
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::org::Tier;

    use super::super::testing::*;
    use super::*;

    /// o1 with owner u1 (m1), a second owner u2 (m2) unless `solo`, and a Security admin u3 (m3).
    fn org(solo: bool) -> DbConnInner {
        let mut c = db();
        add_org(&mut c, "o1");
        add_member(&mut c, "o1", "m1", "u1", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", Role::OrgOwner);
        if !solo {
            add_member(&mut c, "o1", "m2", "u2", ACTIVE_STATUS);
            grant(&mut c, "o1", SUBJECT_MEMBER, "m2", Role::OrgOwner);
        }
        add_member(&mut c, "o1", "m3", "u3", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m3", Role::SecurityAdmin);
        add_member(&mut c, "o1", "m4", "u4", ACTIVE_STATUS);
        c
    }

    fn grant_role(m: &str, role: Role) -> AdminAction {
        AdminAction::GrantRole {
            subject: Subject::Member(m.to_owned()),
            role,
        }
    }

    fn remove(m: &str) -> AdminAction {
        AdminAction::RemoveMember {
            member: m.to_owned(),
        }
    }

    fn pending(p: (Proposed, Followups)) -> String {
        match p.0 {
            Proposed::Pending(id) => id,
            Proposed::Applied => panic!("applied at once"),
        }
    }

    #[test]
    fn a_sole_owner_acts_alone_and_it_is_recorded() {
        let mut c = org(true);
        let (p, _) = propose(&mut c, "o1", "u1", &grant_role("m4", Role::SecurityAdmin), None, 1).unwrap();
        assert_eq!(p, Proposed::Applied);
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap(), [Role::SecurityAdmin]);
        let e = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 1).unwrap().remove(0);
        assert_eq!(e.kind, "change.applied");
        assert!(e.detail.contains("\"sole_holder\":true"), "{}", e.detail);
    }

    #[test]
    fn with_a_second_holder_changes_wait_for_approval() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::SecurityAdmin), None, 1).unwrap());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
        approve(&mut c, "o1", &id, "u2", None, 2).unwrap();
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap(), [Role::SecurityAdmin]);
        assert!(approve(&mut c, "o1", &id, "u2", None, 3).is_err(), "decided once");
    }

    #[test]
    fn the_proposer_can_never_approve_their_own_change() {
        let mut c = org(false);
        // u1 also holds Org owner through a group: approving through it is still approving their own change.
        add_group(&mut c, "o1", "g1", &["m1"]);
        grant(&mut c, "o1", SUBJECT_GROUP, "g1", Role::Auditor);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::OrgOwner), None, 1).unwrap());
        assert_eq!(
            approve(&mut c, "o1", &id, "u1", None, 2).unwrap_err(),
            OrgError::Invalid("You can't approve your own change.")
        );
        let views = pending_for(&mut c, "o1", "u1").unwrap();
        assert!(!views[0].can_approve && views[0].can_reject);
        assert!(pending_for(&mut c, "o1", "u2").unwrap()[0].can_approve);
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
    }

    #[test]
    fn approvers_need_the_capability() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::OrgOwner), None, 1).unwrap());
        // A Security admin can't approve making an owner; a plain member can't approve anything.
        assert_eq!(approve(&mut c, "o1", &id, "u3", None, 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(approve(&mut c, "o1", &id, "u4", None, 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(approve(&mut c, "o1", &id, "stranger", None, 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(
            propose(&mut c, "o1", "u4", &grant_role("m4", Role::Auditor), None, 1).unwrap_err(),
            OrgError::Forbidden
        );
    }

    #[test]
    fn expired_changes_are_not_applied() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::Auditor), None, 1).unwrap());
        assert!(approve(&mut c, "o1", &id, "u2", None, 1 + CHANGE_TTL_SECS).is_err());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
        assert_eq!(reins_org::q_change(&mut c, "o1", &id).unwrap().unwrap().state, CHANGE_EXPIRED);
    }

    #[test]
    fn a_change_lapses_when_its_proposer_lost_the_capability() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u3", &grant_role("m4", Role::Auditor), None, 1).unwrap());
        reins_org::q_revoke(&mut c, "o1", SUBJECT_MEMBER, "m3", Role::SecurityAdmin.as_str()).unwrap();
        assert!(approve(&mut c, "o1", &id, "u1", None, 2).is_err());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
    }

    #[test]
    fn the_last_owner_cannot_be_removed_or_demoted() {
        let mut c = org(true);
        let demote = AdminAction::RevokeRole {
            subject: Subject::Member("m1".to_owned()),
            role: Role::OrgOwner,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &demote, None, 1), Err(OrgError::Invalid(_))));
        assert!(matches!(propose(&mut c, "o1", "u1", &remove("m1"), None, 1), Err(OrgError::Invalid(_))));
        // With two owners one may go, but only with the other's approval; then the last one stays.
        grant(&mut c, "o1", SUBJECT_MEMBER, "m4", Role::OrgOwner);
        let id = pending(propose(&mut c, "o1", "u1", &demote, None, 2).unwrap());
        approve(&mut c, "o1", &id, "u4", None, 3).unwrap();
        let demote_u4 = AdminAction::RevokeRole {
            subject: Subject::Member("m4".to_owned()),
            role: Role::OrgOwner,
        };
        assert!(matches!(propose(&mut c, "o1", "u4", &demote_u4, None, 4), Err(OrgError::Invalid(_))));
    }

    #[test]
    fn manage_members_alone_cannot_remove_a_security_admin() {
        let mut c = org(false);
        add_member(&mut c, "o1", "m5", "u5", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m5", Role::SecurityAdmin);
        // u3 (Security admin: ManageMembers, not AssignAdmins) can't remove another Security admin, nor a Connection
        // admin; it can remove a plain member.
        assert_eq!(propose(&mut c, "o1", "u3", &remove("m5"), None, 1).unwrap_err(), OrgError::Forbidden);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m4", Role::ConnectionAdmin);
        assert_eq!(propose(&mut c, "o1", "u3", &remove("m4"), None, 1).unwrap_err(), OrgError::Forbidden);
        add_member(&mut c, "o1", "m6", "u6", ACTIVE_STATUS);
        assert!(propose(&mut c, "o1", "u3", &remove("m6"), None, 1).is_ok());
        // Nor can it approve an owner's proposal to remove one.
        let id = pending(propose(&mut c, "o1", "u1", &remove("m5"), None, 2).unwrap());
        assert_eq!(approve(&mut c, "o1", &id, "u3", None, 3).unwrap_err(), OrgError::Forbidden);
    }

    #[test]
    fn removing_an_admin_still_takes_two_people() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &remove("m3"), None, 1).unwrap());
        assert!(reins_org::q_membership_by_uuid(&mut c, "o1", "m3").unwrap().unwrap().status >= ACTIVE_STATUS);
        let followups = approve(&mut c, "o1", &id, "u2", None, 2).unwrap();
        assert_eq!(followups.len(), 1);
        assert!(reins_org::q_membership_by_uuid(&mut c, "o1", "m3").unwrap().unwrap().status < 0);
    }

    #[test]
    fn admin_roles_are_never_given_to_groups() {
        let mut c = org(true);
        add_group(&mut c, "o1", "g1", &["m4"]);
        let to_group = AdminAction::GrantRole {
            subject: Subject::Group("g1".to_owned()),
            role: Role::SecurityAdmin,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &to_group, None, 1), Err(OrgError::Invalid(_))));
    }

    #[test]
    fn team_cannot_set_saml_sso() {
        let mut c = org(true);
        propose(
            &mut c,
            "o1",
            "u1",
            &AdminAction::SetTier {
                tier: Tier::Team,
            },
            None,
            1,
        )
        .unwrap();
        let sso = AdminAction::SetIdentity {
            identity: IdentityKind::Sso,
            workos_org_id: Some("org_01ABC".to_owned()),
            directory_id: None,
            github_org: None,
        };
        let proof = VerifiedBinding::for_tests("org_01ABC", None);
        assert!(matches!(propose(&mut c, "o1", "u1", &sso, Some(&proof), 2), Err(OrgError::Invalid(_))));
        let gh = AdminAction::SetIdentity {
            identity: IdentityKind::GithubOrg,
            workos_org_id: Some("org_01ABC".to_owned()),
            directory_id: None,
            github_org: Some("acme-inc".to_owned()),
        };
        propose(&mut c, "o1", "u1", &gh, Some(&proof), 3).unwrap();
        let t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        assert_eq!((t.github_org.as_deref(), t.workos_org_id.as_deref()), (Some("acme-inc"), Some("org_01ABC")));
    }

    fn sso(org: &str, dir: Option<&str>) -> AdminAction {
        AdminAction::SetIdentity {
            identity: IdentityKind::Sso,
            workos_org_id: Some(org.to_owned()),
            directory_id: dir.map(str::to_owned),
            github_org: None,
        }
    }

    #[test]
    fn identities_need_workos_confirmation() {
        let mut c = org(true);
        // Typed ids prove nothing; a proof for other ids neither.
        assert!(matches!(
            propose(&mut c, "o1", "u1", &sso("org_A", Some("directory_A")), None, 1),
            Err(OrgError::Invalid(_))
        ));
        let other = VerifiedBinding::for_tests("org_A", Some("directory_B"));
        assert!(matches!(
            propose(&mut c, "o1", "u1", &sso("org_A", Some("directory_A")), Some(&other), 1),
            Err(OrgError::Invalid(_))
        ));
        let google = AdminAction::SetIdentity {
            identity: IdentityKind::GoogleWorkspace,
            workos_org_id: None,
            directory_id: None,
            github_org: None,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &google, None, 1), Err(OrgError::Invalid(_))));
        let proof = VerifiedBinding::for_tests("org_A", Some("directory_A"));
        propose(&mut c, "o1", "u1", &sso("org_A", Some("directory_A")), Some(&proof), 1).unwrap();
        // A pending change is re-checked on approval: the approver must bring the proof too.
        add_member(&mut c, "o1", "m2", "u2", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m2", Role::OrgOwner);
        let proof_b = VerifiedBinding::for_tests("org_B", None);
        let id = pending(propose(&mut c, "o1", "u1", &sso("org_B", None), Some(&proof_b), 2).unwrap());
        assert!(approve(&mut c, "o1", &id, "u2", None, 3).is_err());
        approve(&mut c, "o1", &id, "u2", Some(&proof_b), 3).unwrap();
    }

    #[test]
    fn a_workos_org_and_a_directory_are_bound_to_one_tenant() {
        let mut c = org(true);
        add_org(&mut c, "o2");
        add_member(&mut c, "o2", "n1", "v1", ACTIVE_STATUS);
        grant(&mut c, "o2", SUBJECT_MEMBER, "n1", Role::OrgOwner);
        let proof = VerifiedBinding::for_tests("org_01ABC", Some("directory_01X"));
        propose(&mut c, "o1", "u1", &sso("org_01ABC", Some("directory_01X")), Some(&proof), 1).unwrap();
        assert!(matches!(
            propose(&mut c, "o2", "v1", &sso("org_01ABC", Some("directory_01X")), Some(&proof), 2),
            Err(OrgError::Taken(_))
        ));
        let proof2 = VerifiedBinding::for_tests("org_02DEF", Some("directory_01X"));
        assert!(matches!(
            propose(&mut c, "o2", "v1", &sso("org_02DEF", Some("directory_01X")), Some(&proof2), 2),
            Err(OrgError::Taken(_))
        ));
        let t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        assert_eq!((t.workos_org_id.as_deref(), t.directory_id.as_deref()), (Some("org_01ABC"), Some("directory_01X")));
        // The database has the last word: a second tenant row with the same directory is refused.
        let mut dup = reins_org::q_org(&mut c, "o2").unwrap().unwrap();
        dup.directory_id = Some("directory_01X".to_owned());
        assert!(reins_org::q_save_org(&mut c, &dup).is_err());
    }

    #[test]
    fn owner_recovery_takes_two_security_admins() {
        let mut c = org(true);
        // The directory removed the last owner: kept, flagged.
        invariants::set_owner_recovery(&mut c, "o1", Some(1)).unwrap();
        add_user(&mut c, "u1", "owner@acme.com");
        diesel::update(crate::db::schema::users::table)
            .filter(crate::db::schema::users::uuid.eq("u1"))
            .set(crate::db::schema::users::enabled.eq(false))
            .execute(&mut c)
            .unwrap();
        diesel::update(crate::db::schema::users_organizations::table)
            .filter(crate::db::schema::users_organizations::uuid.eq("m1"))
            .set(crate::db::schema::users_organizations::status.eq(ACTIVE_STATUS))
            .execute(&mut c)
            .unwrap();
        // One Security admin alone can't make an owner.
        assert!(matches!(
            propose(&mut c, "o1", "u3", &grant_role("m4", Role::OrgOwner), None, 2),
            Err(OrgError::Invalid(_))
        ));
        add_member(&mut c, "o1", "m5", "u5", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m5", Role::SecurityAdmin);
        let id = pending(propose(&mut c, "o1", "u3", &grant_role("m4", Role::OrgOwner), None, 3).unwrap());
        assert_eq!(
            approve(&mut c, "o1", &id, "u3", None, 4).unwrap_err(),
            OrgError::Invalid("You can't approve your own change.")
        );
        approve(&mut c, "o1", &id, "u5", None, 4).unwrap();
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().contains(&Role::OrgOwner));
        assert!(!invariants::owner_recovery(&mut c, "o1").unwrap());
        // Without the flag, Security admins can't make owners.
        assert_eq!(
            propose(&mut c, "o1", "u3", &grant_role("m5", Role::OrgOwner), None, 5).unwrap_err(),
            OrgError::Forbidden
        );
    }
}
