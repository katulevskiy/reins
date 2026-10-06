use crate::harness::{Server, client};
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn encrypted_state_is_private_to_its_authenticated_owner_and_only_approval_devices_write() {
    let server = Server::start().await;
    assert_eq!(
        client().get(server.url("/reins/api/account-state")).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let a = server.phone("alice@example.com").await;
    let b = server.phone("bob@example.com").await;
    let update = json!({"revision":0,"ciphertext":"c2VhbGVkLWVuY3J5cHRlZC1zdGF0ZQ"});
    assert_eq!(a.put("/account-state", &update).await.0, StatusCode::FORBIDDEN);
    a.register_device().await;
    assert_eq!(a.put("/account-state", &update).await.0, StatusCode::OK);
    let (status, state) = a.get("/account-state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["revision"], 1);
    assert_eq!(state["ciphertext"], update["ciphertext"]);
    assert_eq!(b.get("/account-state").await.1, json!({"revision":0,"ciphertext":null}));
    assert_eq!(a.put("/account-state", &update).await.0, StatusCode::CONFLICT);
    let second = server.second_device("alice@example.com").await;
    assert_eq!(
        second.get("/account-state").await.1,
        state,
        "the owner's other phone may download its encrypted state before key transfer"
    );
    assert_eq!(
        second.put("/account-state", &json!({"revision":1,"ciphertext":"c2VhbGVk"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        a.put("/account-state", &json!({"revision":1,"ciphertext":"plaintext!"})).await.0,
        StatusCode::BAD_REQUEST
    );
}
