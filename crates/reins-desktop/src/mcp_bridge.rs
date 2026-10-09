//! `reins mcp [--via NAME]`: a stdio MCP server for harnesses that start local servers, bridging each JSON-RPC
//! message to the Reins server's MCP endpoint (`<server>/mcp`, Streamable HTTP) with this app's session. The access
//! token is renewed when it expires or is refused (401) and never leaves this process; `X-Reins-Via` names the
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
use crate::journal::{Entry, Journal, Kind, Outcome};
use crate::server::LinkError;
use crate::server::oauth::{Access, SessionTokens};

/// The largest message passed on (either way).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
/// A tool call may wait for the phone; the server answers before this.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(15);
const MAX_VIA: usize = 40;
/// JSON-RPC error code for "the bridge could not get an answer".
const BRIDGE_ERROR: i64 = -32000;
/// A tool call unanswered this long is waiting for the phone: the harness gets a progress notification (when it asked
/// for them), then one every [`PROGRESS_EVERY`].
const PROGRESS_AFTER: Duration = Duration::from_secs(2);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// Unanswered this long, the user is most likely asked: the activity log shows it waiting and the desktop says "check
/// your phone". (Calls the phone runs on its own, under a grant or Autopilot, usually finish well before.)
const NOTIFY_AFTER: Duration = Duration::from_secs(5);
/// What the progress notifications say.
pub const WAITING_MESSAGE: &str = "Waiting for Reins 2FA on your phone…";

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
    journal: Journal,
    /// This app's key: card details of a purchase are sealed to it ([`crate::mcp_payments`]).
    identity: Option<crate::identity::Identity>,
}

/// A `tools/call` request: the tool, the progress token the client gave (if any), the argument names.
struct ToolCall {
    name: String,
    progress_token: Option<Value>,
    arguments: Vec<String>,
}

fn tool_call(msg: &Value) -> Option<ToolCall> {
    if msg.get("method").and_then(Value::as_str) != Some("tools/call") || msg.get("id").is_none() {
        return None;
    }
    let params = msg.get("params")?;
    Some(ToolCall {
        name: params.get("name").and_then(Value::as_str).unwrap_or("a tool").to_owned(),
        progress_token: params.pointer("/_meta/progressToken").filter(|t| t.is_string() || t.is_number()).cloned(),
        arguments: params
            .get("arguments")
            .and_then(Value::as_object)
            .map(|a| a.keys().cloned().collect())
            .unwrap_or_default(),
    })
}

/// How a tool call ended, from the server's answer: its texts for a denial and for "still waiting" (`reins_get_result`
/// is offered when the phone did not answer in time, or is offline).
#[must_use]
pub fn tool_outcome(response: Option<&Value>) -> (Outcome, Option<String>) {
    let Some(r) = response else {
        return (Outcome::Failed, Some("No answer from the Reins server.".to_owned()));
    };
    if let Some(message) = r.pointer("/error/message").and_then(Value::as_str) {
        return (Outcome::Failed, Some(message.to_owned()));
    }
    let text: String = r
        .pointer("/result/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let error = r.pointer("/result/isError").and_then(Value::as_bool).unwrap_or(false);
    if !error {
        return (Outcome::Approved, None);
    }
    let line = text.lines().next().unwrap_or_default().to_owned();
    if text.starts_with("Denied") {
        (Outcome::Denied, Some(line))
    } else if text.contains("reins_get_result") {
        (Outcome::TimedOut, Some(line))
    } else {
        (Outcome::Failed, Some(line))
    }
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
            LinkError::NotFound => Self::Other("the Reins server does not know this request".to_owned()),
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
            req = req.header("X-Reins-Via", v);
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
            log::info!("the Reins server refused the access token; renewing it");
            let renewed = self.tokens.after_rejection(&access.token).await?;
            resp = self.post(&renewed, body).await?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(Failure::LoggedOut(format!(
                    "{} does not accept this app's session; run `reins login {}` again",
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
                "the Reins server answered {}",
                crate::server::error_text(status, &body)
            )));
        }
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *lock(&self.session) = Some(s.to_owned());
        }
        Ok(resp)
    }

    /// Passes the messages of the answer to `out` as they come (JSON, or an event stream), each through `fix`.
    async fn relay(
        resp: reqwest::Response,
        out: &mpsc::UnboundedSender<Value>,
        fix: impl Fn(&mut Value),
    ) -> Result<Vec<Value>, Failure> {
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
            for mut v in items {
                fix(&mut v);
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
                .map_err(|_| Failure::Other("the Reins server's answer is not JSON".to_owned()))?;
            emit(v);
        }
        Ok(responses)
    }

    /// Starts a new MCP session with the client's `initialize` (the server forgot the old one).
    async fn reinitialize(&self) -> Result<(), Failure> {
        *lock(&self.session) = None;
        let Some(init) = lock(&self.initialize).clone() else {
            return Err(Failure::Other("the Reins server ended the MCP session".to_owned()));
        };
        let resp = self.send(&init).await?;
        drop(resp);
        let resp = self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await?;
        drop(resp);
        Ok(())
    }

    /// Forwards one message; a tool call is also logged, and while it waits for the phone the harness gets progress
    /// notifications and the desktop says "check your phone".
    async fn forward(&self, msg: Value, out: &mpsc::UnboundedSender<Value>) {
        let Some(call) = tool_call(&msg) else {
            self.forward_plain(msg, out).await;
            return;
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let who = self.via.clone().unwrap_or_else(|| "An AI agent".to_owned());
        let mut entry = Entry::new(Kind::Mcp, &format!("{who}: {}", call.name))
            .source(self.via.as_deref())
            .service(Some(&call.name))
            .detail(
                (!call.arguments.is_empty()).then(|| format!("Arguments: {}", call.arguments.join(", "))).as_deref(),
            );
        // Answers pass through here so the call's own response can be found.
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let watch = {
            let out = out.clone();
            let journal = self.journal.clone();
            let entry = entry.clone();
            let token = call.progress_token.clone();
            tokio::spawn(async move {
                let started = tokio::time::Instant::now();
                tokio::time::sleep(PROGRESS_AFTER).await;
                let mut told = false;
                loop {
                    if let Some(token) = &token {
                        let _closed =
                            out.send(json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {
                                "progressToken": token,
                                "progress": started.elapsed().as_secs(),
                                "message": WAITING_MESSAGE,
                            }}));
                    }
                    if !told && started.elapsed() >= NOTIFY_AFTER {
                        told = true;
                        journal.record(&entry);
                        journal.notify_waiting(&entry.what);
                        eprintln!("reins mcp: {} {}", WAITING_MESSAGE.trim_end_matches('…'), entry.what);
                    }
                    let next = if told {
                        PROGRESS_EVERY
                    } else {
                        NOTIFY_AFTER.saturating_sub(started.elapsed())
                    };
                    tokio::time::sleep(next.max(Duration::from_millis(100))).await;
                }
            })
        };
        let passing = {
            let out = out.clone();
            let id = id.clone();
            tokio::spawn(async move {
                let mut own = None;
                while let Some(v) = rx.recv().await {
                    if v.get("id") == Some(&id) && v.get("method").is_none() {
                        own = Some(v.clone());
                    }
                    let _closed = out.send(v);
                }
                own
            })
        };
        self.forward_plain(msg, &tx).await;
        drop(tx);
        watch.abort();
        let own = passing.await.ok().flatten();
        let (outcome, reason) = tool_outcome(own.as_ref());
        entry.end(outcome, reason.as_deref());
        self.journal.record(&entry);
    }

    /// Forwards one message from the harness; writes the answers (or an error for each request) to `out`.
    async fn forward_plain(&self, mut msg: Value, out: &mpsc::UnboundedSender<Value>) {
        let ids = request_ids(&msg);
        let nonce = self.identity.as_ref().and_then(|i| crate::mcp_payments::seal_request(&mut msg, i));
        let is_initialize = msg.get("method").and_then(Value::as_str) == Some("initialize");
        if is_initialize {
            *lock(&self.initialize) = Some(msg.clone());
            *lock(&self.session) = None;
        }
        let mut result = self.send(&msg).await;
        if matches!(result, Err(Failure::SessionGone)) {
            log::info!("the Reins server forgot the MCP session; starting a new one");
            result = match self.reinitialize().await {
                Ok(()) => self.send(&msg).await,
                Err(e) => Err(e),
            };
        }
        let fix = |v: &mut Value| {
            if let Some(identity) = &self.identity {
                crate::mcp_payments::open_answer(v, identity, nonce.as_deref());
            }
        };
        let outcome = match result {
            Ok(resp) => Self::relay(resp, out, fix).await,
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
                    Failure::LoggedOut(m) => format!("Reins: {m} (in a terminal)."),
                    Failure::SessionGone => {
                        "Reins: the server ended the MCP session; restart this MCP server.".to_owned()
                    }
                    Failure::Other(m) => format!("Reins: {m}"),
                };
                if ids.is_empty() {
                    eprintln!("reins mcp: {message}");
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

/// Bridges `input` (the harness's messages) to the Reins server and its answers to `output`, until `input` ends.
pub async fn run<R, W>(paths: &Paths, via: Option<&str>, input: R, output: W) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let http = crate::http::client(None)?;
    if crate::server::oauth::logged_in_server(paths).is_none() {
        eprintln!("reins mcp: not logged in to a Reins server; run `reins login` in a terminal.");
    }
    let bridge = Arc::new(Bridge {
        tokens: SessionTokens::new(paths, http.clone()),
        http,
        via: via.and_then(sanitize_via),
        session: Mutex::new(None),
        protocol: Mutex::new(None),
        initialize: Mutex::new(None),
        journal: Journal::new(paths),
        identity: crate::identity::Identity::load_or_create(&paths.identity_file()).ok(),
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
                eprintln!("reins mcp: reading stdin: {e}");
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
    fn tool_calls_are_recognised_with_their_progress_token() {
        let call = tool_call(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "gmail_send", "arguments": {"to": "a@b", "body": "x"}, "_meta": {"progressToken": "p1"}}}))
        .unwrap();
        assert_eq!(call.name, "gmail_send");
        assert_eq!(call.progress_token, Some(json!("p1")));
        assert_eq!(call.arguments.len(), 2);
        assert!(tool_call(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).is_none());
        assert!(tool_call(&json!({"jsonrpc": "2.0", "method": "tools/call", "params": {}})).is_none());
    }

    #[test]
    fn the_servers_answers_say_how_a_tool_call_ended() {
        let reply = |error: bool, text: &str| json!({"jsonrpc": "2.0", "id": 1, "result": {"isError": error, "content": [{"type": "text", "text": text}]}});
        assert_eq!(tool_outcome(Some(&reply(false, "sent"))).0, Outcome::Approved);
        assert_eq!(tool_outcome(Some(&reply(true, "Denied by the user on their Reins device."))).0, Outcome::Denied);
        let waiting = "Waiting for the user to approve on their phone. When they confirm, call reins_get_result \
                       with request_id=r1.";
        assert_eq!(tool_outcome(Some(&reply(true, waiting))).0, Outcome::TimedOut);
        assert_eq!(tool_outcome(Some(&reply(true, "Gmail said no"))).0, Outcome::Failed);
        assert_eq!(tool_outcome(Some(&json!({"id": 1, "error": {"code": -1, "message": "x"}}))).0, Outcome::Failed);
        assert_eq!(tool_outcome(None).0, Outcome::Failed);
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
