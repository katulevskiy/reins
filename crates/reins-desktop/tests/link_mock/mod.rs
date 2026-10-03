//! A mock Reins server for the Reins link tests: OAuth discovery, dynamic registration, authorize (redirecting a
//! scripted browser to the app's loopback callback), token (code with PKCE, refresh with rotation), and the desktop
//! calls API, whose "phone" answers from a script and seals grants with `identity::seal_to`.

#![allow(dead_code, reason = "each test binary uses a different part of the mock")]

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use reins_desktop::config::{Config, Paths};
use reins_desktop::identity::{Identity, seal_to};
use reins_proto::desktop::{
    CredentialGrant, FETCH_LEASE_SECS, GIT_FETCH_OP, PUSH_LEASE_SECS, PushSummary, RefChange, RefUpdate,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// What the phone does with one request: the first step answers the call, each poll takes the next (the last repeats).
#[derive(Clone, Copy)]
pub enum Step {
    Approve,
    /// Approves with a grant changed by the function (a wrong nonce, an old expiry...).
    Tamper(fn(&mut CredentialGrant)),
    /// Approves, sealed to a key that is not the app's.
    OtherKey,
    /// Answers something that is not a sealed box.
    Garbage,
    Pending,
    Offline,
    Denied(Option<&'static str>),
    Error(&'static str),
}

pub struct Call {
    pub tool: String,
    pub arguments: Value,
    pub account: Value,
}

struct Request {
    tool: String,
    arguments: Value,
    steps: Vec<Step>,
    next: usize,
}

struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
}

#[derive(Default)]
pub struct State {
    pub base: String,
    /// `expires_in` of issued access tokens.
    pub expires_in: i64,
    /// The refresh grant is refused (`invalid_grant`).
    pub refuse_refresh: bool,
    /// The next desktop API request gets a 401 even with a good token.
    pub reject_bearer_once: bool,
    /// The key the phone pinned at pairing (from `reins_client_key`).
    pub pinned_key: Option<String>,
    pub client_names: Vec<String>,
    pub authorize_params: Option<HashMap<String, String>>,
    pub refreshes: u32,
    pub calls: Vec<Call>,
    pub polls: Vec<String>,
    /// Scripts for the next calls, in order; `[Approve]` when empty.
    pub plans: VecDeque<Vec<Step>>,
    clients: HashMap<String, Vec<String>>,
    codes: HashMap<String, Code>,
    access: HashSet<String>,
    refresh: HashMap<String, String>,
    requests: HashMap<String, Request>,
    counter: u32,
    request_seq: u32,
}

impl State {
    /// The server forgets every request (they expired).
    pub fn forget_requests(&mut self) {
        self.requests.clear();
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
            expires_in: 3600,
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
}

type Resp = hyper::Response<Full<Bytes>>;

fn respond(status: u16, content_type: &str, body: impl Into<Bytes>) -> Resp {
    hyper::Response::builder().status(status).header("Content-Type", content_type).body(Full::new(body.into())).unwrap()
}

fn json_resp(status: u16, v: &Value) -> Resp {
    respond(status, "application/json", v.to_string())
}

fn form(body: &[u8]) -> HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

async fn handle(state: &Mutex<State>, req: hyper::Request<hyper::body::Incoming>) -> Resp {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or_default().to_owned();
    let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let body = req.into_body().collect().await.map(http_body_util::Collected::to_bytes).unwrap_or_default();
    let mut s = state.lock().unwrap();
    let base = s.base.clone();
    match (method.as_str(), path.as_str()) {
        ("GET", "/.well-known/oauth-authorization-server") => json_resp(
            200,
            &json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/reins/oauth/authorize"),
                "token_endpoint": format!("{base}/reins/oauth/token"),
                "registration_endpoint": format!("{base}/reins/oauth/register"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
            }),
        ),
        ("POST", "/reins/oauth/register") => register(&mut s, &body),
        ("GET", "/reins/oauth/authorize") => authorize(&mut s, &form(query.as_bytes())),
        ("POST", "/reins/oauth/token") => token(&mut s, &form(&body)),
        ("POST", "/reins/desktop/calls") => match bearer(&mut s, auth.as_deref()) {
            Some(denied) => denied,
            None => submit(&mut s, &body),
        },
        ("GET", p) if p.starts_with("/reins/desktop/calls/") => match bearer(&mut s, auth.as_deref()) {
            Some(denied) => denied,
            None => poll(&mut s, &p["/reins/desktop/calls/".len()..]),
        },
        _ => json_resp(404, &json!({"error": "not_found"})),
    }
}

fn register(s: &mut State, body: &[u8]) -> Resp {
    let doc: Value = serde_json::from_slice(body).unwrap_or_default();
    let name = doc["client_name"].as_str().unwrap_or_default().to_owned();
    if doc["redirect_uris"] != json!(["http://127.0.0.1/callback"])
        || doc["token_endpoint_auth_method"] != "none"
        || doc["grant_types"]
            != json!(["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:device_code"])
        || !name.starts_with("Reins desktop app on ")
    {
        return json_resp(400, &json!({"error": "invalid_client_metadata", "error_description": doc.to_string()}));
    }
    s.counter += 1;
    let id = format!("client-{}", s.counter);
    s.clients.insert(id.clone(), vec!["http://127.0.0.1/callback".to_owned()]);
    s.client_names.push(name);
    json_resp(201, &json!({"client_id": id, "redirect_uris": ["http://127.0.0.1/callback"]}))
}

/// Like the real server: the registered loopback redirect matches whatever port the app listens on.
fn loopback_redirect_ok(registered: &[String], requested: &str) -> bool {
    let Ok(want) = url::Url::parse(requested) else {
        return false;
    };
    registered.iter().any(|r| {
        let have = url::Url::parse(r).unwrap();
        have.scheme() == want.scheme() && have.host() == want.host() && have.path() == want.path()
    })
}

fn authorize(s: &mut State, p: &HashMap<String, String>) -> Resp {
    s.authorize_params = Some(p.clone());
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or_default();
    let Some(registered) = s.clients.get(get("client_id")) else {
        return respond(400, "text/plain", "unknown client");
    };
    let checks = [
        ("redirect_uri", loopback_redirect_ok(registered, get("redirect_uri"))),
        ("response_type", get("response_type") == "code"),
        ("code_challenge_method", get("code_challenge_method") == "S256"),
        ("code_challenge", get("code_challenge").len() == 43),
        ("state", get("state").len() >= 16),
        ("resource", get("resource") == format!("{}/mcp", s.base)),
        ("reins_client_key", reins_proto::desktop::decode_key(get("reins_client_key")).is_some()),
    ];
    if let Some((bad, _)) = checks.iter().find(|(_, ok)| !ok) {
        return respond(400, "text/plain", format!("bad {bad}"));
    }
    // The phone approves the pairing and pins the key.
    s.pinned_key = Some(get("reins_client_key").to_owned());
    s.counter += 1;
    let code = format!("code-{}", s.counter);
    s.codes.insert(
        code.clone(),
        Code {
            client_id: get("client_id").to_owned(),
            redirect_uri: get("redirect_uri").to_owned(),
            challenge: get("code_challenge").to_owned(),
        },
    );
    let mut to = url::Url::parse(get("redirect_uri")).unwrap();
    to.query_pairs_mut().append_pair("code", &code).append_pair("state", get("state")).append_pair("iss", &s.base);
    hyper::Response::builder().status(302).header("Location", to.as_str()).body(Full::new(Bytes::new())).unwrap()
}

fn issue(s: &mut State, client_id: &str) -> Resp {
    s.counter += 1;
    let access = format!("at-{}", s.counter);
    let refresh = format!("rt-{}", s.counter);
    s.access.insert(access.clone());
    s.refresh.insert(refresh.clone(), client_id.to_owned());
    json_resp(
        200,
        &json!({"access_token": access, "token_type": "Bearer", "expires_in": s.expires_in, "refresh_token": refresh, "scope": "mcp"}),
    )
}

fn token(s: &mut State, f: &HashMap<String, String>) -> Resp {
    let get = |k: &str| f.get(k).map(String::as_str).unwrap_or_default();
    let invalid = |d: &str| json_resp(400, &json!({"error": "invalid_grant", "error_description": d}));
    if get("resource") != format!("{}/mcp", s.base) {
        return json_resp(400, &json!({"error": "invalid_target"}));
    }
    match get("grant_type") {
        "authorization_code" => {
            let Some(code) = s.codes.remove(get("code")) else {
                return invalid("unknown code");
            };
            let challenge = BASE64URL_NOPAD.encode(&Sha256::digest(get("code_verifier").as_bytes()));
            if code.client_id != get("client_id")
                || code.redirect_uri != get("redirect_uri")
                || code.challenge != challenge
            {
                return invalid("PKCE or client mismatch");
            }
            issue(s, &code.client_id)
        }
        "refresh_token" => {
            if s.refuse_refresh {
                return invalid("the refresh token is invalid or expired");
            }
            // Rotation: a refresh token works once.
            let Some(client) = s.refresh.remove(get("refresh_token")) else {
                return invalid("used refresh token");
            };
            if client != get("client_id") {
                return invalid("another client");
            }
            s.refreshes += 1;
            issue(s, &client)
        }
        _ => json_resp(400, &json!({"error": "unsupported_grant_type"})),
    }
}

fn bearer(s: &mut State, auth: Option<&str>) -> Option<Resp> {
    let ok = auth.and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|t| s.access.contains(t));
    if ok && !std::mem::take(&mut s.reject_bearer_once) {
        return None;
    }
    Some(
        hyper::Response::builder()
            .status(401)
            .header("WWW-Authenticate", "Bearer error=\"invalid_token\"")
            .body(Full::new(Bytes::from_static(b"{\"error\":\"invalid_token\"}")))
            .unwrap(),
    )
}

fn submit(s: &mut State, body: &[u8]) -> Resp {
    let doc: Value = serde_json::from_slice(body).unwrap_or_default();
    let tool = doc["tool"].as_str().unwrap_or_default().to_owned();
    let Some(spec) = reins_proto::connector::spec_for_tool(&tool).filter(|s| s.desktop_only) else {
        return json_resp(400, &json!({"error": "unknown_tool"}));
    };
    if let Err(message) = spec.parse(&doc["arguments"]) {
        return json_resp(400, &json!({"error": "invalid_arguments", "message": message}));
    }
    s.calls.push(Call {
        tool: tool.clone(),
        arguments: doc["arguments"].clone(),
        account: doc["account"].clone(),
    });
    s.request_seq += 1;
    let id = format!("req-{}", s.request_seq);
    let steps = s.plans.pop_front().unwrap_or_else(|| vec![Step::Approve]);
    s.requests.insert(
        id.clone(),
        Request {
            tool,
            arguments: doc["arguments"].clone(),
            steps,
            next: 0,
        },
    );
    answer(s, &id)
}

fn poll(s: &mut State, id: &str) -> Resp {
    s.polls.push(id.to_owned());
    if !s.requests.contains_key(id) {
        return json_resp(404, &json!({"error": "not_found"}));
    }
    answer(s, id)
}

fn answer(s: &mut State, id: &str) -> Resp {
    let pinned = s.pinned_key.clone();
    let r = s.requests.get_mut(id).unwrap();
    let step = r.steps[r.next.min(r.steps.len() - 1)];
    r.next += 1;
    let status = |st: &str| json_resp(200, &json!({"request_id": id, "status": st}));
    let answered =
        |outcome: Value| json_resp(200, &json!({"request_id": id, "status": "answered", "outcome": outcome}));
    let key = r.arguments["client_key"].as_str().unwrap_or_default();
    if pinned.as_deref() != Some(key) {
        return answered(
            json!({"outcome": "error", "message": "This must come from the Reins desktop app paired with this phone."}),
        );
    }
    let fetch = r.tool.ends_with(&format!("_{GIT_FETCH_OP}"));
    let repo = r.arguments["repo"].as_str().unwrap_or_default().to_owned();
    let mut grant = CredentialGrant {
        v: 1,
        nonce: r.arguments["nonce"].as_str().unwrap_or_default().to_owned(),
        digest: r.arguments["digest"].as_str().map(str::to_owned),
        repo: repo.clone(),
        access: if fetch {
            "read"
        } else {
            "write"
        }
        .to_owned(),
        username: "x-access-token".to_owned(),
        token: format!("ghs-{id}"),
        expires_at: reins_desktop::now_unix()
            + if fetch {
                FETCH_LEASE_SECS
            } else {
                PUSH_LEASE_SECS
            },
    };
    let sealed = match step {
        Step::Pending => return status("pending"),
        Step::Offline => return status("offline"),
        Step::Denied(reason) => return answered(json!({"outcome": "denied", "reason": reason})),
        Step::Error(message) => return answered(json!({"outcome": "error", "message": message})),
        Step::Garbage => "%% not a sealed box %%".to_owned(),
        Step::OtherKey => seal_to(&Identity::generate().public_key(), &serde_json::to_vec(&grant).unwrap()).unwrap(),
        Step::Approve | Step::Tamper(_) => {
            if let Step::Tamper(f) = step {
                f(&mut grant);
            }
            seal_to(key, &serde_json::to_vec(&grant).unwrap()).unwrap()
        }
    };
    let data = if fetch {
        json!({"items": [{"id": repo, "title": format!("Clone and fetch {repo}"), "sealed": sealed, "expires_at": grant.expires_at}]})
    } else {
        json!({"sealed": sealed})
    };
    answered(json!({"outcome": "result", "result": {"kind": "connector", "data": data}}))
}

/// Follows the sign-in link like a browser: the server redirects to the app's loopback callback, which shows a page.
pub async fn browse(link: String) -> (u16, String) {
    let http = reins_desktop::http::client(Some(std::time::Duration::from_secs(10))).unwrap();
    let first = http.get(&link).send().await.unwrap();
    assert_eq!(first.status(), 302, "authorize: {}", first.text().await.unwrap());
    let to = first.headers()["location"].to_str().unwrap().to_owned();
    assert!(to.starts_with("http://127.0.0.1:"), "{to}");
    let page = http.get(&to).send().await.unwrap();
    (page.status().as_u16(), page.text().await.unwrap())
}

/// A logged-in app: a temp state directory with a session from a full login against `mock`.
pub struct App {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
    pub identity: Arc<Identity>,
}

pub async fn logged_in(mock: &Mock) -> App {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let identity = Arc::new(Identity::generate());
    let server = reins_desktop::server::oauth::login_with_browser(&paths, &identity, &mock.base, |link| {
        tokio::spawn(browse(link.to_owned()));
    })
    .await
    .unwrap();
    assert_eq!(server, mock.base);
    App {
        dir,
        paths,
        identity,
    }
}

pub fn config(timeout_secs: u64) -> Config {
    let mut c = Config {
        approval_timeout_secs: timeout_secs,
        ..Config::default()
    };
    c.github.account = Some("octo".to_owned());
    c
}

pub fn repo(name: &str) -> reins_desktop::auth::Repo {
    reins_desktop::auth::Repo::new("github.com", "github", name)
}

pub const A: &str = "1111111111111111111111111111111111111111";
pub const B: &str = "2222222222222222222222222222222222222222";

pub fn push_to(branch: &str) -> PushSummary {
    PushSummary {
        updates: vec![RefUpdate {
            name: format!("refs/heads/{branch}"),
            change: RefChange::Update,
            old: A.to_owned(),
            new: B.to_owned(),
            fast_forward: Some(true),
            commit_count: 0,
            commits: vec![],
            files_changed: 0,
            files: vec![],
            additions: None,
            deletions: None,
        }],
        pack_bytes: 10,
        ..PushSummary::default()
    }
}

pub fn tag_push() -> PushSummary {
    let mut s = push_to("x");
    "refs/tags/v1".clone_into(&mut s.updates[0].name);
    s.updates[0].change = RefChange::Create;
    reins_proto::desktop::ZERO_OID.clone_into(&mut s.updates[0].old);
    s.updates[0].fast_forward = None;
    s
}
