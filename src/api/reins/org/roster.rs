//! The org's roster as executors trust it (`docs/design/decisions-v2/06-trust-and-compliance.md` §3: the org issuer key
//! signs "roster/role changes"): the active members with their groups, roles and enrolled devices, each device marked
//! `may_decide` under the active policy's posture rule. An executor checks reviewers' device signatures against it.
//!
//! Signed on demand and cached per org: re-signed when its content changes, and at least every
//! [`ROSTER_RESIGN_SECS`], so an executor can refuse an old one (a revoked device drops out within minutes).

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use diesel::prelude::*;
use reins_policy::org::{OrgPolicy, PostureRule};
use reins_proto::managed::{Envelope, EnvelopeKind, Roster, RosterDevice, RosterMember, canonical_json};

use super::{
    OrgError,
    kms::{self, KeyPurpose},
    posture,
};
use crate::db::{
    DbConnInner,
    models::reins_org::{self, ReinsOrgDevice, state},
    schema::{reins_org_devices, reins_org_members, users},
};

/// A signed roster is re-signed at least this often.
pub const ROSTER_RESIGN_SECS: i64 = 300;

/// The roster of `org` as of `now` (`issued_at` = `now`).
pub fn build(c: &mut DbConnInner, org: &str, policy: Option<&OrgPolicy>, now: i64) -> Result<Roster, OrgError> {
    let rule = policy.map_or_else(PostureRule::default, |p| p.posture.clone());
    let rows: Vec<(String, String)> = reins_org_members::table
        .filter(reins_org_members::org_uuid.eq(org))
        .filter(reins_org_members::state.eq(state::ACTIVE))
        .select((reins_org_members::user_uuid, reins_org_members::membership_uuid))
        .order(reins_org_members::user_uuid.asc())
        .load(c)?;
    let mut members = Vec::with_capacity(rows.len());
    for (user, membership) in rows {
        let email: String =
            users::table.filter(users::uuid.eq(&user)).select(users::email).first(c).optional()?.unwrap_or_default();
        let mut groups = reins_org::q_groups_of(c, &membership)?;
        groups.sort();
        let mut roles = reins_org::q_roles_of(c, org, &membership)?;
        roles.sort_by_key(|r| r.as_str());
        roles.dedup();
        let devices: Vec<ReinsOrgDevice> = reins_org_devices::table
            .filter(reins_org_devices::org_uuid.eq(org))
            .filter(reins_org_devices::user_uuid.eq(&user))
            .filter(reins_org_devices::revoked_at.is_null())
            .order(reins_org_devices::device_uuid.asc())
            .load(c)?;
        members.push(RosterMember {
            user,
            email,
            groups,
            roles,
            devices: devices
                .iter()
                .map(|d| RosterDevice {
                    device: d.device_uuid.clone(),
                    key: d.public_key.clone(),
                    may_decide: posture::may_decide(d, &rule, now).is_ok(),
                })
                .collect(),
        });
    }
    Ok(Roster {
        org: org.to_owned(),
        version: u64::try_from(now).unwrap_or(0),
        issued_at: now,
        members,
    })
}

/// org → (content hash without the time, issued at, envelope).
static CACHE: LazyLock<Mutex<HashMap<String, (String, i64, Envelope)>>> = LazyLock::new(Mutex::default);

fn content_hash(r: &Roster) -> String {
    let mut timeless = r.clone();
    timeless.issued_at = 0;
    timeless.version = 0;
    let text = canonical_json(&serde_json::to_value(&timeless).unwrap_or_default());
    reins_proto::canon::hex32(&reins_proto::canon::sha256(text.as_bytes()))
}

/// `org`'s roster, signed by its issuer key.
pub fn signed(c: &mut DbConnInner, org: &str, now: i64) -> Result<Envelope, OrgError> {
    let policy = super::policy::active(c, org)?.map(|(p, _)| p);
    let roster = build(c, org, policy.as_ref(), now)?;
    let hash = content_hash(&roster);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((h, at, env)) = cache.get(org)
        && *h == hash
        && now - at < ROSTER_RESIGN_SECS
    {
        return Ok(env.clone());
    }
    let env = kms::envelope(
        c,
        org,
        KeyPurpose::Issuer,
        EnvelopeKind::Roster,
        &serde_json::to_value(&roster).map_err(|e| OrgError::Internal(e.to_string()))?,
        now,
    )?;
    cache.insert(org.to_owned(), (hash, now, env.clone()));
    Ok(env)
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::org::{Posture, Role};

    use super::*;
    use crate::api::reins::org::{
        kms::tests::use_test_kms,
        testing::{add_org, db, member, set_state},
    };
    use crate::db::models::reins_org::attestation;

    fn enroll(c: &mut DbConnInner, org: &str, user: &str, device: &str, rooted: Option<bool>) {
        let posture = Posture {
            screen_lock: true,
            os_name: "Android".into(),
            os_version: "16".into(),
            rooted,
            app_version: "1.0".into(),
            collected_at: 100,
        };
        reins_org::q_enroll_device(
            c,
            &ReinsOrgDevice {
                org_uuid: org.into(),
                user_uuid: user.into(),
                device_uuid: device.into(),
                key_algorithm: "p256".into(),
                public_key: format!("key-{device}"),
                hardware: true,
                attestations: "{}".into(),
                posture: serde_json::to_string(&posture).unwrap(),
                posture_at: 100,
                enrolled_at: 100,
                revoked_at: None,
                attestation_state: attestation::NONE.into(),
                attestation_detail: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn revoked_devices_and_removed_members_are_absent() {
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "u1", &[Role::SecurityAdmin]);
        member(&mut c, "o1", "u2", &[]);
        enroll(&mut c, "o1", "u1", "d1", Some(false));
        enroll(&mut c, "o1", "u1", "d2", Some(false));
        enroll(&mut c, "o1", "u2", "d3", Some(false));
        reins_org::q_revoke_devices(&mut c, "o1", "u2", 150).unwrap();
        set_state(&mut c, "o1", "u2", state::REMOVED_BY_ADMIN);
        let r = build(&mut c, "o1", None, 200).unwrap();
        assert_eq!(r.members.len(), 1);
        let m = r.member("u1").unwrap();
        assert_eq!(m.roles, [Role::SecurityAdmin]);
        assert_eq!(m.devices.len(), 2);
        assert!(r.member("u2").is_none());
        assert!(build(&mut c, "o2", None, 200).unwrap().members.is_empty(), "another tenant");
    }

    #[test]
    fn posture_failures_are_flagged_not_hidden() {
        let mut c = db();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "u1", &[]);
        enroll(&mut c, "o1", "u1", "d1", Some(true));
        enroll(&mut c, "o1", "u1", "d2", Some(false));
        let r = build(&mut c, "o1", None, 200).unwrap();
        let flags: Vec<(String, bool)> =
            r.member("u1").unwrap().devices.iter().map(|d| (d.device.clone(), d.may_decide)).collect();
        assert_eq!(flags, [("d1".to_owned(), false), ("d2".to_owned(), true)]);
    }

    #[test]
    fn the_signed_roster_is_cached_until_it_changes_or_ages() {
        use_test_kms();
        let mut c = db();
        add_org(&mut c, "o9");
        member(&mut c, "o9", "u1", &[]);
        let a = signed(&mut c, "o9", 1_000).unwrap();
        assert_eq!(signed(&mut c, "o9", 1_010).unwrap(), a);
        assert_ne!(signed(&mut c, "o9", 1_000 + ROSTER_RESIGN_SECS).unwrap(), a, "aged");
        enroll(&mut c, "o9", "u1", "d1", Some(false));
        let b = signed(&mut c, "o9", 1_000 + ROSTER_RESIGN_SECS + 1).unwrap();
        assert_eq!(b.parse::<Roster>().unwrap().member("u1").unwrap().devices.len(), 1, "changed");
    }
}
