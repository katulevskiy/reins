//! In-app account deletion (`POST /reins/api/account/delete`): the typed email, a freshly issued access token and the
//! approval device (or the master password hash) are what it takes; then the account and everything Reins kept for it
//! are gone, and a retry with the old token is just unauthorized.

use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Options, PASSWORD_HASH, Phone, Server, client};

fn error(body: &Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

async fn delete(phone: &Phone, body: &Value) -> (StatusCode, Value) {
    phone.post("/account/delete", body).await
}

/// Whether a password sign-in to `email` still works.
async fn can_sign_in(server: &Server, email: &str) -> bool {
    let form = [
        ("grant_type", "password"),
        ("username", email),
        ("password", PASSWORD_HASH),
        ("scope", "api offline_access"),
        ("client_id", "mobile"),
        ("deviceType", "0"),
        ("deviceIdentifier", "c1b6f0de-3f4e-4c49-9a8f-6d1c0e0d1a01"),
        ("deviceName", "Reins"),
    ];
    client().post(server.url("/identity/connect/token")).form(&form).send().await.unwrap().status().is_success()
}

/// `token`, signed again with the server's own key, as if it had been issued `secs` seconds ago.
fn issued_ago(server: &Server, token: &str, secs: i64) -> String {
    let payload = token.split('.').nth(1).expect("jwt payload");
    let mut claims: Value =
        serde_json::from_slice(&data_encoding::BASE64URL_NOPAD.decode(payload.as_bytes()).unwrap()).unwrap();
    claims["nbf"] = json!(claims["nbf"].as_i64().unwrap() - secs);
    let pem = std::fs::read(server.data_dir().join("rsa_key.pem")).expect("server key");
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem).unwrap();
    jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256), &claims, &key).unwrap()
}

#[tokio::test]
async fn the_approval_device_deletes_the_account_with_everything_reins_kept() {
    let server = Server::start().await;
    let r = client().post(server.url("/reins/api/account/delete")).json(&json!({"confirm_email": "x"})).send().await;
    assert_eq!(r.unwrap().status(), StatusCode::UNAUTHORIZED);

    let phone = server.phone("del@example.com").await;
    phone.register_device().await;
    let state = json!({"revision": 0, "ciphertext": "c2VhbGVkLWVuY3J5cHRlZC1zdGF0ZQ"});
    assert_eq!(phone.put("/account-state", &state).await.0, StatusCode::OK);
    let tokens = server.connect_ai(&phone, "del@example.com", Some("Work AI")).await;
    let other = server.phone("keep@example.com").await;
    other.register_device().await;

    for typed in ["", "keep@example.com", "DELETE"] {
        let (status, body) = delete(&phone, &json!({"confirm_email": typed})).await;
        assert_eq!((status, error(&body)), (StatusCode::BAD_REQUEST, "confirmation_mismatch"), "{typed:?}");
    }
    assert_eq!(phone.get("/connections").await.0, StatusCode::OK, "nothing was deleted");

    let (status, body) = delete(&phone, &json!({"confirm_email": "  Del@Example.com "})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(!can_sign_in(&server, "del@example.com").await, "the account is gone");
    // The same request again, with the same token: the session went with the account.
    assert_eq!(delete(&phone, &json!({"confirm_email": "del@example.com"})).await.0, StatusCode::UNAUTHORIZED);
    let (status, body) = server.token(&[("grant_type", "refresh_token"), ("refresh_token", &tokens.refresh)]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")), "{body}");
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}});
    let r = client().post(server.url("/mcp")).bearer_auth(&tokens.access).json(&ping).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "the AI's access ends at once");
    assert_eq!(std::fs::read_dir(server.data_dir().join("reins-account-state")).unwrap().count(), 0);
    assert!(server.log().contains("was deleted at its owner's request"), "audit log");

    // Other accounts are untouched, and the email is free for a new, empty account.
    assert!(can_sign_in(&server, "keep@example.com").await);
    assert_eq!(other.get("/pending?wait=0").await.0, StatusCode::OK);
    let again = server.phone("del@example.com").await;
    assert_eq!(again.get("/account-state").await.1, json!({"revision": 0, "ciphertext": null}));
}

#[tokio::test]
async fn another_device_needs_the_proof_and_an_account_without_approval_device_needs_none() {
    let server = Server::start().await;
    let a = server.phone("two@example.com").await;
    a.register_device().await;
    let b = server.second_device("two@example.com").await;
    let (status, body) = delete(&b, &json!({"confirm_email": "two@example.com"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "proof_required"));
    let (status, body) =
        delete(&b, &json!({"confirm_email": "two@example.com", "master_password_hash": "not-the-hash"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "wrong_proof"));
    assert_eq!(a.get("/pending?wait=0").await.0, StatusCode::OK, "still there");
    let (status, body) =
        delete(&b, &json!({"confirm_email": "two@example.com", "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(!can_sign_in(&server, "two@example.com").await);

    // Never registered an approval device: a signed-in device with the typed email is enough.
    let fresh = server.phone("fresh@example.com").await;
    assert_eq!(delete(&fresh, &json!({"confirm_email": "fresh@example.com"})).await.0, StatusCode::NO_CONTENT);
    assert!(!can_sign_in(&server, "fresh@example.com").await);
}

#[tokio::test]
async fn an_old_access_token_must_be_refreshed_first() {
    let server = Server::start().await;
    let mut phone = server.phone("old@example.com").await;
    phone.register_device().await;
    let fresh = phone.token.clone();
    phone.token = issued_ago(&server, &fresh, 3600);
    assert_eq!(phone.get("/pending?wait=0").await.0, StatusCode::OK, "the old token still works elsewhere");
    let (status, body) = delete(&phone, &json!({"confirm_email": "old@example.com"})).await;
    assert_eq!((status, error(&body)), (StatusCode::FORBIDDEN, "fresh_token_required"));
    assert!(can_sign_in(&server, "old@example.com").await);
    phone.token = fresh;
    assert_eq!(delete(&phone, &json!({"confirm_email": "old@example.com"})).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn deleting_the_account_in_the_web_vault_also_deletes_what_reins_kept() {
    let server = Server::start().await;
    let phone = server.phone("web@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "web@example.com", None).await;
    let state = json!({"revision": 0, "ciphertext": "c2VhbGVkLWVuY3J5cHRlZC1zdGF0ZQ"});
    assert_eq!(phone.put("/account-state", &state).await.0, StatusCode::OK);
    let r = client()
        .post(server.url("/api/accounts/delete"))
        .bearer_auth(&phone.token)
        .json(&json!({"masterPasswordHash": PASSWORD_HASH}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert!(!can_sign_in(&server, "web@example.com").await);
    assert_eq!(std::fs::read_dir(server.data_dir().join("reins-account-state")).unwrap().count(), 0);
    let (status, body) = server.token(&[("grant_type", "refresh_token"), ("refresh_token", &tokens.refresh)]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")), "{body}");
}

#[tokio::test]
async fn wrong_confirmations_are_rate_limited_per_account() {
    let options = Options {
        env: vec![("REINS_DEVICE_PROOF_MAX_FAILURES", "2".to_owned())],
        ..Options::default()
    };
    let server = Server::start_with(options).await;
    let phone = server.phone("typo@example.com").await;
    phone.register_device().await;
    for _ in 0..2 {
        assert_eq!(delete(&phone, &json!({"confirm_email": "typo@example.org"})).await.0, StatusCode::BAD_REQUEST);
    }
    let (status, body) = delete(&phone, &json!({"confirm_email": "typo@example.com"})).await;
    assert_eq!((status, error(&body)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"), "{body}");
    assert!(can_sign_in(&server, "typo@example.com").await);
    let other = server.phone("other@example.com").await;
    assert_eq!(delete(&other, &json!({"confirm_email": "other@example.com"})).await.0, StatusCode::NO_CONTENT);
}
