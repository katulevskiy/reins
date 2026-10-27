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

use super::{OrgError, can, deprovision, event};
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
    Ok(action.capability(&target_roles(c, org, action)?))
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
    now: i64,
) -> Result<(Proposed, Followups), OrgError> {
    action.check_shape().map_err(OrgError::Invalid)?;
    c.transaction(|c| {
        let tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
        let cap = needed(c, org, action)?;
        if !can(c, org, proposer, cap)? {
            return Err(OrgError::Forbidden);
        }
        validate(c, org, action)?;
        let detail = serde_json::to_value(action).unwrap_or_default();
        if !tenant.two_person || other_holders(c, org, cap, proposer)? == 0 {
            let followups = apply(c, org, action, proposer, now)?;
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
pub fn approve(c: &mut DbConnInner, org: &str, change: &str, approver: &str, now: i64) -> Result<Followups, OrgError> {
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
        validate(c, org, &action)?;
        if !reins_org::q_decide(c, change, CHANGE_APPLIED, approver, now)? {
            return Err(OrgError::Invalid("This change was already decided."));
        }
        let followups = apply(c, org, &action, approver, now)?;
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

/// Org owners holding the role directly on an active membership.
fn owners(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
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

fn is_last_owner(c: &mut DbConnInner, org: &str, membership: &str) -> Result<bool, OrgError> {
    let owners = owners(c, org)?;
    Ok(owners.len() <= 1 && owners.iter().any(|o| o == membership))
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

/// Whether `action` can be applied to the org as it is now.
fn validate(c: &mut DbConnInner, org: &str, action: &AdminAction) -> Result<(), OrgError> {
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
        } => {
            if let Subject::Member(m) = subject
                && *role == Role::OrgOwner
                && is_last_owner(c, org, m)?
            {
                return Err(OrgError::Invalid("An organization keeps at least one Org owner."));
            }
            Ok(())
        }
        AdminAction::RemoveMember {
            member,
        } => {
            active_member(c, org, member)?;
            if is_last_owner(c, org, member)? {
                return Err(OrgError::Invalid("An organization keeps at least one Org owner."));
            }
            Ok(())
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
            match identity {
                IdentityKind::Sso => {
                    let Some(id) = workos_org_id.as_deref().filter(|id| workos_id_ok(id)) else {
                        return Err(OrgError::Invalid("SSO needs the WorkOS organization id (org_…)."));
                    };
                    if reins_org::q_org_by_workos(c, id)?.is_some_and(|other| other.org_uuid != org) {
                        return Err(OrgError::Taken("That WorkOS organization belongs to another tenant."));
                    }
                    if directory_id.as_deref().is_some_and(|d| !workos_id_ok(d)) {
                        return Err(OrgError::Invalid("That isn't a WorkOS directory id (directory_…)."));
                    }
                }
                IdentityKind::GithubOrg => {
                    if !github_org.as_deref().is_some_and(github_login_ok) {
                        return Err(OrgError::Invalid("Name the GitHub organization (its login)."));
                    }
                }
                IdentityKind::GoogleWorkspace | IdentityKind::None => {}
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
fn apply(c: &mut DbConnInner, org: &str, action: &AdminAction, actor: &str, now: i64) -> Result<Followups, OrgError> {
    validate(c, org, action)?;
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
            tenant.workos_org_id = workos_org_id.clone().filter(|_| sso);
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
        let (p, _) = propose(&mut c, "o1", "u1", &grant_role("m4", Role::SecurityAdmin), 1).unwrap();
        assert_eq!(p, Proposed::Applied);
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap(), [Role::SecurityAdmin]);
        let e = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 1).unwrap().remove(0);
        assert_eq!(e.kind, "change.applied");
        assert!(e.detail.contains("\"sole_holder\":true"), "{}", e.detail);
    }

    #[test]
    fn with_a_second_holder_changes_wait_for_approval() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::SecurityAdmin), 1).unwrap());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
        approve(&mut c, "o1", &id, "u2", 2).unwrap();
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap(), [Role::SecurityAdmin]);
        assert!(approve(&mut c, "o1", &id, "u2", 3).is_err(), "decided once");
    }

    #[test]
    fn the_proposer_can_never_approve_their_own_change() {
        let mut c = org(false);
        // u1 also holds Org owner through a group: approving through it is still approving their own change.
        add_group(&mut c, "o1", "g1", &["m1"]);
        grant(&mut c, "o1", SUBJECT_GROUP, "g1", Role::Auditor);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::OrgOwner), 1).unwrap());
        assert_eq!(
            approve(&mut c, "o1", &id, "u1", 2).unwrap_err(),
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
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::OrgOwner), 1).unwrap());
        // A Security admin can't approve making an owner; a plain member can't approve anything.
        assert_eq!(approve(&mut c, "o1", &id, "u3", 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(approve(&mut c, "o1", &id, "u4", 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(approve(&mut c, "o1", &id, "stranger", 2).unwrap_err(), OrgError::Forbidden);
        assert_eq!(propose(&mut c, "o1", "u4", &grant_role("m4", Role::Auditor), 1).unwrap_err(), OrgError::Forbidden);
    }

    #[test]
    fn expired_changes_are_not_applied() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::Auditor), 1).unwrap());
        assert!(approve(&mut c, "o1", &id, "u2", 1 + CHANGE_TTL_SECS).is_err());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
        assert_eq!(reins_org::q_change(&mut c, "o1", &id).unwrap().unwrap().state, CHANGE_EXPIRED);
    }

    #[test]
    fn a_change_lapses_when_its_proposer_lost_the_capability() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u3", &grant_role("m4", Role::Auditor), 1).unwrap());
        reins_org::q_revoke(&mut c, "o1", SUBJECT_MEMBER, "m3", Role::SecurityAdmin.as_str()).unwrap();
        assert!(approve(&mut c, "o1", &id, "u1", 2).is_err());
        assert!(reins_org::q_roles_of(&mut c, "o1", "m4").unwrap().is_empty());
    }

    #[test]
    fn the_last_owner_cannot_be_removed_or_demoted() {
        let mut c = org(true);
        let demote = AdminAction::RevokeRole {
            subject: Subject::Member("m1".to_owned()),
            role: Role::OrgOwner,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &demote, 1), Err(OrgError::Invalid(_))));
        assert!(matches!(propose(&mut c, "o1", "u1", &remove("m1"), 1), Err(OrgError::Invalid(_))));
        // With two owners one may go, but only with the other's approval; then the last one stays.
        grant(&mut c, "o1", SUBJECT_MEMBER, "m4", Role::OrgOwner);
        let id = pending(propose(&mut c, "o1", "u1", &demote, 2).unwrap());
        approve(&mut c, "o1", &id, "u4", 3).unwrap();
        let demote_u4 = AdminAction::RevokeRole {
            subject: Subject::Member("m4".to_owned()),
            role: Role::OrgOwner,
        };
        assert!(matches!(propose(&mut c, "o1", "u4", &demote_u4, 4), Err(OrgError::Invalid(_))));
    }

    #[test]
    fn manage_members_alone_cannot_remove_a_security_admin() {
        let mut c = org(false);
        add_member(&mut c, "o1", "m5", "u5", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m5", Role::SecurityAdmin);
        // u3 (Security admin: ManageMembers, not AssignAdmins) can't remove another Security admin, nor a Connection
        // admin; it can remove a plain member.
        assert_eq!(propose(&mut c, "o1", "u3", &remove("m5"), 1).unwrap_err(), OrgError::Forbidden);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m4", Role::ConnectionAdmin);
        assert_eq!(propose(&mut c, "o1", "u3", &remove("m4"), 1).unwrap_err(), OrgError::Forbidden);
        add_member(&mut c, "o1", "m6", "u6", ACTIVE_STATUS);
        assert!(propose(&mut c, "o1", "u3", &remove("m6"), 1).is_ok());
        // Nor can it approve an owner's proposal to remove one.
        let id = pending(propose(&mut c, "o1", "u1", &remove("m5"), 2).unwrap());
        assert_eq!(approve(&mut c, "o1", &id, "u3", 3).unwrap_err(), OrgError::Forbidden);
    }

    #[test]
    fn removing_an_admin_still_takes_two_people() {
        let mut c = org(false);
        let id = pending(propose(&mut c, "o1", "u1", &remove("m3"), 1).unwrap());
        assert!(reins_org::q_membership_by_uuid(&mut c, "o1", "m3").unwrap().unwrap().status >= ACTIVE_STATUS);
        let followups = approve(&mut c, "o1", &id, "u2", 2).unwrap();
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
        assert!(matches!(propose(&mut c, "o1", "u1", &to_group, 1), Err(OrgError::Invalid(_))));
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
            1,
        )
        .unwrap();
        let sso = AdminAction::SetIdentity {
            identity: IdentityKind::Sso,
            workos_org_id: Some("org_01ABC".to_owned()),
            directory_id: None,
            github_org: None,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &sso, 2), Err(OrgError::Invalid(_))));
        let gh = AdminAction::SetIdentity {
            identity: IdentityKind::GithubOrg,
            workos_org_id: None,
            directory_id: None,
            github_org: Some("acme-inc".to_owned()),
        };
        propose(&mut c, "o1", "u1", &gh, 3).unwrap();
        assert_eq!(reins_org::q_org(&mut c, "o1").unwrap().unwrap().github_org.as_deref(), Some("acme-inc"));
    }

    #[test]
    fn a_workos_org_is_bound_to_one_tenant() {
        let mut c = org(true);
        add_org(&mut c, "o2");
        add_member(&mut c, "o2", "n1", "v1", ACTIVE_STATUS);
        grant(&mut c, "o2", SUBJECT_MEMBER, "n1", Role::OrgOwner);
        let sso = AdminAction::SetIdentity {
            identity: IdentityKind::Sso,
            workos_org_id: Some("org_01ABC".to_owned()),
            directory_id: Some("directory_01X".to_owned()),
            github_org: None,
        };
        propose(&mut c, "o1", "u1", &sso, 1).unwrap();
        assert!(matches!(propose(&mut c, "o2", "v1", &sso, 2), Err(OrgError::Taken(_))));
        let t = reins_org::q_org(&mut c, "o1").unwrap().unwrap();
        assert_eq!((t.workos_org_id.as_deref(), t.directory_id.as_deref()), (Some("org_01ABC"), Some("directory_01X")));
    }
}
