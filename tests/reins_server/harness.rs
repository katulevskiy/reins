use std::{
    fs::File,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

use reqwest::{Client, StatusCode, redirect::Policy};
use serde_json::{Value, json};

/// Any string works: the server stores a salted PBKDF2 of whatever the client sends.
pub const PASSWORD_HASH: &str = "reins-test-master-password-hash";

pub struct Options {
    pub relay_wait_secs: u64,
    pub offline_secs: u64,
    /// Lets the server send to and fetch from `http://127.0.0.1` mock servers (test-only switch).
    pub allow_loopback: bool,
    /// More settings (limits, mostly).
    pub env: Vec<(&'static str, String)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            relay_wait_secs: 4,
            offline_secs: 2,
            allow_loopback: false,
            env: Vec::new(),
        }
    }
}

pub struct Server {
    pub base: String,
    child: Child,
    dir: PathBuf,
}

pub fn client() -> Client {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Another test thread may win the race; either way a provider is installed.
        rustls::crypto::ring::default_provider().install_default().ok();
    }
    Client::builder().redirect(Policy::none()).timeout(Duration::from_secs(90)).build().expect("reqwest client")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

impl Server {
    pub async fn start() -> Self {
        Self::start_with(Options::default()).await
    }

    pub async fn start_with(options: Options) -> Self {
        let port = free_port();
        let dir = std::env::temp_dir().join(format!("reins-it-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let log = File::create(dir.join("server.log")).expect("log file");
        let base = format!("http://127.0.0.1:{port}");
        let mut command = Command::new(env!("CARGO_BIN_EXE_vaultwarden"));
        command
            .current_dir(&dir)
            .env_clear()
            .env("DATA_FOLDER", &dir)
            .env("DOMAIN", &base)
            .env("ROCKET_ADDRESS", "127.0.0.1")
            .env("ROCKET_PORT", port.to_string())
            .env("WEB_VAULT_ENABLED", "false")
            .env("LOG_LEVEL", "info")
            .env("REINS_ENABLED", "true")
            .env("REINS_RELAY_WAIT_SECS", options.relay_wait_secs.to_string())
            .env("REINS_OFFLINE_SECS", options.offline_secs.to_string());
        if options.allow_loopback {
            command.env("REINS_TEST_ALLOW_LOOPBACK", "true");
        }
        command.envs(options.env.iter().map(|(k, v)| (*k, v.as_str())));
        let child = command.stdout(log.try_clone().expect("log clone")).stderr(log).spawn().expect("spawn vaultwarden");
        let server = Self {
            base,
            child,
            dir,
        };
        server.wait_alive().await;
        server
    }

    async fn wait_alive(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Ok(r) = client().get(self.url("/alive")).send().await
                && r.status() == StatusCode::OK
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let log = std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default();
        panic!("server did not start:\n{log}");
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Everything the server logged so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default()
    }

    /// The server's data folder.
    pub fn data_dir(&self) -> &std::path::Path {
        &self.dir
    }

    pub async fn register(&self, email: &str) {
        let body = json!({
            "email": email,
            "name": "Reins Test",
            "masterPasswordHash": PASSWORD_HASH,
            "masterPasswordHint": null,
            "key": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "kdf": 0,
            "kdfIterations": 600_000
        });
        let r = client().post(self.url("/identity/accounts/register")).json(&body).send().await.expect("register");
        let status = r.status();
        assert!(status.is_success(), "register {email}: {status} {}", r.text().await.unwrap_or_default());
    }

    /// Password login as an Android device (contracts §B); returns the access token.
    pub async fn login(&self, email: &str, device_id: &str) -> String {
        let form = [
            ("grant_type", "password"),
            ("username", email),
            ("password", PASSWORD_HASH),
            ("scope", "api offline_access"),
            ("client_id", "mobile"),
            ("deviceType", "0"),
            ("deviceIdentifier", device_id),
            ("deviceName", "Reins"),
        ];
        let r = client().post(self.url("/identity/connect/token")).form(&form).send().await.expect("login");
        let status = r.status();
        let body: Value = r.json().await.expect("login json");
        assert!(status.is_success(), "login {email}: {status} {body}");
        body["access_token"].as_str().expect("access_token").to_owned()
    }

    /// Registers `email` and logs in a fresh "phone" device.
    pub async fn phone(&self, email: &str) -> Phone {
        self.register(email).await;
        self.second_device(email).await
    }

    /// Another logged-in device of an already registered user, with its own device key.
    pub async fn second_device(&self, email: &str) -> Phone {
        self.device_with_id(email, &uuid::Uuid::new_v4().to_string(), Some(new_device_key())).await
    }

    /// A sign-in of `email` as the Vaultwarden device `device_id` (any id, also one another device uses) that sends
    /// `key` as its device key.
    pub async fn device_with_id(&self, email: &str, device_id: &str, key: Option<String>) -> Phone {
        let token = self.login(email, device_id).await;
        Phone {
            base: self.url("/reins/api"),
            token,
            device_id: device_id.to_owned(),
            key,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// A fresh device key (`Reins-Device-Key`), as a phone makes it.
pub fn new_device_key() -> String {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    reins_proto::desktop::encode_key(&key)
}

/// A logged-in device talking to the phone API.
pub struct Phone {
    pub base: String,
    pub token: String,
    /// Its Vaultwarden device id.
    pub device_id: String,
    /// Its device key, sent with every call.
    pub key: Option<String>,
}

impl Phone {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> (StatusCode, Value) {
        let mut request = self.raw(method, path);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("phone request");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::PUT, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.call(reqwest::Method::DELETE, path, None).await
    }

    /// A raw request (bytes in, response out) as this device.
    pub fn raw(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = client().request(method, format!("{}{path}", self.base)).bearer_auth(&self.token);
        match &self.key {
            Some(key) => request.header(reins_proto::device::DEVICE_KEY_HEADER, key),
            None => request,
        }
    }

    /// Registers this device as the approval device.
    pub async fn register_device(&self) {
        let (status, body) = self.put("/device", &json!({"fcm_token": null})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

/// Redirect URI of Claude's hosted connector; registered by `register_client`.
pub const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
/// RFC 7636 appendix B pair.
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// What an AI client holds after connecting.
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub connection_id: String,
    pub client_id: String,
}

/// The text between `before` and the next `after` in `haystack`.
pub fn between<'a>(haystack: &'a str, before: &str, after: &str) -> &'a str {
    let start = haystack.find(before).unwrap_or_else(|| panic!("`{before}` not in page:\n{haystack}")) + before.len();
    let end = haystack[start..].find(after).unwrap_or_else(|| panic!("`{after}` not in page:\n{haystack}"));
    &haystack[start..start + end]
}

pub fn query_param(url: &str, key: &str) -> Option<String> {
    url::Url::parse(url).ok()?.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

impl Server {
    /// Dynamic client registration; returns the new `client_id`.
    pub async fn register_client(&self) -> String {
        let body = json!({"client_name": "Claude", "redirect_uris": [REDIRECT], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]});
        let r = client().post(self.url("/reins/oauth/register")).json(&body).send().await.expect("register client");
        assert_eq!(r.status(), StatusCode::CREATED);
        let doc: Value = r.json().await.expect("registration json");
        doc["client_id"].as_str().expect("client_id").to_owned()
    }

    pub fn authorize_url(&self, client_id: &str) -> String {
        self.authorize_url_for(client_id, &self.url("/mcp"))
    }

    pub fn authorize_url_for(&self, client_id: &str, resource: &str) -> String {
        self.authorize_url_with(client_id, resource, &[])
    }

    /// The authorize URL with extra query parameters (the desktop app adds `reins_client_key`).
    pub fn authorize_url_with(&self, client_id: &str, resource: &str, extra: &[(&str, &str)]) -> String {
        let mut url = url::Url::parse(&self.url("/reins/oauth/authorize")).expect("url");
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", REDIRECT)
            .append_pair("response_type", "code")
            .append_pair("code_challenge", CHALLENGE)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", "st-1")
            .append_pair("resource", resource)
            .extend_pairs(extra);
        url.into()
    }

    /// Opens the authorize page and submits `email`; returns the wait-page URL.
    pub async fn start_authorization(&self, client_id: &str, email: &str) -> String {
        self.start_authorization_at(&self.authorize_url(client_id), email).await
    }

    /// Opens the authorize page at `authorize_url` and submits `email`; returns the wait-page URL.
    pub async fn start_authorization_at(&self, authorize_url: &str, email: &str) -> String {
        let page = client().get(authorize_url).send().await.expect("authorize");
        assert_eq!(page.status(), StatusCode::OK);
        let html = page.text().await.expect("authorize html");
        let session = between(&html, "name=\"session\" value=\"", "\"").to_owned();
        let r = client()
            .post(self.url("/reins/oauth/authorize"))
            .form(&[("session", session.as_str()), ("email", email)])
            .send()
            .await
            .expect("submit email");
        assert_eq!(r.status(), StatusCode::SEE_OTHER, "{}", r.text().await.unwrap_or_default());
        let location = r.headers()["location"].to_str().expect("location").to_owned();
        self.url(&location)
    }

    /// POST /token with a form; returns status and JSON body.
    pub async fn token(&self, form: &[(&str, &str)]) -> (StatusCode, Value) {
        let r = client().post(self.url("/reins/oauth/token")).form(form).send().await.expect("token");
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    /// The whole flow: register a client, authorize through the browser pages, approve on
    /// `phone` with the right code, exchange the code.
    pub async fn connect_ai(&self, phone: &Phone, email: &str, label: Option<&str>) -> Tokens {
        self.connect_with(phone, email, label, &[]).await.0
    }

    /// `connect_ai` with extra authorize parameters; also returns the pairing request the phone fetched and the
    /// wait page the browser showed.
    pub async fn connect_with(
        &self,
        phone: &Phone,
        email: &str,
        label: Option<&str>,
        extra: &[(&str, &str)],
    ) -> (Tokens, Value, String) {
        let client_id = self.register_client().await;
        let authorize = self.authorize_url_with(&client_id, &self.url("/mcp"), extra);
        let wait_url = self.start_authorization_at(&authorize, email).await;
        let html = client().get(&wait_url).send().await.expect("wait").text().await.expect("wait html");
        let code: u8 = between(&html, "aria-label=\"code\">", "<").parse().expect("code");
        let (status, pending) = phone.get("/pending?wait=5").await;
        assert_eq!(status, StatusCode::OK, "{pending}");
        let pairing = pending["pairings"][0].clone();
        let pairing_id = pairing["id"].as_str().expect("pairing id").to_owned();
        let (status, result) = phone
            .post(
                &format!("/pairings/{pairing_id}/response"),
                &json!({"v": 1, "approved": true, "chosen_code": code, "label": label}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let connection_id = result["connection_id"].as_str().expect("connection_id").to_owned();
        let done = client().get(&wait_url).send().await.expect("wait again");
        assert_eq!(done.status(), StatusCode::SEE_OTHER);
        let location = done.headers()["location"].to_str().expect("location").to_owned();
        let auth_code = query_param(&location, "code").expect("authorization code");
        let mcp = self.url("/mcp");
        let (status, tokens) = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", &auth_code),
                ("redirect_uri", REDIRECT),
                ("client_id", &client_id),
                ("code_verifier", VERIFIER),
                ("resource", &mcp),
            ])
            .await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        let tokens = Tokens {
            access: tokens["access_token"].as_str().expect("access_token").to_owned(),
            refresh: tokens["refresh_token"].as_str().expect("refresh_token").to_owned(),
            connection_id,
            client_id,
        };
        (tokens, pairing, html)
    }
}
