//! FCM HTTP v1 sender (spec §4.6). Pushes carry only `{t, id}`; no request content
//! ever transits Google.

use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reins_proto::pairing::PushMessage;
use serde_json::Value;

use crate::{CONFIG, http_client::make_http_request};

pub const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
pub const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const ASSERTION_SECS: i64 = 3600;
/// FCM drops an undelivered wake-up after this long (matches the relay item TTL).
const MESSAGE_TTL: &str = "600s";
const ERROR_EXCERPT_CHARS: usize = 200;

/// The fields of a Google service-account key file that the sender needs.
#[derive(Clone, Debug, Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    DEFAULT_TOKEN_URI.to_owned()
}

impl ServiceAccount {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let account: Self = serde_json::from_str(json).map_err(|e| format!("invalid service-account JSON: {e}"))?;
        if account.project_id.is_empty() || account.client_email.is_empty() || account.private_key.is_empty() {
            return Err("service-account JSON lacks project_id, client_email or private_key".to_owned());
        }
        if !account.token_uri.starts_with("https://") {
            return Err("service-account token_uri must be https".to_owned());
        }
        Ok(account)
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
        Self::from_json(&json)
    }
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// Signs the RFC 7523 JWT-bearer assertion exchanged at `token_uri` for an access token.
pub fn sign_assertion(account: &ServiceAccount, now: i64) -> Result<String, String> {
    // Google ships PKCS#8 keys; OpenSSL accepts both PKCS#8 and PKCS#1 and re-encodes as PKCS#1,
    // the form `EncodingKey::from_rsa_pem` is known to accept (as in `auth::initialize_keys`).
    let rsa = openssl::rsa::Rsa::private_key_from_pem(account.private_key.as_bytes())
        .map_err(|e| format!("invalid service-account private key: {e}"))?;
    let pkcs1 = rsa.private_key_to_pem().map_err(|e| format!("cannot encode private key: {e}"))?;
    let key = EncodingKey::from_rsa_pem(&pkcs1).map_err(|e| format!("invalid service-account private key: {e}"))?;
    let claims = AssertionClaims {
        iss: &account.client_email,
        scope: FCM_SCOPE,
        aud: &account.token_uri,
        iat: now,
        exp: now + ASSERTION_SECS,
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| format!("cannot sign assertion: {e}"))
}

/// FCM v1 `messages:send` body: data-only, high priority (spec §4.6).
pub fn message_body(fcm_token: &str, push: &PushMessage) -> Value {
    json!({
        "message": {
            "token": fcm_token,
            "data": push,
            "android": {"priority": "HIGH", "ttl": MESSAGE_TTL}
        }
    })
}

/// The outcome of a push that reached the push service (FCM or APNs).
#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// The device token is no longer valid (app uninstalled or token rotated).
    Unregistered,
}

pub fn classify_response(status: u16, body: &str) -> Result<SendOutcome, String> {
    if (200..300).contains(&status) {
        return Ok(SendOutcome::Sent);
    }
    if status == 404 || body.contains("UNREGISTERED") {
        return Ok(SendOutcome::Unregistered);
    }
    let excerpt: String = body.chars().take(ERROR_EXCERPT_CHARS).collect();
    Err(format!("FCM answered {status}: {excerpt}"))
}

/// When a token valid for `expires_in` seconds should be replaced (spec: half its lifetime).
fn refresh_at(now: Instant, expires_in: i64) -> Instant {
    now + Duration::from_secs(u64::try_from(expires_in / 2).unwrap_or(0))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

struct CachedToken {
    token: String,
    refresh_at: Instant,
}

pub struct FcmSender {
    account: ServiceAccount,
    cached: tokio::sync::Mutex<Option<CachedToken>>,
}

impl FcmSender {
    pub fn new(account: ServiceAccount) -> Self {
        Self {
            account,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        let mut cached = self.cached.lock().await;
        if let Some(c) = cached.as_ref()
            && Instant::now() < c.refresh_at
        {
            return Ok(c.token.clone());
        }
        let assertion = sign_assertion(&self.account, chrono::Utc::now().timestamp())?;
        let response = make_http_request(reqwest::Method::POST, &self.account.token_uri)
            .map_err(|e| e.to_string())?
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())])
            .send()
            .await
            .map_err(|e| format!("token request failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let excerpt: String = body.chars().take(ERROR_EXCERPT_CHARS).collect();
            return Err(format!("token endpoint answered {status}: {excerpt}"));
        }
        let token: TokenResponse = response.json().await.map_err(|e| format!("invalid token response: {e}"))?;
        *cached = Some(CachedToken {
            token: token.access_token.clone(),
            refresh_at: refresh_at(Instant::now(), token.expires_in),
        });
        Ok(token.access_token)
    }

    pub async fn send(&self, fcm_token: &str, push: &PushMessage) -> Result<SendOutcome, String> {
        let access_token = self.access_token().await?;
        let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.account.project_id);
        let response = make_http_request(reqwest::Method::POST, &url)
            .map_err(|e| e.to_string())?
            .bearer_auth(access_token)
            .json(&message_body(fcm_token, push))
            .send()
            .await
            .map_err(|e| format!("FCM request failed: {e}"))?;
        let status = response.status().as_u16();
        if status == 401 {
            // Revoked or rotated key: fetch a new access token next time.
            *self.cached.lock().await = None;
        }
        let body = response.text().await.unwrap_or_default();
        classify_response(status, &body)
    }
}

static SENDER: LazyLock<Option<FcmSender>> = LazyLock::new(|| {
    let path = CONFIG.reins_fcm_service_account();
    if path.is_empty() {
        return None;
    }
    match ServiceAccount::from_file(&path) {
        Ok(account) => Some(FcmSender::new(account)),
        Err(e) => {
            error!("Reins push disabled: {e}");
            None
        }
    }
});

/// The FCM sender, when `REINS_FCM_SERVICE_ACCOUNT` is configured.
pub fn sender() -> Option<&'static FcmSender> {
    SENDER.as_ref()
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation};
    use openssl::{pkey::PKey, rsa::Rsa};
    use reins_proto::pairing::PushKind;
    use serde_json::Value;

    use super::*;

    /// A throwaway service account with a fresh PKCS#8 key, as Google issues them.
    fn account() -> (ServiceAccount, String) {
        let pkey = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let private_pem = String::from_utf8(pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let public_pem = String::from_utf8(pkey.public_key_to_pem().unwrap()).unwrap();
        let json = json!({
            "type": "service_account",
            "project_id": "reins-test",
            "private_key_id": "abc",
            "private_key": private_pem,
            "client_email": "fcm@reins-test.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token"
        });
        (ServiceAccount::from_json(&json.to_string()).unwrap(), public_pem)
    }

    #[test]
    fn parses_and_validates_service_accounts() {
        let (sa, _) = account();
        assert_eq!(sa.project_id, "reins-test");
        assert_eq!(sa.token_uri, "https://oauth2.googleapis.com/token");
        assert!(ServiceAccount::from_json("{}").is_err());
        assert!(ServiceAccount::from_json("not json").is_err());
        let no_uri = json!({"project_id": "p", "client_email": "e@x", "private_key": "k"});
        assert_eq!(ServiceAccount::from_json(&no_uri.to_string()).unwrap().token_uri, DEFAULT_TOKEN_URI);
        let http_uri =
            json!({"project_id": "p", "client_email": "e@x", "private_key": "k", "token_uri": "http://evil"});
        assert!(ServiceAccount::from_json(&http_uri.to_string()).is_err());
        let empty_project = json!({"project_id": "", "client_email": "e@x", "private_key": "k"});
        assert!(ServiceAccount::from_json(&empty_project.to_string()).is_err());
        assert!(ServiceAccount::from_file("/nonexistent/reins-sa.json").unwrap_err().contains("/nonexistent"));
    }

    #[test]
    fn assertion_is_a_google_jwt_bearer_token() {
        let (sa, public_pem) = account();
        let jwt = sign_assertion(&sa, 1_700_000_000).unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&["https://oauth2.googleapis.com/token"]);
        validation.validate_exp = false;
        let data = jsonwebtoken::decode::<Value>(
            &jwt,
            &DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap();
        assert_eq!(data.claims["iss"], "fcm@reins-test.iam.gserviceaccount.com");
        assert_eq!(data.claims["scope"], FCM_SCOPE);
        assert_eq!(data.claims["iat"], 1_700_000_000);
        assert_eq!(data.claims["exp"], 1_700_003_600);
        let bad = ServiceAccount {
            private_key: "garbage".to_owned(),
            ..sa
        };
        assert!(sign_assertion(&bad, 0).is_err());
    }

    #[test]
    fn message_is_data_only_high_priority_and_carries_only_the_id() {
        let push = PushMessage {
            t: PushKind::Pair,
            id: "p1".to_owned(),
        };
        let body = message_body("device-token", &push);
        assert_eq!(
            body,
            json!({"message": {
                "token": "device-token",
                "data": {"t": "pair", "id": "p1"},
                "android": {"priority": "HIGH", "ttl": "600s"}
            }})
        );
        assert!(body["message"].get("notification").is_none());
    }

    #[test]
    fn classifies_fcm_responses() {
        assert!(matches!(classify_response(200, "{}"), Ok(SendOutcome::Sent)));
        assert!(matches!(classify_response(404, "{}"), Ok(SendOutcome::Unregistered)));
        let unregistered =
            r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{"errorCode":"UNREGISTERED"}]}}"#;
        assert!(matches!(classify_response(400, unregistered), Ok(SendOutcome::Unregistered)));
        let err = classify_response(503, "overloaded").unwrap_err();
        assert!(err.contains("503") && err.contains("overloaded"), "{err}");
        assert!(classify_response(500, &"x".repeat(10_000)).unwrap_err().len() < 400);
    }

    #[test]
    fn token_refresh_happens_at_half_lifetime() {
        let now = Instant::now();
        assert_eq!(refresh_at(now, 3600), now + Duration::from_mins(30));
        assert_eq!(refresh_at(now, -5), now);
    }

    /// Opt-in check against the real FCM service (`REINS_LIVE_FCM=/path/to/service-account.json`): a bogus device
    /// token must be rejected as an invalid *argument*, which only happens after Google accepted our credentials.
    #[tokio::test]
    async fn live_fcm_accepts_our_credentials_and_rejects_a_bogus_token() {
        let Ok(path) = std::env::var("REINS_LIVE_FCM") else {
            return;
        };
        // main() installs the process-wide TLS provider; a test process has to do it itself.
        rustls::crypto::ring::default_provider().install_default().ok();
        let sender = FcmSender::new(ServiceAccount::from_file(&path).expect("service account"));
        let push = PushMessage {
            t: PushKind::Req,
            id: "live-check".to_owned(),
        };
        let error = sender.send("not-a-real-device-token", &push).await.expect_err("a bogus token must not be sent");
        assert!(error.contains("400") && error.contains("INVALID_ARGUMENT"), "unexpected answer: {error}");
    }
}
