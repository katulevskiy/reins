//! A simulated AI client (Claude/ChatGPT): dynamic registration, the browser-based
//! authorization with PKCE, and MCP calls with the resulting bearer token.

use reqwest::StatusCode;
use serde_json::{Value, json};

/// Claude's hosted connector callback (registered by [`AiClient::register_client`]).
pub const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
/// RFC 7636 appendix B pair.
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

pub struct AiClient {
    pub base: String,
    http: reqwest::Client,
    pub client_id: String,
    pub access: String,
    pub refresh: String,
}

fn between<'a>(haystack: &'a str, before: &str, after: &str) -> &'a str {
    let start = haystack.find(before).unwrap_or_else(|| panic!("`{before}` not found in:\n{haystack}")) + before.len();
    let end = haystack[start..].find(after).unwrap_or_else(|| panic!("`{after}` not found in:\n{haystack}"));
    &haystack[start..start + end]
}

impl AiClient {
    pub fn new(base: &str) -> Self {
        crate::init_tls();
        Self {
            base: base.to_owned(),
            http: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client"),
            client_id: String::new(),
            access: String::new(),
            refresh: String::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// RFC 7591 dynamic client registration.
    pub async fn register_client(&mut self) {
        let body = json!({"client_name": "Claude", "redirect_uris": [REDIRECT], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]});
        let r = self.http.post(self.url("/rewarden/oauth/register")).json(&body).send().await.expect("register client");
        assert_eq!(r.status(), StatusCode::CREATED);
        let doc: Value = r.json().await.expect("json");
        self.client_id = doc["client_id"].as_str().expect("client_id").to_owned();
    }

    /// Opens the authorize page and submits the email; returns the "wait" page URL.
    pub async fn start_authorization(&self, email: &str) -> String {
        let mut url = url::Url::parse(&self.url("/rewarden/oauth/authorize")).expect("url");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", REDIRECT)
            .append_pair("response_type", "code")
            .append_pair("code_challenge", CHALLENGE)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", "state-1")
            .append_pair("resource", &self.url("/mcp"));
        let page = self.http.get(url).send().await.expect("authorize");
        assert_eq!(page.status(), StatusCode::OK);
        let html = page.text().await.expect("html");
        let session = between(&html, "name=\"session\" value=\"", "\"").to_owned();
        let r = self
            .http
            .post(self.url("/rewarden/oauth/authorize"))
            .form(&[("session", session.as_str()), ("email", email)])
            .send()
            .await
            .expect("submit email");
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        self.url(r.headers()["location"].to_str().expect("location"))
    }

    /// The number the browser shows while waiting for the phone.
    pub async fn browser_code(&self, wait_url: &str) -> u8 {
        let html = self.http.get(wait_url).send().await.expect("wait page").text().await.expect("html");
        between(&html, "aria-label=\"code\">", "<").parse().expect("code")
    }

    /// After the phone approved: follow the redirect, exchange the code, keep the tokens.
    pub async fn finish_authorization(&mut self, wait_url: &str) {
        let done = self.http.get(wait_url).send().await.expect("wait");
        assert_eq!(done.status(), StatusCode::SEE_OTHER, "the phone has not approved yet");
        let location = done.headers()["location"].to_str().expect("location").to_owned();
        self.exchange_code(&location).await;
    }

    /// Polls the wait page until the phone approved, then completes the authorization.
    pub async fn finish_when_approved(&mut self, wait_url: &str, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let done = self.http.get(wait_url).send().await.expect("wait");
            if done.status() == StatusCode::SEE_OTHER {
                let location = done.headers()["location"].to_str().expect("location").to_owned();
                self.exchange_code(&location).await;
                return;
            }
            assert!(std::time::Instant::now() < deadline, "the phone never approved the connection");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    async fn exchange_code(&mut self, location: &str) {
        let parsed = url::Url::parse(location).expect("redirect url");
        let code = parsed
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_else(|| panic!("no code in {location}"));
        let mcp = self.url("/mcp");
        let r = self
            .http
            .post(self.url("/rewarden/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("client_id", self.client_id.as_str()),
                ("code_verifier", VERIFIER),
                ("resource", mcp.as_str()),
            ])
            .send()
            .await
            .expect("token");
        assert_eq!(r.status(), StatusCode::OK);
        let tokens: Value = r.json().await.expect("json");
        self.access = tokens["access_token"].as_str().expect("access").to_owned();
        self.refresh = tokens["refresh_token"].as_str().expect("refresh").to_owned();
    }

    /// One JSON-RPC POST to `/mcp` with the bearer token.
    pub async fn rpc(&self, body: &Value) -> (StatusCode, Value) {
        let r = self.http.post(self.url("/mcp")).bearer_auth(&self.access).json(body).send().await.expect("mcp");
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// `tools/call`; returns the tool result.
    pub async fn tool(&self, name: &str, arguments: &Value) -> Value {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
        let (status, reply) = self.rpc(&body).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        reply["result"].clone()
    }
}
