use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::json;

use crate::harness::{PASSWORD_HASH, Server, client};

#[tokio::test]
async fn unauthenticated_calls_get_json_401() {
    let server = Server::start().await;
    let r = client().get(server.url("/rewarden/api/pending")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn only_the_registered_device_may_use_the_api() {
    let server = Server::start().await;
    let a = server.phone("alice@example.com").await;
    let (status, body) = a.get("/pending").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));

    let (status, body) = a.put("/device", &json!({"fcm_token": "tok-a"})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})));
    let (status, body) = a.put("/device", &json!({"fcm_token": "tok-a2"})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": false})), "same device re-registers");
    let (status, body) = a.get("/pending?wait=0").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"requests": [], "pairings": []})));

    // Another device takes the role with a proof (see `takeover.rs` for the rest of the rule).
    let b = server.second_device("alice@example.com").await;
    assert_eq!(b.get("/connections").await.0, StatusCode::FORBIDDEN);
    let (status, body) = b.put("/device", &json!({"master_password_hash": PASSWORD_HASH})).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"replaced_previous": true})));
    assert_eq!(b.get("/connections").await.0, StatusCode::OK);
    let (status, body) = a.get("/pending").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_approval_device")));
}

#[tokio::test]
async fn pending_long_polls_for_the_requested_time() {
    let server = Server::start().await;
    let phone = server.phone("bob@example.com").await;
    phone.register_device().await;
    let start = Instant::now();
    let (status, body) = phone.get("/pending?wait=1").await;
    let elapsed = start.elapsed();
    assert_eq!((status, body), (StatusCode::OK, json!({"requests": [], "pairings": []})));
    assert!(elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(5), "{elapsed:?}");
    let start = Instant::now();
    assert_eq!(phone.get("/pending?wait=abc").await.0, StatusCode::OK);
    assert!(start.elapsed() < Duration::from_millis(900), "invalid wait means no wait");
}

#[tokio::test]
async fn unknown_items_and_malformed_answers() {
    let server = Server::start().await;
    let phone = server.phone("carol@example.com").await;
    phone.register_device().await;
    let (status, body) = phone.get("/requests/nope").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    assert_eq!(phone.post("/requests/nope/response", &denied).await.0, StatusCode::NOT_FOUND);
    let (status, body) = phone.post("/requests/nope/response", &json!({"v": 2, "outcome": "denied"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_version")));
    let (status, body) = phone.post("/requests/nope/response", &json!({"v": 1, "outcome": "maybe"})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));
    assert_eq!(phone.get("/pairings/nope").await.0, StatusCode::NOT_FOUND);
    let approve = json!({"v": 1, "approved": true, "chosen_code": 12, "label": null});
    assert_eq!(phone.post("/pairings/nope/response", &approve).await.0, StatusCode::NOT_FOUND);
    let (status, body) = phone.get("/no-such-endpoint").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
}

#[tokio::test]
async fn connections_start_empty_and_fcm_tokens_are_validated() {
    let server = Server::start().await;
    let phone = server.phone("dave@example.com").await;
    let (status, body) = phone.put("/device", &json!({"fcm_token": "x".repeat(5000)})).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));
    phone.register_device().await;
    let (status, body) = phone.get("/connections").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"connections": []})));
    let (status, body) = phone.delete("/connections/nope").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
}
