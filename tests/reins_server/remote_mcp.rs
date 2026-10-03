//! Universal MCP against the real server: tools from the phone's report, calls relayed as `ToolCall::Mcp` with results
//! passed through, `reins_upload`, the `X-Reins-Via` label, and `POST /reins/api/mcp/call`.

use data_encoding::BASE64;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::{
    harness::{Options, Phone, Server, Tokens, client},
    mock::{MockServer, Reply},
};

async fn connected(allow_loopback: bool) -> (Server, Phone, Tokens) {
    let server = Server::start_with(Options {
        allow_loopback,
        ..Options::default()
    })
    .await;
    let phone = server.phone("mcp@example.com").await;
    phone.register_device().await;
    let tokens = server.connect_ai(&phone, "mcp@example.com", Some("Laptop")).await;
    (server, phone, tokens)
}

async fn rpc(server: &Server, token: &str, headers: &[(&str, &str)], method: &str, params: Value) -> Value {
    let mut request = client()
        .post(server.url("/mcp"))
        .bearer_auth(token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}));
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let response = request.send().await.expect("mcp");
    response.json().await.expect("mcp json")
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

fn linear_report() -> Value {
    json!({"services": ["github"], "mcp": [
        {"id": "linear", "name": "Linear", "tools": [
            {"name": "search.issues", "title": "Search issues", "description": "Finds issues",
             "input_schema": {"type": "object", "properties": {"q": {"type": "string"}}}, "read_only": true},
            {"name": "delete_issue", "description": "Deletes an issue", "input_schema": {"type": "object"},
             "destructive": true}
        ]},
        {"id": "Not_Valid", "name": "Broken", "tools": []},
        {"id": "schemaless", "name": "Bad", "tools": [{"name": "t", "description": "d", "input_schema": "nope"}]}
    ]})
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn tools_of_the_phones_mcp_servers_are_listed_relayed_and_answered_as_they_are() {
    let (server, phone, tokens) = connected(false).await;
    let before = names(&rpc(&server, &tokens.access, &[], "tools/list", json!({})).await);
    assert!(!before.iter().any(|n| n.contains("__")), "nothing reported yet");

    let (status, body) = phone.put("/services", &linear_report()).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let list = rpc(&server, &tokens.access, &[], "tools/list", json!({})).await;
    let listed = names(&list);
    let mcp: Vec<&String> = listed.iter().filter(|n| n.contains("__")).collect();
    assert_eq!(mcp, ["linear__search_issues", "linear__delete_issue"], "invalid servers are dropped");
    assert!(listed.iter().any(|n| n == "reins_upload") && listed.iter().any(|n| n == "github_list_repos"));
    let search =
        list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "linear__search_issues").unwrap();
    assert_eq!(
        (search["title"].as_str(), search["description"].as_str()),
        (Some("Linear: Search issues"), Some("Linear: Finds issues"))
    );
    assert_eq!(search["inputSchema"]["properties"]["q"]["type"], "string");
    assert_eq!(search["annotations"]["readOnlyHint"], true);
    let delete =
        list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "linear__delete_issue").unwrap();
    assert_eq!(
        (delete["annotations"]["readOnlyHint"].as_bool(), delete["annotations"]["destructiveHint"].as_bool()),
        (Some(false), Some(true))
    );

    let ai = rpc(
        &server,
        &tokens.access,
        &[("X-Reins-Via", "  Claude\tCode  ")],
        "tools/call",
        json!({"name": "linear__search_issues", "arguments": {"q": "bug"}}),
    );
    let answer = json!({"v": 1, "outcome": "result", "result": {"kind": "mcp", "result": {
        "content": [{"type": "text", "text": "2 issues"}, {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}],
        "structuredContent": {"count": 2}, "isError": false, "_meta": {"from": "linear"}}}});
    let (reply, request) = tokio::join!(ai, answer_next(&phone, answer));
    assert_eq!(request["connection_label"], "Laptop \u{b7} Claude Code", "who asked is shown");
    assert_eq!(request["call"]["server"], "linear");
    assert_eq!(request["call"]["arguments"], json!({"q": "bug"}));
    assert_eq!(
        reply["result"],
        json!({"content": [{"type": "text", "text": "2 issues"}, {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}],
            "structuredContent": {"count": 2}, "isError": false}),
        "passed through, not rendered as JSON text"
    );

    let unknown =
        rpc(&server, &tokens.access, &[], "tools/call", json!({"name": "linear__nope", "arguments": {}})).await;
    assert_eq!(unknown["error"]["code"], -32602);
    let bad =
        rpc(&server, &tokens.access, &[], "tools/call", json!({"name": "linear__search_issues", "arguments": [1]}))
            .await;
    assert_eq!(bad["result"]["isError"], true);

    // Without the header the label is the connection's own; without MCP servers in the report the tools are gone.
    let ai = rpc(&server, &tokens.access, &[], "tools/call", json!({"name": "linear__delete_issue", "arguments": {}}));
    let denied = json!({"v": 1, "outcome": "denied", "reason": null});
    let (_, request) = tokio::join!(ai, answer_next(&phone, denied));
    assert_eq!(request["connection_label"], "Laptop");
    assert_eq!(phone.put("/services", &json!({"services": ["github"]})).await.0, StatusCode::NO_CONTENT);
    let after = names(&rpc(&server, &tokens.access, &[], "tools/list", json!({})).await);
    assert!(!after.iter().any(|n| n.contains("__")));
}

#[tokio::test]
async fn reins_upload_asks_the_phone_for_an_upload_link() {
    let (server, phone, tokens) = connected(false).await;
    let ai = rpc(
        &server,
        &tokens.access,
        &[],
        "tools/call",
        json!({"name": "reins_upload", "arguments": {"name": "build.zip", "size": 5_000_000, "reason": "Attach the build"}}),
    );
    let ready = json!({"v": 1, "outcome": "result", "result": {"kind": "connector", "data": {"status": "upload_ready",
        "upload_url": "https://rw.example/reins/blob/x"}}});
    let (reply, request) = tokio::join!(ai, answer_next(&phone, ready));
    assert_eq!(
        request["call"],
        json!({"tool": "request_upload", "name": "build.zip", "size": 5_000_000, "content_type": null, "reason": "Attach the build"})
    );
    assert_eq!(reply["result"]["structuredContent"]["status"], "upload_ready");
    let invalid =
        rpc(&server, &tokens.access, &[], "tools/call", json!({"name": "reins_upload", "arguments": {"name": "x"}}))
            .await;
    assert_eq!(invalid["result"]["isError"], true);
}

/// An MCP server: `json` answers as JSON, `sse` as an event stream, `image` with a large picture.
async fn mcp_server(image: Vec<u8>) -> MockServer {
    MockServer::start(move |r| {
        let request: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        let id = request["id"].clone();
        let reply = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
        match request["params"]["name"].as_str() {
            Some("json") => {
                Reply::json(200, &reply(json!({"content": [{"type": "text", "text": "small"}], "isError": false})))
                    .with_header("Mcp-Session-Id", "session-2")
            }
            Some("sse") => {
                let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}});
                let done = reply(json!({"content": [{"type": "text", "text": "streamed"}], "isError": false}));
                Reply::new(
                    200,
                    "text/event-stream",
                    format!("event: message\ndata: {progress}\n\nevent: message\ndata: {done}\n\n"),
                )
            }
            Some("image") => Reply::json(
                200,
                &reply(json!({"content": [{"type": "text", "text": "a chart"},
                    {"type": "image", "data": BASE64.encode(&image), "mimeType": "image/png"}], "isError": false})),
            ),
            _ => {
                Reply::json(200, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "unknown"}}))
            }
        }
    })
    .await
}

fn proxy_call(tokens: &Tokens, endpoint: &str, tool: &str) -> Value {
    json!({"v": 1, "connection_id": tokens.connection_id, "endpoint": endpoint,
        "headers": [["Authorization", "Bearer mcp-token"], ["Mcp-Session-Id", "session-1"], ["MCP-Protocol-Version", "2025-06-18"]],
        "request": {"jsonrpc": "2.0", "id": 41, "method": "tools/call", "params": {"name": tool, "arguments": {}}}})
}

#[tokio::test]
async fn proxy_calls_read_json_and_sse_and_turn_large_content_into_links() {
    let (_server, phone, tokens) = connected(true).await;
    let mut image = b"\x89PNG\r\n\x1a\n".to_vec();
    image.resize(300 * 1024, 7);
    let mcp = mcp_server(image.clone()).await;
    let endpoint = mcp.url("/mcp");

    let (status, got) = phone.post("/mcp/call", &proxy_call(&tokens, &endpoint, "json")).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!((got["status"].as_u64(), got["session_id"].as_str()), (Some(200), Some("session-2")));
    assert_eq!(got["response"]["result"]["content"][0]["text"], "small");
    assert_eq!(got["response"]["id"], 41);
    assert_eq!(got["downloads"], json!([]));
    let seen = &mcp.requests()[0];
    assert_eq!(seen.header("authorization"), Some("Bearer mcp-token"));
    assert_eq!(seen.header("mcp-session-id"), Some("session-1"));
    assert_eq!(seen.header("accept"), Some("application/json, text/event-stream"));
    assert_eq!(seen.header("content-type"), Some("application/json"));
    assert_eq!(serde_json::from_slice::<Value>(&seen.body).unwrap()["params"]["name"], "json");

    let (status, got) = phone.post("/mcp/call", &proxy_call(&tokens, &endpoint, "sse")).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["response"]["result"]["content"][0]["text"], "streamed", "the response, not the notification");

    let (status, got) = phone.post("/mcp/call", &proxy_call(&tokens, &endpoint, "image")).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let content = &got["response"]["result"]["content"];
    assert_eq!(content[0], json!({"type": "text", "text": "a chart"}), "small items stay");
    assert_eq!(
        (content[1]["type"].as_str(), content[1]["mimeType"].as_str()),
        (Some("resource_link"), Some("image/png"))
    );
    assert_eq!(content[1]["name"], "image-2.png");
    let download = &got["downloads"][0];
    assert_eq!(content[1]["uri"], download["download_url"]);
    assert_eq!(
        (download["size"].as_u64(), download["content_type"].as_str()),
        (Some(image.len() as u64), Some("image/png"))
    );
    let r = client().get(download["download_url"].as_str().unwrap()).send().await.unwrap();
    assert_eq!(r.bytes().await.unwrap().as_ref(), image.as_slice());

    let (status, got) = phone.post("/mcp/call", &proxy_call(&tokens, &endpoint, "other")).await;
    assert_eq!((status, got["response"]["error"]["code"].as_i64()), (StatusCode::OK, Some(-32602)));

    let mut no_id = proxy_call(&tokens, &endpoint, "json");
    no_id["request"].as_object_mut().unwrap().remove("id");
    assert_eq!(phone.post("/mcp/call", &no_id).await.0, StatusCode::BAD_REQUEST);
    let mut private = proxy_call(&tokens, "https://192.168.1.10/mcp", "json");
    assert_eq!(phone.post("/mcp/call", &private).await.0, StatusCode::BAD_REQUEST);
    private["connection_id"] = json!("someone-elses");
    assert_eq!(phone.post("/mcp/call", &private).await.0, StatusCode::NOT_FOUND);
}
