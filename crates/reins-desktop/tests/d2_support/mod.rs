//! Shared by the D2 tests (`reins run`, the API proxy, the SSH agent): a mock Reins server whose phone answers
//! the desktop calls from a vault (secrets sealed in a `SecretGrant`, an SSH key list, signatures made with a real
//! ed25519 key and sealed in an `SshSignature`), an app state directory logged in to it, and a log capture.

#![allow(dead_code, reason = "each test binary uses a different part")]

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};

use data_encoding::BASE64;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use reins_desktop::config::Paths;
use reins_desktop::identity::{Identity, seal_to};
use reins_proto::desktop::{SecretGrant, SshSignature};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use serde_json::{Value, json};

pub const ACCESS_TOKEN: &str = "at-d2-test";

/// What the phone does with one call: the first step answers it, each poll takes the next (the last repeats).
#[derive(Clone, Copy, Debug)]
pub enum Step {
    Approve,
    Pending,
    Denied(&'static str),
    /// Approves with another nonce in the sealed answer.
    ForgedNonce,
    /// Approves with an answer that expired already.
    Expired,
    /// Approves, sealed to a key that is not the app's.
    OtherKey,
    /// Signs other data than asked (`vault_ssh_sign`).
    WrongSignature,
}

#[derive(Clone, Debug)]
pub struct Call {
    pub tool: String,
    pub arguments: Value,
}

struct Request {
    tool: String,
    arguments: Value,
    steps: Vec<Step>,
    next: usize,
}

pub struct State {
    pub calls: Vec<Call>,
    pub polls: Vec<String>,
    pub plans: VecDeque<Vec<Step>>,
    /// `Item/field` → value.
    pub vault: HashMap<String, String>,
    requests: HashMap<String, Request>,
    seq: u32,
}

impl State {
    /// The user answers request `id` from now on with `steps`.
    pub fn set_steps(&mut self, id: &str, steps: &[Step]) {
        let r = self.requests.get_mut(id).unwrap();
        r.steps = steps.to_vec();
        r.next = 0;
    }
}

/// The phone's SSH key: a real ed25519 key from a fixed seed.
pub struct SshKey {
    pub pair: Ed25519KeyPair,
    pub name: String,
}

impl SshKey {
    pub fn blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for part in [&b"ssh-ed25519"[..], self.pair.public_key().as_ref()] {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    /// `ssh-ed25519 AAAA… name`
    pub fn line(&self) -> String {
        format!("ssh-ed25519 {} {}", BASE64.encode(&self.blob()), self.name.replace(' ', "_"))
    }

    pub fn fingerprint(&self) -> String {
        reins_desktop::ssh_agent::keys::fingerprint(&self.blob())
    }

    fn sign(&self, data: &[u8]) -> String {
        let sig = self.pair.sign(data);
        let mut out = Vec::new();
        for part in [&b"ssh-ed25519"[..], sig.as_ref()] {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
            out.extend_from_slice(part);
        }
        BASE64.encode(&out)
    }
}

#[derive(Clone)]
pub struct Mock {
    pub base: String,
    pub state: Arc<Mutex<State>>,
    pub key: Arc<SshKey>,
}

impl Mock {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let state = Arc::new(Mutex::new(State {
            calls: Vec::new(),
            polls: Vec::new(),
            plans: VecDeque::new(),
            vault: HashMap::from([
                ("OpenAI/password".to_owned(), "sk-live-5ecret-0penai".to_owned()),
                ("Deploy/password".to_owned(), "deploy-5ecret value".to_owned()),
                ("Deploy/username".to_owned(), "deployer".to_owned()),
            ]),
            requests: HashMap::new(),
            seq: 0,
        }));
        let key = Arc::new(SshKey {
            pair: Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap(),
            name: "Deploy key".to_owned(),
        });
        let (shared, k) = (Arc::clone(&state), Arc::clone(&key));
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let (state, key) = (Arc::clone(&shared), Arc::clone(&k));
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        let (state, key) = (Arc::clone(&state), Arc::clone(&key));
                        async move { Ok::<_, Infallible>(handle(&state, &key, req).await) }
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
            key,
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn plan(&self, steps: &[Step]) {
        self.with(|s| s.plans.push_back(steps.to_vec()));
    }

    pub fn calls(&self) -> Vec<Call> {
        self.with(|s| s.calls.clone())
    }

    pub fn calls_of(&self, tool: &str) -> Vec<Call> {
        self.calls().into_iter().filter(|c| c.tool == tool).collect()
    }
}

type Resp = hyper::Response<Full<Bytes>>;

fn json_resp(status: u16, v: &Value) -> Resp {
    hyper::Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(v.to_string())))
        .unwrap()
}

async fn handle(state: &Mutex<State>, key: &SshKey, req: hyper::Request<hyper::body::Incoming>) -> Resp {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let authorized =
        req.headers().get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {ACCESS_TOKEN}"));
    let body = req.into_body().collect().await.map(http_body_util::Collected::to_bytes).unwrap_or_default();
    if !authorized {
        return json_resp(401, &json!({"error": "invalid_token"}));
    }
    let mut s = state.lock().unwrap();
    match (method.as_str(), path.as_str()) {
        ("POST", "/reins/desktop/calls") => {
            let doc: Value = serde_json::from_slice(&body).unwrap_or_default();
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
            });
            s.seq += 1;
            let id = format!("req-{}", s.seq);
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
            answer(&mut s, key, &id)
        }
        ("GET", p) if p.starts_with("/reins/desktop/calls/") => {
            let id = p["/reins/desktop/calls/".len()..].to_owned();
            s.polls.push(id.clone());
            if !s.requests.contains_key(&id) {
                return json_resp(404, &json!({"error": "not_found"}));
            }
            answer(&mut s, key, &id)
        }
        _ => json_resp(404, &json!({"error": "not_found"})),
    }
}

fn answer(s: &mut State, key: &SshKey, id: &str) -> Resp {
    let vault = s.vault.clone();
    let r = s.requests.get_mut(id).unwrap();
    let step = r.steps[r.next.min(r.steps.len() - 1)];
    r.next += 1;
    let answered =
        |outcome: Value| json_resp(200, &json!({"request_id": id, "status": "answered", "outcome": outcome}));
    let result = |data: Value| answered(json!({"outcome": "result", "result": {"kind": "connector", "data": data}}));
    match step {
        Step::Pending => return json_resp(200, &json!({"request_id": id, "status": "pending"})),
        Step::Denied(reason) => return answered(json!({"outcome": "denied", "reason": reason})),
        _ => {}
    }
    let client_key = r.arguments["client_key"].as_str().unwrap_or_default().to_owned();
    let nonce = if matches!(step, Step::ForgedNonce) {
        "forged-nonce-from-elsewhere".to_owned()
    } else {
        r.arguments["nonce"].as_str().unwrap_or_default().to_owned()
    };
    let seal_key = if matches!(step, Step::OtherKey) {
        Identity::generate().public_key()
    } else {
        client_key
    };
    let now = reins_desktop::now_unix();
    match r.tool.as_str() {
        "vault_secret_release" => {
            let lease = r.arguments["lease_secs"].as_i64().unwrap_or(3600);
            let secrets: Vec<(String, String)> = r.arguments["secrets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let name = v.as_str().unwrap().to_owned();
                    let value = vault.get(&name).cloned().unwrap_or_default();
                    (name, value)
                })
                .collect();
            if secrets.iter().any(|(_, v)| v.is_empty()) {
                return answered(json!({"outcome": "error", "message": "No such item or field in the vault."}));
            }
            let grant = SecretGrant {
                v: 1,
                nonce,
                secrets,
                expires_at: if matches!(step, Step::Expired) {
                    now - 1
                } else {
                    now + lease
                },
            };
            result(json!({"sealed": seal_to(&seal_key, &serde_json::to_vec(&grant).unwrap()).unwrap()}))
        }
        "vault_ssh_keys" => result(json!({"items": [{
            "id": "ssh-1",
            "title": key.name,
            "public_key": key.line(),
            "fingerprint": key.fingerprint(),
        }]})),
        "vault_ssh_sign" => {
            let data = BASE64.decode(r.arguments["data_base64"].as_str().unwrap().as_bytes()).unwrap();
            let signed = if matches!(step, Step::WrongSignature) {
                b"other data".to_vec()
            } else {
                data
            };
            let sig = SshSignature {
                v: 1,
                nonce,
                signature_base64: key.sign(&signed),
            };
            result(json!({"sealed": seal_to(&seal_key, &serde_json::to_vec(&sig).unwrap()).unwrap()}))
        }
        other => answered(json!({"outcome": "error", "message": format!("unexpected tool {other}")})),
    }
}

/// An app state directory logged in to `mock` (a session with a long-lived access token).
pub struct App {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
}

pub fn logged_in(mock: &Mock) -> App {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    quiet(&paths);
    let session = json!({
        "server": mock.base,
        "client_id": "client-1",
        "token_endpoint": format!("{}/reins/oauth/token", mock.base),
        "access_token": ACCESS_TOKEN,
        "access_expires_at": reins_desktop::now_unix() + 86_400,
        "refresh_token": null,
    });
    reins_desktop::config::write_private(&paths.session_file(), session.to_string().as_bytes()).unwrap();
    Identity::load_or_create(&paths.identity_file()).unwrap();
    App {
        dir,
        paths,
    }
}

impl App {
    pub fn public_key(&self) -> String {
        Identity::load_or_create(&self.paths.identity_file()).unwrap().public_key()
    }
}

/// Everything logged in this test binary.
static LOGS: Mutex<String> = Mutex::new(String::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _m: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, r: &log::Record<'_>) {
        use std::fmt::Write as _;
        writeln!(LOGS.lock().unwrap(), "{} {} {}", r.level(), r.target(), r.args()).unwrap();
    }
    fn flush(&self) {}
}

pub fn capture_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

pub fn logs() -> String {
    LOGS.lock().unwrap().clone()
}

/// No "check your phone" notifications from tests (they would pop up on the desktop running them).
pub fn quiet(paths: &Paths) {
    paths.ensure().unwrap();
    std::fs::write(paths.config_file(), "[notify]\nphone = false\n").unwrap();
}
