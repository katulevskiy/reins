//! A fake MCP server (Streamable HTTP, JSON or SSE answers, sessions, bearer tokens with a 401 that points at its
//! resource metadata), a fake OAuth authorization server (metadata, registration, token) and the Reins server
//! endpoints universal MCP uses.

use std::sync::{Arc, Mutex};

use reins_proto::gmail::ToolCall;
use reins_proto::relay::RelayRequest;
use reins_proto::remote_mcp::McpCall;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// What the fake MCP server does and what it saw.
#[derive(Default)]
pub struct McpState {
    /// Answer with Server-Sent Events instead of JSON.
    pub sse: bool,
    /// The one token accepted (`None`: no sign-in needed).
    pub valid_token: Option<String>,
    /// Named in the 401's `WWW-Authenticate`.
    pub resource_metadata: Option<String>,
    /// Refuse protocol version 2025-06-18 (an older server).
    pub reject_latest: bool,
    /// The session the server keeps; set it to something else to end the session.
    pub session: String,
    /// Size of the text `export` returns.
    pub big: usize,
    pub inits: u32,
    pub initialized: u32,
    /// Every `MCP-Protocol-Version` header seen.
    pub protocols: Vec<String>,
    /// Every `Authorization` header seen.
    pub tokens_seen: Vec<String>,
    /// Every tool call: name and arguments.
    pub calls: Vec<(String, Value)>,
}

#[derive(Clone)]
pub struct McpMock(pub Arc<Mutex<McpState>>);

fn tool_page(cursor: Option<&str>) -> Value {
    match cursor {
        None => json!({"tools": [
            {"name": "search", "title": "Search issues", "description": "Finds issues",
             "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}},
             "annotations": {"readOnlyHint": true}},
            {"name": "create_issue", "description": "Creates an issue",
             "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}}},
             "annotations": {"destructiveHint": false}}],
            "nextCursor": "page-2"}),
        Some(_) => json!({"tools": [
            {"name": "delete_issue", "description": "Deletes an issue", "inputSchema": {"type": "object"}},
            {"name": "export", "description": "Exports everything", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}}]}),
    }
}

fn reply(sse: bool, session: Option<&str>, message: &Value) -> ResponseTemplate {
    let mut response = if sse {
        let note = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "working"}});
        ResponseTemplate::new(200).set_body_raw(
            format!("event: message\ndata: {note}\n\n: keep-alive\n\nevent: message\ndata: {message}\n\n"),
            "text/event-stream",
        )
    } else {
        ResponseTemplate::new(200).set_body_json(message)
    };
    if let Some(session) = session {
        response = response.insert_header("Mcp-Session-Id", session);
    }
    response
}

impl Respond for McpMock {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let mut st = self.0.lock().unwrap();
        let header = |name: &str| req.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        if let Some(auth) = header("authorization") {
            st.tokens_seen.push(auth);
        }
        if let Some(valid) = st.valid_token.clone()
            && header("authorization") != Some(format!("Bearer {valid}"))
        {
            let mut r = ResponseTemplate::new(401);
            if let Some(at) = &st.resource_metadata {
                r = r.insert_header(
                    "WWW-Authenticate",
                    format!("Bearer error=\"invalid_token\", resource_metadata=\"{at}\""),
                );
            }
            return r;
        }
        assert!(header("accept").is_some_and(|a| a.contains("application/json") && a.contains("text/event-stream")));
        let Ok(body) = serde_json::from_slice::<Value>(&req.body) else {
            return ResponseTemplate::new(400);
        };
        let method = body["method"].as_str().unwrap_or_default().to_owned();
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        if method == "initialize" {
            st.inits += 1;
            let version = body["params"]["protocolVersion"].as_str().unwrap_or_default().to_owned();
            if st.reject_latest && version == "2025-06-18" {
                let error = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "Unsupported protocol version"}});
                return reply(st.sse, None, &error);
            }
            st.session = format!("sess-{}", st.inits);
            let result = json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": version,
                "capabilities": {"tools": {}}, "serverInfo": {"name": "mock", "version": "1"}}});
            let session = st.session.clone();
            return reply(st.sse, Some(&session), &result);
        }
        // Everything else belongs to the session.
        match header("mcp-session-id") {
            None => return ResponseTemplate::new(400),
            Some(s) if s != st.session => return ResponseTemplate::new(404),
            Some(_) => {}
        }
        let Some(version) = header("mcp-protocol-version") else {
            return ResponseTemplate::new(400);
        };
        st.protocols.push(version);
        let answer = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
        match method.as_str() {
            "notifications/initialized" => {
                st.initialized += 1;
                ResponseTemplate::new(202)
            }
            "tools/list" => reply(st.sse, None, &answer(tool_page(body["params"]["cursor"].as_str()))),
            "tools/call" => {
                let name = body["params"]["name"].as_str().unwrap_or_default().to_owned();
                let args = body["params"]["arguments"].clone();
                st.calls.push((name.clone(), args.clone()));
                let result = match name.as_str() {
                    "search" if !args["query"].is_string() => {
                        let error = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "query must be a string"}});
                        return reply(st.sse, None, &error);
                    }
                    "search" => {
                        json!({"content": [{"type": "text", "text": format!("found 1 issue for {}", args["query"])}],
                        "structuredContent": {"issues": [{"id": "ISS-1"}]}})
                    }
                    "create_issue" => json!({"content": [{"type": "text", "text": "created ISS-2"}]}),
                    "delete_issue" => json!({"content": [{"type": "text", "text": "deleted"}]}),
                    "export" => {
                        json!({"content": [{"type": "text", "text": "x".repeat(st.big)}, {"type": "text", "text": "done"}]})
                    }
                    _ => {
                        let error =
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "Unknown tool"}});
                        return reply(st.sse, None, &error);
                    }
                };
                reply(st.sse, None, &answer(result))
            }
            _ => reply(
                st.sse,
                None,
                &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no such method"}}),
            ),
        }
    }
}

/// The fake MCP server at `<mcp>/mcp`, with its resource metadata naming `auth` as its authorization server.
pub async fn mount_mcp(mcp: &MockServer, auth: &MockServer, state: &Arc<Mutex<McpState>>) {
    state.lock().unwrap().resource_metadata = Some(format!("{}/.well-known/oauth-protected-resource/mcp", mcp.uri()));
    Mock::given(method("POST")).and(path("/mcp")).respond_with(McpMock(Arc::clone(state))).mount(mcp).await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", mcp.uri()), "authorization_servers": [auth.uri()], "scopes_supported": ["issues"]})))
        .mount(mcp)
        .await;
}

/// The tokens the fake authorization server hands out: the first pair for the code, the second for the first refresh
/// token. Any other grant is refused (`invalid_grant`).
pub struct AuthTokens<'a> {
    pub access: [&'a str; 2],
    pub refresh: [&'a str; 2],
    pub client_secret: Option<&'a str>,
}

pub async fn mount_auth(auth: &MockServer, tokens: &AuthTokens<'_>) {
    let base = auth.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base, "authorization_endpoint": format!("{base}/authorize"), "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"), "code_challenge_methods_supported": ["S256"],
            "response_types_supported": ["code"]})))
        .mount(auth)
        .await;
    let registered = match tokens.client_secret {
        Some(secret) => {
            json!({"client_id": "client-1", "client_secret": secret, "token_endpoint_auth_method": "client_secret_post"})
        }
        None => json!({"client_id": "client-1", "token_endpoint_auth_method": "none"}),
    };
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(ResponseTemplate::new(201).set_body_json(registered))
        .mount(auth)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=CODE-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": tokens.access[0], "refresh_token": tokens.refresh[0], "expires_in": 3600, "token_type": "Bearer"})))
        .mount(auth)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains(format!("refresh_token={}", tokens.refresh[0])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": tokens.access[1], "refresh_token": tokens.refresh[1], "expires_in": 3600, "token_type": "Bearer"})))
        .mount(auth)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .with_priority(10)
        .mount(auth)
        .await;
}

/// `PUT /reins/api/blobs/output`: a download for the bytes received.
struct OutputBlob;

impl Respond for OutputBlob {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let name = req.url.query_pairs().find(|(k, _)| k == "name").map(|(_, v)| v.into_owned()).unwrap_or_default();
        let content_type = req.headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "blob-0123456789abcdef", "download_url": format!("https://rw.example/reins/blob/dl-{name}"),
            "name": name, "size": req.body.len(), "sha256": "00", "content_type": content_type, "expires_at": 1_800_000_000}))
    }
}

/// `POST /reins/api/mcp/call`: the server made the call and kept the large content.
struct Proxy;

impl Respond for Proxy {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let call: Value = serde_json::from_slice(&req.body).unwrap();
        let id = call["request"]["id"].clone();
        ResponseTemplate::new(200).set_body_json(json!({
            "status": 200,
            "response": {"jsonrpc": "2.0", "id": id, "result": {"content": [
                {"type": "resource_link", "uri": "https://rw.example/reins/blob/proxied", "name": "export-1.txt"}]}},
            "session_id": null, "downloads": []}))
    }
}

/// The Reins server: sign-in, the services report, answers, output blobs and proxied calls; nothing pending.
pub async fn mount_reins(rw: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/identity/accounts/prelogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kdf": 0, "kdfIterations": 5000})))
        .mount(rw)
        .await;
    Mock::given(method("POST"))
        .and(path("/identity/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "RW-ACCESS", "refresh_token": "RW-REFRESH", "expires_in": 7200})))
        .mount(rw)
        .await;
    Mock::given(method("PUT"))
        .and(path("/reins/api/services"))
        .respond_with(ResponseTemplate::new(204))
        .mount(rw)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/reins/api/requests/[^/]+/response$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(rw)
        .await;
    Mock::given(method("PUT")).and(path("/reins/api/blobs/output")).respond_with(OutputBlob).mount(rw).await;
    Mock::given(method("POST")).and(path("/reins/api/mcp/call")).respond_with(Proxy).mount(rw).await;
    Mock::given(method("GET"))
        .and(path("/reins/api/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requests": [], "pairings": []})))
        .mount(rw)
        .await;
}

/// A relayed call to a tool of an added server, from connection `c1` ("Claude").
pub fn mcp_request(id: &str, server: &str, tool: &str, arguments: &Value) -> RelayRequest {
    RelayRequest {
        v: 1,
        id: id.into(),
        connection_id: "c1".into(),
        connection_label: "Claude".into(),
        created_at: 1,
        wait_until: None,
        account: None,
        call: ToolCall::Mcp(McpCall {
            server: server.to_owned(),
            tool: tool.to_owned(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
        }),
    }
}

/// The bodies of every request to `path` the server received, oldest first.
pub async fn bodies(server: &MockServer, http_method: &str, at: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == http_method && r.url.path() == at)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}
