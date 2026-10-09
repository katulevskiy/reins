//! A fake WorkOS User Management API (AuthKit authorize, `authenticate`, deleting users, the Events API), and a
//! browser that runs the server's SSO sign-in against it the way ASWebAuthenticationSession or a Custom Tab would.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The fake application's client id and secret (the API key).
pub const CLIENT_ID: &str = "client_01E2ETEST";
pub const API_KEY: &str = "sk_test_e2e";

/// A WorkOS user the fake signs in.
#[derive(Clone, Debug)]
pub struct User {
    pub id: String,
    pub email: String,
}

#[derive(Default)]
struct State {
    /// The user the next authorize signs in (the person at the browser).
    signing_in: Option<User>,
    authentication_method: Option<String>,
    /// code -> (user, code_challenge)
    codes: HashMap<String, (User, Option<String>)>,
    /// Session ids handed out, in order.
    sessions: Vec<String>,
    events: Vec<Value>,
    serial: u32,
    /// Users deleted through the User Management API, in order.
    deleted: Vec<String>,
    /// Deleting users fails (WorkOS is down).
    deletes_fail: bool,
}

pub struct FakeWorkos {
    pub server: MockServer,
    state: Arc<Mutex<State>>,
}

fn unsigned_jwt(claims: &Value) -> String {
    let b64 = |v: &Value| BASE64URL_NOPAD.encode(v.to_string().as_bytes());
    format!("{}.{}.fake-signature", b64(&json!({"alg": "RS256", "kid": "fake"})), b64(claims))
}

fn query(req: &Request) -> HashMap<String, String> {
    req.url.query_pairs().into_owned().collect()
}

struct Authorize(Arc<Mutex<State>>);

impl Respond for Authorize {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let q = query(req);
        if q.get("client_id").map(String::as_str) != Some(CLIENT_ID)
            || q.get("response_type").map(String::as_str) != Some("code")
            || q.get("provider").map(String::as_str) != Some("authkit")
        {
            return ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_request"}));
        }
        let mut st = self.0.lock().expect("state");
        let Some(user) = st.signing_in.clone() else {
            return ResponseTemplate::new(400).set_body_json(json!({"error": "nobody at the browser"}));
        };
        st.serial += 1;
        let code = format!("code-{}", st.serial);
        st.codes.insert(code.clone(), (user, q.get("code_challenge").cloned()));
        let mut back = url::Url::parse(&q["redirect_uri"]).expect("redirect_uri");
        back.query_pairs_mut().append_pair("code", &code).append_pair("state", &q["state"]);
        ResponseTemplate::new(302).insert_header("Location", back.as_str())
    }
}

struct Authenticate(Arc<Mutex<State>>);

impl Respond for Authenticate {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let refuse = |why: &str| {
            ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant", "error_description": why}))
        };
        if body["client_id"] != CLIENT_ID || body["client_secret"] != API_KEY {
            return refuse("bad client");
        }
        if body["grant_type"] != "authorization_code" {
            return refuse("unsupported grant");
        }
        let mut st = self.0.lock().expect("state");
        let Some((user, challenge)) = body["code"].as_str().and_then(|c| st.codes.remove(c)) else {
            return refuse("unknown code");
        };
        if let Some(challenge) = challenge {
            let verifier = body["code_verifier"].as_str().unwrap_or_default();
            let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
            if BASE64URL_NOPAD.encode(digest.as_ref()) != challenge {
                return refuse("PKCE verification failed");
            }
        }
        st.serial += 1;
        let sid = format!("session_{}", st.serial);
        st.sessions.push(sid.clone());
        let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_secs() + 300;
        ResponseTemplate::new(200).set_body_json(json!({
            "user": {"object": "user", "id": user.id, "email": user.email, "email_verified": true,
                     "first_name": "E2E", "last_name": "User"},
            "organization_id": null,
            "access_token": unsigned_jwt(&json!({"sub": user.id, "sid": sid, "exp": exp})),
            "refresh_token": format!("refresh-{sid}"),
            "authentication_method": st.authentication_method.as_deref().unwrap_or("Passkey"),
        }))
    }
}

struct Events(Arc<Mutex<State>>);

impl Respond for Events {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req.headers.get("authorization").and_then(|v| v.to_str().ok());
        if auth != Some(&format!("Bearer {API_KEY}")) {
            return ResponseTemplate::new(401).set_body_json(json!({"message": "Unauthorized"}));
        }
        let q = query(&req.clone());
        let st = self.0.lock().expect("state");
        let wanted: Vec<&str> = q.get("events").map(|e| e.split(',').collect()).unwrap_or_default();
        let start = q
            .get("after")
            .and_then(|after| st.events.iter().position(|e| e["id"] == after.as_str()).map(|i| i + 1))
            .unwrap_or(0);
        let limit = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(10);
        let data: Vec<Value> = st.events[start..]
            .iter()
            .filter(|e| wanted.is_empty() || wanted.contains(&e["event"].as_str().unwrap_or_default()))
            .take(limit)
            .cloned()
            .collect();
        ResponseTemplate::new(200)
            .set_body_json(json!({"object": "list", "data": data, "list_metadata": {"after": null}}))
    }
}

/// `DELETE /user_management/users/<id>`: 202, or 404 for a user deleted before.
struct DeleteUser(Arc<Mutex<State>>);

impl Respond for DeleteUser {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req.headers.get("authorization").and_then(|v| v.to_str().ok());
        if auth != Some(&format!("Bearer {API_KEY}")) {
            return ResponseTemplate::new(401).set_body_json(json!({"message": "Unauthorized"}));
        }
        let mut st = self.0.lock().expect("state");
        if st.deletes_fail {
            return ResponseTemplate::new(503).set_body_json(json!({"message": "Service Unavailable"}));
        }
        let id = req.url.path().trim_start_matches("/user_management/users/").to_owned();
        if st.deleted.contains(&id) {
            return ResponseTemplate::new(404)
                .set_body_json(json!({"code": "entity_not_found", "message": "User not found"}));
        }
        st.deleted.push(id);
        ResponseTemplate::new(202)
    }
}

impl FakeWorkos {
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let state = Arc::new(Mutex::new(State::default()));
        Mock::given(method("GET"))
            .and(path("/user_management/authorize"))
            .respond_with(Authorize(Arc::clone(&state)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/user_management/authenticate"))
            .respond_with(Authenticate(Arc::clone(&state)))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/events")).respond_with(Events(Arc::clone(&state))).mount(&server).await;
        Mock::given(method("DELETE"))
            .and(path_regex("^/user_management/users/[A-Za-z0-9_-]+$"))
            .respond_with(DeleteUser(Arc::clone(&state)))
            .mount(&server)
            .await;
        Self {
            server,
            state,
        }
    }

    /// `SSO_AUTHORITY` for this fake.
    pub fn authority(&self) -> String {
        format!("{}/user_management/{CLIENT_ID}", self.server.uri())
    }

    /// The server settings that make it sign in through this fake.
    pub fn server_env(&self) -> Vec<(String, String)> {
        [
            ("SSO_ENABLED", "true"),
            ("SSO_ONLY", "true"),
            ("SSO_PROVIDER", "workos"),
            ("SSO_CLIENT_ID", CLIENT_ID),
            ("SSO_CLIENT_SECRET", API_KEY),
            ("SSO_AUTH_ONLY_NOT_SESSION", "true"),
            ("SSO_SIGNUPS_MATCH_EMAIL", "true"),
            ("REINS_WORKOS_SYNC_SECS", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .chain([("SSO_AUTHORITY".to_owned(), self.authority())])
        .collect()
    }

    /// The person at the browser for the next sign-in.
    pub fn sign_in_as(&self, user: &User) {
        self.state.lock().expect("state").signing_in = Some(user.clone());
    }

    /// The method WorkOS reports for subsequent exchanges (the default is a passkey).
    pub fn authenticate_with(&self, method: &str) {
        self.state.lock().expect("state").authentication_method = Some(method.to_owned());
    }

    /// The WorkOS sessions handed out so far, oldest first.
    pub fn sessions(&self) -> Vec<String> {
        self.state.lock().expect("state").sessions.clone()
    }

    /// The users deleted through the User Management API so far.
    pub fn deleted_users(&self) -> Vec<String> {
        self.state.lock().expect("state").deleted.clone()
    }

    /// Makes deleting users fail (or work again).
    pub fn fail_deletes(&self, fail: bool) {
        self.state.lock().expect("state").deletes_fail = fail;
    }

    /// Adds an event to the Events API (`kind` such as `user.updated`).
    pub fn emit(&self, kind: &str, data: Value) {
        let mut st = self.state.lock().expect("state");
        st.serial += 1;
        let id = format!("event_{:04}", st.serial);
        let mut event = json!({"object": "event", "id": id, "event": kind, "created_at": "2026-10-02T10:00:00.000Z"});
        event["data"] = data;
        st.events.push(event);
    }
}

/// Runs the browser part of a sign-in: `authorize_url` (from `sso_begin`) through the server, the identity provider
/// and back, with the server's binding cookie, up to the app's callback URL, which it returns.
pub async fn browse_to_callback(authorize_url: &str, callback_scheme: &str) -> String {
    crate::init_tls();
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client");
    let mut cookies: HashMap<String, String> = HashMap::new();
    let mut url = authorize_url.to_owned();
    for _ in 0..8 {
        let cookie_header = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        let mut req = http.get(&url);
        if !cookie_header.is_empty() {
            req = req.header("Cookie", cookie_header);
        }
        let resp = req.send().await.expect("browser request");
        for value in resp.headers().get_all("set-cookie") {
            let pair = value.to_str().expect("cookie").split(';').next().unwrap_or_default();
            if let Some((k, v)) = pair.split_once('=') {
                cookies.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
        let status = resp.status();
        assert!(status.is_redirection(), "the sign-in stopped at {status}: {}", resp.text().await.unwrap_or_default());
        let location = resp.headers().get("location").expect("location").to_str().expect("location").to_owned();
        if location.starts_with(&format!("{callback_scheme}:")) {
            return location;
        }
        url = location;
    }
    panic!("the sign-in never came back to the app");
}
