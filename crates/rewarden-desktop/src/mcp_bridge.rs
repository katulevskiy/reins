//! `rewarden mcp [--via NAME]`: a stdio MCP server for harnesses that start local servers, bridging each JSON-RPC
//! message to the Rewarden server's MCP endpoint (`<server>/mcp`, Streamable HTTP) with this app's session. The access
//! token is renewed when it expires or is refused (401) and never leaves this process; `X-Rewarden-Via` names the
//! harness so the phone shows who asks ("Laptop · Claude Code").
//!
//! Messages are newline-delimited JSON on stdin and stdout. Requests run concurrently (a tool call waiting for the
//! phone does not hold up a ping); answers given as an event stream are passed on message by message; notifications
//! and responses go through as they are. Errors come back as JSON-RPC errors that say what to do.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::mpsc;

use crate::config::Paths;
use crate::server::LinkError;
use crate::server::oauth::{Access, SessionTokens};

/// The largest message passed on (either way).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
/// A tool call may wait for the phone; the server answers before this.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(15);
const MAX_VIA: usize = 40;
/// JSON-RPC error code for "the bridge could not get an answer".
const BRIDGE_ERROR: i64 = -32000;

/// `NAME` as the server takes it: printable ASCII, at most 40 characters; `None` when nothing is left.
#[must_use]
pub fn sanitize_via(name: &str) -> Option<String> {
    let s: String = name.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).take(MAX_VIA).collect();
    let s = s.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

struct Bridge {
    http: reqwest::Client,
    tokens: SessionTokens,
    via: Option<String>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    /// The client's `initialize`, replayed when the server forgets the session.
    initialize: Mutex<Option<Value>>,
}

enum Failure {
    /// Not logged in, or the server ended the session.
    LoggedOut(String),
    /// The server forgot the MCP session (404 with a session id).
    SessionGone,
    Other(String),
}

impl From<LinkError> for Failure {
    fn from(e: LinkError) -> Self {
        match e {
            LinkError::LoggedOut(m) => Self::LoggedOut(m),
            LinkError::NotFound => Self::Other("the Rewarden server does not know this request".to_owned()),
            LinkError::Failed(m) => Self::Other(m),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The ids of the requests in a message (one, or several in a batch).
fn request_ids(msg: &Value) -> Vec<Value> {
    let one = |m: &Value| m.get("method").and(m.get("id")).cloned();
    match msg {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        m => one(m).into_iter().collect(),
    }
}

fn error_for(id: &Value, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": BRIDGE_ERROR, "message": message}})
}

impl Bridge {
    async fn post(&self, access: &Access, body: &Value) -> Result<reqwest::Response, Failure> {
        let mut req = self
            .http
            .post(format!("{}/mcp", access.server))
            .bearer_auth(access.token.as_str())
            .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
            .json(body);
        if let Some(v) = &self.via {
            req = req.header("X-Rewarden-Via", v);
        }
        if let Some(s) = lock(&self.session).clone() {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(p) = lock(&self.protocol).clone() {
            req = req.header("MCP-Protocol-Version", p);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, req.send()).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(Failure::Other(format!("cannot reach {}: {}", access.server, e.without_url()))),
            Err(_) => Err(Failure::Other(format!("{} did not answer in time", access.server))),
        }
    }

    /// Sends `body`, renewing the token once on 401; the server's answer (status checked).
    async fn send(&self, body: &Value) -> Result<reqwest::Response, Failure> {
        let access = self.tokens.access().await?;
        let mut resp = self.post(&access, body).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            log::info!("the Rewarden server refused the access token; renewing it");
            let renewed = self.tokens.after_rejection(&access.token).await?;
            resp = self.post(&renewed, body).await?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(Failure::LoggedOut(format!(
                    "{} does not accept this app's session; run `rewarden login {}` again",
                    renewed.server, renewed.server
                )));
            }
        }
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND && lock(&self.session).is_some() {
            return Err(Failure::SessionGone);
        }
        if !status.is_success() {
            let body = crate::server::read_limited(resp, 64 * 1024).await.unwrap_or_default();
            return Err(Failure::Other(format!(
                "the Rewarden server answered {}",
                crate::server::error_text(status, &body)
            )));
        }
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *lock(&self.session) = Some(s.to_owned());
        }
        Ok(resp)
    }

    /// Passes the messages of the answer to `out` as they come (JSON, or an event stream).
    async fn relay(resp: reqwest::Response, out: &mpsc::UnboundedSender<Value>) -> Result<Vec<Value>, Failure> {
        let mut responses = Vec::new();
        let sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.trim_start().starts_with("text/event-stream"));
        if resp.status() == reqwest::StatusCode::ACCEPTED || resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(responses);
        }
        let mut emit = |v: Value| {
            let items = match v {
                Value::Array(items) => items,
                v => vec![v],
            };
            for v in items {
                // A copy of the responses for the caller (it looks at `initialize`'s).
                if v.get("id").is_some() && v.get("method").is_none() {
                    responses.push(v.clone());
                }
                let _closed = out.send(v);
            }
        };
        if sse {
            let mut stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut data = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| Failure::Other(format!("the answer broke off: {}", e.without_url())))?;
                buf.extend_from_slice(&chunk);
                if buf.len() > MAX_MESSAGE {
                    return Err(Failure::Other("the server's answer is too large".to_owned()));
                }
                while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if line.is_empty() {
                        if !data.is_empty() {
                            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                                emit(v);
                            }
                            data.clear();
                        }
                    } else if let Some(d) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(d.strip_prefix(' ').unwrap_or(d));
                    }
                }
            }
            if !data.is_empty()
                && let Ok(v) = serde_json::from_str::<Value>(&data)
            {
                emit(v);
            }
        } else {
            let body = crate::server::read_limited(resp, MAX_MESSAGE).await.map_err(Failure::Other)?;
            if body.iter().all(u8::is_ascii_whitespace) {
                return Ok(responses);
            }
            let v: Value = serde_json::from_slice(&body)
                .map_err(|_| Failure::Other("the Rewarden server's answer is not JSON".to_owned()))?;
            emit(v);
        }
        Ok(responses)
    }

    /// Starts a new MCP session with the client's `initialize` (the server forgot the old one).
    async fn reinitialize(&self) -> Result<(), Failure> {
        *lock(&self.session) = None;
        let Some(init) = lock(&self.initialize).clone() else {
            return Err(Failure::Other("the Rewarden server ended the MCP session".to_owned()));
        };
        let resp = self.send(&init).await?;
        drop(resp);
        let resp = self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await?;
        drop(resp);
        Ok(())
    }

    /// Forwards one message from the harness; writes the answers (or an error for each request) to `out`.
    async fn forward(&self, msg: Value, out: &mpsc::UnboundedSender<Value>) {
        let ids = request_ids(&msg);
        let is_initialize = msg.get("method").and_then(Value::as_str) == Some("initialize");
        if is_initialize {
            *lock(&self.initialize) = Some(msg.clone());
            *lock(&self.session) = None;
        }
        let mut result = self.send(&msg).await;
        if matches!(result, Err(Failure::SessionGone)) {
            log::info!("the Rewarden server forgot the MCP session; starting a new one");
            result = match self.reinitialize().await {
                Ok(()) => self.send(&msg).await,
                Err(e) => Err(e),
            };
        }
        let outcome = match result {
            Ok(resp) => Self::relay(resp, out).await,
            Err(e) => Err(e),
        };
        match outcome {
            Ok(responses) => {
                if is_initialize
                    && let Some(v) =
                        responses.iter().find_map(|r| r.pointer("/result/protocolVersion")).and_then(Value::as_str)
                {
                    *lock(&self.protocol) = Some(v.to_owned());
                }
            }
            Err(e) => {
                let message = match e {
                    Failure::LoggedOut(m) => format!("Rewarden: {m} (in a terminal)."),
                    Failure::SessionGone => {
                        "Rewarden: the server ended the MCP session; restart this MCP server.".to_owned()
                    }
                    Failure::Other(m) => format!("Rewarden: {m}"),
                };
                if ids.is_empty() {
                    eprintln!("rewarden mcp: {message}");
                }
                for id in ids {
                    let _closed = out.send(error_for(&id, &message));
                }
            }
        }
    }

    async fn end_session(&self) {
        let Some(session) = lock(&self.session).clone() else {
            return;
        };
        let Ok(access) = self.tokens.access().await else {
            return;
        };
        let req = self
            .http
            .delete(format!("{}/mcp", access.server))
            .bearer_auth(access.token.as_str())
            .header("Mcp-Session-Id", session);
        let _ignored = tokio::time::timeout(Duration::from_secs(5), req.send()).await;
    }
}

/// Bridges `input` (the harness's messages) to the Rewarden server and its answers to `output`, until `input` ends.
pub async fn run<R, W>(paths: &Paths, via: Option<&str>, input: R, output: W) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let http = crate::http::client(None)?;
    if crate::server::oauth::logged_in_server(paths).is_none() {
        eprintln!("rewarden mcp: not logged in to a Rewarden server; run `rewarden login <server>` in a terminal.");
    }
    let bridge = Arc::new(Bridge {
        tokens: SessionTokens::new(paths, http.clone()),
        http,
        via: via.and_then(sanitize_via),
        session: Mutex::new(None),
        protocol: Mutex::new(None),
        initialize: Mutex::new(None),
    });
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(v) = rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if output.write_all(line.as_bytes()).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    let mut tasks = tokio::task::JoinSet::new();
    let mut lines = input.lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(e) => {
                eprintln!("rewarden mcp: reading stdin: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_MESSAGE {
            let _closed = tx
                .send(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "message too large"}}));
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _closed = tx.send(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                continue;
            }
        };
        let bridge = Arc::clone(&bridge);
        let tx = tx.clone();
        // `initialize` first and alone: what follows needs its session.
        if msg.get("method").and_then(Value::as_str) == Some("initialize") {
            while tasks.join_next().await.is_some() {}
            bridge.forward(msg, &tx).await;
        } else {
            tasks.spawn(async move { bridge.forward(msg, &tx).await });
        }
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    bridge.end_session().await;
    drop(tx);
    writer.await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn via_names_are_cut_and_cleaned() {
        assert_eq!(sanitize_via("Claude Code").as_deref(), Some("Claude Code"));
        assert_eq!(sanitize_via(" a\u{7}b\nc é ").as_deref(), Some("abc"));
        assert_eq!(sanitize_via(&"x".repeat(50)).unwrap().len(), 40);
        assert_eq!(sanitize_via("\u{1}"), None);
    }

    #[test]
    fn request_ids_skip_notifications_and_responses() {
        assert_eq!(request_ids(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})), vec![json!(1)]);
        assert!(request_ids(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_empty());
        assert!(request_ids(&json!({"jsonrpc": "2.0", "id": 1, "result": {}})).is_empty());
        assert_eq!(
            request_ids(&json!([{"id": "a", "method": "x"}, {"method": "n"}, {"id": "b", "method": "y"}])),
            vec![json!("a"), json!("b")]
        );
    }
}
