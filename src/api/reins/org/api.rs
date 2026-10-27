//! The Managed account's phone API (O1): its own view of its org, and enrolling the device (06 §6: "a device key
//! generated in secure hardware, attested to the decision service"; posture recorded, gating in O2).
//!
//! - `GET /reins/api/org`: [`OrgMe`] for an active member; 404 otherwise. The same 404 answers "orgs are off", "not a
//!   member" and "a Personal account", so nothing here tells anyone which kind an account is.
//! - `POST /reins/api/org/enroll/challenge`: a single-use challenge for this account and device, 5 minutes.
//! - `POST /reins/api/org/devices`: [`EnrollmentRequest`]: the P-256 public key (SPKI), its signature over the
//!   challenge (proof that the device holds it), the attestations as the platform gave them, and the posture.
//! - `PUT /reins/api/org/posture`: the enrolled device's posture now.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use reins_proto::org::{EnrollmentChallenge, EnrollmentRequest, OrgMe, OrgRoutes, Posture, enrollment_message};
use rocket::{Data, Route, http::Status, serde::json::Json};
use serde_json::json;

use super::{enabled, event, member_of, random_token, roles_of_user};
use crate::{
    api::reins::{
        device_api::{PhoneResult, api_err, bad_request, internal, not_found, read_body_limited},
        now_unix,
    },
    auth::Headers,
    db::{
        DbConn, DbConnInner,
        models::reins_org::{self, ReinsOrgDevice},
    },
};

/// How long an enrollment challenge lives.
pub const CHALLENGE_SECS: i64 = 300;
/// Largest enrollment body (attestations included).
const MAX_ENROLL_BODY: u64 = 256 * 1024;

pub fn routes() -> Vec<Route> {
    routes![get_org, post_challenge, post_device, put_posture, get_reviews, post_review, put_member_policy]
}

/// Challenges by (account, device): one at a time each.
/// (account, device) -> (challenge, until).
type Challenges = HashMap<(String, String), (String, i64)>;

static CHALLENGES: LazyLock<Mutex<Challenges>> = LazyLock::new(Mutex::default);

fn challenges() -> std::sync::MutexGuard<'static, Challenges> {
    CHALLENGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Issues a challenge for `user` on `device`, replacing any earlier one.
pub fn issue_challenge(user: &str, device: &str, now: i64) -> EnrollmentChallenge {
    let mut map = challenges();
    map.retain(|_, (_, until)| *until > now);
    let challenge = random_token();
    let expires_at = now + CHALLENGE_SECS;
    map.insert((user.to_owned(), device.to_owned()), (challenge.clone(), expires_at));
    EnrollmentChallenge {
        challenge,
        expires_at,
    }
}

/// Takes the challenge of `user` on `device` if it is `given` and still valid. Single use: taken either way.
pub fn take_challenge(user: &str, device: &str, given: &str, now: i64) -> bool {
    let taken = challenges().remove(&(user.to_owned(), device.to_owned()));
    taken.is_some_and(|(c, until)| until > now && crate::crypto::ct_eq(&c, given))
}

/// The DER prefix of a P-256 SubjectPublicKeyInfo with an uncompressed point.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The uncompressed point of a base64 P-256 SPKI, if that is what it is.
pub fn p256_point(spki_b64: &str) -> Option<Vec<u8>> {
    let der = data_encoding::BASE64.decode(spki_b64.trim().as_bytes()).ok()?;
    (der.len() == 91 && der[..26] == P256_SPKI_PREFIX && der[26] == 0x04).then(|| der[26..].to_vec())
}

/// Whether `signature_b64` (DER ECDSA) by the key `spki_b64` signs the enrollment message of `challenge`.
pub fn signature_ok(spki_b64: &str, signature_b64: &str, challenge: &str) -> bool {
    let (Some(point), Ok(sig)) = (p256_point(spki_b64), data_encoding::BASE64.decode(signature_b64.trim().as_bytes()))
    else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
        .verify(&enrollment_message(challenge), &sig)
        .is_ok()
}

/// The org `user` is an active member of, if orgs are on.
async fn org_of(user: &str, conn: &DbConn) -> PhoneResult<Option<String>> {
    if !enabled() {
        return Ok(None);
    }
    let user = user.to_owned();
    conn.run(move |c| reins_org::q_orgs_of_user(c, &user))
        .await
        .map(|orgs| orgs.into_iter().next().map(|o| o.org_uuid))
        .map_err(|e| internal(&crate::Error::new("Error reading the account's organization", e.to_string())))
}

fn me(c: &mut DbConnInner, org: &str, user: &str, device: &str) -> Result<Option<OrgMe>, super::OrgError> {
    let Some(tenant) = reins_org::q_org(c, org)? else {
        return Ok(None);
    };
    if member_of(c, org, user)?.is_none() {
        return Ok(None);
    }
    let name = reins_org::q_org_name(c, org)?.unwrap_or_default();
    let domains =
        reins_org::q_domains(c, org)?.into_iter().filter(|d| d.verified_at.is_some()).map(|d| d.domain).collect();
    let enrolled = reins_org::q_device(c, org, user, device)?.is_some_and(|d| d.revoked_at.is_none());
    Ok(Some(OrgMe {
        org_id: tenant.org_uuid.clone(),
        name,
        tier: tenant.tier(),
        identity: tenant.identity(),
        domains,
        roles: roles_of_user(c, org, user)?,
        enrolled,
        routes: OrgRoutes {
            github_orgs: tenant.github_org.iter().cloned().collect(),
            hosts: Vec::new(),
        },
    }))
}

#[get("/reins/api/org")]
async fn get_org(headers: Headers, conn: DbConn) -> PhoneResult<Json<OrgMe>> {
    let user = headers.user.uuid.to_string();
    let device = headers.device.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let found = conn
        .run(move |c| me(c, &org, &user, &device))
        .await
        .map_err(|e| internal(&crate::Error::new("Error reading the organization", format!("{e:?}"))))?;
    found.map(Json).ok_or_else(not_found)
}

#[post("/reins/api/org/enroll/challenge")]
async fn post_challenge(headers: Headers, conn: DbConn) -> PhoneResult<Json<EnrollmentChallenge>> {
    let user = headers.user.uuid.to_string();
    if org_of(&user, &conn).await?.is_none() {
        return Err(not_found());
    }
    Ok(Json(issue_challenge(&user, &headers.device.uuid.to_string(), now_unix())))
}

#[post("/reins/api/org/devices", data = "<body>")]
async fn post_device(headers: Headers, conn: DbConn, body: Data<'_>) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let device = headers.device.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let bytes = read_body_limited(body, MAX_ENROLL_BODY).await?;
    let request: EnrollmentRequest =
        serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid enrollment: {e}")))?;
    request.validate().map_err(bad_request)?;
    let now = now_unix();
    if !take_challenge(&user, &device, &request.challenge, now) {
        return Err(api_err(Status::Forbidden, "bad_challenge", "The enrollment challenge is unknown or expired"));
    }
    if p256_point(&request.device_key).is_none() {
        return Err(bad_request("The device key is not a P-256 public key"));
    }
    if !signature_ok(&request.device_key, &request.signature, &request.challenge) {
        return Err(api_err(Status::Forbidden, "bad_signature", "The device key did not sign the challenge"));
    }
    // What each attestation must vouch for (`reins_proto::org::attestation_binding`), checked now and kept with them.
    let key_der = data_encoding::BASE64.decode(request.device_key.trim().as_bytes()).unwrap_or_default();
    let binding_bytes = reins_proto::org::attestation_binding(&request.challenge, &key_der);
    let binding = data_encoding::HEXLOWER.encode(&binding_bytes);
    let (attestation_state, attestation_detail) =
        super::attest::verify_enrollment(&request.attestations, &binding_bytes, &key_der);
    let row = ReinsOrgDevice {
        org_uuid: org.clone(),
        user_uuid: user.clone(),
        device_uuid: device.clone(),
        key_algorithm: request.key_algorithm.clone(),
        public_key: request.device_key.clone(),
        hardware: request.hardware,
        attestations: json!({"binding": binding, "items": request.attestations}).to_string(),
        posture: serde_json::to_string(&request.posture).unwrap_or_else(|_| "{}".to_owned()),
        posture_at: now,
        enrolled_at: now,
        revoked_at: None,
        attestation_state: attestation_state.to_owned(),
        attestation_detail: Some(attestation_detail.to_string()),
    };
    let kinds: Vec<_> = request.attestations.iter().map(|a| a.kind).collect();
    let detail = json!({"device": device, "hardware": request.hardware, "attestations": kinds,
                        "attestation_binding": binding, "attestation": attestation_state,
                        "posture": posture_summary(&request.posture)});
    let enrolled = conn
        .run(move |c| -> Result<bool, super::OrgError> {
            use diesel::Connection;
            c.transaction(|c| {
                // A device the org revoked stays revoked (a tombstone): it can't enroll itself again.
                if !reins_org::q_enroll_device(c, &row)? {
                    event(c, &org, "device.reenroll_refused", Some(&user), Some(&device), &json!({}), now)?;
                    return Ok(false);
                }
                event(c, &org, "device.enrolled", Some(&user), Some(&device), &detail, now)?;
                Ok(true)
            })
        })
        .await
        .map_err(|e| internal(&crate::Error::new("Error enrolling the device", format!("{e:?}"))))?;
    if !enrolled {
        return Err(api_err(Status::Forbidden, "device_revoked", "This device was removed by your organization"));
    }
    Ok(Status::Created)
}

fn posture_summary(p: &Posture) -> serde_json::Value {
    json!({"screen_lock": p.screen_lock, "os": format!("{} {}", p.os_name, p.os_version), "rooted": p.rooted,
           "app": p.app_version})
}

#[put("/reins/api/org/posture", data = "<body>")]
async fn put_posture(headers: Headers, conn: DbConn, body: Data<'_>) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let device = headers.device.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let bytes = read_body_limited(body, 8 * 1024).await?;
    let posture: Posture = serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid posture: {e}")))?;
    posture.validate().map_err(bad_request)?;
    let now = now_unix();
    let found = conn
        .run(move |c| -> Result<bool, super::OrgError> {
            let Some(current) = reins_org::q_device(c, &org, &user, &device)?.filter(|d| d.revoked_at.is_none()) else {
                return Ok(false);
            };
            let old: Option<Posture> = serde_json::from_str(&current.posture).ok();
            let changed = old.as_ref().is_none_or(|o| posture_summary(o) != posture_summary(&posture));
            let text = serde_json::to_string(&posture).unwrap_or_default();
            reins_org::q_set_posture(c, &org, &user, &device, &text, now)?;
            if changed {
                event(c, &org, "device.posture", Some(&user), Some(&device), &posture_summary(&posture), now)?;
            }
            Ok(true)
        })
        .await
        .map_err(|e| internal(&crate::Error::new("Error saving the posture", format!("{e:?}"))))?;
    if found {
        Ok(Status::NoContent)
    } else {
        Err(not_found())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenges_are_single_use_and_expire() {
        let c = issue_challenge("u-api-1", "d1", 100);
        assert!(!take_challenge("u-api-1", "d2", &c.challenge, 101), "another device");
        let c = issue_challenge("u-api-1", "d1", 100);
        assert!(take_challenge("u-api-1", "d1", &c.challenge, 101));
        assert!(!take_challenge("u-api-1", "d1", &c.challenge, 102), "used");
        let c = issue_challenge("u-api-1", "d1", 100);
        assert!(!take_challenge("u-api-1", "d1", &c.challenge, 100 + CHALLENGE_SECS), "expired");
        drop(issue_challenge("u-api-1", "d1", 100));
        assert!(!take_challenge("u-api-1", "d1", "guess", 101));
    }

    #[test]
    fn only_a_p256_key_that_signed_the_challenge_enrolls() {
        use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let mut spki = P256_SPKI_PREFIX.to_vec();
        spki.extend_from_slice(key.public_key().as_ref());
        let spki = data_encoding::BASE64.encode(&spki);
        let sig = data_encoding::BASE64.encode(key.sign(&rng, &enrollment_message("ch")).unwrap().as_ref());
        assert!(signature_ok(&spki, &sig, "ch"));
        assert!(!signature_ok(&spki, &sig, "other"));
        assert!(!signature_ok(&spki, "AAAA", "ch"));
        assert!(p256_point("MFkw").is_none());
        assert!(p256_point(&data_encoding::BASE64.encode(&[0u8; 91])).is_none());
    }
}

fn review_err(e: &super::OrgError) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    match e {
        super::OrgError::NotFound => not_found(),
        super::OrgError::Forbidden => api_err(Status::Forbidden, "forbidden", e.message()),
        super::OrgError::Invalid(m) | super::OrgError::Taken(m) => api_err(Status::Conflict, "refused", *m),
        super::OrgError::Internal(m) => internal(&crate::Error::new("Org review", m.clone())),
    }
}

/// `GET /reins/api/org/reviews`: the org reviews this member can answer (and their own to confirm).
#[get("/reins/api/org/reviews")]
async fn get_reviews(headers: Headers, conn: DbConn) -> PhoneResult<Json<Vec<reins_proto::managed::ReviewItem>>> {
    let user = headers.user.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let now = now_unix();
    conn.run(move |c| super::reviews::items_for(c, &org, &user, now)).await.map(Json).map_err(|e| review_err(&e))
}

/// `POST /reins/api/org/reviews/<id>`: a device-signed answer from this phone.
#[post("/reins/api/org/reviews/<id>", data = "<body>")]
async fn post_review(
    id: &str,
    headers: Headers,
    conn: DbConn,
    body: Data<'_>,
    pool: &rocket::State<crate::db::DbPool>,
) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let device = headers.device.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let bytes = read_body_limited(body, 8 * 1024).await?;
    let answer: reins_proto::managed::ReviewAnswer =
        serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid answer: {e}")))?;
    let review_id = id.to_owned();
    let (id, now) = (id.to_owned(), now_unix());
    let answered = conn
        .run(move |c| super::reviews::answer(c, &org, &user, &device, &id, &answer, now))
        .await
        .map_err(|e| review_err(&e))?;
    if matches!(answered, super::reviews::Answered::Approved | super::reviews::Answered::Denied) {
        // O3: Slack and Teams messages say how it ended.
        super::channels::spawn_update(pool.inner(), &review_id, None);
    }
    if let super::reviews::Answered::Applied(followups) = answered {
        for removal in followups {
            super::deprovision::finish(removal, Some(pool.inner())).await;
        }
    }
    Ok(Status::NoContent)
}

/// `PUT /reins/api/org/member-policy`: the member's own stricter policy, signed by this enrolled phone.
#[put("/reins/api/org/member-policy", data = "<body>")]
async fn put_member_policy(headers: Headers, conn: DbConn, body: Data<'_>) -> PhoneResult<Status> {
    let user = headers.user.uuid.to_string();
    let device = headers.device.uuid.to_string();
    let Some(org) = org_of(&user, &conn).await? else {
        return Err(not_found());
    };
    let bytes = read_body_limited(body, 64 * 1024).await?;
    let signed: reins_proto::managed::SignedMemberPolicy =
        serde_json::from_slice(&bytes).map_err(|e| bad_request(format!("invalid policy: {e}")))?;
    if signed.device != device {
        return Err(bad_request("Sign with this phone's key"));
    }
    let now = now_unix();
    conn.run(move |c| set_member_policy(c, &org, &user, &signed, now)).await.map_err(|e| review_err(&e))?;
    Ok(Status::NoContent)
}

/// Stores a member's own policy after checking it: signed now (within the clock skew) by this member's enrolled device
/// that may decide, and strictly newer than the one it replaces (an old signed policy can't be replayed).
pub fn set_member_policy(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    signed: &reins_proto::managed::SignedMemberPolicy,
    now: i64,
) -> Result<(), super::OrgError> {
    use super::OrgError;
    if (now - signed.at).abs() > super::reviews::SKEW_SECS {
        return Err(OrgError::Invalid("This phone's clock is off. Check the time and try again."));
    }
    let version = i64::try_from(signed.version).map_err(|_| OrgError::Invalid("That version can't be stored."))?;
    let dev = super::reviews::deciding_device(c, org, user, &signed.device, now)?;
    let msg = reins_proto::managed::member_policy_message(org, user, signed.version, signed.at, &signed.body);
    if !super::reviews::device_sig_ok(&dev.public_key, &signed.sig, &msg) {
        return Err(OrgError::Invalid("The policy isn't signed by this device."));
    }
    if crate::db::models::reins_org_keys::q_member_policy(c, org, user)?.is_some_and(|old| old.version >= version) {
        return Err(OrgError::Invalid("A newer policy is already set."));
    }
    let policy: reins_policy::org::MemberPolicy =
        serde_json::from_str(&signed.body).map_err(|_| OrgError::Invalid("That policy can't be read."))?;
    let hash = policy.hash();
    crate::db::models::reins_org_keys::q_set_member_policy(
        c,
        &crate::db::models::reins_org_keys::ReinsOrgMemberPolicy {
            org_uuid: org.to_owned(),
            user_uuid: user.to_owned(),
            body: signed.body.clone(),
            hash: hash.clone(),
            device_uuid: signed.device.clone(),
            version,
            at: signed.at,
            sig: signed.sig.clone(),
            updated_at: now,
        },
    )?;
    super::audit::record(
        c,
        org,
        super::audit::kinds::MEMBER_POLICY_SET,
        Some(user),
        None,
        &json!({"hash": hash, "device": signed.device, "version": signed.version}),
        now,
    )?;
    Ok(())
}

#[cfg(all(test, sqlite))]
mod member_policy_tests {
    use reins_proto::managed::{SignedMemberPolicy, member_policy_message};

    use super::*;
    use crate::api::reins::org::{
        OrgError,
        decisions::tests::pilot,
        reviews::tests::{Phone, enroll},
        testing::db,
    };

    fn signed(phone: &Phone, version: u64, at: i64) -> SignedMemberPolicy {
        let body = r#"{"matrix":null,"rules":[]}"#.to_owned();
        SignedMemberPolicy {
            sig: phone.sign(&member_policy_message("o1", "lee", version, at, &body)),
            body,
            version,
            at,
            device: phone.device.clone(),
        }
    }

    #[test]
    fn an_old_signed_policy_cannot_be_replayed() {
        let mut c = db();
        pilot(&mut c);
        let lee = enroll(&mut c, "o1", "lee", "lee-phone", Some(false), 100);
        let v1 = signed(&lee, 1, 100);
        set_member_policy(&mut c, "o1", "lee", &v1, 100).unwrap();
        set_member_policy(&mut c, "o1", "lee", &signed(&lee, 2, 101), 101).unwrap();
        assert_eq!(
            set_member_policy(&mut c, "o1", "lee", &v1, 102),
            Err(OrgError::Invalid("A newer policy is already set."))
        );
        assert!(set_member_policy(&mut c, "o1", "lee", &signed(&lee, 2, 103), 103).is_err(), "not strictly newer");
        // Signed long ago.
        assert!(set_member_policy(&mut c, "o1", "lee", &signed(&lee, 9, 100), 100 + 301).is_err());
        // Another version under the same signature.
        let mut forged = signed(&lee, 3, 104);
        forged.version = 4;
        assert!(set_member_policy(&mut c, "o1", "lee", &forged, 104).is_err());
    }

    #[test]
    fn a_device_that_may_not_decide_cannot_set_it() {
        let mut c = db();
        pilot(&mut c);
        let rooted = enroll(&mut c, "o1", "lee", "lee-phone", Some(true), 100);
        assert!(set_member_policy(&mut c, "o1", "lee", &signed(&rooted, 1, 100), 100).is_err());
    }
}
