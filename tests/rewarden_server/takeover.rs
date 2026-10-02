//! Who may become an account's approval device (`PUT /rewarden/api/device`): the first device freely, the approval
//! device again freely, any other device only with the master password hash or after the approval device approved its
//! "add another phone" request; a device id without the device key it registered with is another device.

use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Options, PASSWORD_HASH, Phone, Server, new_device_key};

const REFUSED: &str =
    "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code.";

fn error(body: &Value) -> (&str, &str) {
    (body["error"].as_str().unwrap_or_default(), body["message"].as_str().unwrap_or_default())
}

async fn assert_approves(phone: &Phone, yes: bool) {
    let (status, body) = phone.get("/pending?wait=0").await;
    if yes {
        assert_eq!(status, StatusCode::OK, "{body}");
    } else {
        assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));
    }
}

#[tokio::test]
async fn another_device_needs_a_proof_and_the_master_password_hash_is_one() {
    let server = Server::start().await;
    let a = server.phone("erin@example.com").await;
    let (status, body) = a.put("/device", &json!({})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})), "the first device: no proof");
    let (status, body) = a.put("/device", &json!({"fcm_token": "tok-a"})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})), "the same device: no proof");

    let b = server.second_device("erin@example.com").await;
    let (status, body) = b.put("/device", &json!({"fcm_token": "tok-b"})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error(&body), ("proof_required", REFUSED));
    assert_approves(&a, true).await;
    assert_approves(&b, false).await;

    let (status, body) = b.put("/device", &json!({"fcm_token": "tok-b", "master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": true})));
    assert_approves(&b, true).await;
    assert_approves(&a, false).await;
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})), "now b re-registers freely");
    let (status, _) = a.put("/device", &json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a, replaced, needs a proof now");
}

#[tokio::test]
async fn wrong_proofs_are_refused_then_rate_limited_per_account() {
    let options = Options {
        env: vec![("REWARDEN_DEVICE_PROOF_MAX_FAILURES", "2".to_owned())],
        ..Options::default()
    };
    let server = Server::start_with(options).await;
    let a = server.phone("gina@example.com").await;
    a.register_device().await;
    let b = server.second_device("gina@example.com").await;
    for _ in 0..2 {
        let (status, body) = b.put("/device", &json!({"master_password_hash": "not-the-hash"})).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error(&body), ("wrong_proof", REFUSED));
    }
    // The limit is reached: even the right hash waits now, and nothing changed.
    let (status, body) = b.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{body}");
    assert!(body["message"].as_str().unwrap().contains("try again in"), "{body}");
    assert_approves(&a, true).await;
    assert_approves(&b, false).await;
    // Asking without a proof costs nothing and says what to do.
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, error(&body).0), (StatusCode::FORBIDDEN, "proof_required"));
    // The approval device itself is not limited, and other accounts are not either.
    assert_eq!(a.put("/device", &json!({})).await.0, StatusCode::OK);
    let h = server.phone("hal@example.com").await;
    h.register_device().await;
    let h2 = server.second_device("hal@example.com").await;
    assert_eq!(h2.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await.0, StatusCode::OK);
}

/// The new phone asks to join; the approval device approves (the sealed secret is opaque to the server).
async fn approved_join(approver: &Phone, asking: &Phone) {
    let new = json!({"v": 1, "device_name": "Pixel", "public_key": new_device_key()});
    let (status, created) = asking.post("/joins", &new).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let (status, body) =
        approver.post(&format!("/joins/{id}/response"), &json!({"v": 1, "approve": true, "sealed": "SEALED"})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

#[tokio::test]
async fn a_join_approval_lets_the_asking_device_take_over_once() {
    let server = Server::start().await;
    let a = server.phone("ivy@example.com").await;
    a.register_device().await;
    let b = server.second_device("ivy@example.com").await;

    // A denied request allows nothing.
    let (_, created) = b.post("/joins", &json!({"v": 1, "device_name": "Pixel", "public_key": new_device_key()})).await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(
        a.post(&format!("/joins/{id}/response"), &json!({"v": 1, "approve": false})).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(error(&b.put("/device", &json!({})).await.1).0, "proof_required");

    approved_join(&a, &b).await;
    // The same device id with another key, or without one, is not the device that asked.
    for key in [Some(new_device_key()), None] {
        let spoof = server.device_with_id("ivy@example.com", &b.device_id, key).await;
        assert_eq!(error(&spoof.put("/device", &json!({})).await.1).0, "proof_required");
    }
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": true})));
    assert_approves(&b, true).await;

    // a takes the role back with the password; the approval was spent, so b cannot just take it again.
    assert_eq!(a.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await.0, StatusCode::OK);
    let (status, body) = b.put("/device", &json!({})).await;
    assert_eq!((status, error(&body).0), (StatusCode::FORBIDDEN, "proof_required"));
    assert_approves(&a, true).await;
}

#[tokio::test]
async fn a_device_id_without_its_device_key_is_another_device() {
    let server = Server::start().await;
    let a = server.phone("jo@example.com").await;
    a.register_device().await;
    // Whoever signs in to the account sees the device ids (`GET /api/devices`) and may sign in naming one.
    for key in [Some(new_device_key()), None] {
        let spoof = server.device_with_id("jo@example.com", &a.device_id, key).await;
        assert_approves(&spoof, false).await;
        assert_eq!(spoof.get("/connections").await.0, StatusCode::FORBIDDEN);
        let (status, body) = spoof.put("/device", &json!({"fcm_token": "evil"})).await;
        assert_eq!((status, error(&body).0), (StatusCode::FORBIDDEN, "proof_required"));
        let (status, body) =
            spoof.post("/joins", &json!({"v": 1, "device_name": "x", "public_key": new_device_key()})).await;
        assert_eq!(status, StatusCode::OK, "it may ask the approval device like any other device: {body}");
    }
    assert_approves(&a, true).await;
}

#[tokio::test]
async fn a_row_from_before_device_keys_takes_its_device_key_once() {
    let server = Server::start().await;
    let a = server.phone("kim@example.com").await;
    // An app from before device keys registers without one.
    let old = server.device_with_id("kim@example.com", &a.device_id, None).await;
    old.register_device().await;
    assert_approves(&old, true).await;
    // The updated app (same device id, now with its key) registers: the key is kept from then on.
    let (status, body) = a.put("/device", &json!({})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})));
    assert_approves(&a, true).await;
    assert_approves(&old, false).await;
    assert_eq!(error(&old.put("/device", &json!({})).await.1).0, "proof_required");
}
