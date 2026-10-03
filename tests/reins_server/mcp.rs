use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::harness::{Phone, Server, between, client};

const MODERN: &str = "2026-07-28";

/// A JSON-RPC POST to /mcp; returns status, the WWW-Authenticate header and the JSON body.
async fn mcp(
    server: &Server,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
) -> (StatusCode, Option<String>, Value) {
    let mut request = client().post(server.url("/mcp")).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let response = request.send().await.expect("mcp request");
    let status = response.status();
    let challenge = response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let text = response.text().await.unwrap_or_default();
    (status, challenge, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    let mut request = json!({"jsonrpc": "2.0", "id": id, "method": method});
    request["params"] = params;
    request
}

fn call_tool(id: u64, name: &str, arguments: Value) -> Value {
    let mut params = json!({"name": name});
    params["arguments"] = arguments;
    rpc(id, "tools/call", params)
}

async fn tool(server: &Server, token: &str, name: &str, arguments: Value) -> Value {
    let (status, _, body) = mcp(server, Some(token), &[], &call_tool(1, name, arguments)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"].clone()
}

fn text_of(result: &Value) -> &str {
    result["content"][0]["text"].as_str().expect("text content")
}

fn search_result() -> Value {
    json!({"v": 1, "outcome": "result", "result": {"kind": "search", "messages": [{
        "id": "m1", "thread_id": "t1", "from": "alerts@bank.com", "from_name": "Bank Alerts",
        "to": ["me@example.com"], "cc": [], "subject": "Statement", "date": 1_700_000_000, "snippet": "Ready"}]}})
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

async fn connected() -> (Server, Phone, crate::harness::Tokens) {
    let server = Server::start().await;
    let phone = server.phone("mia@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "mia@example.com", Some("Claude")).await;
    (server, phone, tokens)
}

#[tokio::test]
async fn unauthenticated_and_foreign_tokens_get_a_challenge() {
    let server = Server::start().await;
    let (status, challenge, _) = mcp(&server, None, &[], &rpc(1, "tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = challenge.expect("WWW-Authenticate");
    assert!(challenge.starts_with("Bearer resource_metadata=\""), "{challenge}");
    assert!(challenge.contains("/.well-known/oauth-protected-resource/mcp"), "{challenge}");
    let (status, challenge, _) = mcp(&server, Some("garbage"), &[], &rpc(1, "tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(challenge.unwrap().contains("invalid_token"));
    // A regular Vaultwarden login token is not an MCP token (issuer/audience differ).
    let phone = server.phone("nina@example.com").await;
    let (status, _, _) = mcp(&server, Some(&phone.token), &[], &rpc(1, "tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Even initialize needs auth: Claude only starts OAuth after a 401.
    let init = rpc(1, "initialize", json!({"protocolVersion": "2025-11-25"}));
    assert_eq!(mcp(&server, None, &[], &init).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_delete_and_foreign_origins_are_refused() {
    let (server, _phone, tokens) = connected().await;
    for r in [
        client().get(server.url("/mcp")).send().await.unwrap(),
        client().delete(server.url("/mcp")).send().await.unwrap(),
    ] {
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(r.headers()["allow"], "POST");
    }
    let (status, _, _) =
        mcp(&server, Some(&tokens.access), &[("Origin", "https://evil.example")], &rpc(1, "ping", json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) =
        mcp(&server, Some(&tokens.access), &[("Origin", "https://claude.ai")], &rpc(1, "ping", json!({}))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn both_protocol_eras_are_served() {
    let (server, _phone, tokens) = connected().await;
    let token = Some(tokens.access.as_str());
    // Claude: legacy initialize, then the initialized notification (202).
    let init = rpc(
        1,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "claude-ai", "version": "1"}}),
    );
    let (status, _, body) = mcp(&server, token, &[], &init).await;
    assert_eq!((status, body["result"]["protocolVersion"].as_str()), (StatusCode::OK, Some("2025-11-25")));
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let (status, _, body) = mcp(&server, token, &[], &note).await;
    assert_eq!((status, body), (StatusCode::ACCEPTED, Value::Null));
    let (_, _, legacy) = mcp(&server, token, &[], &rpc(2, "tools/list", json!({}))).await;
    let names: Vec<&str> =
        legacy["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "gmail_read",
            "gmail_search",
            "gmail_send",
            "reins_request_access",
            "reins_list_accounts",
            "reins_get_result",
            "reins_upload"
        ]
        .into_iter()
        .chain(reins_proto::connector::specs().iter().filter(|s| !s.desktop_only).map(|s| s.tool))
        .collect::<Vec<_>>()
    );

    // ChatGPT: server/discover with the modern headers, then a modern tools/list.
    let meta = json!({"_meta": {"io.modelcontextprotocol/protocolVersion": MODERN}});
    let h = [("MCP-Protocol-Version", MODERN), ("Mcp-Method", "server/discover")];
    let (status, _, body) = mcp(&server, token, &h, &rpc(3, "server/discover", meta.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["supportedVersions"], json!([MODERN, "2025-11-25"]));
    let h = [("MCP-Protocol-Version", MODERN), ("Mcp-Method", "tools/list")];
    let (_, _, modern) = mcp(&server, token, &h, &rpc(4, "tools/list", meta)).await;
    assert_eq!(modern["result"]["resultType"], "complete");
    assert_eq!(modern["result"]["tools"], legacy["result"]["tools"]);

    // Errors are 400/404 JSON-RPC, never 422.
    let (status, _, body) = mcp(&server, token, &[("Mcp-Method", "ping")], &rpc(5, "tools/list", json!({}))).await;
    assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32020)));
    let (status, _, body) = mcp(&server, token, &[], &rpc(6, "resources/list", json!({}))).await;
    assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::NOT_FOUND, Some(-32601)));
    let unsupported = rpc(7, "tools/list", json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2099-01-01"}}));
    let (status, _, body) = mcp(&server, token, &[], &unsupported).await;
    assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32022)));
    let r = client().post(server.url("/mcp")).bearer_auth(&tokens.access).body("not json").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_tool_call_reaches_the_phone_and_the_answer_comes_back() {
    let (server, phone, tokens) = connected().await;
    let ai = tool(&server, &tokens.access, "gmail_search", json!({"query": "from:bank"}));
    let phone_side = answer_next(&phone, search_result());
    let (result, request) = tokio::join!(ai, phone_side);
    assert_eq!(result["isError"], false, "{result}");
    let msg = &result["structuredContent"]["messages"][0];
    assert_eq!((msg["id"].as_str(), msg["date"].as_str()), (Some("m1"), Some("2023-11-14T22:13:20Z")));
    assert_eq!(serde_json::from_str::<Value>(text_of(&result)).unwrap(), result["structuredContent"]);
    // What the phone saw: the relayed, normalized call attributed to the connection.
    assert_eq!(request["connection_label"], "Claude");
    assert_eq!(request["connection_id"], tokens.connection_id);
    assert_eq!(request["call"], json!({"tool": "gmail_search", "query": "from:bank", "max_results": 10}));
}

#[tokio::test]
async fn denials_and_device_errors_are_tool_errors() {
    let (server, phone, tokens) = connected().await;
    let ai = tool(&server, &tokens.access, "gmail_read", json!({"message_ids": ["m1"]}));
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    let (result, _) = tokio::join!(ai, answer_next(&phone, denied));
    assert_eq!(result["isError"], true);
    assert_eq!(text_of(&result), "Denied by the user on their Reins device.");
    let ai = tool(&server, &tokens.access, "gmail_read", json!({"message_ids": ["m1"]}));
    let failed = json!({"v": 1, "outcome": "error", "message": "Gmail needs consent"});
    let (result, _) = tokio::join!(ai, answer_next(&phone, failed));
    assert_eq!(text_of(&result), "The Reins device could not complete the request: Gmail needs consent");
}

#[tokio::test]
async fn invalid_arguments_never_reach_the_phone() {
    let (server, phone, tokens) = connected().await;
    let bcc = tool(
        &server,
        &tokens.access,
        "gmail_send",
        json!({"to": ["a@b.com"], "subject": "s", "body": "b", "bcc": ["x@evil.com"]}),
    )
    .await;
    assert_eq!(bcc["isError"], true);
    assert!(text_of(&bcc).contains("cannot send Bcc"), "{bcc}");
    let injected = tool(
        &server,
        &tokens.access,
        "gmail_send",
        json!({"to": ["a@b.com\r\nBcc: x@evil.com"], "subject": "s", "body": "b"}),
    )
    .await;
    assert_eq!(injected["isError"], true);
    let too_many = tool(&server, &tokens.access, "gmail_search", json!({"query": "x", "max_results": 500})).await;
    assert_eq!(too_many["isError"], true);
    let (status, _, body) =
        mcp(&server, Some(&tokens.access), &[], &call_tool(9, "delete_everything", json!({}))).await;
    assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32602)));
    let (_, pending) = phone.get("/pending?wait=0").await;
    assert_eq!(pending["requests"], json!([]), "rejected calls must not be relayed");
}

#[tokio::test]
async fn an_absent_phone_is_reported_as_offline_after_the_short_threshold() {
    let (server, _phone, tokens) = connected().await;
    let start = Instant::now();
    let result = tool(&server, &tokens.access, "gmail_search", json!({"query": "x"})).await;
    let elapsed = start.elapsed();
    assert_eq!(result["isError"], true);
    let text = text_of(&result);
    assert!(text.starts_with("Reins: your approval device is offline. Ask the user to open the Reins app; the request is waiting there. Then call reins_get_result with request_id="), "{text}");
    assert!(
        elapsed >= Duration::from_millis(1800) && elapsed < Duration::from_millis(3900),
        "offline after {elapsed:?}, not after the full wait"
    );
}

#[tokio::test]
async fn a_fetched_but_undecided_request_is_pending_then_delivered_late_exactly_to_its_connection() {
    let (server, phone, tokens) = connected().await;
    // Offline first, so the request is waiting on the phone.
    let offline = tool(&server, &tokens.access, "gmail_search", json!({"query": "x"})).await;
    let request_id = between(text_of(&offline), "request_id=", ".").to_owned();
    // The user opens the app later: fetching marks it delivered; the AI asks again and now waits the full time.
    let (status, request) = phone.get(&format!("/requests/{request_id}")).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    let start = Instant::now();
    let pending = tool(&server, &tokens.access, "reins_get_result", json!({"request_id": request_id})).await;
    assert!(start.elapsed() >= Duration::from_millis(3800), "pending only after the relay wait");
    assert_eq!(
        text_of(&pending),
        format!(
            "Waiting for the user to approve on their phone. When they confirm (the user can approve even after this message), call reins_get_result with request_id={request_id}, or repeat the same request: a one-time approval may already cover it."
        )
    );
    // The user approves after the AI gave up.
    let (status, _) = phone.post(&format!("/requests/{request_id}/response"), &search_result()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let late = tool(&server, &tokens.access, "reins_get_result", json!({"request_id": request_id})).await;
    assert_eq!(late["isError"], false, "{late}");
    assert_eq!(late["structuredContent"]["messages"][0]["id"], "m1");
    let again = tool(&server, &tokens.access, "reins_get_result", json!({"request_id": request_id})).await;
    assert_eq!(again["structuredContent"], late["structuredContent"], "readable until it expires");
    // Another AI connection of the same user cannot read it.
    let other = server.connect_ai(&phone, "mia@example.com", Some("ChatGPT")).await;
    let stolen = tool(&server, &other.access, "reins_get_result", json!({"request_id": request_id})).await;
    assert_eq!(stolen["isError"], true);
    assert!(text_of(&stolen).contains("Unknown or expired request_id"), "{stolen}");
}

#[tokio::test]
async fn revoking_a_connection_kills_its_tokens_at_once() {
    let (server, phone, tokens) = connected().await;
    assert_eq!(mcp(&server, Some(&tokens.access), &[], &rpc(1, "ping", json!({}))).await.0, StatusCode::OK);
    let (status, _) = phone.delete(&format!("/connections/{}", tokens.connection_id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The access token is still a valid JWT, but its connection is gone.
    let (status, challenge, _) = mcp(&server, Some(&tokens.access), &[], &rpc(2, "ping", json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(challenge.unwrap().contains("invalid_token"));
    let (status, body) =
        server.token(&[("grant_type", "refresh_token"), ("refresh_token", tokens.refresh.as_str())]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
}

#[tokio::test]
async fn an_ai_never_gets_another_users_relay() {
    let server = Server::start().await;
    let alice = server.phone("alice2@example.com").await;
    alice.register_device().await;
    let bob = server.phone("bob2@example.com").await;
    bob.register_device().await;
    let alice_ai = server.connect_ai(&alice, "alice2@example.com", Some("Alice AI")).await;
    let ai = tool(&server, &alice_ai.access, "gmail_search", json!({"query": "secret"}));
    let bob_side = async {
        let (_, pending) = bob.get("/pending?wait=3").await;
        pending["requests"].clone()
    };
    let alice_side = answer_next(&alice, search_result());
    let (result, bob_saw, _) = tokio::join!(ai, bob_side, alice_side);
    assert_eq!(bob_saw, json!([]), "bob's phone must never see alice's request");
    assert_eq!(result["isError"], false);
}

fn tool_names(body: &Value) -> Vec<String> {
    body["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_owned())
        .collect()
}

#[tokio::test]
async fn tools_of_integrations_without_an_account_are_not_listed_once_the_phone_says_which_it_has() {
    let (server, phone, tokens) = connected().await;
    let list = || rpc(1, "tools/list", json!({}));

    // Nothing reported yet: every tool is listed.
    let (_, _, all) = mcp(&server, Some(&tokens.access), &[], &list()).await;
    let all = tool_names(&all);
    assert!(all.iter().any(|n| n == "gmail_search") && all.iter().any(|n| n == "github_list_repos"));

    let (status, body) = phone.put("/services", &json!({"services": ["github", "nonsense"]})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, _, some) = mcp(&server, Some(&tokens.access), &[], &list()).await;
    let some = tool_names(&some);
    assert!(some.iter().any(|n| n == "github_list_repos"));
    assert!(!some.iter().any(|n| n.starts_with("gmail_") || n.starts_with("telegram_") || n.starts_with("vault_")));
    for always in ["reins_get_result", "reins_list_accounts", "reins_request_access"] {
        assert!(some.iter().any(|n| n == always), "{always}");
    }

    let (status, _) = phone.put("/services", &json!({"services": ["gmail"]})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, gmail) = mcp(&server, Some(&tokens.access), &[], &list()).await;
    let gmail = tool_names(&gmail);
    assert!(gmail.iter().any(|n| n == "gmail_search") && !gmail.iter().any(|n| n.starts_with("github_")));

    let (status, _) = phone.put("/services", &json!("not an object")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
