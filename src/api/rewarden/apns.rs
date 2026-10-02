//! APNs sender for the iOS app: token-based provider authentication (ES256 JWT) and HTTP/2 to Apple. Like the FCM
//! path, a push carries only `{t, id}` next to a fixed alert text; no request content ever transits Apple. The app's
//! notification service extension fetches the request over HTTPS and rewrites the alert on the phone.

use std::{
    sync::{LazyLock, Mutex, PoisonError},
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use openssl::{nid::Nid, pkey::PKey};
use rewarden_proto::pairing::{PushKind, PushMessage};
use serde_json::Value;

use super::fcm::SendOutcome;
use crate::{CONFIG, http_client::make_http_request};

/// Stored push-token prefix of a production APNs device token (App Store and TestFlight builds).
pub const PRODUCTION_PREFIX: &str = "apns:";
/// Stored push-token prefix of a development APNs device token (builds signed for development).
pub const SANDBOX_PREFIX: &str = "apns-sandbox:";
/// An APNs device token is 32 bytes, sent by the app as lowercase or uppercase hex.
pub const DEVICE_TOKEN_HEX_LEN: usize = 64;
/// The iOS app's bundle id, the default `apns-topic`.
pub const DEFAULT_TOPIC: &str = "dev.rewarden.ios";
const PRODUCTION_ORIGIN: &str = "https://api.push.apple.com";
const SANDBOX_ORIGIN: &str = "https://api.sandbox.push.apple.com";
/// Apple refuses provider tokens older than an hour and throttles new ones signed more often than every 20 minutes.
const PROVIDER_TOKEN_REFRESH: Duration = Duration::from_mins(50);
/// Apple drops an undelivered notification after this long (matches the relay item TTL).
const EXPIRATION_SECS: i64 = 600;
const MAX_COLLAPSE_ID_BYTES: usize = 64;
const ERROR_EXCERPT_CHARS: usize = 200;
const ALERT_TITLE: &str = "Reins";
const ALERT_BODY: &str = "Something is waiting for you";
const ALERT_SOUND: &str = "reins_request.caf";
const THREAD_ID: &str = "requests";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Production,
    Sandbox,
}

impl Environment {
    fn origin(self) -> &'static str {
        match self {
            Self::Production => PRODUCTION_ORIGIN,
            Self::Sandbox => SANDBOX_ORIGIN,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Production => PRODUCTION_PREFIX,
            Self::Sandbox => SANDBOX_PREFIX,
        }
    }
}

/// The APNs environment and device token behind a stored push token; `None` for an FCM token.
pub fn split_token(token: &str) -> Option<(Environment, &str)> {
    if let Some(device) = token.strip_prefix(SANDBOX_PREFIX) {
        return Some((Environment::Sandbox, device));
    }
    token.strip_prefix(PRODUCTION_PREFIX).map(|device| (Environment::Production, device))
}

/// `None` when `token` is not an APNs token; otherwise the token with its hex lowercased, or why it is malformed.
pub fn normalize_token(token: &str) -> Option<Result<String, String>> {
    let (environment, device) = split_token(token)?;
    if device.len() != DEVICE_TOKEN_HEX_LEN || !device.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(Err(format!(
            "an APNs token is `{PRODUCTION_PREFIX}` or `{SANDBOX_PREFIX}` followed by {DEVICE_TOKEN_HEX_LEN} hex digits"
        )));
    }
    Some(Ok(format!("{}{}", environment.prefix(), device.to_ascii_lowercase())))
}

#[derive(Serialize)]
struct ProviderClaims<'a> {
    iss: &'a str,
    iat: i64,
}

/// The `.p8` signing key of an Apple Developer account, with the ids that go into the provider token.
pub struct ProviderKey {
    key: EncodingKey,
    key_id: String,
    team_id: String,
}

impl std::fmt::Debug for ProviderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderKey")
            .field("key_id", &self.key_id)
            .field("team_id", &self.team_id)
            .finish_non_exhaustive()
    }
}

impl ProviderKey {
    /// Accepts the PKCS#8 PEM Apple issues (and SEC1 `EC PRIVATE KEY` PEM); the key must be on P-256.
    pub fn from_pem(pem: &[u8], key_id: &str, team_id: &str) -> Result<Self, String> {
        for (name, value) in [("key id", key_id), ("team id", team_id)] {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(format!(
                    "the APNs {name} must be letters and digits (as shown in the Apple Developer account)"
                ));
            }
        }
        let pkey = PKey::private_key_from_pem(pem).map_err(|e| format!("invalid APNs key: {e}"))?;
        let ec = pkey.ec_key().map_err(|_| "the APNs key is not an elliptic-curve key".to_owned())?;
        if ec.group().curve_name() != Some(Nid::X9_62_PRIME256V1) {
            return Err("the APNs key is not a P-256 key".to_owned());
        }
        // The rust_crypto backend of jsonwebtoken signs with PKCS#8 only.
        let pkcs8 = pkey.private_key_to_pem_pkcs8().map_err(|e| format!("cannot encode the APNs key: {e}"))?;
        let key = EncodingKey::from_ec_pem(&pkcs8).map_err(|e| format!("invalid APNs key: {e}"))?;
        let this = Self {
            key,
            key_id: key_id.to_owned(),
            team_id: team_id.to_owned(),
        };
        this.sign(0)?;
        Ok(this)
    }

    /// The provider token: `{"alg":"ES256","kid":<key id>}` `{"iss":<team id>,"iat":<now>}`.
    pub fn sign(&self, iat: i64) -> Result<String, String> {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = None;
        header.kid = Some(self.key_id.clone());
        let claims = ProviderClaims {
            iss: &self.team_id,
            iat,
        };
        jsonwebtoken::encode(&header, &claims, &self.key)
            .map_err(|e| format!("cannot sign the APNs provider token: {e}"))
    }
}

/// What APNs is asked to deliver: the JSON payload and the headers that depend on the kind of push.
#[derive(Debug, PartialEq)]
pub struct Notification {
    pub push_type: &'static str,
    pub priority: u8,
    pub payload: Value,
}

/// The payload the iOS notification service extension relies on: a fixed, content-free alert for requests, pairings
/// and files, and a silent background push for `replaced`. `t` and `id` are the FCM data fields, unchanged.
pub fn notification(push: &PushMessage) -> Notification {
    let category = match push.t {
        PushKind::Req => "request",
        PushKind::Pair => "pairing",
        PushKind::Blob => "blob",
        PushKind::Replaced => {
            let mut payload = json!(push);
            payload["aps"] = json!({"content-available": 1});
            return Notification {
                push_type: "background",
                priority: 5,
                payload,
            };
        }
    };
    let mut payload = json!(push);
    payload["aps"] = json!({
        "alert": {"title": ALERT_TITLE, "body": ALERT_BODY},
        "sound": ALERT_SOUND,
        "mutable-content": 1,
        // Also wakes the app in the background (when iOS allows): it runs Autopilot's model, which the notification
        // extension cannot.
        "content-available": 1,
        "category": category,
        "thread-id": THREAD_ID,
        "interruption-level": "time-sensitive"
    });
    Notification {
        push_type: "alert",
        priority: 10,
        payload,
    }
}

/// How an APNs answer is handled.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Sent,
    /// The device token is gone (app uninstalled, token rotated, or not a token of this app): forget it.
    Unregistered,
    /// Apple refused the provider token: sign a new one before the next push.
    ProviderTokenRefused(String),
    Failed(String),
}

#[derive(Deserialize)]
struct ErrorBody {
    reason: String,
}

pub fn classify_response(status: u16, body: &str) -> Answer {
    if status == 200 {
        return Answer::Sent;
    }
    let reason = serde_json::from_str::<ErrorBody>(body).map(|b| b.reason).unwrap_or_default();
    match (status, reason.as_str()) {
        (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic" | "Unregistered") => Answer::Unregistered,
        (403, "ExpiredProviderToken" | "InvalidProviderToken") => {
            Answer::ProviderTokenRefused(format!("APNs answered 403 {reason}"))
        }
        _ => {
            let detail = if reason.is_empty() {
                body
            } else {
                &reason
            };
            let excerpt: String = detail.chars().take(ERROR_EXCERPT_CHARS).collect();
            Answer::Failed(format!("APNs answered {status}: {excerpt}"))
        }
    }
}

struct CachedProviderToken {
    token: String,
    refresh_at: Instant,
}

/// The configured APNs credentials, as strings from the environment.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub key_file: String,
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
}

impl Settings {
    fn from_config() -> Self {
        Self {
            key_file: CONFIG.rewarden_apns_key_file(),
            key_id: CONFIG.rewarden_apns_key_id(),
            team_id: CONFIG.rewarden_apns_team_id(),
            topic: CONFIG.rewarden_apns_topic(),
        }
    }

    /// `None` when APNs is not configured. The key file, key id and team id are set together or not at all.
    pub fn load(&self) -> Result<Option<ApnsSender>, String> {
        let set = [&self.key_file, &self.key_id, &self.team_id].iter().filter(|v| !v.is_empty()).count();
        if set == 0 {
            return Ok(None);
        }
        if set < 3 {
            return Err(
                "`REWARDEN_APNS_KEY_FILE`, `REWARDEN_APNS_KEY_ID` and `REWARDEN_APNS_TEAM_ID` must be set together"
                    .to_owned(),
            );
        }
        let topic = self.topic.trim();
        if topic.is_empty() || topic.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("`REWARDEN_APNS_TOPIC` must be the iOS app's bundle id".to_owned());
        }
        let pem = std::fs::read(&self.key_file)
            .map_err(|e| format!("`REWARDEN_APNS_KEY_FILE`: cannot read `{}`: {e}", self.key_file))?;
        let key = ProviderKey::from_pem(&pem, self.key_id.trim(), self.team_id.trim())
            .map_err(|e| format!("`REWARDEN_APNS_KEY_FILE`: {e}"))?;
        Ok(Some(ApnsSender::new(key, topic.to_owned())))
    }
}

pub struct ApnsSender {
    key: ProviderKey,
    topic: String,
    cached: Mutex<Option<CachedProviderToken>>,
    /// Tests only: a plain-HTTP/2 client and the origin of a local server standing in for Apple.
    test_server: Option<(reqwest::Client, String)>,
}

impl ApnsSender {
    pub fn new(key: ProviderKey, topic: String) -> Self {
        Self {
            key,
            topic,
            cached: Mutex::new(None),
            test_server: None,
        }
    }

    /// The cached provider token, or a new one when it is older than [`PROVIDER_TOKEN_REFRESH`] or was dropped.
    fn provider_token(&self, now: Instant, now_unix: i64) -> Result<String, String> {
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = cached.as_ref()
            && now < c.refresh_at
        {
            return Ok(c.token.clone());
        }
        let token = self.key.sign(now_unix)?;
        *cached = Some(CachedProviderToken {
            token: token.clone(),
            refresh_at: now + PROVIDER_TOKEN_REFRESH,
        });
        Ok(token)
    }

    fn drop_provider_token(&self) {
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Sends `push` to a stored `apns:`/`apns-sandbox:` token. Errors never contain the device token.
    pub async fn send(&self, token: &str, push: &PushMessage) -> Result<SendOutcome, String> {
        let Some((environment, device)) = split_token(token) else {
            return Err("not an APNs token".to_owned());
        };
        let now_unix = chrono::Utc::now().timestamp();
        let provider_token = self.provider_token(Instant::now(), now_unix)?;
        let notification = notification(push);
        let origin = self.test_server.as_ref().map_or(environment.origin(), |(_, origin)| origin.as_str());
        let url = format!("{origin}/3/device/{device}");
        let mut request = match &self.test_server {
            Some((client, _)) => client.post(&url),
            None => make_http_request(reqwest::Method::POST, &url).map_err(|e| e.to_string())?,
        }
        .bearer_auth(provider_token)
        .header("apns-topic", &self.topic)
        .header("apns-push-type", notification.push_type)
        .header("apns-priority", notification.priority.to_string())
        .header("apns-expiration", (now_unix + EXPIRATION_SECS).to_string());
        if !push.id.is_empty() && push.id.len() <= MAX_COLLAPSE_ID_BYTES {
            request = request.header("apns-collapse-id", &push.id);
        }
        // The URL holds the device token; keep it out of the error.
        let response = request
            .json(&notification.payload)
            .send()
            .await
            .map_err(|e| format!("APNs request failed: {}", e.without_url()))?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        match classify_response(status, &body) {
            Answer::Sent => Ok(SendOutcome::Sent),
            Answer::Unregistered => Ok(SendOutcome::Unregistered),
            Answer::ProviderTokenRefused(e) => {
                self.drop_provider_token();
                Err(e)
            }
            Answer::Failed(e) => Err(e),
        }
    }
}

static SENDER: LazyLock<Option<ApnsSender>> = LazyLock::new(|| match Settings::from_config().load() {
    Ok(sender) => sender,
    Err(e) => {
        error!("Rewarden APNs push disabled: {e}");
        None
    }
});

/// The APNs sender, when `REWARDEN_APNS_*` is configured.
pub fn sender() -> Option<&'static ApnsSender> {
    SENDER.as_ref()
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{DecodingKey, Validation};
    use openssl::{ec::EcGroup, ec::EcKey};

    use super::*;

    const DEVICE: &str = "00fc13adff785122b4ad28809a3420982341241421348097878e577c991de8f0";

    /// A fresh P-256 key as Apple issues it (PKCS#8 PEM), and its public key.
    fn key_pair() -> (String, String) {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let pkey = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let private_pem = String::from_utf8(pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let public_pem = String::from_utf8(pkey.public_key_to_pem().unwrap()).unwrap();
        (private_pem, public_pem)
    }

    fn decode(jwt: &str, public_pem: &str) -> (Header, Value) {
        let mut validation = Validation::new(Algorithm::ES256);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let data =
            jsonwebtoken::decode::<Value>(jwt, &DecodingKey::from_ec_pem(public_pem.as_bytes()).unwrap(), &validation)
                .unwrap();
        (data.header, data.claims)
    }

    fn push(t: PushKind, id: &str) -> PushMessage {
        PushMessage {
            t,
            id: id.to_owned(),
        }
    }

    #[test]
    fn provider_token_is_an_es256_jwt_with_key_and_team_ids() {
        let (private_pem, public_pem) = key_pair();
        let key = ProviderKey::from_pem(private_pem.as_bytes(), "ABC123DEFG", "DEF123GHIJ").unwrap();
        let jwt = key.sign(1_700_000_000).unwrap();
        let (header, claims) = decode(&jwt, &public_pem);
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("ABC123DEFG"));
        assert_eq!(header.typ, None);
        assert_eq!(claims, json!({"iss": "DEF123GHIJ", "iat": 1_700_000_000}));
        // Apple's header carries only `alg` and `kid`.
        let raw_header = data_encoding::BASE64URL_NOPAD.decode(jwt.split('.').next().unwrap().as_bytes()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&raw_header).unwrap(), json!({"alg": "ES256", "kid": "ABC123DEFG"}));
    }

    #[test]
    fn provider_keys_must_be_p256_with_plain_ids() {
        let (private_pem, _) = key_pair();
        // SEC1 PEM is accepted too.
        let sec1 = PKey::private_key_from_pem(private_pem.as_bytes()).unwrap().ec_key().unwrap();
        assert!(ProviderKey::from_pem(&sec1.private_key_to_pem().unwrap(), "K", "T").is_ok());
        assert!(ProviderKey::from_pem(b"garbage", "K", "T").unwrap_err().contains("invalid APNs key"));
        let p384 = EcKey::generate(&EcGroup::from_curve_name(Nid::SECP384R1).unwrap()).unwrap();
        let p384_pem = PKey::from_ec_key(p384).unwrap().private_key_to_pem_pkcs8().unwrap();
        assert!(ProviderKey::from_pem(&p384_pem, "K", "T").unwrap_err().contains("P-256"));
        let rsa = PKey::from_rsa(openssl::rsa::Rsa::generate(2048).unwrap()).unwrap();
        let rsa_pem = rsa.private_key_to_pem_pkcs8().unwrap();
        assert!(ProviderKey::from_pem(&rsa_pem, "K", "T").unwrap_err().contains("elliptic-curve"));
        assert!(ProviderKey::from_pem(private_pem.as_bytes(), "", "T").unwrap_err().contains("key id"));
        assert!(ProviderKey::from_pem(private_pem.as_bytes(), "K", "T\"x").unwrap_err().contains("team id"));
    }

    #[test]
    fn provider_token_is_cached_for_fifty_minutes_and_dropped_on_demand() {
        let (private_pem, public_pem) = key_pair();
        let sender =
            ApnsSender::new(ProviderKey::from_pem(private_pem.as_bytes(), "K", "T").unwrap(), DEFAULT_TOPIC.to_owned());
        let start = Instant::now();
        let first = sender.provider_token(start, 1000).unwrap();
        assert_eq!(sender.provider_token(start + Duration::from_mins(49), 3940).unwrap(), first);
        let refreshed = sender.provider_token(start + Duration::from_mins(50), 4000).unwrap();
        assert_eq!(decode(&refreshed, &public_pem).1["iat"], 4000);
        sender.drop_provider_token();
        let after_drop = sender.provider_token(start + Duration::from_mins(51), 4060).unwrap();
        assert_eq!(decode(&after_drop, &public_pem).1["iat"], 4060);
    }

    #[test]
    fn alert_payloads_carry_a_fixed_text_and_only_the_id() {
        for (kind, t, category) in
            [(PushKind::Req, "req", "request"), (PushKind::Pair, "pair", "pairing"), (PushKind::Blob, "blob", "blob")]
        {
            let n = notification(&push(kind, "id-1"));
            assert_eq!((n.push_type, n.priority), ("alert", 10));
            assert_eq!(
                n.payload,
                json!({
                    "aps": {
                        "alert": {"title": "Reins", "body": "Something is waiting for you"},
                        "sound": "reins_request.caf",
                        "mutable-content": 1,
                        "content-available": 1,
                        "category": category,
                        "thread-id": "requests",
                        "interruption-level": "time-sensitive"
                    },
                    "t": t,
                    "id": "id-1"
                })
            );
            // `t` is exactly the FCM data field.
            assert_eq!(n.payload["t"], json!(push(kind, "").t));
        }
    }

    #[test]
    fn replaced_is_a_silent_background_push() {
        let n = notification(&push(PushKind::Replaced, ""));
        assert_eq!((n.push_type, n.priority), ("background", 5));
        assert_eq!(n.payload, json!({"aps": {"content-available": 1}, "t": "replaced", "id": ""}));
    }

    #[test]
    fn classifies_apns_responses() {
        assert_eq!(classify_response(200, ""), Answer::Sent);
        assert_eq!(
            classify_response(410, r#"{"reason":"Unregistered","timestamp":1700000000000}"#),
            Answer::Unregistered
        );
        assert_eq!(classify_response(410, ""), Answer::Unregistered);
        for reason in ["BadDeviceToken", "DeviceTokenNotForTopic", "Unregistered"] {
            assert_eq!(
                classify_response(400, &format!(r#"{{"reason":"{reason}"}}"#)),
                Answer::Unregistered,
                "{reason}"
            );
        }
        for reason in ["ExpiredProviderToken", "InvalidProviderToken"] {
            let answer = classify_response(403, &format!(r#"{{"reason":"{reason}"}}"#));
            assert_eq!(answer, Answer::ProviderTokenRefused(format!("APNs answered 403 {reason}")));
        }
        assert_eq!(
            classify_response(400, r#"{"reason":"BadCollapseId"}"#),
            Answer::Failed("APNs answered 400: BadCollapseId".to_owned())
        );
        assert_eq!(
            classify_response(403, r#"{"reason":"BadCertificate"}"#),
            Answer::Failed("APNs answered 403: BadCertificate".to_owned())
        );
        assert_eq!(
            classify_response(429, r#"{"reason":"TooManyProviderTokenUpdates"}"#),
            Answer::Failed("APNs answered 429: TooManyProviderTokenUpdates".to_owned())
        );
        let Answer::Failed(e) = classify_response(503, &"x".repeat(10_000)) else {
            panic!("a 503 is a failure");
        };
        assert!(e.contains("503") && e.len() < 400, "{e}");
    }

    #[test]
    fn tokens_are_split_and_normalized_by_prefix() {
        assert_eq!(split_token(&format!("apns:{DEVICE}")), Some((Environment::Production, DEVICE)));
        assert_eq!(split_token(&format!("apns-sandbox:{DEVICE}")), Some((Environment::Sandbox, DEVICE)));
        assert_eq!(split_token("fMEP0vJqS0:APA91bHqX"), None);
        assert_eq!(split_token("APNS:abc"), None);
        assert_eq!(Environment::Production.origin(), "https://api.push.apple.com");
        assert_eq!(Environment::Sandbox.origin(), "https://api.sandbox.push.apple.com");

        assert_eq!(normalize_token("fMEP0vJqS0:APA91bHqX"), None);
        let upper = format!("apns-sandbox:{}", DEVICE.to_ascii_uppercase());
        assert_eq!(normalize_token(&upper), Some(Ok(format!("apns-sandbox:{DEVICE}"))));
        assert_eq!(normalize_token(&format!("apns:{DEVICE}")), Some(Ok(format!("apns:{DEVICE}"))));
        for bad in [
            "apns:".to_owned(),
            format!("apns:{}", &DEVICE[1..]),
            format!("apns:{DEVICE}0"),
            format!("apns:{}g", &DEVICE[1..]),
            format!("apns-sandbox:{} ", &DEVICE[1..]),
            format!("apns:{}", "é".repeat(32)),
        ] {
            assert!(matches!(normalize_token(&bad), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn settings_are_all_or_nothing_and_the_key_must_parse() {
        assert!(Settings::default().load().unwrap().is_none());
        let (private_pem, _) = key_pair();
        let path = std::env::temp_dir().join(format!("rewarden-apns-test-{}.p8", std::process::id()));
        std::fs::write(&path, private_pem).unwrap();
        let full = Settings {
            key_file: path.to_str().unwrap().to_owned(),
            key_id: "ABC123DEFG".to_owned(),
            team_id: "DEF123GHIJ".to_owned(),
            topic: DEFAULT_TOPIC.to_owned(),
        };
        assert_eq!(full.load().unwrap().unwrap().topic, DEFAULT_TOPIC);
        let partial = Settings {
            team_id: String::new(),
            ..full.clone()
        };
        assert!(partial.load().err().unwrap().contains("must be set together"));
        let no_topic = Settings {
            topic: " ".to_owned(),
            ..full.clone()
        };
        assert!(no_topic.load().err().unwrap().contains("REWARDEN_APNS_TOPIC"));
        let missing = Settings {
            key_file: "/nonexistent/AuthKey.p8".to_owned(),
            ..full.clone()
        };
        assert!(missing.load().err().unwrap().contains("/nonexistent/AuthKey.p8"));
        std::fs::write(&path, "not a key").unwrap();
        assert!(full.load().err().unwrap().contains("REWARDEN_APNS_KEY_FILE"));
        std::fs::remove_file(&path).ok();
    }

    /// What the mock APNs server saw of one request.
    struct Seen {
        path: String,
        headers: http::HeaderMap,
        body: Value,
    }

    /// A plain-HTTP/2 (prior knowledge) server answering each request with the next of `answers`.
    async fn mock_apns(answers: Vec<(u16, &'static str)>) -> (String, tokio::sync::mpsc::UnboundedReceiver<Seen>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let mut answers = answers.into_iter();
            // Accepting drives the connection's I/O, so each stream is handled in its own task.
            while let Some(Ok((request, mut respond))) = connection.accept().await {
                let (status, body) = answers.next().expect("no more answers");
                let tx = tx.clone();
                tokio::spawn(async move {
                    let (parts, mut stream) = request.into_parts();
                    let mut bytes = Vec::new();
                    while let Some(chunk) = stream.data().await {
                        let chunk = chunk.unwrap();
                        stream.flow_control().release_capacity(chunk.len()).unwrap();
                        bytes.extend_from_slice(&chunk);
                    }
                    tx.send(Seen {
                        path: parts.uri.path().to_owned(),
                        headers: parts.headers,
                        body: serde_json::from_slice(&bytes).unwrap(),
                    })
                    .unwrap();
                    let response = http::Response::builder().status(status).body(()).unwrap();
                    let mut send = respond.send_response(response, body.is_empty()).unwrap();
                    if !body.is_empty() {
                        send.send_data(bytes::Bytes::from_static(body.as_bytes()), true).unwrap();
                    }
                });
            }
        });
        (origin, rx)
    }

    #[tokio::test]
    async fn sends_over_http2_and_handles_apple_answers() {
        let (origin, mut seen) = mock_apns(vec![
            (200, ""),
            (200, ""),
            (410, r#"{"reason":"Unregistered","timestamp":1700000000000}"#),
            (403, r#"{"reason":"ExpiredProviderToken"}"#),
        ])
        .await;
        let (private_pem, public_pem) = key_pair();
        let key = ProviderKey::from_pem(private_pem.as_bytes(), "ABC123DEFG", "DEF123GHIJ").unwrap();
        let mut sender = ApnsSender::new(key, "dev.rewarden.ios".to_owned());
        // main() installs the process-wide TLS provider; reqwest needs one even for plain HTTP.
        rustls::crypto::ring::default_provider().install_default().ok();
        sender.test_server = Some((reqwest::Client::builder().http2_prior_knowledge().build().unwrap(), origin));
        let token = format!("apns:{DEVICE}");

        let before = chrono::Utc::now().timestamp();
        assert_eq!(sender.send(&token, &push(PushKind::Req, "r-1")).await, Ok(SendOutcome::Sent));
        let req = seen.recv().await.unwrap();
        assert_eq!(req.path, format!("/3/device/{DEVICE}"));
        let header = |name: &str| req.headers.get(name).map(|v| v.to_str().unwrap().to_owned());
        assert_eq!(header("apns-topic").as_deref(), Some("dev.rewarden.ios"));
        assert_eq!(header("apns-push-type").as_deref(), Some("alert"));
        assert_eq!(header("apns-priority").as_deref(), Some("10"));
        assert_eq!(header("apns-collapse-id").as_deref(), Some("r-1"));
        let expiration: i64 = header("apns-expiration").unwrap().parse().unwrap();
        assert!((before + 600..=chrono::Utc::now().timestamp() + 600).contains(&expiration), "{expiration}");
        let jwt = header("authorization").unwrap().strip_prefix("Bearer ").unwrap().to_owned();
        let (jwt_header, claims) = decode(&jwt, &public_pem);
        assert_eq!((jwt_header.kid.as_deref(), &claims["iss"]), (Some("ABC123DEFG"), &json!("DEF123GHIJ")));
        assert_eq!(req.body, notification(&push(PushKind::Req, "r-1")).payload);

        assert_eq!(sender.send(&token, &push(PushKind::Replaced, "")).await, Ok(SendOutcome::Sent));
        let replaced = seen.recv().await.unwrap();
        assert_eq!(replaced.headers.get("apns-push-type").unwrap(), "background");
        assert_eq!(replaced.headers.get("apns-priority").unwrap(), "5");
        assert!(replaced.headers.get("apns-collapse-id").is_none());
        assert_eq!(replaced.body, json!({"aps": {"content-available": 1}, "t": "replaced", "id": ""}));

        assert_eq!(sender.send(&token, &push(PushKind::Pair, "p-1")).await, Ok(SendOutcome::Unregistered));
        seen.recv().await.unwrap();

        assert!(sender.cached.lock().unwrap().is_some());
        let refused = sender.send(&token, &push(PushKind::Blob, "b-1")).await.unwrap_err();
        assert_eq!(refused, "APNs answered 403 ExpiredProviderToken");
        assert!(!refused.contains(DEVICE));
        assert!(sender.cached.lock().unwrap().is_none(), "a refused provider token is not reused");

        assert!(sender.send("fcm-token", &push(PushKind::Req, "r-2")).await.is_err());
    }

    /// Opt-in check against Apple's sandbox (`REWARDEN_LIVE_APNS=1`): the server's shared HTTP client reaches APNs
    /// over HTTP/2 (Apple refuses HTTP/1.1) and gets Apple's JSON verdict on a provider token from an unknown key.
    #[tokio::test]
    async fn live_apns_speaks_http2_with_the_shared_client() {
        if std::env::var("REWARDEN_LIVE_APNS").is_err() {
            return;
        }
        // main() installs the process-wide TLS provider; a test process has to do it itself.
        rustls::crypto::ring::default_provider().install_default().ok();
        let (private_pem, _) = key_pair();
        let key = ProviderKey::from_pem(private_pem.as_bytes(), "ABC123DEFG", "DEF123GHIJ").unwrap();
        let response = make_http_request(reqwest::Method::POST, &format!("{SANDBOX_ORIGIN}/3/device/{DEVICE}"))
            .unwrap()
            .bearer_auth(key.sign(chrono::Utc::now().timestamp()).unwrap())
            .header("apns-topic", DEFAULT_TOPIC)
            .header("apns-push-type", "alert")
            .json(&notification(&push(PushKind::Req, "live-check")).payload)
            .send()
            .await
            .expect("APNs is reachable");
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        let status = response.status().as_u16();
        let body = response.text().await.unwrap();
        assert_eq!(status, 403, "{body}");
        assert_eq!(
            classify_response(status, &body),
            Answer::ProviderTokenRefused("APNs answered 403 InvalidProviderToken".to_owned())
        );
    }
}
