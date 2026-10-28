//! A mock Reins server for the D1 tests (`ask`, hooks, `mcp`) and the ticket tests: the token endpoint (refresh with
//! rotation), the desktop calls API whose "phone" answers `desktop_ask` from a script with a sealed `AskAnswer` (and
//! git fetches and pushes with a sealed `CredentialGrant`), tickets (deferred calls, collected when approved, the ticket
//! list, stale reports), and a Streamable HTTP `/mcp` endpoint. An app is "logged in" by writing its session file
//! directly.

#![allow(dead_code, reason = "each test binary uses a different part of the mock")]

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use reins_desktop::config::{Paths, write_private};
use reins_desktop::identity::{Identity, seal_to};
use reins_proto::desktop::{AskAnswer, CredentialGrant, GIT_FETCH_OP};
use serde_json::{Value, json};

/// When a deferred call is expected to be decided: the server's default Deferred window.
pub const DEFERRED_SECS: i64 = 14_400;

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
    /// Parked for a later review: `deferred`, with the ticket `rt_<request id>`.
    Deferred,
    /// Approved later, collected now: an `AskAnswer` echoing the question's digest and valid for an hour, or for a
    /// push a fresh `CredentialGrant`.
    Collect,
    /// The ticket ended without an answer (`expired`, `cancelled`...).
    Closed(&'static str),
    /// Like `Collect`, but the approval names another command's digest.
    CollectOtherDigest,
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
    /// The tool of each call, in order.
    pub tools: Vec<String>,
    /// Whether each call asked to be deferred (`X-Reins-Defer: 1`).
    pub defers: Vec<bool>,
    /// `(ticket, diff)` of each stale report.
    pub stale: Vec<(String, String)>,
    /// How often the ticket list was asked for.
    pub listed: u32,
    /// The token a git credential carries.
    pub git_token: String,
    /// Each `pushed` report, in order.
    pub pushed: Vec<String>,
    /// The next N `pushed` reports fail (the server was unreachable).
    pub pushed_fail: u32,
    /// The approval phone's signing key the ticket list names.
    pub signing_key: Option<String>,
    access: HashSet<String>,
    refresh: HashSet<String>,
    sessions: HashSet<String>,
    requests: HashMap<String, MockRequest>,
    counter: u32,
}

struct MockRequest {
    tool: String,
    args: Value,
    steps: Vec<Step>,
    next: usize,
    /// Answered `deferred` once: a ticket, listed from then on.
    expected_by: Option<i64>,
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

    /// Replaces what the phone does next with the ticket `rt_<id>` (its remaining steps).
    pub fn decide(&self, ticket: &str, steps: &[Step]) {
        let id = ticket.trim_start_matches("rt_").to_owned();
        self.with(|s| {
            let r = s.requests.get_mut(&id).expect("a known ticket");
            r.steps = steps.to_vec();
            r.next = 0;
        });
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

/// Logs the app at `paths` in to `mock` (a session file) and pins its key, as a pairing would.
pub fn log_in(mock: &Mock, paths: &Paths) -> Identity {
    paths.ensure().unwrap();
    let identity = Identity::load_or_create(&paths.identity_file()).unwrap();
    let (access, refresh) = mock.issue();
    let session = json!({
        "server": mock.base,
        "client_id": "client-1",
        "token_endpoint": format!("{}/reins/oauth/token", mock.base),
        "access_token": access,
        "access_expires_at": reins_desktop::now_unix() + 3600,
        "refresh_token": refresh,
    });
    write_private(&paths.session_file(), session.to_string().as_bytes()).unwrap();
    mock.with(|s| s.pinned_key = Some(identity.public_key()));
    identity
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
    let defer = header(&req, "x-reins-defer").as_deref() == Some("1");
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
        ("POST", "/reins/desktop/calls") => submit(&mut s, &body, defer),
        ("GET", p) if p.starts_with("/reins/desktop/calls/") => {
            let polled = p["/reins/desktop/calls/".len()..].to_owned();
            s.polls.push(polled.clone());
            let id = polled.trim_start_matches("rt_").to_owned();
            if s.requests.contains_key(&id) {
                answer(&mut s, &id)
            } else {
                json_resp(404, &json!({"error": "not_found"}))
            }
        }
        ("GET", "/reins/desktop/tickets") => {
            s.listed += 1;
            tickets(&s)
        }
        ("POST", p) if p.starts_with("/reins/desktop/tickets/") && p.ends_with("/pushed") => {
            if s.pushed_fail > 0 {
                s.pushed_fail -= 1;
                return json_resp(503, &json!({"error": "unavailable"}));
            }
            let ticket = p["/reins/desktop/tickets/".len()..p.len() - "/pushed".len()].to_owned();
            s.pushed.push(ticket);
            respond(204, "text/plain", "")
        }
        ("POST", p) if p.starts_with("/reins/desktop/tickets/") && p.ends_with("/stale") => {
            let ticket = p["/reins/desktop/tickets/".len()..p.len() - "/stale".len()].to_owned();
            let doc: Value = serde_json::from_slice(&body).unwrap_or_default();
            s.stale.push((ticket, doc["diff"].as_str().unwrap_or_default().to_owned()));
            respond(204, "text/plain", "")
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

fn submit(s: &mut State, body: &[u8], defer: bool) -> Resp {
    let doc: Value = serde_json::from_slice(body).unwrap_or_default();
    let tool = doc["tool"].as_str().unwrap_or_default().to_owned();
    let Some(spec) = reins_proto::connector::spec_for_tool(&tool).filter(|s| s.desktop_only) else {
        return json_resp(400, &json!({"error": "unknown_tool"}));
    };
    let git = tool.contains("_git_");
    if tool != "desktop_ask" && !git {
        return json_resp(400, &json!({"error": "unexpected_tool"}));
    }
    if let Err(message) = spec.parse(&doc["arguments"]) {
        return json_resp(400, &json!({"error": "invalid_arguments", "message": message}));
    }
    s.calls.push(doc["arguments"].clone());
    s.tools.push(tool.clone());
    s.defers.push(defer);
    s.counter += 1;
    let id = format!("req-{}", s.counter);
    let fetch = tool.ends_with(&format!("_{GIT_FETCH_OP}"));
    // Reads are always approved; the plans are for questions and pushes.
    let mut steps = if fetch {
        vec![Step::Approve]
    } else {
        s.plans.pop_front().unwrap_or_else(|| vec![Step::Approve])
    };
    if defer && !matches!(steps.first(), Some(Step::Deferred)) {
        steps.insert(0, Step::Deferred);
    }
    s.requests.insert(
        id.clone(),
        MockRequest {
            tool,
            args: doc["arguments"].clone(),
            steps,
            next: 0,
            expected_by: None,
        },
    );
    answer(s, &id)
}

/// The ticket list: every call answered `deferred` once, in the state its next step would give.
fn tickets(s: &State) -> Resp {
    let mut list: Vec<(&String, &MockRequest)> = s.requests.iter().filter(|(_, r)| r.expected_by.is_some()).collect();
    list.sort_by_key(|(id, _)| (*id).clone());
    let tickets: Vec<Value> = list
        .into_iter()
        .map(|(id, r)| {
            let step = r.steps[r.next.min(r.steps.len() - 1)];
            let (state, note) = match step {
                Step::Collect | Step::CollectOtherDigest | Step::Approve => ("approved", None),
                Step::Denied(reason) => ("denied", reason),
                Step::Closed(state) => (state, None),
                _ => ("deferred", None),
            };
            let mut t = json!({"ticket": format!("rt_{id}"), "state": state, "deadline": r.expected_by});
            if let Some(n) = note {
                t["note"] = json!(n);
            }
            t
        })
        .collect();
    let mut list = json!({"tickets": tickets});
    if let Some(k) = &s.signing_key {
        list["signing_key"] = json!(k);
    }
    json_resp(200, &list)
}

fn answer(s: &mut State, id: &str) -> Resp {
    let pinned = s.pinned_key.clone();
    let git_token = if s.git_token.is_empty() {
        format!("ghs-{id}")
    } else {
        s.git_token.clone()
    };
    let r = s.requests.get_mut(id).unwrap();
    let step = r.steps[r.next.min(r.steps.len() - 1)];
    r.next += 1;
    let ticket = format!("rt_{id}");
    let status = |st: &str| json_resp(200, &json!({"request_id": id, "status": st}));
    let answered =
        |outcome: Value| json_resp(200, &json!({"request_id": id, "status": "answered", "outcome": outcome}));
    let args = &r.args;
    let key = args["client_key"].as_str().unwrap_or_default().to_owned();
    if pinned.as_deref() != Some(key.as_str()) {
        return answered(json!({"outcome": "error", "message": "This must come from the paired desktop app."}));
    }
    let nonce = args["nonce"].as_str().unwrap_or_default().to_owned();
    let now = reins_desktop::now_unix();
    if r.tool.contains("_git_") {
        let fetch = r.tool.ends_with(&format!("_{GIT_FETCH_OP}"));
        let grant = CredentialGrant {
            v: 1,
            nonce,
            digest: args["digest"].as_str().map(str::to_owned),
            repo: args["repo"].as_str().unwrap_or_default().to_owned(),
            access: if fetch {
                "read"
            } else {
                "write"
            }
            .to_owned(),
            username: "x-access-token".to_owned(),
            token: git_token,
            expires_at: now + 600,
        };
        match step {
            Step::Approve | Step::Collect | Step::CollectOtherDigest => {}
            Step::Deferred => {
                let by = *r.expected_by.get_or_insert(now + DEFERRED_SECS);
                return json_resp(
                    200,
                    &json!({"request_id": id, "status": "deferred", "ticket": ticket, "expected_by": by, "state": "deferred"}),
                );
            }
            Step::Closed(state) => {
                return json_resp(
                    200,
                    &json!({"request_id": id, "status": "closed", "ticket": ticket, "state": state}),
                );
            }
            Step::Denied(reason) => return answered(json!({"outcome": "denied", "reason": reason})),
            Step::Error(message) => return answered(json!({"outcome": "error", "message": message})),
            Step::Pending => return status("pending"),
            _ => return status("offline"),
        }
        let sealed = seal_to(&key, &serde_json::to_vec(&grant).unwrap()).unwrap();
        let data = if fetch {
            json!({"items": [{"sealed": sealed}]})
        } else {
            json!({"sealed": sealed})
        };
        return answered(json!({"outcome": "result", "result": {"kind": "connector", "data": data}}));
    }
    let digest = args["digest"].as_str().map(str::to_owned);
    let sealed_for = |key: &str, nonce: String, approved: bool| {
        let a = AskAnswer {
            v: 1,
            nonce,
            approved,
            digest: None,
            expires_at: None,
        };
        seal_to(key, &serde_json::to_vec(&a).unwrap()).unwrap()
    };
    let sealed = match step {
        Step::Pending => return status("pending"),
        Step::Offline => return status("offline"),
        Step::Denied(reason) => return answered(json!({"outcome": "denied", "reason": reason})),
        Step::Error(message) => return answered(json!({"outcome": "error", "message": message})),
        Step::Deferred => {
            let by = *r.expected_by.get_or_insert(now + DEFERRED_SECS);
            return json_resp(
                200,
                &json!({"request_id": id, "status": "deferred", "ticket": ticket, "expected_by": by, "state": "deferred"}),
            );
        }
        Step::Closed(state) => {
            return json_resp(200, &json!({"request_id": id, "status": "closed", "ticket": ticket, "state": state}));
        }
        Step::Collect | Step::CollectOtherDigest => {
            let a = AskAnswer {
                v: 1,
                nonce,
                approved: true,
                digest: if matches!(step, Step::Collect) {
                    digest
                } else {
                    Some("0".repeat(64))
                },
                expires_at: Some(now + 3600),
            };
            seal_to(&key, &serde_json::to_vec(&a).unwrap()).unwrap()
        }
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

/// The approval phone's device signing key, signing policy for this computer (Decisions v2, 08 R1).
pub struct SigningPhone(ring::signature::Ed25519KeyPair);

impl Default for SigningPhone {
    fn default() -> Self {
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Self(ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap())
    }
}

impl SigningPhone {
    pub fn key(&self) -> String {
        use ring::signature::KeyPair as _;
        data_encoding::BASE64URL_NOPAD.encode(self.0.public_key().as_ref())
    }

    /// Signs `m`, seals it to the app at `paths` and hands it over as the server would.
    pub fn send(
        &self,
        paths: &Paths,
        identity: &Identity,
        m: &reins_policy::desktop_policy::PolicyMessage,
    ) -> reins_desktop::policy::Applied {
        use reins_policy::desktop_policy::{PolicyMessage, sign};
        use reins_proto::desktop::{MailedPolicy, PolicyKind, PolicyMail};
        let signed = sign(m, &self.key(), &|b| data_encoding::BASE64URL_NOPAD.encode(self.0.sign(b).as_ref()));
        let item = MailedPolicy {
            version: m.version(),
            kind: match m {
                PolicyMessage::Snapshot(_) => PolicyKind::Snapshot,
                PolicyMessage::Tighten(_) => PolicyKind::Tighten,
            },
            sealed: seal_to(&identity.public_key(), &serde_json::to_vec(&signed).unwrap()).unwrap(),
        };
        let mail = PolicyMail {
            items: vec![item],
            signing_key: Some(self.key()),
        };
        reins_desktop::policy::apply(paths, identity, &mail, reins_desktop::now_unix()).unwrap()
    }

    /// A fresh Auto snapshot for the app at `paths`: dkat Mine, acme Work, the 01 §4 matrix, `roots` as signed.
    pub fn auto_snapshot(
        identity: &Identity,
        version: u64,
        roots: &[&str],
    ) -> reins_policy::desktop_policy::DesktopPolicy {
        use reins_policy::context::{FactKind, Zone};
        use reins_policy::desktop_policy::{Binding, DesktopPolicy, ModeState, PERSONAL, VALIDITY_SECS};
        let mut snapshot = reins_policy::matrix::Snapshot::default();
        snapshot.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        snapshot.facts.set(FactKind::Owner, "acme", Zone::Work);
        let now = reins_desktop::now_unix();
        DesktopPolicy {
            binding: Binding {
                version,
                issued_at: now,
                not_after: now + VALIDITY_SECS,
                client_key: identity.public_key(),
                connection_id: "conn-desktop".to_owned(),
                profile: PERSONAL.to_owned(),
            },
            decides: true,
            mode: ModeState {
                mode: reins_proto::autopilot::Mode::Auto,
                until: None,
            },
            snapshot,
            github_ids: std::collections::BTreeMap::new(),
            roots: roots.iter().map(|r| (*r).to_owned()).collect(),
            scratch: Vec::new(),
        }
    }
}
