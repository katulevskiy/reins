//! Host half of the iOS live smoke test (`ios/scripts/live-smoke.sh`): runs the real server, a small MCP server the
//! phone adds, and plays the AI client, while the real iOS app on a simulator signs in, pairs and approves.
//!
//! ```text
//! cargo run -p rewarden-e2e --example ios_smoke -- /tmp/reins-live
//! # prints SERVER, EMAIL, PASSWORD and MCP; once the phone registered as the approval device, CODE;
//! # once the phone added the MCP server: one call approved once, one allowed for a while, one the grant answers.
//! ```
//!
//! Unlike `device_smoke` (Android), the calls go to an MCP server on loopback: the simulator has no Google account, so
//! a Gmail call is answered with an error at once and never reaches an approval sheet.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rewarden_e2e::{AiClient, PASSWORD, Server};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EMAIL: &str = "phone@example.com";

async fn wait_file(path: &Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A notes MCP server (JSON answers): one read-only tool and one that changes things.
async fn fake_mcp() -> MockServer {
    let mcp = MockServer::start().await;
    let answer = |method_name: &str, result: Value| {
        Mock::given(method("POST")).and(body_partial_json(json!({"method": method_name}))).respond_with(
            move |req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result.clone()}))
            },
        )
    };
    answer(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "notes", "version": "1"}}),
    )
    .mount(&mcp)
    .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "notifications/initialized"})))
        .respond_with(ResponseTemplate::new(202))
        .mount(&mcp)
        .await;
    answer(
        "tools/list",
        json!({"tools": [
            {"name": "list_notes", "description": "Lists notes.", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}},
            {"name": "add_note", "description": "Adds a note.", "inputSchema": {"type": "object",
             "properties": {"text": {"type": "string"}}, "required": ["text"]},
             "annotations": {"readOnlyHint": false, "destructiveHint": false}}]}),
    )
    .mount(&mcp)
    .await;
    answer("tools/call", json!({"content": [{"type": "text", "text": "Notes: buy milk."}], "isError": false}))
        .mount(&mcp)
        .await;
    mcp
}

/// The AI's name for `tool` once the phone added the notes server (the server lists phone tools as `<id>__<tool>`).
async fn phone_tool(ai: &AiClient, tool: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let (_, listed) = ai.rpc(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})).await;
        let suffix = format!("__{tool}");
        if let Some(name) = listed["result"]["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str())
            .find(|n| n.ends_with(&suffix))
        {
            return name.to_owned();
        }
        assert!(Instant::now() < deadline, "the phone never added the MCP server");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// One call, printed as `TOOL_RESULT <step> <seconds> <result>`.
async fn call(ai: &AiClient, step: &str, name: &str, args: &Value) -> Value {
    let started = Instant::now();
    let result = ai.tool(name, args).await;
    println!("TOOL_RESULT {step} {:.1}s {result}", started.elapsed().as_secs_f32());
    result
}

#[tokio::main]
async fn main() {
    rewarden_e2e::init_tls();
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/reins-live".to_owned()));
    std::fs::create_dir_all(&dir).expect("coordination dir");
    let mcp = fake_mcp().await;
    let server = Server::start(45, 10).await;
    server.register(EMAIL).await;
    println!("SERVER {}", server.base);
    println!("EMAIL {EMAIL}");
    println!("PASSWORD {PASSWORD}");
    println!("MCP {}/mcp", mcp.uri());

    // The AI connects after the phone registered as the approval device.
    while !server.log().contains("PUT /rewarden/api/device") {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("REGISTERED");
    let mut ai = AiClient::new(&server.base);
    ai.register_client().await;
    let wait_url = ai.start_authorization(EMAIL).await;
    let code = ai.browser_code(&wait_url).await;
    println!("CODE {code:02}");
    ai.finish_when_approved(&wait_url, Duration::from_secs(300)).await;
    println!("PAIRED");

    let add_note = phone_tool(&ai, "add_note", Duration::from_secs(300)).await;
    let list_notes = phone_tool(&ai, "list_notes", Duration::from_secs(10)).await;
    println!("MCP_READY {add_note} {list_notes}");

    // Approved once on the phone.
    let added = call(&ai, "add_note", &add_note, &json!({"text": "buy milk"})).await;
    assert_eq!(added["isError"], false, "{added}");
    // Allowed for a while on the phone: a grant.
    let listed = call(&ai, "list_notes", &list_notes, &json!({})).await;
    assert_eq!(listed["isError"], false, "{listed}");
    // The grant answers this one while the app is open; nobody is asked.
    let again = call(&ai, "list_notes_again", &list_notes, &json!({})).await;
    assert_eq!(again["isError"], false, "{again}");
    // No Google account on the simulator: an error at once, never a hang.
    call(&ai, "gmail_search", "gmail_search", &json!({"query": "from:bank"})).await;
    assert!(!server.log().contains("panicked"), "server log: {}", server.log());
    println!("ALL_DONE");

    // Keep the server up while the app is looked at.
    wait_file(&dir.join("done")).await;
}
