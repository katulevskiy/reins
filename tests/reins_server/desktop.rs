//! The desktop app's API (`/reins/desktop/calls`) and its pairing key, against the real server.

use std::time::{Duration, Instant};

use reins_proto::desktop::{GIT_FETCH_TOOL, encode_key, key_fingerprint};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Phone, Server, Tokens, client};

/// Status, `WWW-Authenticate` and JSON body of one desktop API response.
type Answer = (StatusCode, Option<String>, Value);

async fn read(response: reqwest::Response) -> Answer {
    let status = response.status();
    let challenge = response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let text = response.text().await.unwrap_or_default();
    (status, challenge, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn post_call(server: &Server, token: Option<&str>, body: &Value) -> Answer {
    let mut request = client().post(server.url("/reins/desktop/calls")).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    read(request.send().await.expect("desktop call")).await
}

async fn get_call(server: &Server, token: Option<&str>, id: &str) -> Answer {
    let mut request = client().get(server.url(&format!("/reins/desktop/calls/{id}")));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    read(request.send().await.expect("desktop call lookup")).await
}

fn app_key() -> String {
    encode_key(&[42u8; 32])
}

fn fetch(repo: &str) -> Value {
    json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": repo, "client_key": app_key(), "nonce": "nonce-1"},
        "account": null})
}

fn sealed_answer() -> Value {
    json!({"v": 1, "outcome": "result", "result": {"kind": "connector",
        "data": {"items": [{"id": "octo/app", "sealed": "c2VhbGVk", "expires_at": 1_900_000_000}]}}})
}

/// Waits for the next relay request on the phone and answers it.
async fn answer_next(phone: &Phone, response: Value) -> Value {
    let (status, pending) = phone.get("/pending?wait=10").await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    let request = pending["requests"][0].clone();
    assert!(request.is_object(), "no request arrived: {pending}");
    let id = request["id"].as_str().expect("request id").to_owned();
    let (status, body) = phone.post(&format!("/requests/{id}/response"), &response).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    request
}

/// A server, a phone and a desktop app paired with its key.
async fn paired() -> (Server, Phone, Tokens) {
    let server = Server::start().await;
    let phone = server.phone("dev@example.com").await;
    phone.register_device().await;
    let key = app_key();
    let (tokens, _, _) =
        server.connect_with(&phone, "dev@example.com", Some("Laptop"), &[("reins_client_key", key.as_str())]).await;
    (server, phone, tokens)
}

#[tokio::test]
async fn the_app_key_reaches_the_phone_and_its_fingerprint_both_pages() {
    let server = Server::start().await;
    let phone = server.phone("kai@example.com").await;
    phone.register_device().await;
    let key = app_key();
    let shown = format!("Desktop app key: <strong class=\"key\">{}</strong>", key_fingerprint(&key).unwrap());

    let client_id = server.register_client().await;
    let authorize = server.authorize_url_with(&client_id, &server.url("/mcp"), &[("reins_client_key", key.as_str())]);
    let page = client().get(&authorize).send().await.unwrap().text().await.unwrap();
    assert!(page.contains(&shown), "{page}");

    let (_, pairing, wait_page) =
        server.connect_with(&phone, "kai@example.com", None, &[("reins_client_key", key.as_str())]).await;
    assert_eq!(pairing["client_key"], key);
    assert!(wait_page.contains(&shown), "{wait_page}");

    // An AI client has no key: nothing is shown and the pairing carries none.
    let (_, pairing, wait_page) = server.connect_with(&phone, "kai@example.com", None, &[]).await;
    assert!(pairing.get("client_key").is_none(), "{pairing}");
    assert!(!wait_page.contains("Desktop app key"));
}

#[tokio::test]
async fn an_invalid_app_key_is_refused_before_any_pairing() {
    let server = Server::start().await;
    let phone = server.phone("lea@example.com").await;
    phone.register_device().await;
    let client_id = server.register_client().await;
    for bad in ["", "not-a-key", &encode_key(&[1u8; 32])[..20]] {
        let url = server.authorize_url_with(&client_id, &server.url("/mcp"), &[("reins_client_key", bad)]);
        let r = client().get(&url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad:?}");
        assert!(r.text().await.unwrap().contains("The desktop app key is not valid."), "{bad:?}");
    }
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["pairings"], json!([]));
}

#[tokio::test]
async fn a_desktop_call_reaches_the_phone_and_the_sealed_answer_comes_back() {
    let (server, phone, tokens) = paired().await;
    let call = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app", "client_key": app_key(), "nonce": "n-7"},
        "account": " Octo-Cat "});
    let app = post_call(&server, Some(&tokens.access), &call);
    let (answer, request) = tokio::join!(app, answer_next(&phone, sealed_answer()));
    let (status, _, body) = answer;
    assert_eq!(status, StatusCode::OK, "{body}");
    let request_id = request["id"].as_str().unwrap();
    assert_eq!(body["request_id"], request_id);
    assert_eq!(body["status"], "answered");
    assert_eq!(body["outcome"], json!({"outcome": "result", "result": sealed_answer()["result"]}));
    // What the phone saw: the normalized connector call, attributed to the desktop app's connection.
    assert_eq!(request["connection_id"], tokens.connection_id);
    assert_eq!(request["connection_label"], "Laptop");
    assert_eq!(request["account"], "octo-cat");
    assert_eq!(
        request["call"],
        json!({"tool": "connector", "service": "github", "op": "git_fetch",
            "args": {"repo": "octo/app", "client_key": app_key(), "nonce": "n-7"}})
    );
    // A denial is an answer too.
    let again = fetch("octo/app");
    let app = post_call(&server, Some(&tokens.access), &again);
    let denied = json!({"v": 1, "outcome": "denied", "reason": "Not now"});
    let ((status, _, body), _) = tokio::join!(app, answer_next(&phone, denied));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"], json!({"outcome": "denied", "reason": "Not now"}));
}

#[tokio::test]
async fn an_absent_phone_is_offline_then_pending_and_a_late_answer_is_fetched_by_id() {
    let (server, phone, tokens) = paired().await;
    let start = Instant::now();
    let (status, _, offline) = post_call(&server, Some(&tokens.access), &fetch("octo/app")).await;
    let elapsed = start.elapsed();
    assert_eq!(status, StatusCode::OK, "{offline}");
    assert_eq!(offline["status"], "offline");
    assert!(elapsed >= Duration::from_millis(1800) && elapsed < Duration::from_millis(3900), "{elapsed:?}");
    let request_id = offline["request_id"].as_str().expect("request_id").to_owned();
    assert_eq!(offline, json!({"request_id": request_id, "status": "offline"}));

    // The phone opens the request but does not decide: pending after the full relay wait.
    let (status, _) = phone.get(&format!("/requests/{request_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let start = Instant::now();
    let (status, _, pending) = get_call(&server, Some(&tokens.access), &request_id).await;
    assert!(start.elapsed() >= Duration::from_millis(3800));
    assert_eq!((status, pending), (StatusCode::OK, json!({"request_id": request_id, "status": "pending"})));

    // The user approves later; the app asks again and gets it, as often as it asks until it expires.
    let (status, _) = phone.post(&format!("/requests/{request_id}/response"), &sealed_answer()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for _ in 0..2 {
        let (status, _, late) = get_call(&server, Some(&tokens.access), &request_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(late["status"], "answered");
        assert_eq!(late["outcome"]["result"]["data"]["items"][0]["sealed"], "c2VhbGVk");
    }

    // Another connection of the same user, and unknown ids, get nothing.
    let other = server.connect_ai(&phone, "dev@example.com", Some("Claude")).await;
    let (status, _, body) = get_call(&server, Some(&other.access), &request_id).await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, json!({"error": "not_found"})));
    let (status, _, _) = get_call(&server, Some(&tokens.access), "no-such-request").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get_call(&server, Some(&tokens.access), &"x".repeat(65)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_desktop_tools_with_valid_arguments_are_relayed() {
    let (server, phone, tokens) = paired().await;
    let token = Some(tokens.access.as_str());
    let ai_tool = json!({"tool": "github_list_repos", "arguments": {}});
    let (status, _, body) = post_call(&server, token, &ai_tool).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_tool")), "{body}");
    assert!(body["message"].is_string());

    let missing_key = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "octo/app", "nonce": "n"}});
    let (status, _, body) = post_call(&server, token, &missing_key).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_arguments")), "{body}");
    assert!(body["message"].as_str().unwrap().contains("client_key"), "{body}");

    let extra = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "o/r", "client_key": app_key(), "nonce": "n",
        "token": "ghp_x"}});
    assert_eq!(post_call(&server, token, &extra).await.2["error"], "invalid_arguments");

    let r = client()
        .post(server.url("/reins/desktop/calls"))
        .bearer_auth(&tokens.access)
        .body("not json")
        .send()
        .await
        .unwrap();
    let (status, _, body) = read(r).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")));

    let huge = json!({"tool": GIT_FETCH_TOOL, "arguments": {"repo": "x".repeat(600 * 1024)}});
    let (status, _, body) = post_call(&server, token, &huge).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large")), "{body}");

    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"], json!([]), "refused calls must not be relayed");
}

#[tokio::test]
async fn mcp_never_lists_or_runs_desktop_tools() {
    let (server, phone, tokens) = paired().await;
    let rpc = |body: Value| client().post(server.url("/mcp")).bearer_auth(&tokens.access).json(&body).send();
    let list: Value =
        rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await.unwrap().json().await.unwrap();
    let names: Vec<&str> =
        list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"github_list_repos"));
    assert!(!names.iter().any(|n| n.starts_with("github_git_")), "{names:?}");

    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": GIT_FETCH_TOOL, "arguments": fetch("octo/app")["arguments"]}});
    let r = rpc(call).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32602, "{body}");
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"], json!([]));
}

#[tokio::test]
async fn bad_bearers_get_the_mcp_challenge_and_browsers_are_refused() {
    let (server, phone, tokens) = paired().await;
    for (status, challenge, body) in
        [post_call(&server, None, &fetch("o/r")).await, get_call(&server, None, "some-id").await]
    {
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let challenge = challenge.expect("WWW-Authenticate");
        assert!(challenge.starts_with("Bearer resource_metadata=\""), "{challenge}");
        assert!(challenge.contains("/.well-known/oauth-protected-resource/mcp"), "{challenge}");
        assert_eq!(body["error"], "unauthorized");
    }
    // Garbage, and a regular Vaultwarden login token, are invalid tokens.
    for token in ["garbage", phone.token.as_str()] {
        let (status, challenge, body) = post_call(&server, Some(token), &fetch("o/r")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(challenge.unwrap().contains("invalid_token"));
        assert_eq!(body["error"], "invalid_token");
    }
    // A revoked connection's token dies at once.
    let (status, _) = phone.delete(&format!("/connections/{}", tokens.connection_id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(get_call(&server, Some(&tokens.access), "some-id").await.0, StatusCode::UNAUTHORIZED);

    let other = server.connect_ai(&phone, "dev@example.com", None).await;
    let r = client()
        .post(server.url("/reins/desktop/calls"))
        .bearer_auth(&other.access)
        .header("Origin", "https://evil.example")
        .json(&fetch("o/r"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}
