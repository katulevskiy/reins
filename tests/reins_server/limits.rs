//! Quotas and rate limits of a public deployment, against the real server with small settings: per-connection
//! requests, per-account relayed calls, waiting calls, phone long-polls, pairings per email, client registrations per
//! IP, outbound requests per account, and the file quotas (size, files held, daily transfer).

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::{
    harness::{Options, Phone, Server, Tokens, between, client},
    mock::{MockServer, Reply},
};

fn with_env(env: &[(&'static str, &str)]) -> Options {
    Options {
        env: env.iter().map(|(k, v)| (*k, (*v).to_owned())).collect(),
        ..Options::default()
    }
}

async fn connected(options: Options, email: &str) -> (Server, Phone, Tokens) {
    let server = Server::start_with(options).await;
    let phone = server.phone(email).await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, email, Some("Claude")).await;
    (server, phone, tokens)
}

/// A JSON-RPC POST to /mcp: status, `Retry-After` and body.
async fn mcp(server: &Server, token: &str, body: &Value) -> (StatusCode, Option<String>, Value) {
    let response = client().post(server.url("/mcp")).bearer_auth(token).json(body).send().await.expect("mcp request");
    let status = response.status();
    let retry = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_owned);
    (status, retry, response.json().await.unwrap_or(Value::Null))
}

fn tools_list(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {}})
}

fn search(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "gmail_search", "arguments": {"query": "from:bank"}}})
}

/// The text of a tool result that must be an error.
fn tool_error(body: &Value) -> String {
    assert_eq!(body["result"]["isError"], true, "{body}");
    body["result"]["content"][0]["text"].as_str().expect("text").to_owned()
}

fn assert_retry_hint(text: &str) {
    let secs: u64 = between(text, "try again in ", " s").parse().unwrap_or_else(|_| panic!("no retry time: {text}"));
    assert!((1..=86_400).contains(&secs), "{text}");
}

#[tokio::test]
async fn mcp_requests_are_limited_per_connection_and_relayed_calls_per_account() {
    let options = with_env(&[("REINS_CONNECTION_REQUESTS_PER_MINUTE", "4"), ("REINS_ACCOUNT_CALLS_PER_MINUTE", "2")]);
    let (server, phone, a) = connected(options, "quota@example.com").await;
    let b = server.connect_ai(&phone, "quota@example.com", Some("Other")).await;

    // Two relayed calls use up the account's calls (nobody answers: they come back as offline).
    let (one, two) = (search(1), search(2));
    let (first, second) = tokio::join!(mcp(&server, &a.access, &one), mcp(&server, &a.access, &two));
    for (status, _, body) in [first, second] {
        assert_eq!(status, StatusCode::OK);
        assert!(tool_error(&body).contains("offline"), "{body}");
    }
    // The account's phone gets no third call, whichever connection asks; the model reads why.
    let (status, retry, body) = mcp(&server, &b.access, &search(3)).await;
    assert_eq!((status, retry), (StatusCode::OK, None));
    let text = tool_error(&body);
    assert!(text.starts_with("Rate limited: too many calls to this account's phone; try again in "), "{text}");
    assert_retry_hint(&text);
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"].as_array().map(Vec::len), Some(2), "the limited call never reached the phone");

    // Connection A has made 2 of its 4 requests: two more pass, then it is limited, B is not.
    for id in [4, 5] {
        assert_eq!(mcp(&server, &a.access, &tools_list(id)).await.0, StatusCode::OK);
    }
    let (status, retry, body) = mcp(&server, &a.access, &tools_list(6)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!((body["id"].as_u64(), body["error"]["code"].as_i64()), (Some(6), Some(-32029)), "{body}");
    assert_retry_hint(body["error"]["message"].as_str().unwrap());
    assert!(retry.unwrap().parse::<u64>().unwrap() >= 1);
    let (status, retry, body) = mcp(&server, &a.access, &search(7)).await;
    assert_eq!(status, StatusCode::OK, "a tool call gets a tool error");
    assert!(retry.is_some());
    assert!(tool_error(&body).starts_with("Rate limited: too many requests on this connection"), "{body}");
    // Notifications cost nothing.
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(mcp(&server, &a.access, &note).await.0, StatusCode::ACCEPTED);
    assert_eq!(mcp(&server, &b.access, &tools_list(8)).await.0, StatusCode::OK, "per connection");
}

#[tokio::test]
async fn calls_waiting_for_the_phone_are_bounded_per_connection() {
    let options = with_env(&[("REINS_CONNECTION_MAX_WAITING", "1")]);
    let (server, _phone, a) = connected(options, "wait@example.com").await;
    let (one, two) = (search(1), search(2));
    let waiting = mcp(&server, &a.access, &one);
    let second = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        mcp(&server, &a.access, &two).await
    };
    let ((_, _, first), (_, _, second)) = tokio::join!(waiting, second);
    assert!(tool_error(&first).contains("offline"), "{first}");
    let text = tool_error(&second);
    assert!(text.starts_with("Rate limited: too many calls of this connection are waiting for the phone"), "{text}");
    assert_retry_hint(&text);
    // Once the first call is done, there is room again.
    assert!(tool_error(&mcp(&server, &a.access, &search(3)).await.2).contains("offline"));
}

#[tokio::test]
async fn an_account_queues_a_bounded_number_of_unanswered_calls() {
    let options = with_env(&[("REINS_ACCOUNT_MAX_QUEUED", "1")]);
    let (server, phone, a) = connected(options, "queue@example.com").await;
    assert!(tool_error(&mcp(&server, &a.access, &search(1)).await.2).contains("offline"));
    let text = tool_error(&mcp(&server, &a.access, &search(2)).await.2);
    assert!(text.starts_with("Rate limited: too many requests are waiting for the user's phone"), "{text}");
    // Once the phone answers, the account may queue again.
    let (_, pending) = phone.get("/pending?wait=0").await;
    let id = pending["requests"][0]["id"].as_str().unwrap().to_owned();
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    assert_eq!(phone.post(&format!("/requests/{id}/response"), &denied).await.0, StatusCode::NO_CONTENT);
    assert!(tool_error(&mcp(&server, &a.access, &search(3)).await.2).contains("offline"));
}

#[tokio::test]
async fn a_device_holds_a_bounded_number_of_long_polls() {
    let server = Server::start_with(with_env(&[("REINS_DEVICE_MAX_POLLS", "1")])).await;
    let phone = server.phone("polls@example.com").await;
    phone.register_device().await;
    let open = phone.get("/pending?wait=3");
    let another = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        phone.get("/pending?wait=0").await
    };
    let ((status, _), (limited, body)) = tokio::join!(open, another);
    assert_eq!(status, StatusCode::OK);
    assert_eq!((limited, body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{body}");
    assert_eq!(phone.get("/pending?wait=0").await.0, StatusCode::OK, "the place is free again");
}

/// Opens the authorize page and submits `email`: the status and the page (or redirect target).
async fn submit_email(server: &Server, client_id: &str, email: &str) -> (StatusCode, String) {
    let page = client().get(server.authorize_url(client_id)).send().await.unwrap().text().await.unwrap();
    let session = between(&page, "name=\"session\" value=\"", "\"").to_owned();
    let r = client()
        .post(server.url("/reins/oauth/authorize"))
        .form(&[("session", session.as_str()), ("email", email)])
        .send()
        .await
        .unwrap();
    (r.status(), r.text().await.unwrap_or_default())
}

#[tokio::test]
async fn connection_requests_are_limited_per_email_whether_it_has_an_account_or_not() {
    let options = with_env(&[("REINS_PAIRING_RATELIMIT_SECONDS", "60"), ("REINS_PAIRING_RATELIMIT_MAX_BURST", "2")]);
    let server = Server::start_with(options).await;
    let phone = server.phone("pair@example.com").await;
    phone.register_device().await;
    let client_id = server.register_client().await;
    for email in ["pair@example.com", "nobody@example.com"] {
        for _ in 0..2 {
            assert_eq!(submit_email(&server, &client_id, email).await.0, StatusCode::SEE_OTHER, "{email}");
        }
        let (status, page) = submit_email(&server, &client_id, email).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{email}");
        assert!(page.contains("Too many connection requests for this account"), "{page}");
        assert_retry_hint(&page);
    }
    // Only the two allowed pairings reached the phone (each would be a push).
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["pairings"].as_array().map(Vec::len), Some(2), "{pending}");
    assert_eq!(submit_email(&server, &client_id, "else@example.com").await.0, StatusCode::SEE_OTHER, "per email");
}

#[tokio::test]
async fn dynamic_client_registration_is_limited_per_address() {
    let options = with_env(&[("REINS_REGISTER_RATELIMIT_SECONDS", "600"), ("REINS_REGISTER_RATELIMIT_MAX_BURST", "2")]);
    let server = Server::start_with(options).await;
    server.register_client().await;
    server.register_client().await;
    let body = json!({"client_name": "Spam", "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"]});
    let r = client().post(server.url("/reins/oauth/register")).json(&body).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let answer: Value = r.json().await.unwrap();
    assert_eq!(answer["error"], "slow_down");
    let description = answer["error_description"].as_str().unwrap();
    assert!(description.starts_with("Rate limited: too many client registrations from this address"), "{answer}");
    assert_retry_hint(description);
}

#[tokio::test]
async fn outbound_requests_are_limited_per_account() {
    let mut options = with_env(&[("REINS_OUTBOUND_REQUESTS_PER_MINUTE", "1")]);
    options.allow_loopback = true;
    let (_server, phone, tokens) = connected(options, "out@example.com").await;
    let host = MockServer::start(|_| Reply::new(200, "text/plain", "data")).await;
    let fetch = json!({"v": 1, "connection_id": tokens.connection_id, "url": host.url("/a.txt"), "headers": [],
        "name": "a.txt", "max_bytes": 100, "ttl_secs": 60});
    assert_eq!(phone.post("/blobs/fetch", &fetch).await.0, StatusCode::OK);
    let (status, body) = phone.post("/blobs/fetch", &fetch).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{body}");
    assert_retry_hint(body["message"].as_str().unwrap());
    assert_eq!(host.requests().len(), 1, "the limited request never left");
}

async fn put(url: &str, size: usize) -> (StatusCode, Value) {
    let r = client().put(url).body(vec![b'x'; size]).send().await.expect("upload");
    let status = r.status();
    (status, r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn file_quotas_cap_sizes_count_files_and_bound_the_daily_transfer() {
    let options = with_env(&[
        ("REINS_BLOB_MAX_BYTES", "100"),
        ("REINS_BLOB_ACCOUNT_FILES", "2"),
        ("REINS_BLOB_ACCOUNT_DAILY_BYTES", "250"),
    ]);
    let (server, phone, tokens) = connected(options, "files@example.com").await;
    let purpose = json!({"kind": "upload", "reason": "share"});
    let slot = |max_bytes: u64| {
        json!({"v": 1, "connection_id": tokens.connection_id, "name": "f.txt", "max_bytes": max_bytes,
            "purpose": purpose, "ttl_secs": 600})
    };
    // A link asked for 1000 bytes takes 100, the server's per-file size; a failed upload costs nothing.
    let (status, first) = phone.post("/blobs", &slot(1000)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let upload_url = first["upload_url"].as_str().unwrap();
    let (status, body) = put(upload_url, 101).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(body["message"].as_str().unwrap().contains("100 bytes"), "{body}");
    assert_eq!(put(upload_url, 100).await.0, StatusCode::CREATED);
    let id = first["id"].as_str().unwrap();
    phone.post(&format!("/blobs/{id}/decision"), &json!({"v": 1, "approved": true})).await;

    // Downloads count: 100 uploaded + 100 downloaded, and another 100 would pass the 250 of the day.
    let download = first["download_url"].as_str().unwrap();
    assert_eq!(client().get(download).send().await.unwrap().status(), StatusCode::OK);
    let r = client().get(download).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "rate_limited");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("daily limit of 250 bytes"), "{message}");
    assert_retry_hint(message);

    // What the day leaves (50 bytes) caps the next link; the third file is one too many.
    let (_, second) = phone.post("/blobs", &slot(100)).await;
    let (status, body) = phone.post("/blobs", &slot(10)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body["message"].as_str().unwrap().contains("At most 2 files"), "{body}");
    let second_url = second["upload_url"].as_str().unwrap();
    let (status, body) = put(second_url, 60).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(body["message"].as_str().unwrap().contains("50 bytes"), "{body}");
    assert_eq!(put(second_url, 50).await.0, StatusCode::CREATED);

    // Nothing is left today: the phone's own reads and outputs are refused too, with a time to retry.
    let (status, body) = phone.get(&format!("/blobs/{id}/content")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    phone.delete(&format!("/blobs/{id}")).await;
    let path = format!("/blobs/output?connection_id={}&name=o.txt", tokens.connection_id);
    let r = phone.raw(Method::PUT, &path).body("x").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = r.json().await.unwrap();
    assert_retry_hint(body["message"].as_str().unwrap());
    drop(server);
}

#[tokio::test]
async fn bad_limit_settings_stop_the_server() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("reins-bad-limits-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vaultwarden"))
        .current_dir(&dir)
        .env_clear()
        .env("DATA_FOLDER", &dir)
        .env("DOMAIN", "http://127.0.0.1:9")
        .env("REINS_ENABLED", "true")
        .env("REINS_BLOB_MAX_BYTES", "2000000000")
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!output.status.success());
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(text.contains("REINS_BLOB_MAX_BYTES"), "{text}");
}
