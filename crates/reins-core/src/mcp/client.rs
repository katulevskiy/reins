//! An MCP client over Streamable HTTP: every message is a POST to the endpoint, answered with JSON or with a Server-Sent
//! Events stream that is read until the answer to the request arrives. `initialize` (2025-06-18, falling back to
//! 2025-03-26), `notifications/initialized`, `tools/list` page by page, `tools/call`. The session the server opens
//! (`Mcp-Session-Id`) is sent back on every later request, with `MCP-Protocol-Version`.
//!
//! Nothing here is logged: requests carry the token, answers carry the user's data.

use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
use serde_json::{Map, Value, json};

use super::Connected;
use crate::store::McpTool;
use crate::{CoreError, text};

/// The protocol version asked for first.
pub const LATEST: &str = "2025-06-18";
/// The one asked for when a server does not take [`LATEST`].
pub const FALLBACK: &str = "2025-03-26";
/// Largest answer read on the phone; a larger one is [`McpError::TooLarge`].
pub const MAX_BODY: usize = 32 << 20;
const MAX_PAGES: usize = 50;
const MAX_LISTED_TOOLS: usize = 1_000;
const ACCEPT: &str = "application/json, text/event-stream";

/// Why an MCP request failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpError {
    /// 401: no token, or not a good one. `challenge` is the `WWW-Authenticate` header (it may point at the resource
    /// metadata).
    Unauthorized {
        challenge: Option<String>,
    },
    /// 404 for a request in a session: the server ended the session; initialize again.
    SessionGone,
    /// The server answered with a JSON-RPC error (already made safe to show).
    Rpc {
        code: i64,
        message: String,
    },
    /// Any other HTTP status.
    Status(u16),
    /// The answer was larger than [`MAX_BODY`].
    TooLarge,
    /// The answer made no sense.
    Protocol(String),
    Core(CoreError),
}

impl From<reqwest::Error> for McpError {
    fn from(e: reqwest::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<CoreError> for McpError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl McpError {
    /// What the user (or the AI) is told; never a URL or a token.
    pub fn describe(&self) -> String {
        match self {
            Self::Unauthorized {
                ..
            } => "the server needs a sign-in".to_owned(),
            Self::SessionGone => "the server ended the session".to_owned(),
            Self::Rpc {
                message,
                ..
            } => format!("the server answered with an error: {message}"),
            Self::Status(status) => format!("the server answered with HTTP {status}"),
            Self::TooLarge => "the answer was too large for the phone".to_owned(),
            Self::Protocol(why) => format!("the server's answer was not understood ({why})"),
            Self::Core(e) => e.to_string(),
        }
    }
}

/// One MCP endpoint, with the token to send (if any).
pub struct Endpoint<'a> {
    pub http: &'a reqwest::Client,
    pub url: &'a str,
    pub token: Option<&'a str>,
    pub ids: &'a AtomicU64,
}

/// What one POST brought back.
struct Reply {
    session_id: Option<String>,
    message: Option<Value>,
}

impl Endpoint<'_> {
    /// The headers of a request in `conn` (for the server to make a call on the phone's behalf too).
    pub fn headers(&self, conn: Option<&Connected>) -> Vec<(String, String)> {
        let mut headers = Vec::new();
        if let Some(token) = self.token {
            headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
        }
        if let Some(conn) = conn {
            if let Some(session) = &conn.session_id {
                headers.push(("Mcp-Session-Id".to_owned(), session.clone()));
            }
            headers.push(("MCP-Protocol-Version".to_owned(), conn.protocol.clone()));
        }
        headers
    }

    /// A JSON-RPC request with a fresh id.
    pub fn request(&self, method: &str, params: &Value) -> (u64, Value) {
        let id = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        (id, json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
    }

    async fn post(&self, conn: Option<&Connected>, body: &Value, id: Option<u64>) -> Result<Reply, McpError> {
        let mut builder = self.http.post(self.url).header("Accept", ACCEPT).json(body);
        for (name, value) in self.headers(conn) {
            builder = builder.header(name, value);
        }
        let mut resp = builder.send().await?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED {
            let challenge = resp.headers().get(WWW_AUTHENTICATE).and_then(|v| v.to_str().ok()).map(str::to_owned);
            return Err(McpError::Unauthorized {
                challenge,
            });
        }
        if status == StatusCode::NOT_FOUND && conn.is_some_and(|c| c.session_id.is_some()) {
            return Err(McpError::SessionGone);
        }
        if !status.is_success() {
            return Err(McpError::Status(status.as_u16()));
        }
        let session_id = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 512 && s.bytes().all(|b| (0x21..=0x7e).contains(&b)))
            .map(str::to_owned);
        let Some(id) = id else {
            // A notification: nothing comes back but the status.
            return Ok(Reply {
                session_id,
                message: None,
            });
        };
        let sse = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.trim_start().to_ascii_lowercase().starts_with("text/event-stream"));
        let mut total = 0usize;
        let message = if sse {
            let mut events = SseReader::default();
            let mut found = None;
            'read: while let Some(chunk) = resp.chunk().await? {
                total += chunk.len();
                if total > MAX_BODY {
                    return Err(McpError::TooLarge);
                }
                for data in events.feed(&chunk) {
                    if let Some(answer) = answer_in(&data, id) {
                        found = Some(answer);
                        break 'read;
                    }
                }
            }
            if found.is_none() {
                found = events.finish().and_then(|data| answer_in(&data, id));
            }
            found
        } else {
            let mut body = Vec::new();
            while let Some(chunk) = resp.chunk().await? {
                total += chunk.len();
                if total > MAX_BODY {
                    return Err(McpError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            std::str::from_utf8(&body).ok().and_then(|s| answer_in(s, id))
        };
        match message {
            Some(message) => Ok(Reply {
                session_id,
                message: Some(message),
            }),
            None => Err(McpError::Protocol("no answer to the request".to_owned())),
        }
    }

    /// Sends one request in `conn` and returns its `result`.
    async fn call(
        &self,
        conn: Option<&Connected>,
        method: &str,
        params: Value,
    ) -> Result<(Value, Option<String>), McpError> {
        let (id, body) = self.request(method, &params);
        let reply = self.post(conn, &body, Some(id)).await?;
        let message = reply.message.ok_or_else(|| McpError::Protocol("no answer".to_owned()))?;
        Ok((result_of(message)?, reply.session_id))
    }

    /// Opens a session: `initialize` with the latest protocol version (the older one when the server does not take
    /// it), then `notifications/initialized`.
    pub async fn initialize(&self) -> Result<Connected, McpError> {
        let mut last = McpError::Protocol("no protocol version in common".to_owned());
        for version in [LATEST, FALLBACK] {
            let params = json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "Reins", "version": env!("CARGO_PKG_VERSION")},
            });
            let (result, session_id) = match self.call(None, "initialize", params).await {
                Ok(answer) => answer,
                Err(
                    e @ (McpError::Rpc {
                        ..
                    }
                    | McpError::Status(400)),
                ) => {
                    last = e;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let agreed = result.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            if agreed != LATEST && agreed != FALLBACK {
                last = McpError::Protocol(format!("unsupported protocol version {}", text::truncate_chars(agreed, 20)));
                continue;
            }
            let conn = Connected {
                session_id,
                protocol: agreed.to_owned(),
            };
            let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
            match self.post(Some(&conn), &note, None).await {
                Ok(_) | Err(McpError::Status(_)) => {}
                Err(e) => return Err(e),
            }
            return Ok(conn);
        }
        Err(last)
    }

    /// Every tool of the server, following the pages.
    pub async fn list_tools(&self, conn: &Connected) -> Result<Vec<McpTool>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = cursor.as_ref().map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let (result, _) = self.call(Some(conn), "tools/list", params).await?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Protocol("tools/list did not return tools".to_owned()))?;
            tools.extend(page.iter().filter_map(parse_tool));
            cursor = result.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
            if cursor.is_none() || tools.len() >= MAX_LISTED_TOOLS {
                break;
            }
        }
        tools.truncate(MAX_LISTED_TOOLS);
        Ok(tools)
    }

    /// Calls one tool; the result is the server's `CallToolResult` as given.
    pub async fn call_tool(
        &self,
        conn: &Connected,
        tool: &str,
        arguments: &Map<String, Value>,
    ) -> Result<Value, McpError> {
        let (result, _) = self.call(Some(conn), "tools/call", json!({"name": tool, "arguments": arguments})).await?;
        if result.is_object() {
            Ok(result)
        } else {
            Err(McpError::Protocol("the tool result is not an object".to_owned()))
        }
    }
}

/// The JSON-RPC answer to request `id` among the messages in `data` (one message or a batch).
fn answer_in(data: &str, id: u64) -> Option<Value> {
    let value: Value = serde_json::from_str(data.trim()).ok()?;
    let mine = |m: &Value| {
        m.get("id").is_some_and(|i| i.as_u64() == Some(id) || i.as_str() == Some(&id.to_string()))
            && (m.get("result").is_some() || m.get("error").is_some())
    };
    match value {
        Value::Array(batch) => batch.into_iter().find(|m| mine(m)),
        single if mine(&single) => Some(single),
        _ => None,
    }
}

/// The `result` of an answer, or its error made safe to show.
pub fn result_of(message: Value) -> Result<Value, McpError> {
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let raw = error.get("message").and_then(Value::as_str).unwrap_or("unknown error");
        return Err(McpError::Rpc {
            code,
            message: text::truncate_chars(&text::one_line(raw), 500),
        });
    }
    match message {
        Value::Object(mut o) => o.remove("result").ok_or_else(|| McpError::Protocol("no result".to_owned())),
        _ => Err(McpError::Protocol("not a JSON-RPC answer".to_owned())),
    }
}

/// One tool of a `tools/list` page. `readOnlyHint` defaults to false and `destructiveHint` to true, as the MCP spec
/// says: a tool that says nothing is a write that is asked for every time.
pub fn parse_tool(t: &Value) -> Option<McpTool> {
    let name = t.get("name").and_then(Value::as_str)?.to_owned();
    let annotations = t.get("annotations").cloned().unwrap_or(Value::Null);
    let title = t
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| annotations.get("title").and_then(Value::as_str))
        .map(str::to_owned);
    let read_only = annotations.get("readOnlyHint").and_then(Value::as_bool).unwrap_or(false);
    let destructive = !read_only && annotations.get("destructiveHint").and_then(Value::as_bool).unwrap_or(true);
    Some(McpTool {
        name,
        title,
        description: t.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
        input_schema: t
            .get("inputSchema")
            .filter(|s| s.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"})),
        read_only,
        destructive,
    })
}

/// Splits a Server-Sent Events stream into the data of its events.
#[derive(Default)]
pub struct SseReader {
    buf: Vec<u8>,
    data: String,
    has_data: bool,
}

impl SseReader {
    /// Feeds bytes; returns the data of every event they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if self.has_data {
                    events.push(std::mem::take(&mut self.data));
                    self.has_data = false;
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) =
                line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
            if field == "data" {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
        }
        events
    }

    /// The stream ended: the data of an event that was not closed by a blank line.
    pub fn finish(&mut self) -> Option<String> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let mut events = self.feed(&rest);
            events.extend(self.feed(b"\n\n"));
            return events.pop();
        }
        self.has_data.then(|| {
            self.has_data = false;
            std::mem::take(&mut self.data)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_are_split_across_chunks_and_lines() {
        let mut r = SseReader::default();
        assert_eq!(r.feed(b"event: message\r\nid: 1\r\ndata: {\"a\"").len(), 0);
        let events = r.feed(b":1}\r\n\r\n: comment\n\ndata: x\ndata: y\n\n");
        assert_eq!(events, ["{\"a\":1}", "x\ny"]);
        assert_eq!(r.feed(b"data: tail").len(), 0);
        assert_eq!(r.finish().as_deref(), Some("tail"));
    }

    #[test]
    fn answers_are_found_by_id_and_errors_are_cleaned() {
        assert_eq!(answer_in(r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#, 3), None);
        let batch = r#"[{"jsonrpc":"2.0","id":2,"result":{}},{"jsonrpc":"2.0","id":3,"result":{"ok":1}}]"#;
        assert_eq!(answer_in(batch, 3).unwrap()["result"]["ok"], 1);
        assert_eq!(answer_in(r#"{"id":"3","result":1}"#, 3).unwrap()["result"], 1);
        let err = result_of(json!({"id": 1, "error": {"code": -32602, "message": "bad\u{202e}\nargs"}})).unwrap_err();
        assert_eq!(
            err,
            McpError::Rpc {
                code: -32602,
                message: "bad args".to_owned()
            }
        );
    }

    #[test]
    fn tool_hints_follow_the_spec_defaults() {
        let plain = parse_tool(&json!({"name": "t", "inputSchema": {"type": "object"}})).unwrap();
        assert!(!plain.read_only && plain.destructive, "no hints: a write asked every time");
        let read = parse_tool(&json!({"name": "r", "annotations": {"readOnlyHint": true, "title": "Read"}})).unwrap();
        assert!(read.read_only && !read.destructive);
        assert_eq!(read.title.as_deref(), Some("Read"));
        assert_eq!(read.input_schema, json!({"type": "object"}));
        let safe = parse_tool(&json!({"name": "w", "annotations": {"destructiveHint": false}})).unwrap();
        assert!(!safe.read_only && !safe.destructive);
        assert!(parse_tool(&json!({"title": "no name"})).is_none());
    }
}
