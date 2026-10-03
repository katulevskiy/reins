//! Signing in to an MCP server as the MCP authorization spec describes: the protected resource metadata (RFC 9728,
//! found through `WWW-Authenticate: Bearer resource_metadata=…` or the well-known address), the authorization server's
//! metadata (RFC 8414, or OpenID Connect discovery), dynamic client registration (RFC 7591) with the app's redirect,
//! the authorization code flow with PKCE (S256) and the `resource` parameter (RFC 8707), and refreshing.
//!
//! Tokens and the PKCE verifier never leave this module except into the sealed store; nothing here is logged.

use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroize;

use super::{REDIRECT_URI, checked_url};
use crate::{CoreError, text};

/// The client this phone registered with an authorization server, and the endpoints it uses (not secret).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthClient {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    /// "none" | "client_secret_basic" | "client_secret_post"
    pub auth_method: String,
    /// The `resource` asked for (the MCP server).
    pub resource: String,
    #[serde(default)]
    pub scope: Option<String>,
}

/// A sign-in that was started: what the redirect must match and what proves it is the same sign-in.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSignIn {
    pub verifier: String,
    pub state: String,
}

impl fmt::Debug for PendingSignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingSignIn(<redacted>)")
    }
}

impl Drop for PendingSignIn {
    fn drop(&mut self) {
        self.verifier.zeroize();
        self.state.zeroize();
    }
}

/// What a token endpoint gave.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TokenAnswer {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

impl fmt::Debug for TokenAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenAnswer").field("expires_in", &self.expires_in).finish_non_exhaustive()
    }
}

impl Drop for TokenAnswer {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

/// Why refreshing failed.
#[derive(Debug)]
pub enum RefreshError {
    /// The authorization server no longer takes the refresh token: the user has to sign in again.
    Rejected,
    Other(CoreError),
}

#[derive(Debug, Default, Deserialize)]
struct ResourceMetadata {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ServerMetadata {
    #[serde(default)]
    issuer: Option<String>,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Option<Vec<String>>,
}

fn sign_in_error(why: impl Into<String>) -> CoreError {
    CoreError::service(why)
}

/// A parameter of a `WWW-Authenticate: Bearer k="v", …` challenge.
pub fn auth_param(challenge: &str, name: &str) -> Option<String> {
    let rest = challenge.trim_start();
    let rest = rest.strip_prefix("Bearer").or_else(|| rest.strip_prefix("bearer")).unwrap_or(rest);
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let key: String =
            std::iter::from_fn(|| chars.next_if(|c| *c != '=' && *c != ',' && !c.is_whitespace())).collect();
        if key.is_empty() {
            return None;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next_if_eq(&'=').is_none() {
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let value: String = if chars.next_if_eq(&'"').is_some() {
            let mut v = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => v.extend(chars.next()),
                    '"' => break,
                    c => v.push(c),
                }
            }
            v
        } else {
            std::iter::from_fn(|| chars.next_if(|c| *c != ',' && !c.is_whitespace())).collect()
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
}

/// RFC 9728 §3.1: the well-known metadata addresses of a resource (path-aware first, then the host's).
pub fn resource_metadata_urls(resource: &Url) -> Vec<String> {
    let origin = resource.origin().ascii_serialization();
    let path = resource.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!("{origin}/.well-known/oauth-protected-resource{path}"));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    urls
}

/// RFC 8414 §3.1 and OpenID Connect discovery addresses of an authorization server, in the order the MCP spec gives.
pub fn server_metadata_urls(issuer: &Url) -> Vec<String> {
    let origin = issuer.origin().ascii_serialization();
    let path = issuer.path().trim_end_matches('/');
    if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    }
}

/// `n` random bytes, base64url without padding.
pub fn random_token(n: usize) -> Result<String, CoreError> {
    use ring::rand::SecureRandom;
    let mut bytes = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| CoreError::storage("no randomness available"))?;
    let token = BASE64URL_NOPAD.encode(&bytes);
    bytes.zeroize();
    Ok(token)
}

/// RFC 7636 S256: base64url(SHA-256(verifier)).
pub fn code_challenge(verifier: &str) -> String {
    BASE64URL_NOPAD.encode(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref())
}

/// GETs a JSON document; `None` when it is not there (any failure).
async fn get_json<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: &str) -> Option<T> {
    let resp = http.get(url).header("Accept", "application/json").send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

/// Whether `declared` (the metadata's `resource`) names the server at `url`: the same address, or one it is under.
fn resource_matches(declared: &str, url: &Url) -> bool {
    let Ok(declared) = Url::parse(declared) else {
        return false;
    };
    if declared.origin() != url.origin() {
        return false;
    }
    let want = declared.path().trim_end_matches('/');
    let have = url.path().trim_end_matches('/');
    have == want || have.strip_prefix(want).is_some_and(|rest| rest.starts_with('/'))
}

/// The address the sign-in is for, as the MCP spec has it: the endpoint without a fragment.
pub fn canonical_resource(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.to_string()
}

/// Discovers how to sign in to the MCP server at `mcp` and registers this phone as a client. `challenge` is the
/// `WWW-Authenticate` header of its 401. Returns the client and its secret (if the server gave one).
pub async fn register(
    http: &reqwest::Client,
    mcp: &Url,
    challenge: Option<&str>,
) -> Result<(OAuthClient, Option<String>), CoreError> {
    // RFC 9728: where the server says its metadata is, else the well-known addresses.
    let mut metadata: Option<ResourceMetadata> = match challenge.and_then(|c| auth_param(c, "resource_metadata")) {
        Some(at) => get_json(http, checked_url(&at)?.as_str()).await,
        None => None,
    };
    if metadata.is_none() {
        for at in resource_metadata_urls(mcp) {
            if let Some(found) = get_json(http, &at).await {
                metadata = Some(found);
                break;
            }
        }
    }
    let mut resource = canonical_resource(mcp);
    let (issuer, scopes) = match &metadata {
        Some(m) => {
            if let Some(declared) = &m.resource {
                if !resource_matches(declared, mcp) {
                    return Err(sign_in_error("The server's sign-in information is for another address."));
                }
                resource.clone_from(declared);
            }
            let issuer =
                m.authorization_servers.first().ok_or_else(|| sign_in_error("The server names no sign-in server."))?;
            (checked_url(issuer)?, m.scopes_supported.clone())
        }
        // Servers of the 2025-03-26 spec: the authorization server is at the MCP server's origin.
        None => (checked_url(&mcp.origin().ascii_serialization())?, Vec::new()),
    };
    let scope = challenge
        .and_then(|c| auth_param(c, "scope"))
        .or_else(|| (!scopes.is_empty()).then(|| scopes.join(" ")))
        .map(|s| text::one_line(&s))
        .filter(|s| !s.is_empty());

    let mut server: Option<ServerMetadata> = None;
    for at in server_metadata_urls(&issuer) {
        if let Some(found) = get_json::<ServerMetadata>(http, &at).await {
            server = Some(found);
            break;
        }
    }
    let issuer_text = issuer.as_str().trim_end_matches('/').to_owned();
    let server = match server {
        Some(s) => {
            if s.issuer.as_deref().is_some_and(|i| i.trim_end_matches('/') != issuer_text) {
                return Err(sign_in_error("The sign-in server's information names another server."));
            }
            s
        }
        None if metadata.is_none() => {
            // 2025-03-26 fallback: the default endpoints at the origin.
            let origin = issuer.origin().ascii_serialization();
            ServerMetadata {
                issuer: None,
                authorization_endpoint: format!("{origin}/authorize"),
                token_endpoint: format!("{origin}/token"),
                registration_endpoint: Some(format!("{origin}/register")),
                code_challenge_methods_supported: None,
            }
        }
        None => return Err(sign_in_error("The sign-in server could not be found.")),
    };
    if server.code_challenge_methods_supported.as_ref().is_some_and(|m| !m.iter().any(|x| x == "S256")) {
        return Err(sign_in_error("The sign-in server does not support a secure sign-in from an app (PKCE)."));
    }
    let authorization_endpoint = checked_url(&server.authorization_endpoint)?.to_string();
    let token_endpoint = checked_url(&server.token_endpoint)?.to_string();
    let registration = server.registration_endpoint.as_deref().ok_or_else(|| {
        sign_in_error("This server does not let apps register for a sign-in. Add it with an access token instead.")
    })?;
    let registration = checked_url(registration)?;

    // RFC 7591.
    let mut body = json!({
        "client_name": "Reins",
        "redirect_uris": [REDIRECT_URI],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    if let Some(scope) = &scope {
        body["scope"] = json!(scope);
    }
    let resp = http.post(registration.as_str()).header("Accept", "application/json").json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(sign_in_error(format!(
            "The sign-in server refused to register Reins (HTTP {}). Add the server with an access token instead.",
            resp.status().as_u16()
        )));
    }
    let mut answer: Value =
        resp.json().await.map_err(|_| sign_in_error("The sign-in server's answer was not understood."))?;
    let client_id = answer
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty() && c.len() <= 512)
        .ok_or_else(|| sign_in_error("The sign-in server gave no client id."))?
        .to_owned();
    let secret = answer.get("client_secret").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    if let Some(Value::String(s)) = answer.get_mut("client_secret") {
        s.zeroize();
    }
    let auth_method = match (answer.get("token_endpoint_auth_method").and_then(Value::as_str), &secret) {
        (_, None) => "none",
        (Some("client_secret_post"), Some(_)) => "client_secret_post",
        (_, Some(_)) => "client_secret_basic",
    }
    .to_owned();
    Ok((
        OAuthClient {
            issuer: issuer_text,
            authorization_endpoint,
            token_endpoint,
            client_id,
            auth_method,
            resource,
            scope,
        },
        secret,
    ))
}

/// A new verifier and state, and the address the browser opens.
pub fn start(client: &OAuthClient) -> Result<(PendingSignIn, String), CoreError> {
    let pending = PendingSignIn {
        verifier: random_token(32)?,
        state: random_token(16)?,
    };
    let mut url = checked_url(&client.authorization_endpoint)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("code_challenge", &code_challenge(&pending.verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &pending.state)
            .append_pair("resource", &client.resource);
        if let Some(scope) = &client.scope {
            q.append_pair("scope", scope);
        }
    }
    Ok((pending, url.to_string()))
}

async fn token_request(
    http: &reqwest::Client,
    client: &OAuthClient,
    secret: Option<&str>,
    mut form: Vec<(&str, String)>,
) -> Result<TokenAnswer, RefreshError> {
    form.push(("client_id", client.client_id.clone()));
    form.push(("resource", client.resource.clone()));
    let mut builder = http.post(&client.token_endpoint).header("Accept", "application/json");
    match (client.auth_method.as_str(), secret) {
        ("client_secret_basic", Some(secret)) => {
            builder = builder.basic_auth(&client.client_id, Some(secret));
        }
        ("client_secret_post", Some(secret)) => form.push(("client_secret", secret.to_owned())),
        _ => {}
    }
    let resp = builder.form(&form).send().await;
    for (_, value) in &mut form {
        value.zeroize();
    }
    let resp = resp.map_err(|e| RefreshError::Other(e.into()))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| RefreshError::Other(e.into()))?;
    if status.is_success() {
        let answer: TokenAnswer = serde_json::from_str(&body)
            .map_err(|_| RefreshError::Other(sign_in_error("The sign-in server's answer was not understood.")))?;
        if answer.access_token.is_empty() {
            return Err(RefreshError::Other(sign_in_error("The sign-in server gave no access token.")));
        }
        return Ok(answer);
    }
    let code = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    if matches!(code.as_str(), "invalid_grant" | "invalid_client" | "unauthorized_client")
        || status == StatusCode::UNAUTHORIZED
    {
        return Err(RefreshError::Rejected);
    }
    Err(RefreshError::Other(sign_in_error(format!("The sign-in server answered with HTTP {}.", status.as_u16()))))
}

/// Trades the code from the redirect for tokens.
pub async fn exchange(
    http: &reqwest::Client,
    client: &OAuthClient,
    secret: Option<&str>,
    code: &str,
    pending: &PendingSignIn,
) -> Result<TokenAnswer, CoreError> {
    let form = vec![
        ("grant_type", "authorization_code".to_owned()),
        ("code", code.to_owned()),
        ("redirect_uri", REDIRECT_URI.to_owned()),
        ("code_verifier", pending.verifier.clone()),
    ];
    token_request(http, client, secret, form).await.map_err(|e| match e {
        RefreshError::Rejected => sign_in_error("The sign-in was not accepted. Try again."),
        RefreshError::Other(e) => e,
    })
}

/// A new access token from the refresh token.
pub async fn refresh(
    http: &reqwest::Client,
    client: &OAuthClient,
    secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenAnswer, RefreshError> {
    let form = vec![("grant_type", "refresh_token".to_owned()), ("refresh_token", refresh_token.to_owned())];
    token_request(http, client, secret, form).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenges_are_parsed() {
        let c = r#"Bearer error="invalid_token", resource_metadata="https://x.com/.well-known/oauth-protected-resource/mcp", scope="read write""#;
        assert_eq!(
            auth_param(c, "resource_metadata").as_deref(),
            Some("https://x.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(auth_param(c, "scope").as_deref(), Some("read write"));
        assert_eq!(auth_param(c, "error").as_deref(), Some("invalid_token"));
        assert_eq!(auth_param("Bearer realm=x,scope=a", "scope").as_deref(), Some("a"));
        assert_eq!(auth_param(r#"Bearer realm="a\"b""#, "realm").as_deref(), Some("a\"b"));
        assert_eq!(auth_param("Bearer", "scope"), None);
    }

    #[test]
    fn well_known_addresses_follow_the_rfcs() {
        let mcp = Url::parse("https://mcp.example.com/v1/mcp").unwrap();
        assert_eq!(
            resource_metadata_urls(&mcp),
            [
                "https://mcp.example.com/.well-known/oauth-protected-resource/v1/mcp",
                "https://mcp.example.com/.well-known/oauth-protected-resource"
            ]
        );
        let tenant = Url::parse("https://auth.example.com/tenant1").unwrap();
        assert_eq!(
            server_metadata_urls(&tenant),
            [
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1",
                "https://auth.example.com/.well-known/openid-configuration/tenant1",
                "https://auth.example.com/tenant1/.well-known/openid-configuration"
            ]
        );
        assert_eq!(server_metadata_urls(&Url::parse("https://auth.example.com/").unwrap()).len(), 2);
        assert!(resource_matches("https://mcp.example.com/v1", &mcp));
        assert!(resource_matches("https://mcp.example.com/v1/mcp/", &mcp));
        assert!(!resource_matches("https://mcp.example.com/v", &mcp));
        assert!(!resource_matches("https://evil.example.com/v1/mcp", &mcp));
    }

    #[test]
    fn pkce_matches_the_rfc_example_and_tokens_are_random() {
        // RFC 7636 appendix B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let a = random_token(32).unwrap();
        assert_eq!(a.len(), 43);
        assert_ne!(a, random_token(32).unwrap());
    }

    #[test]
    fn the_authorize_address_carries_pkce_state_and_resource() {
        let client = OAuthClient {
            issuer: "https://auth.example.com".into(),
            authorization_endpoint: "https://auth.example.com/authorize?prompt=consent".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            client_id: "c 1".into(),
            auth_method: "none".into(),
            resource: "https://mcp.example.com/mcp".into(),
            scope: Some("read".into()),
        };
        let (pending, url) = start(&client).unwrap();
        let url = Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["prompt"], "consent", "the endpoint's own query is kept");
        assert_eq!(q["client_id"], "c 1");
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["code_challenge"], code_challenge(&pending.verifier));
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], pending.state);
        assert_eq!(q["resource"], "https://mcp.example.com/mcp");
        assert_eq!(q["scope"], "read");
        assert!(!format!("{pending:?}").contains(&pending.verifier));
    }
}
