//! Org reviews answered on the reviewer's phone with the device key enrolled to the org
//! (`docs/design/decisions-v2/06-trust-and-compliance.md` §3: "the reviewer's device signature over (request digest,
//! decision, policy snapshot hash, time)"; §6: "A failing device can't approve, and its reviews route to others").
//!
//! The same path answers admin changes (05 §3, 06 §3): every second-person approval is a device signature, and the
//! actions 06 §3 names (policy activation, retention, executors) also need the proposer's own device signature
//! (`ReviewDecision::Propose`) before anyone can approve them. Two device keys are needed, so one person with two
//! accounts can't approve with one phone.

use reins_policy::org::{Approval, Met, Obligation, OrgPolicy, PostureRule, Reviewers, obligations_met};
use reins_proto::{
    managed::{
        RequestBody, ReviewAnswer, ReviewDecision, ReviewItem, ReviewSignature, ReviewSubject, change_digest,
        review_message,
    },
    org::{AdminAction, Role},
};
use serde_json::{Value, json};

use super::{OrgError, admin, audit, decisions, kms, member_of, posture};
use crate::db::{
    DbConnInner, DbPool,
    models::{
        UserId,
        reins_org::{self, CHANGE_PENDING, ReinsOrgChange, ReinsOrgDevice},
        reins_org_request::{self, ReinsOrgApproval, ReinsOrgRequest, request_state},
    },
};

/// How far a reviewer's signed time may be from the server's.
pub const SKEW_SECS: i64 = 300;

/// Whether `sig_b64` (DER ECDSA P-256, base64) by the key `spki_b64` signs `msg`.
#[must_use]
pub fn device_sig_ok(spki_b64: &str, sig_b64: &str, msg: &[u8]) -> bool {
    let (Some(point), Ok(sig)) =
        (super::api::p256_point(spki_b64), data_encoding::BASE64.decode(sig_b64.trim().as_bytes()))
    else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point).verify(msg, &sig).is_ok()
}

/// What answering did.
#[derive(Debug)]
pub enum Answered {
    /// Counted; more is needed.
    Recorded,
    /// The request is approved and countersigned.
    Approved,
    Denied,
    /// The admin change was applied.
    Applied(admin::Followups),
}

fn posture_rule(c: &mut DbConnInner, org: &str) -> Result<PostureRule, OrgError> {
    Ok(super::policy::active(c, org)?.map_or_else(PostureRule::default, |(p, _): (OrgPolicy, _)| p.posture))
}

/// A member's groups and roles.
type Standing = (Vec<String>, Vec<Role>);

/// `user`'s groups and roles in `org`, if an active member.
fn standing(c: &mut DbConnInner, org: &str, user: &str) -> Result<Option<Standing>, OrgError> {
    let Some(m) = member_of(c, org, user)? else {
        return Ok(None);
    };
    let membership = m.uuid.to_string();
    Ok(Some((reins_org::q_groups_of(c, &membership)?, reins_org::q_roles_of(c, org, &membership)?)))
}

fn email_of(c: &mut DbConnInner, user: &str) -> Result<String, OrgError> {
    use crate::db::schema::users;
    use diesel::prelude::*;
    Ok(users::table
        .filter(users::uuid.eq(user))
        .select(users::email)
        .first::<String>(c)
        .optional()?
        .unwrap_or_default())
}

fn connection_label(c: &mut DbConnInner, connection: &str) -> Result<String, OrgError> {
    use crate::db::schema::reins_connections;
    use diesel::prelude::*;
    Ok(reins_connections::table
        .filter(reins_connections::uuid.eq(connection))
        .select(reins_connections::label)
        .first::<String>(c)
        .optional()?
        .unwrap_or_default())
}

fn group_name(c: &mut DbConnInner, org: &str, group: &str) -> Result<String, OrgError> {
    Ok(reins_org::q_groups(c, org)?
        .into_iter()
        .find(|(id, _, _)| id == group)
        .map_or_else(|| "a group".to_owned(), |g| g.1))
}

/// Who an obligation asks, in short words ("Security", "You").
fn requires(c: &mut DbConnInner, org: &str, o: &Obligation, own: bool) -> Result<String, OrgError> {
    Ok(match &o.spec.reviewers {
        Reviewers::Requester => {
            if own {
                "You".to_owned()
            } else {
                "The requester".to_owned()
            }
        }
        Reviewers::Group(g) => group_name(c, org, g)?,
        Reviewers::Role(r) => r.label().to_owned(),
        Reviewers::Members(_) => "Named reviewers".to_owned(),
        other => super::routing::label(c, org, other)?,
    })
}

/// Every approval of a request as policy weighs it: from members still active, with their current groups and roles.
fn weigh(c: &mut DbConnInner, org: &str, row: &ReinsOrgRequest) -> Result<(Met, Vec<ReviewSignature>), OrgError> {
    let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    let mut people: Vec<(String, Vec<String>, Vec<Role>, bool)> = Vec::new();
    let mut signatures = Vec::new();
    for a in reins_org_request::q_approvals(c, org, &row.uuid)? {
        let Some((groups, roles)) = standing(c, org, &a.user_uuid)? else {
            continue;
        };
        let approve = a.decision == ReviewDecision::Approve.as_str();
        if approve {
            signatures.push(ReviewSignature {
                member: a.user_uuid.clone(),
                device: a.device_uuid.clone(),
                decision: ReviewDecision::Approve,
                at: a.at,
                sig: a.sig.clone(),
                webauthn: None,
            });
        }
        people.push((a.user_uuid, groups, roles, approve));
    }
    let approvals: Vec<Approval<'_>> = people
        .iter()
        .map(|(m, g, r, approve)| Approval {
            member: m,
            groups: g,
            roles: r,
            approve: *approve,
        })
        .collect();
    Ok((obligations_met(&obligations, &row.user_uuid, &approvals), signatures))
}

/// The open reviews `user` can answer in `org` (and their own to confirm).
pub fn items_for(c: &mut DbConnInner, org: &str, user: &str, now: i64) -> Result<Vec<ReviewItem>, OrgError> {
    let Some((groups, roles)) = standing(c, org, user)? else {
        return Ok(Vec::new());
    };
    let org_name = reins_org::q_org_name(c, org)?.unwrap_or_default();
    let mut out = Vec::new();
    for row in reins_org_request::q_open_reviews(c, org)? {
        if row.expires_at <= now
            || reins_org_request::q_approvals(c, org, &row.uuid)?.iter().any(|a| a.user_uuid == user)
        {
            continue;
        }
        let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
        let Met::Waiting(waiting) = weigh(c, org, &row)?.0 else {
            continue;
        };
        let mine: Vec<&Obligation> = obligations
            .iter()
            .filter(|o| waiting.contains(&o.id) && o.spec.eligible(user, &groups, &roles, &row.user_uuid))
            .collect();
        let Some(first) = mine.first() else {
            continue;
        };
        let own = row.user_uuid == user;
        let body = reins_org_request::q_body(c, org, &row.uuid, "request")?.ok_or(OrgError::NotFound)?;
        let plain = kms::open_data(c, org, &body.key_id, &format!("{}/request", row.uuid), &body.sealed)?;
        let call: Value = serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(e.to_string()))?;
        let trace: Value = serde_json::from_str(&row.trace).unwrap_or_default();
        out.push(ReviewItem {
            id: row.uuid.clone(),
            subject: ReviewSubject::Request,
            org_name: org_name.clone(),
            requester: email_of(c, &row.user_uuid)?,
            connection: connection_label(c, &row.connection_uuid)?,
            title: trace["lines"]
                .as_array()
                .and_then(|l| l.first())
                .and_then(Value::as_str)
                .unwrap_or(&row.tool)
                .to_owned(),
            request: Some(RequestBody {
                org: org.to_owned(),
                member: row.user_uuid.clone(),
                connection: row.connection_uuid.clone(),
                tool: call["tool"].as_str().unwrap_or_default().to_owned(),
                args: call["args"].clone(),
            }),
            action: None,
            action_json: None,
            digest: row.digest.clone(),
            snapshot: row.snapshot.clone(),
            requires: requires(c, org, first, own)?,
            level: trace["level"].as_str().unwrap_or_default().to_owned(),
            zone: trace["zone"].as_str().unwrap_or_default().to_owned(),
            expires_at: row.expires_at,
            own,
            needs: Vec::new(),
            progress: String::new(),
            step: None,
            urgent: false,
            reason: None,
        });
    }
    for change in reins_org::q_pending_changes(c, org)? {
        if change.expires_at <= now {
            continue;
        }
        let Ok(action) = serde_json::from_str::<AdminAction>(&change.action) else {
            continue;
        };
        let answers = reins_org_request::q_approvals(c, org, &change.uuid)?;
        let proposed = answers.iter().any(|a| a.decision == ReviewDecision::Propose.as_str());
        let own = change.proposed_by == user;
        let show = if own {
            action.needs_device_signatures() && !proposed
        } else {
            let cap = admin::needed(c, org, &action)?;
            (proposed || !action.needs_device_signatures())
                && !answers.iter().any(|a| a.user_uuid == user)
                && super::can(c, org, user, cap)?
        };
        if !show {
            continue;
        }
        out.push(change_item(c, org, &org_name, &change, action, own)?);
    }
    Ok(out)
}

fn change_item(
    c: &mut DbConnInner,
    org: &str,
    org_name: &str,
    change: &ReinsOrgChange,
    action: AdminAction,
    own: bool,
) -> Result<ReviewItem, OrgError> {
    let title = crate::api::reins::org::console_pages::action_title(&action);
    Ok(ReviewItem {
        id: change.uuid.clone(),
        subject: ReviewSubject::Change,
        org_name: org_name.to_owned(),
        requester: email_of(c, &change.proposed_by)?,
        connection: String::new(),
        title,
        request: None,
        action: Some(action),
        action_json: Some(change.action.clone()),
        digest: change_digest(org, &change.uuid, &change.action),
        snapshot: String::new(),
        requires: if own {
            "You".to_owned()
        } else {
            "A second admin".to_owned()
        },
        level: String::new(),
        zone: String::new(),
        expires_at: change.expires_at,
        own,
        needs: Vec::new(),
        progress: String::new(),
        step: None,
        urgent: false,
        reason: None,
    })
}

/// The device answering, if it is enrolled to `org` for `user` and may decide now.
pub(super) fn deciding_device(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    device: &str,
    now: i64,
) -> Result<ReinsOrgDevice, OrgError> {
    let d = reins_org::q_device(c, org, user, device)?.ok_or(OrgError::Invalid("Enroll this phone to approve."))?;
    let rule = posture_rule(c, org)?;
    posture::may_decide(&d, &rule, now).map_err(|f| {
        info!("Org {org}: device {device} of {user} may not decide: {f:?}");
        OrgError::Invalid(match f {
            posture::PostureFail::Revoked => "This device was removed by your organization.",
            posture::PostureFail::NoScreenLock => "Set a screen lock to approve.",
            posture::PostureFail::Rooted => "This device can't approve: its system was modified.",
            posture::PostureFail::NotAttested => "This device couldn't prove it's genuine.",
            posture::PostureFail::OldOs {
                ..
            } => "Update the system to approve.",
            posture::PostureFail::OldApp {
                ..
            } => "Update Reins to approve.",
            posture::PostureFail::Stale => "Open Reins to refresh this device's status.",
        })
    })?;
    Ok(d)
}

/// Answers review `subject` (a request `or_…` or an admin change) as `user` on `device`.
pub fn answer(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    device: &str,
    subject: &str,
    ans: &ReviewAnswer,
    now: i64,
) -> Result<Answered, OrgError> {
    use diesel::Connection;
    if (now - ans.at).abs() > SKEW_SECS {
        return Err(OrgError::Invalid("This phone's clock is off. Check the time and try again."));
    }
    c.transaction(|c| {
        if member_of(c, org, user)?.is_none() {
            return Err(OrgError::NotFound);
        }
        let dev = deciding_device(c, org, user, device, now)?;
        if subject.starts_with("or_") {
            answer_request(c, org, user, &dev, subject, ans, now)
        } else {
            answer_change(c, org, user, &dev, subject, ans, now)
        }
    })
}

fn record_answer(
    c: &mut DbConnInner,
    org: &str,
    subject: &str,
    user: &str,
    dev: &ReinsOrgDevice,
    ans: &ReviewAnswer,
) -> Result<(), OrgError> {
    let fresh = reins_org_request::q_insert_approval(
        c,
        &ReinsOrgApproval {
            subject_uuid: subject.to_owned(),
            user_uuid: user.to_owned(),
            decision: ans.decision.as_str().to_owned(),
            org_uuid: org.to_owned(),
            device_uuid: dev.device_uuid.clone(),
            at: ans.at,
            sig: ans.signature.clone(),
            obligation: None,
        },
    )?;
    if fresh {
        Ok(())
    } else {
        Err(OrgError::Invalid("You already answered this."))
    }
}

fn answer_request(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    dev: &ReinsOrgDevice,
    id: &str,
    ans: &ReviewAnswer,
    now: i64,
) -> Result<Answered, OrgError> {
    if ans.decision == ReviewDecision::Propose {
        return Err(OrgError::Invalid("Approve or deny."));
    }
    let row = reins_org_request::q_request(c, org, id)?.ok_or(OrgError::NotFound)?;
    if row.state != request_state::REVIEWING || row.expires_at <= now {
        return Err(OrgError::Invalid("This was already decided."));
    }
    let msg = review_message(org, ReviewSubject::Request, id, &row.digest, ans.decision, &row.snapshot, ans.at);
    if !device_sig_ok(&dev.public_key, &ans.signature, &msg) {
        return Err(OrgError::Invalid("The answer isn't signed by this device."));
    }
    let (groups, roles) = standing(c, org, user)?.ok_or(OrgError::NotFound)?;
    let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    if !obligations.iter().any(|o| o.spec.eligible(user, &groups, &roles, &row.user_uuid)) {
        return Err(OrgError::Forbidden);
    }
    record_answer(c, org, id, user, dev, ans)?;
    let kind = if ans.decision == ReviewDecision::Approve {
        audit::kinds::REVIEW_APPROVED
    } else {
        audit::kinds::REVIEW_DENIED
    };
    audit::record(
        c,
        org,
        kind,
        Some(user),
        Some(id),
        &json!({"request": {"id": id, "digest": row.digest}, "policy": {"snapshot": row.snapshot},
                "decided_by": [{"member": user, "device": dev.device_uuid, "sig": ans.signature, "at": ans.at}]}),
        now,
    )?;
    match weigh(c, org, &row)? {
        (Met::Yes, signatures) => {
            decisions::finalize(c, org, &row, signatures, now)?;
            Ok(Answered::Approved)
        }
        (Met::Denied, _) => {
            decisions::deny(c, org, &row, user, now)?;
            Ok(Answered::Denied)
        }
        (Met::Waiting(_), _) => Ok(Answered::Recorded),
    }
}

fn answer_change(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    dev: &ReinsOrgDevice,
    id: &str,
    ans: &ReviewAnswer,
    now: i64,
) -> Result<Answered, OrgError> {
    let change = reins_org::q_change(c, org, id)?.ok_or(OrgError::NotFound)?;
    if change.state != CHANGE_PENDING || change.expires_at <= now {
        return Err(OrgError::Invalid("This change was already decided."));
    }
    let action: AdminAction =
        serde_json::from_str(&change.action).map_err(|e| OrgError::Internal(format!("stored change: {e}")))?;
    let digest = change_digest(org, id, &change.action);
    let msg = review_message(org, ReviewSubject::Change, id, &digest, ans.decision, "", ans.at);
    if !device_sig_ok(&dev.public_key, &ans.signature, &msg) {
        return Err(OrgError::Invalid("The answer isn't signed by this device."));
    }
    let answers = reins_org_request::q_approvals(c, org, id)?;
    match ans.decision {
        ReviewDecision::Propose => {
            if change.proposed_by != user || !action.needs_device_signatures() {
                return Err(OrgError::Forbidden);
            }
            record_answer(c, org, id, user, dev, ans)?;
            audit::record(
                c,
                org,
                "change.signed",
                Some(user),
                Some(id),
                &json!({"device": dev.device_uuid, "sig": ans.signature}),
                now,
            )?;
            // With nobody else to approve it (or two-person off), the proposer's device signature applies it.
            if admin::can_apply_alone(c, org, &change, &action)? {
                return admin::apply_alone(c, org, id, user, now).map(Answered::Applied);
            }
            Ok(Answered::Recorded)
        }
        ReviewDecision::Deny => {
            record_answer(c, org, id, user, dev, ans)?;
            admin::reject(c, org, id, user, now)?;
            Ok(Answered::Denied)
        }
        ReviewDecision::Approve => {
            if change.proposed_by == user {
                return Err(OrgError::Invalid("You can't approve your own change."));
            }
            if action.needs_device_signatures() {
                let proposal = answers.iter().find(|a| a.decision == ReviewDecision::Propose.as_str());
                let Some(p) = proposal else {
                    return Err(OrgError::Invalid("Its proposer hasn't confirmed it on their phone yet."));
                };
                let proposer_key = reins_org::q_device(c, org, &p.user_uuid, &p.device_uuid)?.map(|d| d.public_key);
                if p.device_uuid == dev.device_uuid || proposer_key.as_deref() == Some(dev.public_key.as_str()) {
                    return Err(OrgError::Invalid("A second person approves on their own phone."));
                }
            }
            record_answer(c, org, id, user, dev, ans)?;
            audit::record(
                c,
                org,
                "change.signed",
                Some(user),
                Some(id),
                &json!({"device": dev.device_uuid, "sig": ans.signature}),
                now,
            )?;
            if matches!(action, AdminAction::SetIdentity { .. }) {
                // Its binding needs WorkOS's word, which the console asks for: the approver finishes there.
                return Ok(Answered::Recorded);
            }
            admin::approve(c, org, id, user, None, now).map(Answered::Applied)
        }
    }
}

/// Whether `user` approved change `id` on their enrolled phone (the console's Approve requires it).
pub fn device_approved(c: &mut DbConnInner, org: &str, id: &str, user: &str) -> Result<bool, OrgError> {
    Ok(reins_org_request::q_approvals(c, org, id)?
        .iter()
        .any(|a| a.user_uuid == user && a.decision == ReviewDecision::Approve.as_str()))
}

/// Wakes the approval phones of `users`: a review waits for them (they must act, so this notifies).
pub fn notify(pool: &DbPool, users: &[String], subject: &str) {
    for user in users {
        crate::api::reins::push::spawn_push(
            pool.clone(),
            UserId::from(user.clone()),
            None,
            reins_proto::pairing::PushMessage {
                t: reins_proto::pairing::PushKind::OrgReview,
                id: subject.to_owned(),
                p: Some(reins_proto::pairing::push_profile_tag(user)),
            },
        );
    }
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use reins_proto::org::Posture;
    use ring::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
    };

    use super::*;
    use crate::api::reins::org::{
        decisions::tests::{ask, issue, pilot},
        testing::db,
    };
    use crate::db::models::reins_org::attestation;

    /// A phone's enrolled P-256 key.
    pub struct Phone {
        pub pkcs8: Vec<u8>,
        pub device: String,
    }

    impl Phone {
        pub fn sign(&self, msg: &[u8]) -> String {
            let rng = SystemRandom::new();
            let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.pkcs8, &rng).unwrap();
            data_encoding::BASE64.encode(key.sign(&rng, msg).unwrap().as_ref())
        }

        pub fn answer(
            &self,
            org: &str,
            subject: ReviewSubject,
            id: &str,
            digest: &str,
            d: ReviewDecision,
            snapshot: &str,
            at: i64,
        ) -> ReviewAnswer {
            ReviewAnswer {
                decision: d,
                at,
                signature: self.sign(&review_message(org, subject, id, digest, d, snapshot, at)),
            }
        }
    }

    const SPKI_PREFIX: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48,
        0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];

    /// Enrolls a phone for `user` in `org` with a good posture (rooted as given).
    pub fn enroll(c: &mut DbConnInner, org: &str, user: &str, device: &str, rooted: Option<bool>, now: i64) -> Phone {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let mut spki = SPKI_PREFIX.to_vec();
        spki.extend_from_slice(key.public_key().as_ref());
        let posture = Posture {
            screen_lock: true,
            os_name: "iOS".into(),
            os_version: "18".into(),
            rooted,
            app_version: "1.0".into(),
            collected_at: now,
        };
        reins_org::q_enroll_device(
            c,
            &ReinsOrgDevice {
                org_uuid: org.into(),
                user_uuid: user.into(),
                device_uuid: device.into(),
                key_algorithm: "p256".into(),
                public_key: data_encoding::BASE64.encode(&spki),
                hardware: true,
                attestations: "{}".into(),
                posture: serde_json::to_string(&posture).unwrap(),
                posture_at: now,
                enrolled_at: now,
                revoked_at: None,
                attestation_state: attestation::NONE.into(),
                attestation_detail: None,
            },
        )
        .unwrap();
        Phone {
            pkcs8: pkcs8.as_ref().to_vec(),
            device: device.into(),
        }
    }

    fn row(c: &mut DbConnInner, id: &str) -> ReinsOrgRequest {
        reins_org_request::q_request(c, "o1", id).unwrap().unwrap()
    }

    fn approve_as(c: &mut DbConnInner, phone: &Phone, user: &str, id: &str, now: i64) -> Result<Answered, OrgError> {
        let r = row(c, id);
        let a = phone.answer("o1", ReviewSubject::Request, id, &r.digest, ReviewDecision::Approve, &r.snapshot, now);
        answer(c, "o1", user, &phone.device, id, &a, now)
    }

    #[test]
    fn a_signed_approval_from_an_eligible_device_counts() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        let items = items_for(&mut c, "o1", "kim", 101).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].requires.as_str(), items[0].own), ("Group sec", false));
        assert!(matches!(approve_as(&mut c, &kim, "kim", &s.id, 101).unwrap(), Answered::Approved));
        assert_eq!(row(&mut c, &s.id).state, request_state::APPROVED);
        let record: reins_proto::managed::DecisionRecord =
            serde_json::from_str::<reins_proto::managed::Envelope>(row(&mut c, &s.id).decision.as_deref().unwrap())
                .unwrap()
                .parse()
                .unwrap();
        assert_eq!(record.approvals.len(), 1);
        assert_eq!(record.approvals[0].member, "kim");
        assert!(items_for(&mut c, "o1", "kim", 102).unwrap().is_empty());
    }

    #[test]
    fn a_revoked_device_cannot_approve() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        reins_org::q_revoke_devices(&mut c, "o1", "kim", 100).unwrap();
        assert!(approve_as(&mut c, &kim, "kim", &s.id, 101).is_err());
        assert_eq!(row(&mut c, &s.id).state, request_state::REVIEWING);
    }

    #[test]
    fn a_failing_posture_cannot_approve() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(true), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        let e = approve_as(&mut c, &kim, "kim", &s.id, 101).unwrap_err();
        assert_eq!(e, OrgError::Invalid("This device can't approve: its system was modified."));
        // Nobody who may decide: nobody is told.
        let obligations: Vec<Obligation> = serde_json::from_str(&row(&mut c, &s.id).obligations).unwrap();
        assert!(decisions::eligible_reviewers(&mut c, "o1", "lee", &obligations, 101).unwrap().is_empty());
    }

    #[test]
    fn another_orgs_device_is_refused() {
        let mut c = db();
        pilot(&mut c);
        let kim_elsewhere = enroll(&mut c, "o2", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        assert!(approve_as(&mut c, &kim_elsewhere, "kim", &s.id, 101).is_err());
    }

    #[test]
    fn a_signature_over_another_digest_or_snapshot_is_refused() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        let r = row(&mut c, &s.id);
        for a in [
            kim.answer("o1", ReviewSubject::Request, &s.id, "other", ReviewDecision::Approve, &r.snapshot, 101),
            kim.answer("o1", ReviewSubject::Request, &s.id, &r.digest, ReviewDecision::Approve, "other", 101),
            kim.answer("o2", ReviewSubject::Request, &s.id, &r.digest, ReviewDecision::Approve, &r.snapshot, 101),
            kim.answer("o1", ReviewSubject::Request, &s.id, &r.digest, ReviewDecision::Deny, &r.snapshot, 101),
        ] {
            let mut a = a;
            a.decision = ReviewDecision::Approve;
            assert!(answer(&mut c, "o1", "kim", "kim-phone", &s.id, &a, 101).is_err());
        }
    }

    #[test]
    fn an_old_signature_is_refused() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        let r = row(&mut c, &s.id);
        let a = kim.answer("o1", ReviewSubject::Request, &s.id, &r.digest, ReviewDecision::Approve, &r.snapshot, 100);
        assert!(answer(&mut c, "o1", "kim", "kim-phone", &s.id, &a, 100 + SKEW_SECS + 1).is_err());
    }

    #[test]
    fn the_requester_cannot_satisfy_an_org_review() {
        let mut c = db();
        pilot(&mut c);
        crate::api::reins::org::testing::add_group(&mut c, "o1", "sec2", &["m-lee"]);
        let lee = enroll(&mut c, "o1", "lee", "lee-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        assert!(items_for(&mut c, "o1", "lee", 101).unwrap().is_empty());
        assert_eq!(approve_as(&mut c, &lee, "lee", &s.id, 101).unwrap_err(), OrgError::Forbidden);
    }

    #[test]
    fn one_person_counts_once() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        approve_as(&mut c, &kim, "kim", &s.id, 101).unwrap();
        assert!(approve_as(&mut c, &kim, "kim", &s.id, 102).is_err());
    }

    #[test]
    fn a_deny_by_an_eligible_reviewer_denies() {
        let mut c = db();
        pilot(&mut c);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let s = ask(&mut c, &issue("acme/prod-db"), 100);
        let r = row(&mut c, &s.id);
        let a = kim.answer("o1", ReviewSubject::Request, &s.id, &r.digest, ReviewDecision::Deny, &r.snapshot, 101);
        assert!(matches!(answer(&mut c, "o1", "kim", "kim-phone", &s.id, &a, 101).unwrap(), Answered::Denied));
        assert_eq!(row(&mut c, &s.id).state, request_state::DENIED);
    }

    fn change_answer(
        c: &mut DbConnInner,
        phone: &Phone,
        user: &str,
        id: &str,
        d: ReviewDecision,
        now: i64,
    ) -> Result<Answered, OrgError> {
        let ch = reins_org::q_change(c, "o1", id).unwrap().unwrap();
        let digest = change_digest("o1", id, &ch.action);
        let a = phone.answer("o1", ReviewSubject::Change, id, &digest, d, "", now);
        answer(c, "o1", user, &phone.device, id, &a, now)
    }

    #[test]
    fn a_change_needs_the_proposers_signature_then_another_device() {
        use reins_proto::org::ExecutorMode;
        let mut c = db();
        pilot(&mut c);
        let dana = enroll(&mut c, "o1", "dana", "dana-phone", Some(false), 100);
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        let action = AdminAction::RegisterExecutor {
            executor_id: "e1".into(),
            mode: ExecutorMode::Cloud,
            public_key: reins_proto::canon::encode_payload(
                kms::signing_key(&mut c, "o9", kms::KeyPurpose::Issuer, 1).unwrap().public.as_ref(),
            ),
        };
        let (admin::Proposed::Pending(id), _) = admin::propose(&mut c, "o1", "dana", &action, None, 100).unwrap()
        else {
            panic!("pending")
        };
        // Kim can't approve before Dana confirms; Dana sees it as hers to confirm.
        assert!(items_for(&mut c, "o1", "kim", 101).unwrap().iter().all(|i| i.id != id));
        assert!(items_for(&mut c, "o1", "dana", 101).unwrap().iter().any(|i| i.id == id && i.own));
        assert!(change_answer(&mut c, &kim, "kim", &id, ReviewDecision::Approve, 101).is_err());
        assert!(change_answer(&mut c, &kim, "kim", &id, ReviewDecision::Propose, 101).is_err(), "not the proposer");
        assert!(matches!(
            change_answer(&mut c, &dana, "dana", &id, ReviewDecision::Propose, 101).unwrap(),
            Answered::Recorded
        ));
        assert!(change_answer(&mut c, &dana, "dana", &id, ReviewDecision::Approve, 102).is_err(), "not her own");
        assert!(matches!(
            change_answer(&mut c, &kim, "kim", &id, ReviewDecision::Approve, 102).unwrap(),
            Answered::Applied(_)
        ));
        assert!(crate::db::models::reins_org_exec::q_executor(&mut c, "o1", "e1").unwrap().is_some());
    }

    #[test]
    fn the_same_device_key_cannot_propose_and_approve() {
        let mut c = db();
        pilot(&mut c);
        let dana = enroll(&mut c, "o1", "dana", "dana-phone", Some(false), 100);
        // One person, two accounts, one phone: Kim's account enrolled the very same key under another device id.
        let row = reins_org::q_device(&mut c, "o1", "dana", "dana-phone").unwrap().unwrap();
        reins_org::q_enroll_device(
            &mut c,
            &ReinsOrgDevice {
                user_uuid: "kim".into(),
                device_uuid: "kim-phone".into(),
                ..row
            },
        )
        .unwrap();
        let kim_same_key = Phone {
            pkcs8: dana.pkcs8.clone(),
            device: "kim-phone".into(),
        };
        let action = AdminAction::SetRetention {
            days: 365,
        };
        let (admin::Proposed::Pending(id), _) = admin::propose(&mut c, "o1", "dana", &action, None, 100).unwrap()
        else {
            panic!("pending")
        };
        change_answer(&mut c, &dana, "dana", &id, ReviewDecision::Propose, 101).unwrap();
        let e = change_answer(&mut c, &kim_same_key, "kim", &id, ReviewDecision::Approve, 102).unwrap_err();
        assert_eq!(e, OrgError::Invalid("A second person approves on their own phone."));
        assert_eq!(reins_org::q_org(&mut c, "o1").unwrap().unwrap().body_retention_days, 30);
    }

    #[test]
    fn the_console_alone_cannot_approve_a_change() {
        let mut c = db();
        pilot(&mut c);
        let action = AdminAction::GrantRole {
            subject: reins_proto::org::Subject::Member("m-lee".into()),
            role: Role::Auditor,
        };
        let (admin::Proposed::Pending(id), _) = admin::propose(&mut c, "o1", "dana", &action, None, 100).unwrap()
        else {
            panic!("pending")
        };
        assert!(!device_approved(&mut c, "o1", &id, "kim").unwrap());
        assert_eq!(
            admin::approve_from_console(&mut c, "o1", &id, "kim", None, 101).unwrap_err(),
            OrgError::Invalid("Approve it on your phone first.")
        );
        let kim = enroll(&mut c, "o1", "kim", "kim-phone", Some(false), 100);
        assert!(matches!(
            change_answer(&mut c, &kim, "kim", &id, ReviewDecision::Approve, 101).unwrap(),
            Answered::Applied(_)
        ));
        assert_eq!(reins_org::q_roles_of(&mut c, "o1", "m-lee").unwrap(), [Role::Auditor]);
    }
}
