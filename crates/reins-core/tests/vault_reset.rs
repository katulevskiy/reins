//! `reset_account`: an account whose keys this phone cannot open (no other phone, no recovery code) is reset after a
//! fresh sign-in to the same account, and goes on as a new account with new keys.
mod common;

use std::collections::HashMap;
use std::sync::Arc;

use data_encoding::BASE64URL_NOPAD;
use reins_core::sso::AccountSecret;
use reins_core::{AccountKeys, CoreConfig, CoreError, GoogleTokenProvider, Notifier, ReinsCore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const USER_ID: &str = "0b5c-user";
const EMAIL: &str = "me@example.com";

fn token(sub: &str, email: &str) -> String {
    let claims = json!({"sub": sub, "email": email, "exp": 4_000_000_000_i64}).to_string();
    format!("h.{}.s", BASE64URL_NOPAD.encode(claims.as_bytes()))
}

/// Keys made with a secret this phone never had (the lost recovery code).
fn lost_keys() -> String {
    let secret = AccountSecret::generate().unwrap();
    reins_core::sso::new_keys(&secret).unwrap().key
}

/// The code `other` signs in to another account; any other code to this one, whose keys this phone cannot open.
async fn serve(server: &MockServer) {
    let wrapped = lost_keys();
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(move |request: &Request| {
            let form = url::form_urlencoded::parse(&request.body).into_owned().collect::<HashMap<_, _>>();
            let (sub, email) = if form["code"] == "other" {
                ("someone", "else@example.com")
            } else {
                (USER_ID, EMAIL)
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": token(sub, email), "refresh_token": "REFRESH", "expires_in": 7200, "Key": wrapped}))
        })
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/reins/api/account-state"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revision": 0, "ciphertext": null})))
        .mount(server)
        .await;
}

fn core(dir: &std::path::Path) -> Arc<ReinsCore> {
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(common::FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(common::RecordingNotifier::default());
    ReinsCore::with_config(dir.to_str().unwrap(), &common::FakeKeys, google, notifier, CoreConfig::default()).unwrap()
}

/// Signs in through the browser with `code` and hands the callback to `finish`.
async fn sign_in(app: &ReinsCore, server: &MockServer, code: &str) -> (String, String, String) {
    let start = app.sso_begin(server.uri()).await.unwrap();
    (format!("com.reins2fa.app://sso-callback?code={code}&state={}", start.state), start.state, start.verifier)
}

async fn locked(server: &MockServer, dir: &std::path::Path) -> Arc<ReinsCore> {
    let app = core(dir);
    let (callback, state, verifier) = sign_in(&app, server, "first").await;
    let outcome = app.sso_finish(server.uri(), callback, state, verifier).await.unwrap();
    assert_eq!(outcome.keys, AccountKeys::Locked);
    app
}

#[tokio::test]
async fn a_fresh_sign_in_resets_the_vault_and_makes_new_keys() {
    let server = MockServer::start().await;
    serve(&server).await;
    Mock::given(method("POST"))
        .and(path("/reins/api/account/reset"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/set-password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let app = locked(&server, dir.path()).await;

    let (callback, state, verifier) = sign_in(&app, &server, "again").await;
    let outcome = app.reset_account(server.uri(), callback, state, verifier).await.unwrap();
    assert_eq!(outcome.keys, AccountKeys::Created);
    assert_eq!(outcome.session.email, EMAIL);
    let code = app.account_recovery_code().await.expect("the new keys' recovery code");
    assert!(AccountSecret::from_recovery_code(&code).is_some());
    assert!(app.session().await.is_some());
}

#[tokio::test]
async fn a_sign_in_to_another_account_resets_nothing() {
    let server = MockServer::start().await;
    serve(&server).await;
    Mock::given(method("POST"))
        .and(path("/reins/api/account/reset"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let app = locked(&server, dir.path()).await;

    let (callback, state, verifier) = sign_in(&app, &server, "other").await;
    let err = app.reset_account(server.uri(), callback, state, verifier).await.unwrap_err();
    assert!(matches!(&err, CoreError::Invalid { reason } if reason.contains("else@example.com")), "{err:?}");
    assert_eq!(app.session().await.unwrap().email, EMAIL, "still signed in, still locked");
}

#[tokio::test]
async fn a_refused_reset_keeps_the_account_signed_in_and_locked() {
    let server = MockServer::start().await;
    serve(&server).await;
    Mock::given(method("POST"))
        .and(path("/reins/api/account/reset"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"error": "reauth_required", "message": "Sign in again"})),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let app = locked(&server, dir.path()).await;

    let (callback, state, verifier) = sign_in(&app, &server, "again").await;
    let err = app.reset_account(server.uri(), callback, state, verifier).await.unwrap_err();
    assert!(matches!(&err, CoreError::Invalid { reason } if reason.contains("ran out")), "{err:?}");
    assert_eq!(app.session().await.unwrap().email, EMAIL, "signed in with the new sign-in's tokens");
    assert_eq!(
        app.account_recovery_code().await.unwrap_err(),
        CoreError::invalid("This account has no recovery code: it was made with a master password."),
        "no new keys were made"
    );
}
