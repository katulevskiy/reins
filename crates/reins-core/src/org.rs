//! A work profile's organization calls (O1): which org the signed-in account belongs to, and enrolling this device
//! to it (06 §6). They run on the work profile's own core (`profiles`): nothing here can read another profile's store,
//! so what an org receives is built only from the work account's session and what the app passes in about the device.

use reins_proto::org::{Attestation, AttestationKind, EnrollmentRequest, KEY_P256, OrgMe, Posture};

use crate::{CoreError, engine::Engine, phone_api::ApiFailure, session::api_call};

/// The work account's organization, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct OrgView {
    pub org_id: String,
    pub name: String,
    /// `team`, `business`, `enterprise`.
    pub tier: String,
    /// The roles' labels (`Security admin`, …).
    pub roles: Vec<String>,
    /// This device is enrolled (and not removed).
    pub enrolled: bool,
    pub github_orgs: Vec<String>,
}

impl From<OrgMe> for OrgView {
    fn from(me: OrgMe) -> Self {
        Self {
            org_id: me.org_id,
            name: me.name,
            tier: me.tier.as_str().to_owned(),
            roles: me.roles.iter().map(|r| r.label().to_owned()).collect(),
            enrolled: me.enrolled,
            github_orgs: me.routes.github_orgs,
        }
    }
}

/// What the platform measured about the device (06 §6).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PostureInput {
    pub screen_lock: bool,
    pub os_name: String,
    pub os_version: String,
    /// `None`: the check could not run.
    pub rooted: Option<bool>,
    pub app_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AttestationKindInput {
    AppAttest,
    AndroidKeyAttestation,
    PlayIntegrity,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AttestationInput {
    pub kind: AttestationKindInput,
    /// As the platform gave it, base64.
    pub data: String,
}

/// The device key the app made in secure hardware for this org, its signature over
/// [`crate::profiles::org_enrollment_message`] of the challenge, and what vouches for it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrollInput {
    /// SubjectPublicKeyInfo DER, base64 (P-256).
    pub device_key: String,
    /// ECDSA P-256 SHA-256, DER, base64.
    pub signature: String,
    pub hardware: bool,
    pub attestations: Vec<AttestationInput>,
    pub posture: PostureInput,
}

fn posture(p: PostureInput, now: i64) -> Posture {
    Posture {
        screen_lock: p.screen_lock,
        os_name: p.os_name,
        os_version: p.os_version,
        rooted: p.rooted,
        app_version: p.app_version,
        collected_at: now,
    }
}

/// The request sent to the org: the challenge and the app's inputs, nothing else.
#[must_use]
pub fn enrollment_request(challenge: String, input: EnrollInput, now: i64) -> EnrollmentRequest {
    EnrollmentRequest {
        challenge,
        device_key: input.device_key,
        key_algorithm: KEY_P256.to_owned(),
        hardware: input.hardware,
        attestations: input
            .attestations
            .into_iter()
            .map(|a| Attestation {
                kind: match a.kind {
                    AttestationKindInput::AppAttest => AttestationKind::AppAttest,
                    AttestationKindInput::AndroidKeyAttestation => AttestationKind::AndroidKeyAttestation,
                    AttestationKindInput::PlayIntegrity => AttestationKind::PlayIntegrity,
                    AttestationKindInput::None => AttestationKind::None,
                },
                data: a.data,
            })
            .collect(),
        posture: posture(input.posture, now),
        signature: input.signature,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().try_into().unwrap_or(i64::MAX))
}

fn org_error(e: ApiFailure) -> CoreError {
    match e {
        ApiFailure::Status {
            status: 403,
            message,
            ..
        } => CoreError::invalid(message),
        other => other.into_core(),
    }
}

impl Engine {
    /// The signed-in account's organization; `None` for a Personal account (or a server without organizations).
    pub async fn org_membership(&self) -> Result<Option<OrgView>, CoreError> {
        let session = self.session()?;
        let me = api_call!(&session, |api| api.org_me()).map_err(ApiFailure::into_core)?;
        Ok(me.map(OrgView::from))
    }

    /// A fresh enrollment challenge for this device.
    pub async fn org_challenge(&self) -> Result<String, CoreError> {
        let session = self.session()?;
        let c = api_call!(&session, |api| api.org_challenge()).map_err(org_error)?;
        Ok(c.challenge)
    }

    /// Enrolls this device with the key and signature the app made over `challenge`.
    pub async fn org_enroll(&self, challenge: String, input: EnrollInput) -> Result<(), CoreError> {
        let request = enrollment_request(challenge, input, now());
        request.validate().map_err(CoreError::invalid)?;
        let session = self.session()?;
        api_call!(&session, |api| api.org_enroll(&request)).map_err(org_error)
    }

    /// Reports the enrolled device's posture now.
    pub async fn org_report_posture(&self, input: PostureInput) -> Result<(), CoreError> {
        let p = posture(input, now());
        p.validate().map_err(CoreError::invalid)?;
        let session = self.session()?;
        api_call!(&session, |api| api.org_posture(&p)).map_err(org_error)
    }

    /// Whether `server_url` offers organizations (its admin console answers): only then does Settings offer "Add work
    /// account". A server that can't be reached offers none.
    pub async fn orgs_offered(&self, server_url: &str) -> Result<bool, CoreError> {
        let server = crate::http::ServerUrl::parse(server_url)?;
        let response =
            self.http.get(server.join("/reins/org")).timeout(std::time::Duration::from_secs(10)).send().await;
        Ok(response.is_ok_and(|r| r.status().is_success()))
    }

    /// The tag the server puts on this account's pushes ([`reins_proto::pairing::push_profile_tag`]); `None` signed
    /// out. A phone with a work profile routes a push to the profile whose tag it carries.
    pub async fn push_tag(&self) -> Result<Option<String>, CoreError> {
        let Ok(session) = self.session() else {
            return Ok(None);
        };
        Ok(Some(reins_proto::pairing::push_profile_tag(&session.account_user_id().await?)))
    }

    /// Starts an "Add work account" sign-in: like `sso_begin`, sent to the org that verified `domain`.
    pub fn sso_begin_work(&self, server_url: &str, domain: &str) -> Result<crate::types::SsoStart, CoreError> {
        let server = crate::http::ServerUrl::parse(server_url)?;
        let start = crate::sso::begin_work(&server, domain)?;
        Ok(crate::types::SsoStart {
            url: start.url,
            callback_scheme: crate::sso::CALLBACK_SCHEME.to_owned(),
            state: start.state,
            verifier: start.verifier.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> EnrollInput {
        EnrollInput {
            device_key: "MFkw".to_owned(),
            signature: "MEUC".to_owned(),
            hardware: true,
            attestations: vec![AttestationInput {
                kind: AttestationKindInput::AndroidKeyAttestation,
                data: "MII".to_owned(),
            }],
            posture: PostureInput {
                screen_lock: true,
                os_name: "Android".to_owned(),
                os_version: "16".to_owned(),
                rooted: Some(false),
                app_version: "1.4.0".to_owned(),
            },
        }
    }

    #[test]
    fn the_request_is_only_the_challenge_and_the_inputs() {
        let r = enrollment_request("ch".to_owned(), input(), 7);
        assert_eq!(r.validate(), Ok(()));
        let json = serde_json::to_value(&r).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["attestations", "challenge", "device_key", "hardware", "key_algorithm", "posture", "signature"]
        );
        let posture: Vec<&str> = json["posture"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(posture, ["app_version", "collected_at", "os_name", "os_version", "rooted", "screen_lock"]);
        assert_eq!(json["attestations"][0]["kind"], "android_key_attestation");
    }
}
