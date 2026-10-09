//! The vault's passkeys (`/reins/api/vault-passkeys`): any signed-in device reads the sealed copies (useless without
//! the passkey); adding or removing one takes the master password hash of the account secret, counted with the
//! takeover proofs.

use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Options, PASSWORD_HASH, Server};

fn ids(body: &Value) -> Vec<&str> {
    body["passkeys"].as_array().unwrap().iter().map(|p| p["credential_id"].as_str().unwrap()).collect()
}

fn new(id: &str, hash: &str) -> Value {
    json!({"credential_id": id, "wrapped": format!("sealed-{id}"), "name": "Pixel 9", "master_password_hash": hash})
}

#[tokio::test]
async fn a_passkey_is_added_with_the_proof_and_read_by_any_device_of_the_account() {
    let server = Server::start().await;
    let phone = server.phone("ivy@example.com").await;
    let (status, body) = phone.get("/vault-passkeys").await;
    assert_eq!((status, ids(&body).len()), (StatusCode::OK, 0));

    let (status, body) = phone.post("/vault-passkeys", &new("cred-1", "not-the-hash")).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("wrong_proof")), "{body}");
    let (status, body) = phone.post("/vault-passkeys", &new("cred-1", PASSWORD_HASH)).await;
    assert_eq!((status, ids(&body)), (StatusCode::OK, vec!["cred-1"]), "{body}");
    assert_eq!(body["passkeys"][0]["wrapped"], "sealed-cred-1");
    assert_eq!(body["passkeys"][0]["name"], "Pixel 9");

    // A reinstalled app: another device of the account, keys not open yet, reads it to unlock.
    let fresh = server.second_device("ivy@example.com").await;
    let (status, body) = fresh.get("/vault-passkeys").await;
    assert_eq!((status, ids(&body)), (StatusCode::OK, vec!["cred-1"]));
    // Another account sees nothing of it.
    let other = server.phone("jon@example.com").await;
    assert_eq!(ids(&other.get("/vault-passkeys").await.1).len(), 0);

    let (status, _) =
        fresh.post("/vault-passkeys/remove", &json!({"credential_id": "cred-1", "master_password_hash": ""})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "removing takes the proof too");
    let (status, body) = fresh
        .post("/vault-passkeys/remove", &json!({"credential_id": "cred-1", "master_password_hash": PASSWORD_HASH}))
        .await;
    assert_eq!((status, ids(&body).len()), (StatusCode::OK, 0));
}

#[tokio::test]
async fn malformed_ids_and_too_many_passkeys_are_refused() {
    let server = Server::start().await;
    let phone = server.phone("kai@example.com").await;
    let (status, _) = phone.post("/vault-passkeys", &new("not base64url!", PASSWORD_HASH)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for i in 0..reins_proto::vault_passkey::MAX_PASSKEYS {
        assert_eq!(phone.post("/vault-passkeys", &new(&format!("cred-{i}"), PASSWORD_HASH)).await.0, StatusCode::OK);
    }
    let (status, body) = phone.post("/vault-passkeys", &new("one-more", PASSWORD_HASH)).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("too_many_passkeys")), "{body}");
    let (status, _) = phone.post("/vault-passkeys", &new("cred-0", PASSWORD_HASH)).await;
    assert_eq!(status, StatusCode::OK, "replacing the copy of a passkey it keeps is not one more");
}

#[tokio::test]
async fn wrong_proofs_count_with_the_takeover_proofs() {
    let options = Options {
        env: vec![("REINS_DEVICE_PROOF_MAX_FAILURES", "2".to_owned())],
        ..Options::default()
    };
    let server = Server::start_with(options).await;
    let phone = server.phone("lea@example.com").await;
    for _ in 0..2 {
        assert_eq!(phone.post("/vault-passkeys", &new("cred", "nope")).await.0, StatusCode::FORBIDDEN);
    }
    let (status, body) = phone.post("/vault-passkeys", &new("cred", PASSWORD_HASH)).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{body}");
    assert_eq!(
        phone.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await.0,
        StatusCode::OK,
        "the first device needs no proof, so the limit does not stop it"
    );
}
