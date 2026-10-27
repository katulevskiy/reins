//! Removing a member (05 §2): "revokes the member's devices and agent connections; cancels their open tickets, or
//! reassigns them per policy; transfers the reviews waiting for them …; kills any credential leases; writes all of it
//! to audit. Target: under 60 seconds."
//!
//! The database part ([`q_deprovision`]) is one transaction and takes milliseconds: from its commit the member's
//! tokens, devices and connections no longer work. Credential leases die with the connections: a desktop daemon
//! can no longer reach the server for new ones, and its next call is refused, which makes it drop the profile's leases.
//! The rest ([`finish`]) runs after the commit and cannot undo it: the in-memory relay state, one last push to the
//! approval device (it then finds its session gone and removes the work profile), the ticket hook (P4 and O3 plug
//! their tickets and reviews in here) and ending the member's SSO sessions at WorkOS.

use std::{
    sync::{Arc, RwLock},
    time::Instant,
};

use serde_json::json;

use super::{
    OrgError, event,
    invariants::{EndPlan, OnLastOwner},
};
use crate::db::{
    DbConnInner, DbPool,
    models::{
        UserId,
        reins_org::{self, CutOff, SUBJECT_MEMBER, state},
    },
};

/// Why a member goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The directory marked them inactive (SCIM suspend).
    DirectorySuspend,
    /// The directory deleted them: the membership and the account go.
    DirectoryDelete,
    /// An admin removed them (a two-person change).
    AdminRemoval,
    /// Their identity provider no longer counts them (a Team org's GitHub membership), confirmed by a periodic sync
    /// (later; never from one sign-in's answer).
    #[allow(dead_code, reason = "the periodic provider sync that confirms twice comes later")]
    ProviderMembershipLost,
}

impl Reason {
    /// The member's state in the org after this removal.
    #[must_use]
    pub fn state(self) -> &'static str {
        match self {
            Self::DirectorySuspend => state::SUSPENDED_BY_DIRECTORY,
            Self::DirectoryDelete => state::DEPROVISIONED_BY_DIRECTORY,
            Self::AdminRemoval => state::REMOVED_BY_ADMIN,
            Self::ProviderMembershipLost => state::SUSPENDED_BY_PROVIDER,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectorySuspend => "directory_suspend",
            Self::DirectoryDelete => "directory_delete",
            Self::AdminRemoval => "admin_removal",
            Self::ProviderMembershipLost => "provider_membership_lost",
        }
    }
}

/// What a ticket hook did with a removed member's open tickets and the reviews waiting for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TicketOutcome {
    pub cancelled: u32,
    pub reassigned: u32,
}

/// Where deferred decisions (P4's tickets, O3's routed reviews) learn that a member is gone. Called once per removal,
/// after the database part committed; it must cancel or reassign by policy and must not block for long.
pub trait TicketHook: Send + Sync {
    fn on_member_removed(&self, org: &str, user: &str) -> TicketOutcome;
}

static TICKET_HOOK: RwLock<Option<Arc<dyn TicketHook>>> = RwLock::new(None);

/// Plugs in the ticket store (once, at launch). P4's tickets and O3's routed reviews register here.
#[allow(dead_code, reason = "the seam P4 and O3 plug into; nothing registers in O1")]
pub fn set_ticket_hook(hook: Arc<dyn TicketHook>) {
    *TICKET_HOOK.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
}

fn ticket_hook() -> Option<Arc<dyn TicketHook>> {
    TICKET_HOOK.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The result of the database part, carried to [`finish`].
#[derive(Debug)]
pub struct Removal {
    pub org: String,
    pub user: String,
    pub reason: Reason,
    pub cut: CutOff,
    pub enrolled_devices: usize,
    started: Instant,
}

/// The database part, inside the caller's transaction: cuts the user off ([`reins_org::q_cut_off_user`]), revokes
/// their enrolled devices, ends the membership (deleted for [`Reason::DirectoryDelete`]) and records
/// `member.deprovisioned`. Idempotent: a second run finds nothing left and records nothing.
pub fn q_deprovision(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    reason: Reason,
    actor: Option<&str>,
    now: i64,
) -> Result<Option<Removal>, OrgError> {
    let started = Instant::now();
    let Some(member) = reins_org::q_member_state(c, org, user)? else {
        return Ok(None);
    };
    let new_state = reason.state();
    // Only an active member is removed; a suspended one may still be deleted by the directory that suspended them.
    let escalation = reason == Reason::DirectoryDelete && member.state == state::SUSPENDED_BY_DIRECTORY;
    if !member.is_active() && !escalation {
        return Ok(None);
    }
    let membership = member.membership_uuid.clone();
    // The one membership rule every path shares: the directory may cut off the last Org owner, but the tenant keeps
    // its owner (and asks for recovery) rather than being left with none. `escalation`: already decided.
    let keep_owner = if member.is_active() {
        super::invariants::plan_membership_end(c, org, &membership, OnLastOwner::KeepOwner)? == EndPlan::KeepOwner
    } else {
        !reins_org::q_subject_roles(c, org, SUBJECT_MEMBER, &membership)?.is_empty()
            && super::invariants::recovery_mode(c, org)?
    };
    // Account-wide on purpose: this account is this org's alone (`reins_org_members`, one org per account).
    let cut = reins_org::q_cut_off_user(c, user)?;
    let enrolled_devices = reins_org::q_revoke_devices(c, org, user, now)?;
    // O3: their web passkeys go with their phones.
    crate::db::models::reins_org_routing::q_revoke_passkeys_of(c, org, user, now)?;
    // O2: their credential leases die now (the executor revokes the issued ones upstream), and their open requests and
    // review answers go.
    let leases = super::leases::q_revoke_for_deprovision(c, org, user, now)?;
    let requests = super::decisions::on_member_removed(c, org, user, now)?;
    let clear_external = reason == Reason::AdminRemoval;
    reins_org::q_end_membership(c, org, user, &membership, new_state, keep_owner, clear_external, actor, now)?;
    if keep_owner && member.is_active() {
        super::invariants::show_owner_recovery(c, org, now)?;
        event(c, org, "org.owner_recovery_needed", actor, Some(&membership), &json!({"reason": reason.as_str()}), now)?;
    }
    if reason == Reason::DirectoryDelete && !keep_owner {
        // The Managed account itself goes too (its vault, keys and the rest), through the WorkOS sync's retried
        // deletion. It is this org's account; a Personal account is a different one.
        let pending = super::super::workos_sync::pending_deletion(&UserId::from(user.to_owned()));
        crate::db::models::reins_workos::q_set_setting(c, &pending, user)?;
    }
    event(
        c,
        org,
        "member.deprovisioned",
        actor,
        Some(&membership),
        &json!({
            "reason": reason.as_str(),
            "state": new_state,
            "kept_as_last_owner": keep_owner,
            "devices": cut.devices,
            "enrolled_devices": enrolled_devices,
            "connections": cut.connections,
            "leases": leases,
            "requests_cancelled": requests,
            "sso_sessions": cut.sso_sessions.len(),
            "db_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }),
        now,
    )?;
    Ok(Some(Removal {
        org: org.to_owned(),
        user: user.to_owned(),
        reason,
        cut,
        enrolled_devices,
        started,
    }))
}

/// After the commit: forget the member's in-memory relay state, tell the approval device, let the ticket hook cancel or
/// reassign, and end the SSO sessions at WorkOS. Best effort: every failure is logged, none undoes the removal.
pub async fn finish(removal: Removal, pool: Option<&DbPool>) -> TicketOutcome {
    super::super::HUB.forget_user(&removal.user);
    if let (Some(pool), Some(token)) = (pool, removal.cut.push_token.clone()) {
        super::super::push::spawn_push(
            pool.clone(),
            UserId::from(removal.user.clone()),
            Some(token),
            reins_proto::pairing::PushMessage {
                t: reins_proto::pairing::PushKind::Replaced,
                id: String::new(),
                p: None,
            },
        );
    }
    let outcome = ticket_hook().map(|h| h.on_member_removed(&removal.org, &removal.user)).unwrap_or_default();
    if crate::sso_workos::enabled() {
        for session in &removal.cut.sso_sessions {
            if let Err(e) = crate::sso_workos::revoke_session(session).await {
                warn!("Deprovisioning {}: WorkOS session not ended: {e:?}", removal.user);
            }
        }
    }
    info!(
        "Org {}: member {} deprovisioned ({}) in {} ms: {} devices, {} enrolled devices, {} connections, {} tickets \
         cancelled, {} reassigned",
        removal.org,
        removal.user,
        removal.reason.as_str(),
        removal.started.elapsed().as_millis(),
        removal.cut.devices,
        removal.enrolled_devices,
        removal.cut.connections,
        outcome.cancelled,
        outcome.reassigned
    );
    outcome
}

#[cfg(all(test, sqlite))]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};

    use super::super::testing::*;
    use super::*;
    use crate::db::{
        models::{Device, DeviceId, ReinsConnection, ReinsRefreshToken, reins_org::ACTIVE_STATUS},
        schema::{devices, reins_connections, reins_refresh_tokens, users, users_organizations},
    };

    fn with_everything(c: &mut DbConnInner, user: &str, connections: usize, phones: usize) {
        for i in 0..phones {
            let d = Device::new(DeviceId::from(format!("{user}-d{i}")), user.to_owned().into(), "Phone".to_owned(), 0);
            diesel::insert_into(devices::table).values(&d).execute(c).unwrap();
        }
        for i in 0..connections {
            let conn = ReinsConnection::new(
                user.to_owned().into(),
                format!("client{i}"),
                "Claude".to_owned(),
                "claude.ai".to_owned(),
                "Claude".to_owned(),
                1,
            );
            diesel::insert_into(reins_connections::table).values(&conn).execute(c).unwrap();
            let token = ReinsRefreshToken {
                token_hash: format!("hash-{user}-{i}"),
                connection_uuid: conn.uuid.clone(),
                expires_at: 99,
                client_auth: "none".to_owned(),
            };
            diesel::insert_into(reins_refresh_tokens::table).values(&token).execute(c).unwrap();
        }
    }

    fn member_with_everything(c: &mut DbConnInner, org: &str, user: &str, m: &str, connections: usize, phones: usize) {
        add_member(c, org, m, user, ACTIVE_STATUS);
        with_everything(c, user, connections, phones);
    }

    fn setup() -> DbConnInner {
        let mut c = db();
        add_org(&mut c, "o1");
        member_with_everything(&mut c, "o1", "u1", "m1", 3, 2);
        member_with_everything(&mut c, "o1", "u2", "m2", 1, 1);
        add_group(&mut c, "o1", "g1", &["m1", "m2"]);
        c
    }

    fn devices_of(c: &mut DbConnInner, user: &str) -> i64 {
        devices::table.filter(devices::user_uuid.eq(user)).count().get_result(c).unwrap()
    }

    fn connections_of(c: &mut DbConnInner, user: &str) -> i64 {
        reins_connections::table.filter(reins_connections::user_uuid.eq(user)).count().get_result(c).unwrap()
    }

    fn enabled(c: &mut DbConnInner, user: &str) -> bool {
        users::table.filter(users::uuid.eq(user)).select(users::enabled).first(c).unwrap()
    }

    fn state_of(c: &mut DbConnInner, org: &str, user: &str) -> String {
        reins_org::q_member_state(c, org, user).unwrap().unwrap().state
    }

    #[test]
    fn everything_of_the_member_goes_and_others_stay() {
        let mut c = setup();
        let r = q_deprovision(&mut c, "o1", "u1", Reason::DirectorySuspend, None, 5).unwrap().unwrap();
        assert_eq!((r.cut.devices, r.cut.connections), (2, 3));
        assert_eq!((devices_of(&mut c, "u1"), devices_of(&mut c, "u2")), (0, 1));
        assert_eq!((connections_of(&mut c, "u1"), connections_of(&mut c, "u2")), (0, 1));
        assert_eq!(reins_refresh_tokens::table.count().get_result::<i64>(&mut c).unwrap(), 1);
        // The org's own state says it; the account-wide flag is never touched.
        assert_eq!(state_of(&mut c, "o1", "u1"), state::SUSPENDED_BY_DIRECTORY);
        assert!(enabled(&mut c, "u1"));
        let status: i32 = users_organizations::table
            .filter(users_organizations::uuid.eq("m1"))
            .select(users_organizations::status)
            .first(&mut c)
            .unwrap();
        assert!(status < 0, "revoked in Vaultwarden too, restorable");
        assert_eq!(reins_org::q_group_members(&mut c, "g1").unwrap(), ["m2"]);
        let e = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 1).unwrap().remove(0);
        assert_eq!((e.kind.as_str(), e.subject.as_deref()), ("member.deprovisioned", Some("m1")));
    }

    /// Two orgs and a Personal account: removing a member of one org touches nothing of the other org, and nothing of
    /// anyone's Personal account (a different account), and no account flag.
    #[test]
    fn one_orgs_removal_never_reaches_another_org_or_a_personal_account() {
        let mut c = setup();
        add_org(&mut c, "o2");
        member_with_everything(&mut c, "o2", "v1", "n1", 2, 1);
        add_user(&mut c, "personal", "dana@gmail.com");
        with_everything(&mut c, "personal", 2, 1);
        q_deprovision(&mut c, "o1", "u1", Reason::DirectoryDelete, None, 5).unwrap().unwrap();
        q_deprovision(&mut c, "o1", "u2", Reason::AdminRemoval, Some("x"), 5).unwrap().unwrap();
        assert_eq!(state_of(&mut c, "o2", "v1"), state::ACTIVE);
        assert_eq!((devices_of(&mut c, "v1"), connections_of(&mut c, "v1")), (1, 2));
        assert_eq!((devices_of(&mut c, "personal"), connections_of(&mut c, "personal")), (1, 2));
        assert!(enabled(&mut c, "v1") && enabled(&mut c, "personal"));
        // o2's view: no event of o1's removals.
        assert!(crate::db::models::reins_org_event::q_events(&mut c, "o2", None, 100).unwrap().is_empty());
        // And o1 can't remove o2's member: not its member.
        assert!(q_deprovision(&mut c, "o1", "v1", Reason::AdminRemoval, None, 6).unwrap().is_none());
        assert!(q_deprovision(&mut c, "o1", "personal", Reason::DirectoryDelete, None, 6).unwrap().is_none());
    }

    #[test]
    fn an_account_belongs_to_one_org_at_most() {
        let mut c = setup();
        add_org(&mut c, "o2");
        let m = crate::db::models::Membership::new("u1".to_owned().into(), "o2".to_owned().into(), None);
        assert!(reins_org::q_add_member(&mut c, &m, None, 1).is_err());
        assert!(reins_org::q_membership(&mut c, "o2", "u1").unwrap().is_none(), "nothing half-written");
    }

    #[test]
    fn a_directory_delete_keeps_the_membership_record_and_queues_the_account() {
        let mut c = setup();
        q_deprovision(&mut c, "o1", "u1", Reason::DirectoryDelete, None, 5).unwrap().unwrap();
        assert_eq!(state_of(&mut c, "o1", "u1"), state::DEPROVISIONED_BY_DIRECTORY);
        let pending = super::super::super::workos_sync::pending_deletion(&UserId::from("u1".to_owned()));
        assert!(crate::db::models::reins_workos::q_get_setting(&mut c, &pending).unwrap().is_some());
    }

    #[test]
    fn an_admin_removal_unlinks_the_directory() {
        let mut c = setup();
        diesel::update(users_organizations::table.filter(users_organizations::uuid.eq("m1")))
            .set(users_organizations::external_id.eq("directory_user_1"))
            .execute(&mut c)
            .unwrap();
        q_deprovision(&mut c, "o1", "u1", Reason::AdminRemoval, Some("u2"), 5).unwrap().unwrap();
        let m = reins_org::q_membership(&mut c, "o1", "u1").unwrap().unwrap();
        assert_eq!(m.external_id, None);
        assert_eq!(state_of(&mut c, "o1", "u1"), state::REMOVED_BY_ADMIN);
    }

    #[test]
    fn deprovisioning_is_idempotent() {
        let mut c = setup();
        assert!(q_deprovision(&mut c, "o1", "u1", Reason::AdminRemoval, Some("u2"), 5).unwrap().is_some());
        assert!(q_deprovision(&mut c, "o1", "u1", Reason::AdminRemoval, Some("u2"), 6).unwrap().is_none());
        assert!(q_deprovision(&mut c, "o1", "nobody", Reason::DirectorySuspend, None, 6).unwrap().is_none());
    }

    #[test]
    fn the_directory_never_leaves_an_org_without_an_owner() {
        let mut c = setup();
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", reins_proto::org::Role::OrgOwner);
        let r = q_deprovision(&mut c, "o1", "u1", Reason::DirectoryDelete, None, 5).unwrap().unwrap();
        // Cut off like anyone…
        assert_eq!((r.cut.devices, r.cut.connections), (2, 3));
        assert_eq!(state_of(&mut c, "o1", "u1"), state::DEPROVISIONED_BY_DIRECTORY);
        // …but still holding the role, the account kept, recovery open and shown.
        assert_eq!(reins_org::q_granted_roles(&mut c, "o1", "m1").unwrap(), [reins_proto::org::Role::OrgOwner]);
        assert!(super::super::invariants::recovery_mode(&mut c, "o1").unwrap());
        assert!(super::super::invariants::owner_recovery_shown(&mut c, "o1").unwrap());
        let pending = super::super::super::workos_sync::pending_deletion(&UserId::from("u1".to_owned()));
        assert!(crate::db::models::reins_workos::q_get_setting(&mut c, &pending).unwrap().is_none());
        let kinds: Vec<String> = crate::db::models::reins_org_event::q_events(&mut c, "o1", None, 2)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, ["member.deprovisioned", "org.owner_recovery_needed"]);
        // A second owner present: the directory removes the first as usual.
        let mut c = setup();
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", reins_proto::org::Role::OrgOwner);
        grant(&mut c, "o1", SUBJECT_MEMBER, "m2", reins_proto::org::Role::OrgOwner);
        q_deprovision(&mut c, "o1", "u1", Reason::DirectorySuspend, None, 5).unwrap().unwrap();
        assert!(reins_org::q_granted_roles(&mut c, "o1", "m1").unwrap().is_empty());
        assert!(!super::super::invariants::recovery_mode(&mut c, "o1").unwrap());
    }

    #[test]
    fn nothing_outside_the_org_opens_recovery() {
        let mut c = setup();
        grant(&mut c, "o1", SUBJECT_MEMBER, "m1", reins_proto::org::Role::OrgOwner);
        // The account disabled globally (an admin of the server, another reason): the org still has its owner.
        diesel::update(users::table.filter(users::uuid.eq("u1")))
            .set(users::enabled.eq(false))
            .execute(&mut c)
            .unwrap();
        assert!(!super::super::invariants::recovery_mode(&mut c, "o1").unwrap());
        assert_eq!(super::super::invariants::owners(&mut c, "o1").unwrap(), ["m1"]);
    }

    struct Counting(AtomicU32);

    impl TicketHook for Counting {
        fn on_member_removed(&self, org: &str, _user: &str) -> TicketOutcome {
            assert_eq!(org, "o1");
            self.0.fetch_add(1, Ordering::SeqCst);
            TicketOutcome {
                cancelled: 2,
                reassigned: 1,
            }
        }
    }

    #[tokio::test]
    async fn the_ticket_hook_is_called_once() {
        let mut c = setup();
        let hook = Arc::new(Counting(AtomicU32::new(0)));
        let dyn_hook: Arc<dyn TicketHook> = Arc::<Counting>::clone(&hook);
        set_ticket_hook(dyn_hook);
        let r = q_deprovision(&mut c, "o1", "u1", Reason::AdminRemoval, None, 5).unwrap().unwrap();
        let outcome = finish(r, None).await;
        assert_eq!(
            outcome,
            TicketOutcome {
                cancelled: 2,
                reassigned: 1
            }
        );
        assert_eq!(hook.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn it_takes_well_under_a_minute() {
        let mut c = db();
        add_org(&mut c, "o1");
        member_with_everything(&mut c, "o1", "u1", "m1", 50, 5);
        let started = Instant::now();
        let r = c.transaction(|c| q_deprovision(c, "o1", "u1", Reason::DirectoryDelete, None, 5)).unwrap().unwrap();
        let elapsed = started.elapsed();
        assert_eq!((r.cut.devices, r.cut.connections), (5, 50));
        assert!(elapsed.as_secs() < 60, "{elapsed:?}");
        assert!(elapsed.as_millis() < 2_000, "the database part should take milliseconds: {elapsed:?}");
    }
}
