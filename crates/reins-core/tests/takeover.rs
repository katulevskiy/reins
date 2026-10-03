//! `register_device` when another device approves for the account: the core proves it may take over with the master
//! password hash of what it holds (the password it signed in or unlocked with, or the account secret), once, and
//! otherwise reports `OtherApprovalDevice`. Every phone-API call carries the phone's device key.

mod common;

use std::sync::Arc;

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use data_encoding::BASE64URL_NOPAD;
use reins_core::crypto::{self, Kdf};
use reins_core::sso::AccountSecret;
use reins_core::{CoreConfig, CoreError, GoogleTokenProvider, Notifier, ReinsCore};
use reins_proto::device::{DEVICE_KEY_HEADER, device_key_hash};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "me@example.com";
const USER_ID: &str = "0b5c-user";
const PASSWORD: &str = "hunter2-hunter2";
const KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 5000,
};

struct Env {
    server: MockServer,
    core: Arc<ReinsCore>,
    _dir: tempfile::TempDir,
}

/// An access token shaped like the server's (the core reads, not verifies, its claims).
fn access_token() -> String {
    let claims = json!({"sub": USER_ID, "email": EMAIL, "exp": 4_000_000_000_i64}).to_string();
    format!("h.{}.s", BASE64URL_NOPAD.encode(claims.as_bytes()))
}

async fn mount_identity(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": access_token(), "refresh_token": "REFRESH", "expires_in": 7200})))
        .mount(server)
        .await;
}

async fn signed_in() -> Env {
    let server = MockServer::start().await;
    mount_identity(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let core = ReinsCore::with_config(dir.path().to_str().unwrap(), &FakeKeys, google, notifier, CoreConfig::default())
        .unwrap();
    core.login(server.uri(), EMAIL.to_owned(), PASSWORD.to_owned(), None).await.unwrap();
    Env {
        server,
        core,
        _dir: dir,
    }
}

/// `PUT /device` answers 200 to a body with `proof`, else `refusal` (403 with that code).
async fn serve_device(server: &MockServer, proof: Option<&str>, refusal: &str) {
    if let Some(proof) = proof {
        Mock::given(method("PUT"))
            .and(path("/reins/api/device"))
            .and(body_partial_json(json!({"master_password_hash": proof})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": true})))
            .with_priority(1)
            .mount(server)
            .await;
    }
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .and(body_string_contains("master_password_hash"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": "wrong_proof", "message": "m"})))
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": refusal, "message": "m"})))
        .with_priority(3)
        .mount(server)
        .await;
}

/// The bodies and device key headers of the `PUT /device` calls so far.
async fn registrations(server: &MockServer) -> Vec<(Value, Option<String>)> {
    let requests = server.received_requests().await.unwrap();
    requests
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path() == "/reins/api/device")
        .map(|r| {
            let key = r.headers.get(DEVICE_KEY_HEADER).map(|v| v.to_str().unwrap().to_owned());
            (serde_json::from_slice(&r.body).unwrap(), key)
        })
        .collect()
}

fn password_hash() -> String {
    let master = crypto::master_key(PASSWORD, EMAIL, KDF).unwrap();
    crypto::master_password_hash(&master, PASSWORD).to_string()
}

#[tokio::test]
async fn the_device_key_goes_with_every_phone_api_call_and_nowhere_else() {
    let env = signed_in().await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": false})))
        .mount(&env.server)
        .await;
    env.core.register_device(None).await.unwrap();
    env.core.register_device(Some("fcm".to_owned())).await.unwrap();
    let calls = registrations(&env.server).await;
    assert_eq!(calls.len(), 2, "no proof needed, no retry");
    assert_eq!(calls[0].0, json!({"fcm_token": null}), "no proof is sent unasked");
    let key = calls[0].1.clone().expect("the device key header");
    assert!(device_key_hash(&key).is_some(), "32 bytes, base64url");
    assert_eq!(calls[1].1.as_deref(), Some(key.as_str()), "the same key every time");
    for r in env.server.received_requests().await.unwrap() {
        if r.url.path().starts_with("/identity/") {
            assert!(r.headers.get(DEVICE_KEY_HEADER).is_none(), "{} carries the device key", r.url.path());
        }
    }
}

#[tokio::test]
async fn a_refusal_is_answered_with_the_password_hash_once_then_reported() {
    let env = signed_in().await;
    let hash = password_hash();
    serve_device(&env.server, Some(&hash), "proof_required").await;
    env.core.register_device(None).await.expect("the password it signed in with proves it");
    let calls = registrations(&env.server).await;
    assert_eq!(calls.len(), 2);
    assert!(calls[0].0.get("master_password_hash").is_none(), "asked without a proof first");
    assert_eq!(calls[1].0["master_password_hash"], json!(hash));
    assert_eq!(calls[0].1, calls[1].1, "the same device key");

    // The proof is forgotten once the phone is the approval device: replaced later, it has none (no account secret
    // either) and the app is told to ask the other phone or for the recovery code.
    assert_eq!(env.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);
    assert_eq!(registrations(&env.server).await.len(), 3, "nothing to prove with, no second try");
    assert_eq!(
        CoreError::OtherApprovalDevice.to_string(),
        "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code."
    );
}

#[tokio::test]
async fn a_wrong_proof_or_a_limit_is_reported() {
    let env = signed_in().await;
    serve_device(&env.server, None, "proof_required").await;
    assert_eq!(env.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);
    assert_eq!(registrations(&env.server).await.len(), 2);

    let env = signed_in().await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .and(body_string_contains("master_password_hash"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({"error": "rate_limited", "message": "m"})))
        .with_priority(1)
        .mount(&env.server)
        .await;
    serve_device(&env.server, None, "proof_required").await;
    assert!(matches!(
        env.core.register_device(None).await,
        Err(CoreError::Server {
            status: 429,
            ..
        })
    ));
}

#[tokio::test]
async fn the_recovery_code_or_the_master_password_lets_the_phone_prove_itself() {
    // An account made with an account secret (another phone has it): the recovery code opens it here.
    let env = signed_in().await;
    let secret = AccountSecret::generate().unwrap();
    let keys = reins_core::sso::new_keys(&secret).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/accounts/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": USER_ID, "key": keys.key})))
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": false})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    // Spends the password's proof: from here on only the secret can prove anything.
    env.core.register_device(None).await.unwrap();
    serve_device(&env.server, Some(keys.master_password_hash.as_str()), "proof_required").await;
    assert_eq!(env.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);
    env.core.unlock_account(secret.recovery_code().to_string()).await.unwrap();
    env.core.register_device(None).await.expect("the account secret proves it");
    let last = registrations(&env.server).await.pop().unwrap();
    assert_eq!(last.0["master_password_hash"], json!(keys.master_password_hash.as_str()));

    // An account made with a master password: unlocking with it is a proof too.
    let env = signed_in().await;
    let keys = crypto::new_account_keys(PASSWORD, EMAIL, KDF).unwrap();
    Mock::given(method("GET"))
        .and(path("/api/accounts/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": USER_ID, "key": keys.key})))
        .mount(&env.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": false})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&env.server)
        .await;
    env.core.register_device(None).await.unwrap();
    serve_device(&env.server, Some(keys.master_password_hash.as_str()), "proof_required").await;
    assert_eq!(env.core.register_device(None).await.unwrap_err(), CoreError::OtherApprovalDevice);
    env.core.unlock_account(PASSWORD.to_owned()).await.unwrap();
    env.core.register_device(None).await.expect("the master password proves it");
}
