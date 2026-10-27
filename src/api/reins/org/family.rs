//! The Family tenant (Decisions v2 · P7, `docs/design/decisions-v2/04-shared-control.md` §4, 08 R6, R7): enforceable
//! guardianship as a small Managed org. The guardians are the org; the children are its members.
//!
//! - **No domain, no identity provider.** A family is created by its first guardian, who holds every guardian role
//!   ([`GUARDIAN_ROLES`]). Members join only by an **invitation** bound to one exact email: single use, 7 days, its
//!   secret kept as a SHA-256. Inviting another guardian is a two-person admin change ([`AdminAction::AddGuardian`])
//!   once the family has one to approve it.
//! - **A new account only** (08 R6): [`join`] refuses any account that existed before (older than 30 minutes, in an
//!   org, or with phone roles, connections, Circle links, vault items or other devices), with one answer that names
//!   nobody. A family never captures someone's Personal account, and never learns one exists.
//! - **Minors.** A guardian proposes it, at invitation or later, and it counts only **once the member accepts** on
//!   their phone (a guardian can't make an adult a minor alone, not even by inviting them as one).
//! - **Leaving.** An adult leaves after the family's notice (0, 3 or 7 days); a minor's leave waits for a guardian's
//!   consent. When it takes effect, the sweeper deprovisions the member ([`Reason::FamilyLeave`]).
//! - **Policy.** [`family_baseline`]: the child decides their own soft asks; guardians approve money, messages to
//!   Outside people, publishing, permissions and L3, and every hard obligation (`hard_also`).

use diesel::prelude::*;
use reins_policy::{
    context::Zone,
    effects::{EffectKind, Level},
    matrix::{Issuer, Rule, Then, When},
    org::{OrgPolicy, OrgRule, ReviewClass, ReviewSpec, Reviewers},
};
use reins_proto::{
    canon::{hex32, sha256},
    org::{
        AdminAction, Capability, FamilyInviteCreated, FamilyLeaveRequest, FamilyLeaveView, FamilyMe, GUARDIAN_ROLES,
        IdentityKind, Role, Tier,
    },
};
use serde_json::json;

use super::{
    OrgError,
    admin::{self, Followups, Proposed},
    deprovision::{self, Reason},
    event, member_of, require,
};
use crate::db::{
    DbConnInner,
    models::{
        Membership, Organization,
        reins_family::{self, ReinsFamily, ReinsFamilyInvite, ReinsFamilyLeave, ReinsFamilyMember},
        reins_org::{self, ReinsOrg, ReinsOrgRole, SUBJECT_MEMBER},
    },
    schema::{ciphers, devices, organizations, reins_circle_links, reins_connections, reins_rosters, users},
};

/// How long an invitation stays open.
pub const INVITE_TTL_SECS: i64 = 7 * 86_400;
/// Open invitations per family.
pub const MAX_OPEN_INVITES: i64 = 20;
/// An account counts as new for this long after it was created.
pub const NEW_ACCOUNT_SECS: i64 = 30 * 60;
/// An adult's notice before leaving (04 §4: 0, 3 or 7 days, never more).
pub const NOTICE_DAYS: [i32; 3] = [0, 3, 7];
pub const DEFAULT_NOTICE_DAYS: i32 = 3;
/// What any account that isn't new hears: it names nobody and tells nobody which accounts exist.
pub const NEW_ADDRESS: &str = "Use a new address for your family account.";
const NOT_VALID: &str = "This invitation isn't valid.";
/// A guardian's review: any guardian but the requester, Deferred, a day (Review Focus 5).
pub const GUARDIAN_WINDOW_SECS: u64 = 86_400;

/// The SHA-256 (hex) of an invitation's secret, as stored.
#[must_use]
pub fn token_hash(token: &str) -> String {
    hex32(&sha256(token.as_bytes()))
}

/// An address as invitations keep it: trimmed, lowercase, one `@` with something on each side and a dot after it.
pub fn normalize_email(raw: &str) -> Result<String, OrgError> {
    let email = raw.trim().to_lowercase();
    let bad = || OrgError::Invalid("That isn't an email address.");
    if email.len() > 254 || email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let (local, domain) = email.split_once('@').ok_or_else(bad)?;
    if local.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return Err(bad());
    }
    Ok(email)
}

/// The family `org`, refused (as not found) for any other tenant.
fn family(c: &mut DbConnInner, org: &str) -> Result<(ReinsOrg, ReinsFamily), OrgError> {
    let tenant = reins_org::q_org(c, org)?.filter(|t| t.tier() == Tier::Family).ok_or(OrgError::NotFound)?;
    let fam = reins_family::q_family(c, org)?.ok_or(OrgError::NotFound)?;
    Ok((tenant, fam))
}

/// Whether `user` holds a guardian role in `org`.
fn is_guardian(c: &mut DbConnInner, org: &str, user: &str) -> Result<bool, OrgError> {
    Ok(super::roles_of_user(c, org, user)?.iter().any(|r| GUARDIAN_ROLES.contains(r)))
}

/// The guardians to tell: every active member who may approve.
pub fn guardians(c: &mut DbConnInner, org: &str) -> Result<Vec<String>, OrgError> {
    Ok(reins_org::q_holders(c, org, Capability::Approve)?)
}

fn email_of(c: &mut DbConnInner, user: &str) -> Result<String, OrgError> {
    Ok(users::table
        .filter(users::uuid.eq(user))
        .select(users::email)
        .first::<String>(c)
        .optional()?
        .unwrap_or_default())
}

/// 08 R7's guardian review: any guardian (Approver) other than the requester, Deferred, a day.
#[must_use]
pub fn guardian_review() -> ReviewSpec {
    ReviewSpec {
        reviewers: Reviewers::Role(Role::Approver),
        quorum: 1,
        distinct_persons: true,
        exclude_requester: true,
        class: ReviewClass::Deferred,
        window_secs: GUARDIAN_WINDOW_SECS,
    }
}

fn guardian_rule(org: &str, id: &str, when: When, explain: &str) -> OrgRule {
    OrgRule {
        rule: Rule {
            id: id.to_owned(),
            issuer: Issuer::Org(org.to_owned()),
            when,
            then: Then::Review,
            not_after: None,
            accepted: false,
            explain: explain.to_owned(),
        },
        review: Some(guardian_review()),
    }
}

/// A family's first policy: O2's default matrix, the child deciding their own soft asks, and a guardian approving
/// money (any amount), messages to Outside people, publishing, new permissions, L3, and every hard obligation.
#[must_use]
pub fn family_baseline(org: &str) -> OrgPolicy {
    let effects = |e: EffectKind| When {
        effects: vec![e],
        ..When::default()
    };
    let mut p = OrgPolicy::empty(org);
    p.cell_review = ReviewSpec::owner();
    p.rules = vec![
        guardian_rule(org, "family.money", effects(EffectKind::Money), "A guardian approves spending"),
        guardian_rule(
            org,
            "family.outside",
            When {
                effects: vec![EffectKind::Message],
                zones: vec![Zone::Outside],
                ..When::default()
            },
            "A guardian approves messages to new people",
        ),
        guardian_rule(org, "family.publish", effects(EffectKind::Publish), "A guardian approves publishing"),
        guardian_rule(org, "family.permission", effects(EffectKind::Permission), "A guardian approves new access"),
        guardian_rule(
            org,
            "family.level3",
            When {
                levels: vec![Level::L3],
                ..When::default()
            },
            "A guardian approves what can't be undone",
        ),
    ];
    p.hard_also = Some(guardian_review());
    "Family baseline".clone_into(&mut p.explain);
    p
}

/// What [`create_family`] made: the family, and the activation of its baseline policy waiting for the guardian's phone.
#[derive(Debug, PartialEq, Eq)]
pub struct Created {
    pub org: String,
    pub activation: Proposed,
}

/// Creates a family: the tenant (tier Family, invitations only, two-person on), its creator as guardian, its notice
/// (3 days), and [`family_baseline`] as revision 1, proposed for activation (the guardian's device signature applies
/// it; a second guardian, once there is one, approves later changes).
pub fn create_family(
    c: &mut DbConnInner,
    creator: &str,
    creator_email: &str,
    name: &str,
    now: i64,
) -> Result<Created, OrgError> {
    let name = super::tenant::org_name(name)?;
    c.transaction(|c| {
        if !reins_org::q_orgs_of_user_any(c, creator)?.is_empty() {
            return Err(OrgError::Invalid("This account already belongs to an organization."));
        }
        let org = Organization::new(name.clone(), creator_email, None, None);
        let org_uuid = org.uuid.to_string();
        diesel::insert_into(organizations::table).values(&org).execute(c)?;
        reins_org::q_insert_org(
            c,
            &ReinsOrg {
                org_uuid: org_uuid.clone(),
                tier: Tier::Family.as_str().to_owned(),
                identity: IdentityKind::FamilyInvite.as_str().to_owned(),
                workos_org_id: None,
                directory_id: None,
                github_org: None,
                two_person: true,
                owner_recovery_at: None,
                created_by: creator.to_owned(),
                created_at: now,
                updated_at: now,
                // A family keeps no request bodies.
                body_retention_days: 0,
            },
        )?;
        reins_family::q_insert_family(
            c,
            &ReinsFamily {
                org_uuid: org_uuid.clone(),
                notice_days: DEFAULT_NOTICE_DAYS,
                created_at: now,
            },
        )?;
        let membership = Membership::new(creator.to_owned().into(), org.uuid.clone(), None);
        reins_org::q_add_member(c, &membership, Some(creator), now)?;
        grant_guardian(c, &org_uuid, membership.uuid.as_ref(), creator, now)?;
        reins_family::q_save_member(
            c,
            &ReinsFamilyMember {
                org_uuid: org_uuid.clone(),
                user_uuid: creator.to_owned(),
                minor: false,
                minor_proposed_at: None,
                minor_proposed_by: None,
            },
        )?;
        event(c, &org_uuid, "org.created", Some(creator), None, &json!({"name": name, "tier": Tier::Family}), now)?;
        event(
            c,
            &org_uuid,
            "member.joined",
            Some(creator),
            Some(membership.uuid.as_ref()),
            &json!({"role": "guardian"}),
            now,
        )?;
        let body = serde_json::to_string(&family_baseline(&org_uuid)).map_err(|e| OrgError::Internal(e.to_string()))?;
        let (revision, hash) = super::policy::save_draft(c, &org_uuid, creator, &body, now)?;
        let (activation, _) = admin::propose(
            c,
            &org_uuid,
            creator,
            &AdminAction::ActivatePolicy {
                revision,
                hash,
            },
            None,
            now,
        )?;
        Ok(Created {
            org: org_uuid,
            activation,
        })
    })
}

fn grant_guardian(c: &mut DbConnInner, org: &str, membership: &str, by: &str, now: i64) -> Result<(), OrgError> {
    for role in GUARDIAN_ROLES {
        reins_org::q_grant(
            c,
            &ReinsOrgRole {
                org_uuid: org.to_owned(),
                subject_kind: SUBJECT_MEMBER.to_owned(),
                subject_id: membership.to_owned(),
                role: role.as_str().to_owned(),
                granted_at: now,
                granted_by: by.to_owned(),
            },
        )?;
    }
    Ok(())
}

/// Sets the family's notice (0, 3 or 7 days). A guardian's setting; requests already made keep theirs.
pub fn set_notice(c: &mut DbConnInner, org: &str, actor: &str, days: i32, now: i64) -> Result<(), OrgError> {
    family(c, org)?;
    require(c, org, actor, Capability::ManageMembers)?;
    if !NOTICE_DAYS.contains(&days) {
        return Err(OrgError::Invalid("The notice is 0, 3 or 7 days."));
    }
    reins_family::q_set_notice(c, org, days)?;
    event(c, org, "family.notice_set", Some(actor), None, &json!({"days": days}), now)
}

/// Invites `email` to the family (a guardian holding `ManageMembers`). A guardian invitation is an
/// [`AdminAction::AddGuardian`]: it counts at once while the inviter is the only one who could approve it, and otherwise
/// waits for another guardian. Returns the secret once; only its hash is kept.
pub fn invite(
    c: &mut DbConnInner,
    org: &str,
    actor: &str,
    raw_email: &str,
    minor: bool,
    guardian: bool,
    now: i64,
) -> Result<FamilyInviteCreated, OrgError> {
    family(c, org)?;
    require(c, org, actor, Capability::ManageMembers)?;
    let email = normalize_email(raw_email)?;
    if guardian && minor {
        return Err(OrgError::Invalid("A guardian can't be a minor."));
    }
    c.transaction(|c| {
        if reins_family::q_open_invites(c, org, now)? >= MAX_OPEN_INVITES {
            return Err(OrgError::Invalid("Too many invitations are open. Cancel some first."));
        }
        let token = super::random_token();
        let row = ReinsFamilyInvite {
            id: crate::crypto::encode_random_bytes::<16>(&data_encoding::HEXLOWER),
            token_hash: token_hash(&token),
            org_uuid: org.to_owned(),
            email: email.clone(),
            minor,
            guardian,
            approved: !guardian,
            created_by: actor.to_owned(),
            created_at: now,
            expires_at: now + INVITE_TTL_SECS,
            used_at: None,
            used_by: None,
        };
        reins_family::q_insert_invite(c, &row)?;
        event(
            c,
            org,
            "family.invited",
            Some(actor),
            Some(&row.id),
            &json!({"minor": minor, "guardian": guardian}),
            now,
        )?;
        let waiting = if guardian {
            let (proposed, _) = admin::propose(
                c,
                org,
                actor,
                &AdminAction::AddGuardian {
                    invite: row.id.clone(),
                    email,
                },
                None,
                now,
            )?;
            matches!(proposed, Proposed::Pending(_))
        } else {
            false
        };
        Ok(FamilyInviteCreated {
            id: row.id,
            token,
            expires_at: row.expires_at,
            waiting,
        })
    })
}

/// Withdraws an unused invitation.
pub fn cancel_invite(c: &mut DbConnInner, org: &str, actor: &str, id: &str, now: i64) -> Result<(), OrgError> {
    family(c, org)?;
    require(c, org, actor, Capability::ManageMembers)?;
    if !reins_family::q_cancel_invite(c, org, id, now)? {
        return Err(OrgError::NotFound);
    }
    event(c, org, "family.invite_cancelled", Some(actor), Some(id), &json!({}), now)
}

/// [`AdminAction::AddGuardian`]'s check: an open guardian invitation of this family.
pub fn check_add_guardian(c: &mut DbConnInner, org: &str, id: &str, now: i64) -> Result<(), OrgError> {
    match reins_family::q_invite(c, org, id)? {
        Some(i) if i.guardian && i.open(now) => Ok(()),
        _ => Err(OrgError::Invalid("That invitation is no longer open.")),
    }
}

/// [`AdminAction::AddGuardian`] applied: the invitation counts.
pub fn apply_add_guardian(c: &mut DbConnInner, org: &str, id: &str) -> Result<(), OrgError> {
    if reins_family::q_approve_invite(c, org, id)? {
        Ok(())
    } else {
        Err(OrgError::NotFound)
    }
}

/// Whether `user` is an account nobody used yet: created within [`NEW_ACCOUNT_SECS`], in no org, and with no phone
/// roles, connections, Circle links or vault items, and at most the one device it signed in with.
fn is_new_account(c: &mut DbConnInner, user: &str, now: i64) -> Result<bool, OrgError> {
    let created: Option<chrono::NaiveDateTime> =
        users::table.filter(users::uuid.eq(user)).select(users::created_at).first(c).optional()?;
    let Some(created) = created else {
        return Ok(false);
    };
    if now - created.and_utc().timestamp() > NEW_ACCOUNT_SECS {
        return Ok(false);
    }
    if !reins_org::q_orgs_of_user_any(c, user)?.is_empty() {
        return Ok(false);
    }
    let devices: i64 = devices::table.filter(devices::user_uuid.eq(user)).count().get_result(c)?;
    let connections: i64 =
        reins_connections::table.filter(reins_connections::user_uuid.eq(user)).count().get_result(c)?;
    let roster: i64 = reins_rosters::table.filter(reins_rosters::user_uuid.eq(user)).count().get_result(c)?;
    let items: i64 = ciphers::table.filter(ciphers::user_uuid.eq(user)).count().get_result(c)?;
    let links: i64 = reins_circle_links::table
        .filter(reins_circle_links::owner_uuid.eq(user).or(reins_circle_links::helper_uuid.eq(user)))
        .count()
        .get_result(c)?;
    Ok(devices <= 1 && connections == 0 && roster == 0 && items == 0 && links == 0)
}

/// Joins `user` to the family whose invitation `token` is: an open, approved invitation for exactly this account's
/// address, taken by a new account ([`is_new_account`]). Single use. Returns the family's id.
pub fn join(c: &mut DbConnInner, user: &str, token: &str, now: i64) -> Result<String, OrgError> {
    c.transaction(|c| {
        let invite = reins_family::q_invite_by_hash(c, &token_hash(token))?
            .filter(|i| i.open(now))
            .ok_or(OrgError::Invalid(NOT_VALID))?;
        let (tenant, _) = family(c, &invite.org_uuid).map_err(|_| OrgError::Invalid(NOT_VALID))?;
        if !invite.approved {
            return Err(OrgError::Invalid("This invitation waits for another guardian."));
        }
        let email = normalize_email(&email_of(c, user)?).map_err(|_| OrgError::Invalid(NEW_ADDRESS))?;
        if !crate::crypto::ct_eq(&email, &invite.email) {
            return Err(OrgError::Invalid("This invitation is for another address."));
        }
        if !is_new_account(c, user, now)? {
            return Err(OrgError::Invalid(NEW_ADDRESS));
        }
        if !reins_family::q_use_invite(c, &invite.id, user, now)? {
            return Err(OrgError::Invalid(NOT_VALID));
        }
        let org = tenant.org_uuid;
        let membership = Membership::new(user.to_owned().into(), org.clone().into(), None);
        reins_org::q_add_member(c, &membership, Some(&invite.created_by), now)?;
        // An invitation as a minor is a proposal: it counts once the member accepts it on their phone (Review Focus 1).
        reins_family::q_save_member(
            c,
            &ReinsFamilyMember {
                org_uuid: org.clone(),
                user_uuid: user.to_owned(),
                minor: false,
                minor_proposed_at: invite.minor.then_some(now),
                minor_proposed_by: invite.minor.then(|| invite.created_by.clone()),
            },
        )?;
        if invite.guardian {
            grant_guardian(c, &org, membership.uuid.as_ref(), &invite.created_by, now)?;
        }
        event(
            c,
            &org,
            "member.joined",
            Some(user),
            Some(membership.uuid.as_ref()),
            &json!({"invite": invite.id, "minor": invite.minor, "guardian": invite.guardian}),
            now,
        )?;
        Ok(org)
    })
}

fn leave_view(l: &ReinsFamilyLeave) -> FamilyLeaveView {
    FamilyLeaveView {
        requested_at: l.requested_at,
        effective_at: l.effective_at,
        needs_consent: l.effective_at.is_none(),
    }
}

fn active_member(c: &mut DbConnInner, org: &str, user: &str) -> Result<ReinsFamilyMember, OrgError> {
    member_of(c, org, user)?.ok_or(OrgError::Forbidden)?;
    Ok(reins_family::q_member(c, org, user)?.unwrap_or(ReinsFamilyMember {
        org_uuid: org.to_owned(),
        user_uuid: user.to_owned(),
        minor: false,
        minor_proposed_at: None,
        minor_proposed_by: None,
    }))
}

/// A member asks to leave. An adult goes after the family's notice; a minor waits for a guardian's consent. Returns
/// the request and the guardians to tell (a minor's needs their action; an adult's is a status they act on).
pub fn request_leave(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    now: i64,
) -> Result<(FamilyLeaveView, Vec<String>), OrgError> {
    let (_, fam) = family(c, org)?;
    let member = active_member(c, org, user)?;
    if is_guardian(c, org, user)? {
        return Err(OrgError::Invalid("A guardian leaves when another guardian removes them."));
    }
    c.transaction(|c| {
        if let Some(existing) = reins_family::q_leave(c, org, user)? {
            return Ok((leave_view(&existing), Vec::new()));
        }
        let row = ReinsFamilyLeave {
            org_uuid: org.to_owned(),
            user_uuid: user.to_owned(),
            requested_at: now,
            effective_at: (!member.minor).then(|| now + i64::from(fam.notice_days.clamp(0, 7)) * 86_400),
            consented_by: None,
        };
        reins_family::q_insert_leave(c, &row)?;
        event(
            c,
            org,
            "family.leave_requested",
            Some(user),
            None,
            &json!({"minor": member.minor, "effective_at": row.effective_at}),
            now,
        )?;
        Ok((leave_view(&row), guardians(c, org)?))
    })
}

/// The member withdraws their request, before it takes effect.
pub fn cancel_leave(c: &mut DbConnInner, org: &str, user: &str, now: i64) -> Result<(), OrgError> {
    family(c, org)?;
    match reins_family::q_leave(c, org, user)? {
        Some(l) if l.effective_at.is_none_or(|t| t > now) => {
            reins_family::q_delete_leave(c, org, user)?;
            event(c, org, "family.leave_cancelled", Some(user), None, &json!({}), now)
        }
        _ => Err(OrgError::NotFound),
    }
}

/// A guardian consents to a minor's request: they go at the next sweep (within seconds).
pub fn consent_leave(c: &mut DbConnInner, org: &str, guardian: &str, member: &str, now: i64) -> Result<(), OrgError> {
    family(c, org)?;
    require(c, org, guardian, Capability::ManageMembers)?;
    if !reins_family::q_consent_leave(c, org, member, guardian, now)? {
        return Err(OrgError::NotFound);
    }
    event(c, org, "family.leave_consented", Some(guardian), None, &json!({"member": member}), now)
}

/// Leaves whose time came: each member is deprovisioned ([`Reason::FamilyLeave`]). Returns what
/// [`deprovision::finish`] still does after the commit.
pub fn sweep(c: &mut DbConnInner, now: i64) -> Result<Followups, OrgError> {
    let mut out = Vec::new();
    for leave in reins_family::q_due_leaves(c, now)? {
        let removal = c.transaction(|c| {
            let actor = leave.consented_by.clone().unwrap_or_else(|| leave.user_uuid.clone());
            let r = deprovision::q_deprovision(
                c,
                &leave.org_uuid,
                &leave.user_uuid,
                Reason::FamilyLeave,
                Some(&actor),
                now,
            )?;
            reins_family::q_delete_leave(c, &leave.org_uuid, &leave.user_uuid)?;
            Ok::<_, OrgError>(r)
        })?;
        out.extend(removal);
    }
    Ok(out)
}

/// A guardian marks a member a minor (`true`: proposed, counts once the member accepts on their phone, Review Focus
/// 1) or not (`false`: at once). Returns whether it waits for the member.
pub fn set_minor(
    c: &mut DbConnInner,
    org: &str,
    actor: &str,
    member: &str,
    minor: bool,
    now: i64,
) -> Result<bool, OrgError> {
    family(c, org)?;
    require(c, org, actor, Capability::ManageMembers)?;
    let mut row = active_member(c, org, member)?;
    if is_guardian(c, org, member)? {
        return Err(OrgError::Invalid("A guardian can't be a minor."));
    }
    let waits = minor && !row.minor;
    if waits {
        row.minor_proposed_at = Some(now);
        row.minor_proposed_by = Some(actor.to_owned());
    } else {
        row.minor = minor;
        row.minor_proposed_at = None;
        row.minor_proposed_by = None;
    }
    reins_family::q_save_member(c, &row)?;
    let kind = if waits {
        "family.minor_proposed"
    } else {
        "family.minor_set"
    };
    event(c, org, kind, Some(actor), None, &json!({"member": member, "minor": minor}), now)?;
    Ok(waits)
}

/// The member answers a guardian's proposal to mark them a minor.
pub fn answer_minor(c: &mut DbConnInner, org: &str, user: &str, accept: bool, now: i64) -> Result<(), OrgError> {
    family(c, org)?;
    let mut row = active_member(c, org, user)?;
    if row.minor_proposed_at.is_none() {
        return Err(OrgError::NotFound);
    }
    row.minor = row.minor || accept;
    row.minor_proposed_at = None;
    row.minor_proposed_by = None;
    reins_family::q_save_member(c, &row)?;
    let kind = if accept {
        "family.minor_accepted"
    } else {
        "family.minor_declined"
    };
    event(c, org, kind, Some(user), None, &json!({}), now)
}

/// The family as a member's phone shows it (a guardian also sees the requests to leave).
pub fn me(c: &mut DbConnInner, org: &str, user: &str) -> Result<FamilyMe, OrgError> {
    let (tenant, fam) = family(c, org)?;
    let row = active_member(c, org, user)?;
    let guardian = is_guardian(c, org, user)?;
    let mut names = Vec::new();
    for g in guardians(c, org)? {
        names.push(email_of(c, &g)?);
    }
    names.sort();
    let leaves = if guardian {
        let mut out = Vec::new();
        for l in reins_family::q_leaves(c, org)? {
            let minor = reins_family::q_member(c, org, &l.user_uuid)?.is_some_and(|m| m.minor);
            out.push(FamilyLeaveRequest {
                email: email_of(c, &l.user_uuid)?,
                member: l.user_uuid,
                minor,
                requested_at: l.requested_at,
                effective_at: l.effective_at,
            });
        }
        out
    } else {
        Vec::new()
    };
    Ok(FamilyMe {
        org_id: tenant.org_uuid,
        name: reins_org::q_org_name(c, org)?.unwrap_or_default(),
        minor: row.minor,
        guardian,
        guardians: names,
        notice_days: u8::try_from(fam.notice_days).unwrap_or(3),
        leave: reins_family::q_leave(c, org, user)?.as_ref().map(leave_view),
        minor_proposed: row.minor_proposed_at.is_some(),
        leaves,
    })
}

// ---- the phone API ---------------------------------------------------------------------------------------------

use reins_proto::org::{FamilyInviteNew, FamilyJoin, FamilyMinorAnswer};
use rocket::{Data, Route, http::Status, serde::json::Json};

use crate::{
    api::reins::{
        device_api::{PhoneResult, bad_request, internal, not_found, read_body_limited},
        now_unix,
    },
    auth::Headers,
    db::{DbConn, DbPool},
};

pub fn routes() -> Vec<Route> {
    routes![post_join, get_family, post_invite, post_leave, delete_leave, post_consent, post_minor, post_member_minor]
}

fn phone_err(e: &OrgError) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    match e {
        OrgError::Forbidden | OrgError::NotFound => not_found(),
        OrgError::Invalid(m) | OrgError::Taken(m) => bad_request(*m),
        OrgError::Internal(m) => internal(&crate::Error::new("Family", m.clone())),
    }
}

/// The family `user` is an active member of (404 for anyone else, the same as "orgs are off" or "not a family").
async fn family_of(user: &str, conn: &DbConn) -> PhoneResult<String> {
    if !super::enabled() {
        return Err(not_found());
    }
    let user = user.to_owned();
    conn.run(move |c| reins_org::q_orgs_of_user(c, &user))
        .await
        .map_err(|e| internal(&crate::Error::new("Family", e.to_string())))?
        .into_iter()
        .find(|o| o.tier() == Tier::Family)
        .map(|o| o.org_uuid)
        .ok_or_else(not_found)
}

async fn body<T: serde::de::DeserializeOwned>(data: Data<'_>) -> PhoneResult<T> {
    let bytes = read_body_limited(data, 4096).await?;
    serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid body: {e}")))
}

/// A new account takes an invitation (rate-limited by the invitation's secret: 32 random bytes).
#[post("/reins/api/org/family/join", data = "<data>")]
async fn post_join(headers: Headers, data: Data<'_>, conn: DbConn) -> PhoneResult<Json<FamilyMe>> {
    if !super::enabled() {
        return Err(not_found());
    }
    let user = headers.user.uuid.to_string();
    let join_body: FamilyJoin = body(data).await?;
    let now = now_unix();
    let u = user.clone();
    let org = conn.run(move |c| join(c, &u, &join_body.token, now)).await.map_err(|e| phone_err(&e))?;
    conn.run(move |c| me(c, &org, &user)).await.map(Json).map_err(|e| phone_err(&e))
}

#[get("/reins/api/org/family")]
async fn get_family(headers: Headers, conn: DbConn) -> PhoneResult<Json<FamilyMe>> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    conn.run(move |c| me(c, &org, &user)).await.map(Json).map_err(|e| phone_err(&e))
}

/// A guardian invites someone (the console does the same).
#[post("/reins/api/org/family/invites", data = "<data>")]
async fn post_invite(headers: Headers, data: Data<'_>, conn: DbConn) -> PhoneResult<Json<FamilyInviteCreated>> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let new: FamilyInviteNew = body(data).await?;
    let now = now_unix();
    conn.run(move |c| invite(c, &org, &user, &new.email, new.minor, new.guardian, now))
        .await
        .map(Json)
        .map_err(|e| phone_err(&e))
}

#[post("/reins/api/org/family/leave")]
async fn post_leave(
    headers: Headers,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> PhoneResult<Json<FamilyLeaveView>> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let now = now_unix();
    let (view, tell) = conn.run(move |c| request_leave(c, &org, &user, now)).await.map_err(|e| phone_err(&e))?;
    // Guardians must act on it (a minor's consent) or know when a member goes (an adult's notice).
    super::reviews::notify(pool.inner(), &tell, "family:leave");
    if view.effective_at.is_some_and(|t| t <= now) {
        sweep_now(&conn, pool.inner()).await;
    }
    Ok(Json(view))
}

#[delete("/reins/api/org/family/leave")]
async fn delete_leave(headers: Headers, conn: DbConn) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let now = now_unix();
    conn.run(move |c| cancel_leave(c, &org, &user, now)).await.map_err(|e| phone_err(&e))?;
    Ok(Status::NoContent)
}

/// A guardian consents to `member`'s leave; they are deprovisioned right away.
#[post("/reins/api/org/family/leaves/<member>/consent")]
async fn post_consent(
    member: &str,
    headers: Headers,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let (member, now) = (member.to_owned(), now_unix());
    conn.run(move |c| consent_leave(c, &org, &user, &member, now)).await.map_err(|e| phone_err(&e))?;
    sweep_now(&conn, pool.inner()).await;
    Ok(Status::NoContent)
}

/// The member answers a guardian's proposal to mark them a minor.
#[post("/reins/api/org/family/minor", data = "<data>")]
async fn post_minor(headers: Headers, data: Data<'_>, conn: DbConn) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let answer: FamilyMinorAnswer = body(data).await?;
    let now = now_unix();
    conn.run(move |c| answer_minor(c, &org, &user, answer.accept, now)).await.map_err(|e| phone_err(&e))?;
    Ok(Status::NoContent)
}

/// A guardian marks `member` a minor (it waits for their acceptance) or not.
#[post("/reins/api/org/family/members/<member>/minor", data = "<data>")]
async fn post_member_minor(
    member: &str,
    headers: Headers,
    data: Data<'_>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let org = family_of(&user, &conn).await?;
    let answer: FamilyMinorAnswer = body(data).await?;
    let (target, now) = (member.to_owned(), now_unix());
    let t = target.clone();
    let waits =
        conn.run(move |c| set_minor(c, &org, &user, &t, answer.accept, now)).await.map_err(|e| phone_err(&e))?;
    if waits {
        // The member must answer it on their phone.
        super::reviews::notify(pool.inner(), &[target], "family:minor");
    }
    Ok(Status::NoContent)
}

/// Runs [`sweep`] and finishes what it removed (a consent, a notice of 0 days).
pub async fn sweep_now(conn: &DbConn, pool: &DbPool) {
    let now = now_unix();
    match conn.run(move |c| sweep(c, now)).await {
        Ok(removals) => {
            for r in removals {
                deprovision::finish(r, Some(pool)).await;
            }
        }
        Err(e) => warn!("Family leaves: {e:?}"),
    }
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use super::super::{
        kms::tests::use_test_kms,
        testing::{add_user, db},
    };
    use super::*;
    use crate::api::reins::now_unix;
    use reins_policy::org::{ManagedOutcome, ManagedRequest, evaluate_managed};

    /// Mom's new family (Mom: `mom`), its activation applied on her phone.
    pub(crate) fn fam(c: &mut DbConnInner, now: i64) -> String {
        use_test_kms();
        add_user(c, "mom", "mom@family.example");
        let created = create_family(c, "mom", "mom@family.example", "The Lees", now).unwrap();
        let Proposed::Pending(change) = created.activation else {
            panic!("activation waits for her device signature");
        };
        admin::apply_alone(c, &created.org, &change, "mom", now).unwrap();
        created.org
    }

    fn new_user(c: &mut DbConnInner, uuid: &str, email: &str) {
        add_user(c, uuid, email);
    }

    #[test]
    fn a_family_has_no_domain_and_its_creator_is_guardian() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        let t = reins_org::q_org(&mut c, &org).unwrap().unwrap();
        assert_eq!((t.tier(), t.identity(), t.two_person), (Tier::Family, IdentityKind::FamilyInvite, true));
        assert!(reins_org::q_domains(&mut c, &org).unwrap().is_empty());
        let roles = super::super::roles_of_user(&mut c, &org, "mom").unwrap();
        for r in GUARDIAN_ROLES {
            assert!(roles.contains(&r), "{r:?}");
        }
        let (policy, _) = super::super::policy::active(&mut c, &org).unwrap().expect("the baseline is active");
        assert_eq!(policy.hard_also, Some(guardian_review()));
        assert_eq!(reins_family::q_family(&mut c, &org).unwrap().unwrap().notice_days, 3);
        // A second family by the same account is refused: an account belongs to one org.
        assert!(create_family(&mut c, "mom", "mom@family.example", "Again", now).is_err());
    }

    #[test]
    fn an_existing_account_is_never_captured() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        // Old: created an hour ago.
        let invite_for =
            |c: &mut DbConnInner, email: &str| invite(c, &org, "mom", email, true, false, now).unwrap().token;
        new_user(&mut c, "old", "old@example.com");
        diesel::update(users::table.filter(users::uuid.eq("old")))
            .set(users::created_at.eq(chrono::DateTime::from_timestamp(now - 3600, 0).unwrap().naive_utc()))
            .execute(&mut c)
            .unwrap();
        let t = invite_for(&mut c, "old@example.com");
        assert_eq!(join(&mut c, "old", &t, now), Err(OrgError::Invalid(NEW_ADDRESS)));
        // New, but with phone roles (someone's Personal account already in use).
        new_user(&mut c, "used", "used@example.com");
        diesel::insert_into(reins_rosters::table)
            .values((
                reins_rosters::user_uuid.eq("used"),
                reins_rosters::epoch.eq(1_i64),
                reins_rosters::entry.eq("{}"),
                reins_rosters::entry_hash.eq("h"),
                reins_rosters::created_at.eq(now),
            ))
            .execute(&mut c)
            .unwrap();
        let t = invite_for(&mut c, "used@example.com");
        assert_eq!(join(&mut c, "used", &t, now), Err(OrgError::Invalid(NEW_ADDRESS)));
        // In another org.
        new_user(&mut c, "worker", "worker@example.com");
        reins_org::tests::add_org(&mut c, "acme");
        reins_org::tests::add_member(&mut c, "acme", "m-w", "worker", reins_org::ACTIVE_STATUS);
        let t = invite_for(&mut c, "worker@example.com");
        assert_eq!(join(&mut c, "worker", &t, now), Err(OrgError::Invalid(NEW_ADDRESS)));
        // Nothing of any of them reached the family.
        for u in ["old", "used", "worker"] {
            assert!(reins_org::q_member_state(&mut c, &org, u).unwrap().is_none());
        }
        // The same answer each time: it names nobody.
        new_user(&mut c, "sam", "sam@example.com");
        let t = invite_for(&mut c, "Sam@Example.com ");
        assert_eq!(join(&mut c, "sam", &t, now).unwrap(), org);
        // Invited as a minor: proposed, not yet so, until Sam accepts.
        let row = reins_family::q_member(&mut c, &org, "sam").unwrap().unwrap();
        assert!(!row.minor && row.minor_proposed_at.is_some());
        answer_minor(&mut c, &org, "sam", true, now).unwrap();
        assert!(reins_family::q_member(&mut c, &org, "sam").unwrap().unwrap().minor);
    }

    #[test]
    fn invites_are_single_use_and_bound_to_the_email() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        let created = invite(&mut c, &org, "mom", "sam@example.com", false, false, now).unwrap();
        assert_eq!(created.token.len(), 43);
        let row = reins_family::q_invite(&mut c, &org, &created.id).unwrap().unwrap();
        assert_ne!(row.token_hash, created.token, "only the hash is kept");
        // Someone else with the link.
        new_user(&mut c, "eve", "eve@example.com");
        assert_eq!(
            join(&mut c, "eve", &created.token, now),
            Err(OrgError::Invalid("This invitation is for another address."))
        );
        new_user(&mut c, "sam", "sam@example.com");
        assert!(join(&mut c, "sam", "not-the-token", now).is_err());
        join(&mut c, "sam", &created.token, now).unwrap();
        // Used once.
        assert_eq!(join(&mut c, "sam", &created.token, now), Err(OrgError::Invalid(NOT_VALID)));
        // Expired.
        let late = invite(&mut c, &org, "mom", "kim@example.com", false, false, now).unwrap();
        new_user(&mut c, "kim", "kim@example.com");
        assert_eq!(join(&mut c, "kim", &late.token, now + INVITE_TTL_SECS), Err(OrgError::Invalid(NOT_VALID)));
        // Only guardians invite; at most 20 open.
        assert_eq!(invite(&mut c, &org, "sam", "x@example.com", false, false, now), Err(OrgError::Forbidden));
        for i in 0..19 {
            invite(&mut c, &org, "mom", &format!("k{i}@example.com"), false, false, now).unwrap();
        }
        assert!(invite(&mut c, &org, "mom", "one-more@example.com", false, false, now).is_err());
        assert!(normalize_email("not an email").is_err());
    }

    #[test]
    fn a_second_guardian_takes_two_people() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        // Mom alone adds Dad: nobody else could approve it.
        let dad = invite(&mut c, &org, "mom", "dad@example.com", false, true, now).unwrap();
        assert!(!dad.waiting);
        new_user(&mut c, "dad", "dad@example.com");
        join(&mut c, "dad", &dad.token, now).unwrap();
        assert!(is_guardian(&mut c, &org, "dad").unwrap());
        // A third guardian now takes both.
        let gran = invite(&mut c, &org, "mom", "gran@example.com", false, true, now).unwrap();
        assert!(gran.waiting);
        new_user(&mut c, "gran", "gran@example.com");
        assert_eq!(
            join(&mut c, "gran", &gran.token, now),
            Err(OrgError::Invalid("This invitation waits for another guardian."))
        );
        let change = reins_org::q_pending_changes(&mut c, &org).unwrap().pop().unwrap();
        assert!(admin::approve(&mut c, &org, &change.uuid, "mom", None, now).is_err(), "not by the proposer");
        admin::approve(&mut c, &org, &change.uuid, "dad", None, now).unwrap();
        join(&mut c, "gran", &gran.token, now).unwrap();
        assert!(is_guardian(&mut c, &org, "gran").unwrap());
    }

    pub(crate) fn joined(c: &mut DbConnInner, org: &str, user: &str, minor: bool, now: i64) {
        let email = format!("{user}@example.com");
        let t = invite(c, org, "mom", &email, minor, false, now).unwrap().token;
        new_user(c, user, &email);
        join(c, user, &t, now).unwrap();
        if minor {
            answer_minor(c, org, user, true, now).unwrap();
        }
    }

    #[test]
    fn a_minor_cannot_leave_without_consent() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "sam", true, now);
        let (view, tell) = request_leave(&mut c, &org, "sam", now).unwrap();
        assert!(view.needs_consent);
        assert_eq!(tell, ["mom"], "guardians are told: they must act");
        // A year later it still waits.
        assert!(sweep(&mut c, now + 365 * 86_400).unwrap().is_empty());
        assert!(member_of(&mut c, &org, "sam").unwrap().is_some());
        // Sam can't consent for himself.
        assert_eq!(consent_leave(&mut c, &org, "sam", "sam", now), Err(OrgError::Forbidden));
        consent_leave(&mut c, &org, "mom", "sam", now + 10).unwrap();
        let gone = sweep(&mut c, now + 10).unwrap();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].reason, Reason::FamilyLeave);
        assert!(member_of(&mut c, &org, "sam").unwrap().is_none());
    }

    #[test]
    fn an_adult_leaves_after_the_notice() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "ann", false, now);
        let (view, _) = request_leave(&mut c, &org, "ann", now).unwrap();
        assert_eq!(view.effective_at, Some(now + 3 * 86_400));
        assert!(!view.needs_consent);
        assert!(sweep(&mut c, now + 3 * 86_400 - 1).unwrap().is_empty(), "the notice holds");
        assert!(member_of(&mut c, &org, "ann").unwrap().is_some());
        assert_eq!(sweep(&mut c, now + 3 * 86_400).unwrap().len(), 1);
        assert!(member_of(&mut c, &org, "ann").unwrap().is_none());
        // The notice is only ever 0, 3 or 7 days.
        assert!(set_notice(&mut c, &org, "mom", 30, now).is_err());
        set_notice(&mut c, &org, "mom", 0, now).unwrap();
    }

    #[test]
    fn marking_an_adult_minor_needs_their_acceptance() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "ann", false, now);
        assert!(set_minor(&mut c, &org, "mom", "ann", true, now).unwrap(), "it waits for Ann");
        assert!(!reins_family::q_member(&mut c, &org, "ann").unwrap().unwrap().minor);
        // Ann can still leave as an adult meanwhile, and decline.
        answer_minor(&mut c, &org, "ann", false, now).unwrap();
        assert!(!reins_family::q_member(&mut c, &org, "ann").unwrap().unwrap().minor);
        set_minor(&mut c, &org, "mom", "ann", true, now).unwrap();
        answer_minor(&mut c, &org, "ann", true, now).unwrap();
        assert!(reins_family::q_member(&mut c, &org, "ann").unwrap().unwrap().minor);
        // Not a minor any more: at once.
        assert!(!set_minor(&mut c, &org, "mom", "ann", false, now).unwrap());
        assert!(!reins_family::q_member(&mut c, &org, "ann").unwrap().unwrap().minor);
        // Ann can't mark someone else, nor answer a proposal that doesn't exist.
        assert_eq!(set_minor(&mut c, &org, "ann", "mom", true, now), Err(OrgError::Forbidden));
        assert_eq!(answer_minor(&mut c, &org, "ann", true, now), Err(OrgError::NotFound));
    }

    #[test]
    fn the_baseline_asks_guardians_for_money_and_new_people() {
        let p = family_baseline("fam1");
        p.validate().unwrap();
        let eval = |tool: &str, args: serde_json::Value| {
            evaluate_managed(
                &ManagedRequest {
                    org: "fam1",
                    member: "sam",
                    connection: "c1",
                    tool,
                    args: &args,
                    now: 1_000,
                },
                &p,
                None,
            )
        };
        let buy = eval("payments_purchase_request", json!({"merchant": "Toys", "amount": "4.00", "currency": "USD"}));
        assert_eq!(buy.outcome, ManagedOutcome::Review);
        assert!(buy.obligations.iter().any(|o| o.spec == guardian_review()), "{:?}", buy.obligation_ids());
        let mail = eval("gmail_send", json!({"to": ["stranger@elsewhere.example"], "subject": "Hi", "body": "Hello"}));
        assert_eq!(mail.outcome, ManagedOutcome::Review);
        assert!(mail.obligations.iter().any(|o| o.spec == guardian_review()), "{:?}", mail.obligation_ids());
        // A guardian can't approve their own request (exclude_requester); the child can't fill a guardian's slot.
        assert!(!guardian_review().eligible("sam", &[], &[Role::Member], "sam"));
        assert!(!guardian_review().eligible("mom", &[], &[Role::Approver], "mom"));
        assert!(guardian_review().eligible("mom", &[], &[Role::Approver], "sam"));
    }

    #[test]
    fn a_single_guardian_activates_alone_two_guardians_need_both() {
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        // One guardian: her own device signature activated revision 1 (`fam`).
        assert!(super::super::policy::active(&mut c, &org).unwrap().is_some());
        let dad = invite(&mut c, &org, "mom", "dad@example.com", false, true, now).unwrap();
        new_user(&mut c, "dad", "dad@example.com");
        join(&mut c, "dad", &dad.token, now).unwrap();
        // Two guardians: the next activation waits for the other.
        let body = serde_json::to_string(&family_baseline(&org)).unwrap();
        let (revision, hash) = super::super::policy::save_draft(&mut c, &org, "mom", &body, now).unwrap();
        let action = AdminAction::ActivatePolicy {
            revision,
            hash,
        };
        let (Proposed::Pending(change), _) = admin::propose(&mut c, &org, "mom", &action, None, now).unwrap() else {
            panic!("it waits");
        };
        assert!(admin::apply_alone(&mut c, &org, &change, "mom", now).is_err(), "Dad must approve");
        admin::approve(&mut c, &org, &change, "dad", None, now).unwrap();
        assert_eq!(super::super::policy::active(&mut c, &org).unwrap().unwrap().0.revision, revision);
    }
}
