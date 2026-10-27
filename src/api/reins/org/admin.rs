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
    models::reins_org_exec::{self, ReinsOrgExecutor},
    schema::groups,
};

/// How long a change waits for its second person.
pub const CHANGE_TTL_SECS: i64 = 7 * 24 * 3600;
/// Changes that may wait at once in one org (older ones lapse after [`CHANGE_TTL_SECS`]).
pub const MAX_PENDING: usize = 50;

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
        }
        | AdminAction::SetManager {
            member,
            ..
        } => reins_org::q_roles_of(c, org, member)?,
        AdminAction::ConfirmDirectoryRemovals {
            ..
        } => {
            let mut roles = Vec::new();
            for m in super::scim::q_held_memberships(c, org)? {
                roles.extend(reins_org::q_roles_of(c, org, &m)?);
            }
            roles
        }
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
    ) && invariants::recovery_mode(c, org)?)
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
        // 06 §3's actions wait for the proposer's own device signature, whoever else may approve them.
        if (alone || others == 0) && !action.needs_device_signatures() {
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
        let waiting = reins_org::q_pending_changes(c, org)?;
        if waiting.iter().filter(|ch| ch.expires_at > now).count() >= MAX_PENDING {
            return Err(OrgError::Invalid("Too many changes are waiting. Approve or reject some first."));
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

/// Whether a pending change may apply on its proposer's device signature alone: two-person approval is off, or
/// nobody else holds the capability.
pub fn can_apply_alone(
    c: &mut DbConnInner,
    org: &str,
    change: &ReinsOrgChange,
    action: &AdminAction,
) -> Result<bool, OrgError> {
    let tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
    if is_owner_recovery(c, org, action)? {
        return Ok(false);
    }
    let cap = needed(c, org, action)?;
    Ok(!tenant.two_person || other_holders(c, org, cap, &change.proposed_by)? == 0)
}

/// Applies a pending change its proposer confirmed on their device, when [`can_apply_alone`].
pub fn apply_alone(
    c: &mut DbConnInner,
    org: &str,
    change: &str,
    proposer: &str,
    now: i64,
) -> Result<Followups, OrgError> {
    let (row, action) = load_pending(c, org, change)?;
    if row.proposed_by != proposer || !can_apply_alone(c, org, &row, &action)? {
        return Err(OrgError::Forbidden);
    }
    let cap = needed(c, org, &action)?;
    if !can(c, org, proposer, cap)? {
        return Err(OrgError::Forbidden);
    }
    validate(c, org, &action, None)?;
    if !reins_org::q_decide(c, change, CHANGE_APPLIED, proposer, now)? {
        return Err(OrgError::Invalid("This change was already decided."));
    }
    let followups = apply(c, org, &action, proposer, None, now)?;
    event(
        c,
        org,
        "change.applied",
        Some(proposer),
        Some(change),
        &json!({"action": action, "sole_holder": true}),
        now,
    )?;
    Ok(followups)
}

/// The console's Approve: only after the approver approved it on their enrolled phone (06 §3), which binds the
/// approval to a device key. The console then supplies what only it can (WorkOS's word for a new sign-in).
pub fn approve_from_console(
    c: &mut DbConnInner,
    org: &str,
    change: &str,
    approver: &str,
    binding: Option<&VerifiedBinding>,
    now: i64,
) -> Result<Followups, OrgError> {
    if !super::reviews::device_approved(c, org, change, approver)? {
        return Err(OrgError::Invalid("Approve it on your phone first."));
    }
    approve(c, org, change, approver, binding, now)
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
            let github = github_org.as_deref().filter(|_| *identity == IdentityKind::GithubOrg);
            if !binding.is_some_and(|b| b.proves(id, directory, github)) {
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
        AdminAction::ConfirmDirectoryRemovals {
            held,
        } => super::scim::q_confirm_held(c, org, held, true).map(drop),
        AdminAction::ActivatePolicy {
            revision,
            hash,
        } => super::policy::check_draft(c, org, *revision, hash).map(drop),
        AdminAction::SetRetention {
            days,
        } => {
            if [0, 30, 365].contains(days) {
                Ok(())
            } else {
                Err(OrgError::Invalid("Keep request bodies for none, 30 days or 1 year."))
            }
        }
        AdminAction::RegisterExecutor {
            executor_id,
            public_key,
            ..
        } => {
            if !executor_id_ok(executor_id) {
                return Err(OrgError::Invalid("Name the executor with 1-32 of a-z, 0-9 and -."));
            }
            if executor_key(public_key).is_none() {
                return Err(OrgError::Invalid("That isn't an Ed25519 public key (base64url, 32 bytes)."));
            }
            if reins_org_exec::q_executor(c, org, executor_id)?.is_some() {
                return Err(OrgError::Invalid("That executor name is taken."));
            }
            Ok(())
        }
        AdminAction::RemoveExecutor {
            executor_id,
        } => match reins_org_exec::q_executor(c, org, executor_id)? {
            Some(e) if e.revoked_at.is_none() => Ok(()),
            _ => Err(OrgError::NotFound),
        },
        AdminAction::SetManager {
            member,
            manager,
        } => {
            if tenant.tier().scim() {
                return Err(OrgError::Invalid("Managers come from your directory."));
            }
            active_member(c, org, member)?;
            if let Some(m) = manager {
                active_member(c, org, m)?;
                if m == member {
                    return Err(OrgError::Invalid("Nobody is their own manager."));
                }
            }
            Ok(())
        }
        AdminAction::SetGroupRouting {
            group,
            backup,
            window_secs,
        } => {
            group_exists(c, org, group)?;
            if let Some(b) = backup {
                group_exists(c, org, b)?;
                if b == group {
                    return Err(OrgError::Invalid("A group can't be its own backup."));
                }
            }
            if window_secs.is_some_and(|w| w == 0 || w > reins_policy::org::DEFERRED_MAX_SECS) {
                return Err(OrgError::Invalid("A window is 1 second to 7 days."));
            }
            Ok(())
        }
        AdminAction::SetGithubReader {
            app_id,
            installation_id,
        } => {
            if *app_id == 0 || *installation_id == 0 || i64::try_from(*app_id.max(installation_id)).is_err() {
                return Err(OrgError::Invalid("Give the GitHub App's id and its installation id."));
            }
            Ok(())
        }
        AdminAction::SetOnCallSource {
            alias,
            schedule_id,
            ..
        } => {
            if !super::routing::oncall::alias_ok(alias) {
                return Err(OrgError::Invalid("Name it with 1-64 of a-z, 0-9, . _ and -."));
            }
            if !super::routing::oncall::schedule_ok(schedule_id) {
                return Err(OrgError::Invalid("That isn't a schedule id."));
            }
            Ok(())
        }
        AdminAction::SetApprovalChannel {
            channel: kind,
            enabled,
        } => {
            let installed = crate::db::models::reins_org_routing::q_channel_for_use(c, org, kind.as_str())?
                .is_some_and(|ch| ch.sealed.is_some());
            if *enabled && !installed {
                return Err(OrgError::Invalid("Install it first."));
            }
            Ok(())
        }
    }
}

/// An executor's name: 1-32 of `a-z`, `0-9`, `-`.
#[must_use]
pub fn executor_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An executor's Ed25519 public key (base64url without padding, 32 bytes, a strict key).
#[must_use]
pub fn executor_key(b64: &str) -> Option<[u8; 32]> {
    let key: [u8; 32] = reins_proto::canon::decode_payload(b64.trim())?.try_into().ok()?;
    reins_proto::canon::strict_ed25519_key(&key).then_some(key)
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
            if invariants::refresh_owner_recovery(c, org)? || recovery {
                event(
                    c,
                    org,
                    "org.owner_recovered",
                    Some(actor),
                    None,
                    &serde_json::json!({"by_security_admins": recovery}),
                    now,
                )?;
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
            identity.as_str().clone_into(&mut tenant.identity);
            let sso = *identity == IdentityKind::Sso;
            tenant.workos_org_id = workos_org_id.clone().filter(|_| *identity != IdentityKind::None);
            let directory = directory_id.clone().filter(|_| sso && tenant.tier().scim());
            super::scim::q_directory_changed(c, org, tenant.directory_id.as_deref(), directory.as_deref(), now)?;
            tenant.directory_id = directory;
            tenant.github_org = github_org.clone().filter(|_| *identity == IdentityKind::GithubOrg);
            tenant.updated_at = now;
            reins_org::q_save_org(c, &tenant)?;
        }
        AdminAction::SetTier {
            tier,
        } => {
            let mut tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
            tier.as_str().clone_into(&mut tenant.tier);
            if !tier.scim() {
                super::scim::q_directory_changed(c, org, tenant.directory_id.as_deref(), None, now)?;
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
        AdminAction::ConfirmDirectoryRemovals {
            held,
        } => {
            let count = super::scim::q_confirm_held(c, org, held, false)?;
            event(c, org, "directory.removals_confirmed", Some(actor), None, &json!({"count": count}), now)?;
        }
        AdminAction::ActivatePolicy {
            revision,
            hash,
        } => super::policy::apply_activation(c, org, *revision, hash, actor, now)?,
        AdminAction::SetRetention {
            days,
        } => {
            let mut tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
            tenant.body_retention_days = i32::try_from(*days).unwrap_or(30);
            tenant.updated_at = now;
            reins_org::q_save_org(c, &tenant)?;
            super::audit::record(
                c,
                org,
                super::audit::kinds::RETENTION_SET,
                Some(actor),
                None,
                &json!({"days": days}),
                now,
            )?;
        }
        AdminAction::RegisterExecutor {
            executor_id,
            mode,
            public_key,
        } => {
            let row = ReinsOrgExecutor {
                org_uuid: org.to_owned(),
                executor_id: executor_id.clone(),
                mode: mode.as_str().to_owned(),
                public_key: public_key.trim().to_owned(),
                created_at: now,
                last_seen: None,
                revoked_at: None,
            };
            if !reins_org_exec::q_insert_executor(c, &row)? {
                return Err(OrgError::Invalid("That executor name is taken."));
            }
            super::audit::record(
                c,
                org,
                super::audit::kinds::EXECUTOR_REGISTERED,
                Some(actor),
                Some(executor_id),
                &json!({"mode": mode.as_str(), "public_key": row.public_key}),
                now,
            )?;
        }
        AdminAction::SetManager {
            ..
        }
        | AdminAction::SetGroupRouting {
            ..
        }
        | AdminAction::SetGithubReader {
            ..
        }
        | AdminAction::SetOnCallSource {
            ..
        }
        | AdminAction::SetApprovalChannel {
            ..
        } => super::routing::apply_admin(c, org, action, actor, now)?,
        AdminAction::RemoveExecutor {
            executor_id,
        } => {
            reins_org_exec::q_revoke_executor(c, org, executor_id, now)?;
            super::audit::record(
                c,
                org,
                super::audit::kinds::EXECUTOR_REMOVED,
                Some(actor),
                Some(executor_id),
                &json!({}),
                now,
            )?;
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
        let proof = VerifiedBinding::for_tests("org_01ABC", None, None);
        assert!(matches!(propose(&mut c, "o1", "u1", &sso, Some(&proof), 2), Err(OrgError::Invalid(_))));
        let proof = VerifiedBinding::for_tests("org_01ABC", None, Some("acme-inc"));
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
        let other = VerifiedBinding::for_tests("org_A", Some("directory_B"), None);
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
        let proof = VerifiedBinding::for_tests("org_A", Some("directory_A"), None);
        propose(&mut c, "o1", "u1", &sso("org_A", Some("directory_A")), Some(&proof), 1).unwrap();
        // A pending change is re-checked on approval: the approver must bring the proof too.
        add_member(&mut c, "o1", "m2", "u2", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m2", Role::OrgOwner);
        let proof_b = VerifiedBinding::for_tests("org_B", None, None);
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
        let proof = VerifiedBinding::for_tests("org_01ABC", Some("directory_01X"), None);
        propose(&mut c, "o1", "u1", &sso("org_01ABC", Some("directory_01X")), Some(&proof), 1).unwrap();
        assert!(matches!(
            propose(&mut c, "o2", "v1", &sso("org_01ABC", Some("directory_01X")), Some(&proof), 2),
            Err(OrgError::Taken(_))
        ));
        let proof2 = VerifiedBinding::for_tests("org_02DEF", Some("directory_01X"), None);
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
        // The directory removed the last owner: kept as owner, account disabled, flag shown.
        set_state(&mut c, "o1", "u1", reins_org::state::SUSPENDED_BY_DIRECTORY);
        invariants::show_owner_recovery(&mut c, "o1", 1).unwrap();
        assert!(invariants::recovery_mode(&mut c, "o1").unwrap());
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
        assert!(!invariants::owner_recovery_shown(&mut c, "o1").unwrap());
        assert!(!invariants::recovery_mode(&mut c, "o1").unwrap());
        // With an owner again, Security admins can't make owners.
        assert_eq!(
            propose(&mut c, "o1", "u3", &grant_role("m5", Role::OrgOwner), None, 5).unwrap_err(),
            OrgError::Forbidden
        );
    }

    #[test]
    fn a_stale_recovery_flag_grants_nothing() {
        let mut c = org(true);
        set_state(&mut c, "o1", "u1", reins_org::state::SUSPENDED_BY_DIRECTORY);
        invariants::show_owner_recovery(&mut c, "o1", 1).unwrap();
        // The owner comes back (by any path that doesn't clear the flag); the flag is still shown.
        set_state(&mut c, "o1", "u1", reins_org::state::ACTIVE);
        assert!(invariants::owner_recovery_shown(&mut c, "o1").unwrap());
        assert!(!invariants::recovery_mode(&mut c, "o1").unwrap());
        add_member(&mut c, "o1", "m5", "u5", ACTIVE_STATUS);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m5", Role::SecurityAdmin);
        // Granting Org owner needs an owner again: Security admins can't, even two of them.
        assert_eq!(
            propose(&mut c, "o1", "u3", &grant_role("m4", Role::OrgOwner), None, 2).unwrap_err(),
            OrgError::Forbidden
        );
        // The owner's own grant clears the shown flag.
        propose(&mut c, "o1", "u1", &grant_role("m4", Role::Auditor), None, 3).unwrap();
        propose(&mut c, "o1", "u1", &grant_role("m4", Role::OrgOwner), None, 4).unwrap();
        assert!(!invariants::owner_recovery_shown(&mut c, "o1").unwrap());
    }

    #[test]
    fn a_suspended_second_owner_does_not_count() {
        let mut c = org(false);
        set_state(&mut c, "o1", "u2", reins_org::state::SUSPENDED_BY_DIRECTORY);
        let demote = AdminAction::RevokeRole {
            subject: Subject::Member("m1".to_owned()),
            role: Role::OrgOwner,
        };
        assert!(matches!(propose(&mut c, "o1", "u1", &demote, None, 1), Err(OrgError::Invalid(_))));
        assert!(matches!(propose(&mut c, "o1", "u1", &remove("m1"), None, 1), Err(OrgError::Invalid(_))));
        // And the disabled owner can't approve anything.
        let id = pending(propose(&mut c, "o1", "u1", &grant_role("m4", Role::Auditor), None, 2).unwrap());
        assert_eq!(approve(&mut c, "o1", &id, "u2", None, 3).unwrap_err(), OrgError::Forbidden);
    }
}
