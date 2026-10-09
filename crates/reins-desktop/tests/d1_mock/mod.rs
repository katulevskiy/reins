//! A mock Reins server for the D1 tests (`ask`, hooks, `mcp`): the token endpoint (refresh with rotation), the
//! desktop calls API whose "phone" answers `desktop_ask` from a script with a sealed `AskAnswer`, and a Streamable HTTP
//! `/mcp` endpoint. An app is "logged in" by writing its session file directly.

#![allow(dead_code, reason = "each test binary uses a different part of the mock")]

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use reins_desktop::config::{Paths, write_private};
use reins_desktop::identity::{Identity, seal_to};
use reins_proto::desktop::AskAnswer;
use serde_json::{Value, json};

/// What the phone does with one question: the first step answers the call, each poll the next (the last repeats).
#[derive(Clone, Copy, Debug)]
pub enum Step {
    Approve,
    /// A sealed "no".
    Refuse,
    /// Approves, but echoes another nonce.
    ForgeNonce,
    /// Approves, sealed to another key.
    OtherKey,
    /// Answers something that is not a sealed box.
    Garbage,
    Pending,
    Offline,
    Denied(Option<&'static str>),
    Error(&'static str),
}

/// One request to `/mcp` as the server saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub authorization: Option<String>,
    pub via: Option<String>,
    pub session: Option<String>,
    pub protocol: Option<String>,
    pub accept: Option<String>,
    pub body: Value,
}

#[derive(Default)]
pub struct State {
    pub base: String,
    pub pinned_key: Option<String>,
    /// The next N authorized requests get a 401 anyway (the token was revoked).
    pub reject_bearer: u32,
    pub refuse_refresh: bool,
    pub refreshes: u32,
    pub calls: Vec<Value>,
    pub polls: Vec<String>,
    pub plans: VecDeque<Vec<Step>>,
    pub mcp: Vec<Seen>,
    pub deleted_sessions: Vec<String>,
    access: HashSet<String>,
    refresh: HashSet<String>,
    sessions: HashSet<String>,
    requests: HashMap<String, (Value, Vec<Step>, usize)>,
    counter: u32,
}

impl State {
    /// The server forgets every MCP session (a restart).
    pub fn forget_sessions(&mut self) {
        self.sessions.clear();
    }
}

#[derive(Clone)]
pub struct Mock {
    pub base: String,
    pub state: Arc<Mutex<State>>,
}

impl Mock {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let state = Arc::new(Mutex::new(State {
            base: base.clone(),
            ..State::default()
        }));
        let shared = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let state = Arc::clone(&shared);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        let state = Arc::clone(&state);
                        async move { Ok::<_, Infallible>(handle(&state, req).await) }
                    });
                    let _done = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            base,
            state,
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn plan(&self, steps: &[Step]) {
        self.with(|s| s.plans.push_back(steps.to_vec()));
    }

    /// Issues a token pair (as a login would).
    fn issue(&self) -> (String, String) {
        self.with(|s| {
            s.counter += 1;
            let (a, r) = (format!("at-{}", s.counter), format!("rt-{}", s.counter));
            s.access.insert(a.clone());
            s.refresh.insert(r.clone());
            (a, r)
        })
    }
}

/// A logged-in app: state and config directories under a temp dir, a session with the mock, a pinned key.
pub struct App {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
    pub identity: Identity,
}

/// `expires_in`: seconds until the access token in the session expires (negative: already expired).
pub fn logged_in(mock: &Mock, expires_in: i64) -> App {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    quiet(&paths);
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let (access, refresh) = mock.issue();
    let session = json!({
        "server": mock.base,
        "client_id": "client-1",
        "token_endpoint": format!("{}/reins/oauth/token", mock.base),
        "access_token": access,
        "access_expires_at": reins_desktop::now_unix() + expires_in,
        "refresh_token": refresh,
    });
    write_private(&paths.session_file(), session.to_string().as_bytes()).unwrap();
    mock.with(|s| s.pinned_key = Some(identity.public_key()));
    App {
        dir,
        paths,
        identity,
    }
}

/// An app that never logged in.
pub fn logged_out() -> App {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    App {
        dir,
        paths,
        identity,
    }
}

type Resp = hyper::Response<Full<Bytes>>;

fn respond(status: u16, content_type: &str, body: impl Into<Bytes>) -> Resp {
    hyper::Response::builder().status(status).header("Content-Type", content_type).body(Full::new(body.into())).unwrap()
}

fn json_resp(status: u16, v: &Value) -> Resp {
    respond(status, "application/json", v.to_string())
}

fn unauthorized() -> Resp {
    hyper::Response::builder()
        .status(401)
        .header("WWW-Authenticate", "Bearer error=\"invalid_token\"")
        .body(Full::new(Bytes::from_static(b"{\"error\":\"invalid_token\"}")))
        .unwrap()
}

fn header(req: &hyper::Request<hyper::body::Incoming>, name: &str) -> Option<String> {
    req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

async fn handle(state: &Mutex<State>, req: hyper::Request<hyper::body::Incoming>) -> Resp {
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let auth = header(&req, "authorization");
    let via = header(&req, "x-reins-via");
    let session = header(&req, "mcp-session-id");
    let protocol = header(&req, "mcp-protocol-version");
    let accept = header(&req, "accept");
    let body = req.into_body().collect().await.map(http_body_util::Collected::to_bytes).unwrap_or_default();
    let mut s = state.lock().unwrap();
    if path == "/reins/oauth/token" {
        return token(&mut s, &body);
    }
    let ok = auth.as_deref().and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|t| s.access.contains(t));
    if !ok {
        return unauthorized();
    }
    if s.reject_bearer > 0 {
        s.reject_bearer -= 1;
        return unauthorized();
    }
    match (method.as_str(), path.as_str()) {
        ("POST", "/reins/desktop/calls") => submit(&mut s, &body),
        ("GET", p) if p.starts_with("/reins/desktop/calls/") => {
            let id = p["/reins/desktop/calls/".len()..].to_owned();
            s.polls.push(id.clone());
            if s.requests.contains_key(&id) {
                answer(&mut s, &id)
            } else {
                json_resp(404, &json!({"error": "not_found"}))
            }
        }
        ("POST", "/mcp") => {
            let doc: Value = serde_json::from_slice(&body).unwrap_or_default();
            s.mcp.push(Seen {
                method: doc["method"].as_str().unwrap_or_default().to_owned(),
                authorization: auth,
                via,
                session: session.clone(),
                protocol,
                accept,
                body: doc.clone(),
            });
            mcp(&mut s, session.as_deref(), &doc)
        }
        ("DELETE", "/mcp") => {
            if let Some(id) = session {
                s.sessions.remove(&id);
                s.deleted_sessions.push(id);
            }
            respond(200, "text/plain", "")
        }
        _ => json_resp(404, &json!({"error": "not_found"})),
    }
}

fn token(s: &mut State, body: &[u8]) -> Resp {
    let f: HashMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
    if f.get("grant_type").map(String::as_str) != Some("refresh_token")
        || f.get("resource").map(String::as_str) != Some(&format!("{}/mcp", s.base))
    {
        return json_resp(400, &json!({"error": "invalid_request"}));
    }
    if s.refuse_refresh || !s.refresh.remove(f.get("refresh_token").map(String::as_str).unwrap_or_default()) {
        return json_resp(400, &json!({"error": "invalid_grant"}));
    }
    s.refreshes += 1;
    s.counter += 1;
    let (a, r) = (format!("at-{}", s.counter), format!("rt-{}", s.counter));
    s.access.insert(a.clone());
    s.refresh.insert(r.clone());
    json_resp(200, &json!({"access_token": a, "token_type": "Bearer", "expires_in": 3600, "refresh_token": r}))
}

fn submit(s: &mut State, body: &[u8]) -> Resp {
    let doc: Value = serde_json::from_slice(body).unwrap_or_default();
    let tool = doc["tool"].as_str().unwrap_or_default();
    let Some(spec) = reins_proto::connector::spec_for_tool(tool).filter(|s| s.desktop_only) else {
        return json_resp(400, &json!({"error": "unknown_tool"}));
    };
    if tool != "desktop_ask" {
        return json_resp(400, &json!({"error": "unexpected_tool"}));
    }
    if let Err(message) = spec.parse(&doc["arguments"]) {
        return json_resp(400, &json!({"error": "invalid_arguments", "message": message}));
    }
    s.calls.push(doc["arguments"].clone());
    s.counter += 1;
    let id = format!("req-{}", s.counter);
    let steps = s.plans.pop_front().unwrap_or_else(|| vec![Step::Approve]);
    s.requests.insert(id.clone(), (doc["arguments"].clone(), steps, 0));
    answer(s, &id)
}

fn answer(s: &mut State, id: &str) -> Resp {
    let pinned = s.pinned_key.clone();
    let (args, steps, next) = s.requests.get_mut(id).unwrap();
    let step = steps[(*next).min(steps.len() - 1)];
    *next += 1;
    let status = |st: &str| json_resp(200, &json!({"request_id": id, "status": st}));
    let answered =
        |outcome: Value| json_resp(200, &json!({"request_id": id, "status": "answered", "outcome": outcome}));
    let key = args["client_key"].as_str().unwrap_or_default().to_owned();
    if pinned.as_deref() != Some(key.as_str()) {
        return answered(json!({"outcome": "error", "message": "This must come from the paired desktop app."}));
    }
    let nonce = args["nonce"].as_str().unwrap_or_default().to_owned();
    let sealed_for = |key: &str, nonce: String, approved: bool| {
        let a = AskAnswer {
            v: 1,
            nonce,
            approved,
        };
        seal_to(key, &serde_json::to_vec(&a).unwrap()).unwrap()
    };
    let sealed = match step {
        Step::Pending => return status("pending"),
        Step::Offline => return status("offline"),
        Step::Denied(reason) => return answered(json!({"outcome": "denied", "reason": reason})),
        Step::Error(message) => return answered(json!({"outcome": "error", "message": message})),
        Step::Garbage => "%% not sealed %%".to_owned(),
        Step::Approve => sealed_for(&key, nonce, true),
        Step::Refuse => sealed_for(&key, nonce, false),
        Step::ForgeNonce => sealed_for(&key, "another-nonce".to_owned(), true),
        Step::OtherKey => sealed_for(&Identity::generate().public_key(), nonce, true),
    };
    answered(json!({"outcome": "result", "result": {"kind": "connector", "data": {"sealed": sealed}}}))
}

fn mcp(s: &mut State, session: Option<&str>, doc: &Value) -> Resp {
    let id = doc.get("id").cloned();
    let method = doc["method"].as_str().unwrap_or_default();
    if method == "initialize" {
        s.counter += 1;
        let sid = format!("s-{}", s.counter);
        s.sessions.insert(sid.clone());
        let body = json!({"jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "reins", "version": "test"},
        }});
        return hyper::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .header("Mcp-Session-Id", sid)
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap();
    }
    match session {
        None => return json_resp(400, &json!({"error": "no session"})),
        Some(sid) if !s.sessions.contains(sid) => return json_resp(404, &json!({"error": "unknown session"})),
        Some(_) => {}
    }
    let Some(id) = id.filter(|_| !method.is_empty()) else {
        // A notification or a response.
        return respond(202, "text/plain", "");
    };
    match method {
        "tools/list" => {
            let note = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "listing"}});
            let result = json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [
                {"name": "echo", "description": "Echoes", "inputSchema": {"type": "object"}},
            ]}});
            respond(200, "text/event-stream", format!("event: message\ndata: {note}\n\nid: 2\ndata: {result}\n\n"))
        }
        "tools/call" => json_resp(
            200,
            &json!({"jsonrpc": "2.0", "id": id, "result": {
                "content": [{"type": "text", "text": format!("echo {}", doc["params"]["arguments"])}],
                "isError": false,
            }}),
        ),
        "ping" => json_resp(200, &json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        _ => json_resp(
            200,
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}),
        ),
    }
}

/// No "check your phone" notifications from tests (they would pop up on the desktop running them).
pub fn quiet(paths: &Paths) {
    paths.ensure().unwrap();
    std::fs::write(paths.config_file(), "[notify]\nphone = false\n").unwrap();
}
