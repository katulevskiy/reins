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
    routes![get_org, post_challenge, post_device, put_posture]
}

/// Challenges by (account, device): one at a time each.
static CHALLENGES: LazyLock<Mutex<HashMap<(String, String), (String, i64)>>> = LazyLock::new(Mutex::default);

fn challenges() -> std::sync::MutexGuard<'static, HashMap<(String, String), (String, i64)>> {
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
    let row = ReinsOrgDevice {
        org_uuid: org.clone(),
        user_uuid: user.clone(),
        device_uuid: device.clone(),
        key_algorithm: request.key_algorithm.clone(),
        public_key: request.device_key.clone(),
        hardware: request.hardware,
        attestations: serde_json::to_string(&request.attestations).unwrap_or_else(|_| "[]".to_owned()),
        posture: serde_json::to_string(&request.posture).unwrap_or_else(|_| "{}".to_owned()),
        posture_at: now,
        enrolled_at: now,
        revoked_at: None,
    };
    let kinds: Vec<_> = request.attestations.iter().map(|a| a.kind).collect();
    let detail = json!({"device": device, "hardware": request.hardware, "attestations": kinds,
                        "posture": posture_summary(&request.posture)});
    conn.run(move |c| -> Result<(), super::OrgError> {
        use diesel::Connection;
        c.transaction(|c| {
            reins_org::q_upsert_device(c, &row)?;
            event(c, &org, "device.enrolled", Some(&user), Some(&device), &detail, now)
        })
    })
    .await
    .map_err(|e| internal(&crate::Error::new("Error enrolling the device", format!("{e:?}"))))?;
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
