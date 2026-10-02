//! OAuth 2.1 authorization-server logic for AI clients (spec §4.5) as pure functions:
//! metadata documents, PKCE, redirect-URI matching, client metadata (CIMD/DCR) and
//! request validation. Handlers in `oauth_routes.rs` only adapt these to HTTP.

use std::collections::HashMap;

use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};
use url::Url;

use super::ACCESS_TOKEN_SECS;
use crate::crypto::ct_eq;

pub const SCOPE: &str = "mcp";
pub const MAX_REDIRECT_URIS: usize = 10;
pub const MAX_REDIRECT_URI_BYTES: usize = 2048;
pub const MAX_STATE_BYTES: usize = 512;
/// CIMD documents larger than this are rejected (Decision 25).
pub const MAX_CLIENT_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_CLIENT_NAME_BYTES: usize = 256;

/// The OAuth issuer: the configured `DOMAIN` without a trailing slash.
pub fn issuer(domain: &str) -> String {
    domain.trim_end_matches('/').to_owned()
}

/// Canonical resource URL (RFC 8707): lower-case scheme and host, no trailing slash.
pub fn canonicalize_resource(raw: &str) -> String {
    match Url::parse(raw.trim()) {
        Ok(url) => url.as_str().trim_end_matches('/').to_owned(),
        Err(_) => raw.trim().trim_end_matches('/').to_owned(),
    }
}

/// The MCP endpoint URL, which is also the audience of access tokens.
pub fn canonical_mcp_url(domain: &str) -> String {
    canonicalize_resource(&format!("{}/mcp", issuer(domain)))
}

/// RFC 9728 protected-resource metadata (Decision 16).
pub fn protected_resource_metadata(domain: &str) -> Value {
    json!({
        "resource": canonical_mcp_url(domain),
        "authorization_servers": [issuer(domain)],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Rewarden"
    })
}

/// RFC 8414 authorization-server metadata.
pub fn authorization_server_metadata(domain: &str) -> Value {
    let iss = issuer(domain);
    json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/rewarden/oauth/authorize"),
        "token_endpoint": format!("{iss}/rewarden/oauth/token"),
        "registration_endpoint": format!("{iss}/rewarden/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true
    })
}

// ---------------------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------------------

/// `code_challenge` for `verifier` under the S256 method (RFC 7636 §4.2).
pub fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    BASE64URL_NOPAD.encode(digest.as_ref())
}

/// RFC 7636 §4.1: 43..=128 characters from the unreserved set.
pub fn is_valid_code_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// A SHA-256 digest in unpadded base64url is exactly 43 characters.
pub fn is_valid_code_challenge(challenge: &str) -> bool {
    challenge.len() == 43 && challenge.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub fn verify_pkce(verifier: &str, challenge: &str) -> bool {
    is_valid_code_verifier(verifier) && ct_eq(pkce_s256_challenge(verifier).as_bytes(), challenge.as_bytes())
}

// ---------------------------------------------------------------------------------------
// Redirect URIs
// ---------------------------------------------------------------------------------------

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// `https://…` or `http://` on a loopback host; no fragment, no credentials.
pub fn parse_redirect_uri(raw: &str) -> Result<Url, String> {
    if raw.len() > MAX_REDIRECT_URI_BYTES {
        return Err("redirect URI is too long".to_owned());
    }
    let url = Url::parse(raw).map_err(|e| format!("invalid redirect URI: {e}"))?;
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && is_loopback_host(&url));
    if !scheme_ok {
        return Err("redirect URI must be https (http is only allowed for localhost)".to_owned());
    }
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("redirect URI must not contain a fragment or credentials".to_owned());
    }
    Ok(url)
}

/// Decision 26: exact match, except loopback redirect URIs may differ in the port (RFC 8252 §7.3).
pub fn redirect_uri_matches(registered: &[String], requested: &str) -> bool {
    let Ok(want) = parse_redirect_uri(requested) else {
        return false;
    };
    registered.iter().any(|reg| {
        if reg == requested {
            return true;
        }
        let Ok(have) = parse_redirect_uri(reg) else {
            return false;
        };
        is_loopback_host(&have)
            && is_loopback_host(&want)
            && have.scheme() == want.scheme()
            && have.host() == want.host()
            && have.path() == want.path()
            && have.query() == want.query()
    })
}

/// Host shown on the phone for a validated redirect URI.
pub fn redirect_host(redirect_uri: &str) -> String {
    Url::parse(redirect_uri).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

/// `redirect_uri?code=…&state=…&iss=…` (RFC 9207).
pub fn success_redirect(redirect_uri: &str, code: &str, state: Option<&str>, issuer: &str) -> String {
    build_redirect(redirect_uri, &[("code", code)], state, issuer)
}

/// `redirect_uri?error=…&error_description=…&state=…&iss=…`.
pub fn error_redirect(redirect_uri: &str, error: &str, description: &str, state: Option<&str>, issuer: &str) -> String {
    build_redirect(redirect_uri, &[("error", error), ("error_description", description)], state, issuer)
}

fn build_redirect(redirect_uri: &str, params: &[(&str, &str)], state: Option<&str>, issuer: &str) -> String {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return redirect_uri.to_owned();
    };
    {
        let mut query = url.query_pairs_mut();
        for (k, v) in params {
            query.append_pair(k, v);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", issuer);
    }
    url.into()
}

// ---------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------

/// An RFC 6749 §5.2 error response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthError {
    pub status: u16,
    pub error: &'static str,
    pub description: String,
}

impl OAuthError {
    pub fn new(status: u16, error: &'static str, description: impl Into<String>) -> Self {
        Self {
            status,
            error,
            description: description.into(),
        }
    }

    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self::new(400, "invalid_request", description)
    }

    pub fn invalid_grant(description: impl Into<String>) -> Self {
        Self::new(400, "invalid_grant", description)
    }

    pub fn to_json(&self) -> Value {
        json!({"error": self.error, "error_description": self.description})
    }
}

// ---------------------------------------------------------------------------------------
// Client metadata: CIMD documents and dynamic registration
// ---------------------------------------------------------------------------------------

/// What the server needs to know about an OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

/// A `client_id` that is a fetchable metadata-document URL (Decision 25).
pub fn is_cimd_client_id(client_id: &str) -> bool {
    Url::parse(client_id)
        .is_ok_and(|u| u.scheme() == "https" && u.host().is_some() && u.path() != "/" && u.fragment().is_none())
}

fn parse_redirect_uris(value: &Value) -> Result<Vec<String>, String> {
    let list = value.as_array().ok_or("redirect_uris must be an array")?;
    if list.is_empty() || list.len() > MAX_REDIRECT_URIS {
        return Err(format!("redirect_uris must contain 1..={MAX_REDIRECT_URIS} entries"));
    }
    list.iter()
        .map(|v| {
            let uri = v.as_str().ok_or("redirect_uris must be strings")?;
            parse_redirect_uri(uri)?;
            Ok(uri.to_owned())
        })
        .collect()
}

fn check_public_client(doc: &Value) -> Result<(), String> {
    match doc.get("token_endpoint_auth_method").and_then(Value::as_str) {
        None | Some("none") => {}
        Some(other) => return Err(format!("unsupported token_endpoint_auth_method `{other}`")),
    }
    // A client lists everything it *might* use (Claude also lists the JWT-bearer grant); what matters is that it can
    // do the authorization-code flow. Grant types this server does not offer are simply never issued.
    if let Some(grants) = doc.get("grant_types").and_then(Value::as_array)
        && !grants.iter().any(|g| g.as_str() == Some("authorization_code"))
    {
        return Err("grant_types must include authorization_code".to_owned());
    }
    if let Some(types) = doc.get("response_types").and_then(Value::as_array)
        && !types.iter().any(|t| t.as_str() == Some("code"))
    {
        return Err("response_types must include code".to_owned());
    }
    Ok(())
}

fn client_name_of(doc: &Value, fallback: &str) -> String {
    doc.get("client_name")
        .and_then(Value::as_str)
        .map(|n| n.chars().take(MAX_CLIENT_NAME_BYTES).collect::<String>())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// Validates a fetched Client ID Metadata Document.
pub fn parse_client_metadata(client_id: &str, body: &[u8]) -> Result<ClientInfo, String> {
    if body.len() > MAX_CLIENT_METADATA_BYTES {
        return Err("client metadata document is too large".to_owned());
    }
    let doc: Value = serde_json::from_slice(body).map_err(|e| format!("client metadata is not JSON: {e}"))?;
    if doc.get("client_id").and_then(Value::as_str) != Some(client_id) {
        return Err("client metadata `client_id` does not equal the document URL".to_owned());
    }
    check_public_client(&doc)?;
    let redirect_uris = parse_redirect_uris(doc.get("redirect_uris").unwrap_or(&Value::Null))?;
    Ok(ClientInfo {
        client_id: client_id.to_owned(),
        client_name: client_name_of(&doc, &redirect_host(client_id)),
        redirect_uris,
    })
}

/// A validated RFC 7591 registration request (public clients only, Decision 24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

pub fn parse_registration(body: &[u8]) -> Result<Registration, OAuthError> {
    let bad = |error: &'static str, d: String| OAuthError::new(400, error, d);
    if body.len() > MAX_CLIENT_METADATA_BYTES {
        return Err(bad("invalid_client_metadata", "registration request is too large".to_owned()));
    }
    let doc: Value = serde_json::from_slice(body)
        .map_err(|e| bad("invalid_client_metadata", format!("registration request is not JSON: {e}")))?;
    if !doc.is_object() {
        return Err(bad("invalid_client_metadata", "registration request must be a JSON object".to_owned()));
    }
    check_public_client(&doc).map_err(|e| bad("invalid_client_metadata", e))?;
    let redirect_uris = parse_redirect_uris(doc.get("redirect_uris").unwrap_or(&Value::Null))
        .map_err(|e| bad("invalid_redirect_uri", e))?;
    Ok(Registration {
        client_name: client_name_of(&doc, "Unknown client"),
        redirect_uris,
    })
}

/// RFC 7591 §3.2.1 response.
pub fn registration_response(client_id: &str, client_name: &str, redirect_uris: &[String], issued_at: i64) -> Value {
    json!({
        "client_id": client_id,
        "client_id_issued_at": issued_at,
        "client_name": client_name,
        "redirect_uris": redirect_uris,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    })
}

// ---------------------------------------------------------------------------------------
// Authorization request
// ---------------------------------------------------------------------------------------

/// A syntactically and semantically valid authorization request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidAuthorize {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizeFailure {
    /// The client or redirect URI is unusable: show an error page, never redirect.
    Page(String),
    /// Redirect to the (validated) redirect URI with an error.
    Redirect {
        redirect_uri: String,
        state: Option<String>,
        error: &'static str,
        description: String,
    },
}

/// Validates authorize parameters against the client's registered redirect URIs.
pub fn validate_authorize(
    params: &HashMap<String, String>,
    registered_redirects: &[String],
    mcp_url: &str,
) -> Result<ValidAuthorize, AuthorizeFailure> {
    let get = |k: &str| params.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let Some(client_id) = get("client_id") else {
        return Err(AuthorizeFailure::Page("Missing client_id".to_owned()));
    };
    let Some(redirect_uri) = get("redirect_uri") else {
        return Err(AuthorizeFailure::Page("Missing redirect_uri".to_owned()));
    };
    if !redirect_uri_matches(registered_redirects, redirect_uri) {
        return Err(AuthorizeFailure::Page("The redirect_uri is not registered for this client".to_owned()));
    }
    let state = get("state").map(str::to_owned);
    let fail = |error: &'static str, description: &str| AuthorizeFailure::Redirect {
        redirect_uri: redirect_uri.to_owned(),
        state: state.clone(),
        error,
        description: description.to_owned(),
    };
    if state.as_deref().is_some_and(|s| s.len() > MAX_STATE_BYTES) {
        return Err(fail("invalid_request", "state is too long"));
    }
    if get("response_type") != Some("code") {
        return Err(fail("unsupported_response_type", "only response_type=code is supported"));
    }
    if get("code_challenge_method") != Some("S256") {
        return Err(fail("invalid_request", "PKCE with code_challenge_method=S256 is required"));
    }
    let Some(code_challenge) = get("code_challenge").filter(|c| is_valid_code_challenge(c)) else {
        return Err(fail("invalid_request", "code_challenge is missing or malformed"));
    };
    if get("resource").is_some_and(|r| canonicalize_resource(r) != mcp_url) {
        return Err(fail("invalid_target", "resource must be the Rewarden MCP URL"));
    }
    Ok(ValidAuthorize {
        client_id: client_id.to_owned(),
        redirect_uri: redirect_uri.to_owned(),
        code_challenge: code_challenge.to_owned(),
        state,
    })
}

// ---------------------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------------------

pub fn parse_form(body: &[u8]) -> HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

/// Parameters bound to an authorization code when it is issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCode {
    pub user_uuid: String,
    pub connection_uuid: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenGrant {
    AuthorizationCode {
        code: String,
        redirect_uri: String,
        client_id: String,
        code_verifier: String,
    },
    RefreshToken {
        refresh_token: String,
        client_id: Option<String>,
    },
}

pub fn parse_token_request(form: &HashMap<String, String>, mcp_url: &str) -> Result<TokenGrant, OAuthError> {
    let get = |k: &str| form.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let require =
        |k: &str| get(k).map(str::to_owned).ok_or_else(|| OAuthError::invalid_request(format!("missing {k}")));
    if get("resource").is_some_and(|r| canonicalize_resource(r) != mcp_url) {
        return Err(OAuthError::new(400, "invalid_target", "resource must be the Rewarden MCP URL"));
    }
    match get("grant_type") {
        Some("authorization_code") => Ok(TokenGrant::AuthorizationCode {
            code: require("code")?,
            redirect_uri: require("redirect_uri")?,
            client_id: require("client_id")?,
            code_verifier: require("code_verifier")?,
        }),
        Some("refresh_token") => Ok(TokenGrant::RefreshToken {
            refresh_token: require("refresh_token")?,
            client_id: get("client_id").map(str::to_owned),
        }),
        Some(other) => Err(OAuthError::new(400, "unsupported_grant_type", format!("unsupported grant_type `{other}`"))),
        None => Err(OAuthError::invalid_request("missing grant_type")),
    }
}

/// Checks a presented code exchange against what the code was issued for.
pub fn check_code_redemption(
    code: &AuthCode,
    client_id: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<(), OAuthError> {
    if code.client_id != client_id || code.redirect_uri != redirect_uri {
        return Err(OAuthError::invalid_grant("authorization code was issued for a different client or redirect_uri"));
    }
    if !verify_pkce(code_verifier, &code.code_challenge) {
        return Err(OAuthError::invalid_grant("PKCE verification failed"));
    }
    Ok(())
}

pub fn token_response(access_token: &str, refresh_token: &str) -> Value {
    json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": ACCESS_TOKEN_SECS,
        "refresh_token": refresh_token,
        "scope": SCOPE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "https://rw.example.com";
    const MCP: &str = "https://rw.example.com/mcp";
    // RFC 7636 appendix B
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn redirects() -> Vec<String> {
        vec!["https://claude.ai/api/mcp/auth_callback".to_owned(), "http://localhost/callback".to_owned()]
    }

    fn good_authorize() -> HashMap<String, String> {
        params(&[
            ("client_id", "https://claude.ai/oauth/client.json"),
            ("redirect_uri", "https://claude.ai/api/mcp/auth_callback"),
            ("response_type", "code"),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "S256"),
            ("state", "xyz"),
            ("resource", "https://RW.example.com/mcp/"),
        ])
    }

    #[test]
    fn resource_and_metadata_documents() {
        assert_eq!(canonical_mcp_url("https://RW.example.com/"), MCP);
        assert_eq!(canonical_mcp_url("http://127.0.0.1:8000/vw"), "http://127.0.0.1:8000/vw/mcp");
        assert_eq!(canonicalize_resource("HTTPS://RW.Example.com/mcp/"), MCP);
        let prm = protected_resource_metadata(DOMAIN);
        assert_eq!(prm["resource"], MCP);
        assert_eq!(prm["authorization_servers"], json!([DOMAIN]));
        let asm = authorization_server_metadata("https://rw.example.com/");
        assert_eq!(asm["issuer"], DOMAIN);
        assert_eq!(asm["token_endpoint"], "https://rw.example.com/rewarden/oauth/token");
        assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(asm["token_endpoint_auth_methods_supported"], json!(["none"]));
        assert_eq!(asm["client_id_metadata_document_supported"], json!(true));
        assert_eq!(asm["authorization_response_iss_parameter_supported"], json!(true));
    }

    #[test]
    fn pkce_matches_the_rfc_vector_and_rejects_the_rest() {
        assert_eq!(pkce_s256_challenge(VERIFIER), CHALLENGE);
        assert!(verify_pkce(VERIFIER, CHALLENGE));
        assert!(!verify_pkce("x".repeat(43).as_str(), CHALLENGE));
        assert!(!verify_pkce(&VERIFIER.replace('d', "e"), CHALLENGE));
        assert!(!verify_pkce("short", &pkce_s256_challenge("short")), "verifier below 43 characters");
        assert!(!is_valid_code_verifier(&format!("{VERIFIER}!")));
        assert!(!is_valid_code_verifier(&"a".repeat(129)));
        assert!(is_valid_code_challenge(CHALLENGE));
        assert!(!is_valid_code_challenge("abc"));
    }

    #[test]
    fn redirect_uri_syntax() {
        assert!(parse_redirect_uri("https://chatgpt.com/connector_platform_oauth_redirect").is_ok());
        assert!(parse_redirect_uri("http://localhost:8080/callback").is_ok());
        assert!(parse_redirect_uri("http://127.0.0.1/cb").is_ok());
        assert!(parse_redirect_uri("http://[::1]:9/cb").is_ok());
        for bad in [
            "http://evil.example/cb",
            "https://a.example/cb#frag",
            "https://user:pw@a.example/cb",
            "javascript:alert(1)",
            "myapp://cb",
            "not a url",
        ] {
            assert!(parse_redirect_uri(bad).is_err(), "{bad}");
        }
        assert!(parse_redirect_uri(&format!("https://a.example/{}", "x".repeat(3000))).is_err());
    }

    #[test]
    fn redirect_matching_is_exact_except_for_loopback_ports() {
        let r = redirects();
        assert!(redirect_uri_matches(&r, "https://claude.ai/api/mcp/auth_callback"));
        assert!(!redirect_uri_matches(&r, "https://claude.ai/api/mcp/auth_callback/"));
        assert!(!redirect_uri_matches(&r, "https://claude.ai/api/mcp/auth_callback?x=1"));
        assert!(!redirect_uri_matches(&r, "https://evil.claude.ai/api/mcp/auth_callback"));
        assert!(redirect_uri_matches(&r, "http://localhost:53682/callback"));
        assert!(!redirect_uri_matches(&r, "http://localhost:53682/other"));
        assert!(!redirect_uri_matches(&r, "http://127.0.0.1:53682/callback"), "host must match too");
        assert!(!redirect_uri_matches(&r, "https://localhost:53682/callback"));
        assert!(!redirect_uri_matches(&r, "not a url"));
        assert!(!redirect_uri_matches(&[], "https://claude.ai/api/mcp/auth_callback"));
        // A non-loopback registration never gets port latitude.
        assert!(!redirect_uri_matches(&["https://a.example:1/cb".to_owned()], "https://a.example:2/cb"));
    }

    #[test]
    fn redirects_carry_state_and_iss() {
        let ok = success_redirect("https://claude.ai/cb?x=1", "CODE", Some("s t"), DOMAIN);
        let url = Url::parse(&ok).unwrap();
        let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!((q["x"].as_str(), q["code"].as_str(), q["state"].as_str()), ("1", "CODE", "s t"));
        assert_eq!(q["iss"], DOMAIN);
        let err = error_redirect("https://claude.ai/cb", "access_denied", "no", None, DOMAIN);
        let q: HashMap<_, _> = Url::parse(&err).unwrap().query_pairs().into_owned().collect();
        assert_eq!((q["error"].as_str(), q["error_description"].as_str()), ("access_denied", "no"));
        assert!(!q.contains_key("state") && !q.contains_key("code"));
        assert_eq!(redirect_host("https://chatgpt.com/x"), "chatgpt.com");
    }

    #[test]
    fn cimd_documents_are_validated() {
        let id = "https://chatgpt.com/oauth/client.json";
        let doc = json!({"client_id": id, "client_name": "ChatGPT", "redirect_uris": ["https://chatgpt.com/cb"],
            "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"]});
        let info = parse_client_metadata(id, doc.to_string().as_bytes()).unwrap();
        assert_eq!((info.client_name.as_str(), info.redirect_uris.len()), ("ChatGPT", 1));
        let mismatched = json!({"client_id": "https://evil.example/x", "redirect_uris": ["https://chatgpt.com/cb"]});
        assert!(parse_client_metadata(id, mismatched.to_string().as_bytes()).is_err());
        let secret = json!({"client_id": id, "redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "client_secret_basic"});
        assert!(parse_client_metadata(id, secret.to_string().as_bytes()).is_err());
        let http = json!({"client_id": id, "redirect_uris": ["http://a.example/cb"]});
        assert!(parse_client_metadata(id, http.to_string().as_bytes()).is_err());
        let none = json!({"client_id": id});
        assert!(parse_client_metadata(id, none.to_string().as_bytes()).is_err());
        assert!(parse_client_metadata(id, b"nope").is_err());
        assert!(parse_client_metadata(id, &vec![b' '; MAX_CLIENT_METADATA_BYTES + 1]).is_err());
        let nameless = json!({"client_id": id, "redirect_uris": ["https://a.example/cb"]});
        assert_eq!(parse_client_metadata(id, nameless.to_string().as_bytes()).unwrap().client_name, "chatgpt.com");
        // The document Claude actually publishes (fetched 2026-09-29), including its extra JWT-bearer grant type.
        let claude_id = "https://claude.ai/oauth/mcp-oauth-client-metadata";
        let claude = br#"{"client_id":"https://claude.ai/oauth/mcp-oauth-client-metadata","client_name":"Claude","client_uri":"https://claude.ai","redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"grant_types":["authorization_code","refresh_token","urn:ietf:params:oauth:grant-type:jwt-bearer"],"response_types":["code"],"token_endpoint_auth_method":"none"}"#;
        let claude = parse_client_metadata(claude_id, claude).unwrap();
        assert_eq!((claude.client_name.as_str(), claude.redirect_uris.len()), ("Claude", 1));
        assert!(is_cimd_client_id(id));
        assert!(!is_cimd_client_id("https://chatgpt.com/"));
        assert!(!is_cimd_client_id("http://chatgpt.com/x"));
        assert!(!is_cimd_client_id("3f2a-plain-uuid"));
    }

    #[test]
    fn dynamic_registration_accepts_only_public_clients() {
        let ok = json!({"client_name": "Claude", "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"],
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
            "token_endpoint_auth_method": "none", "application_type": "web"});
        let reg = parse_registration(ok.to_string().as_bytes()).unwrap();
        assert_eq!(reg.client_name, "Claude");
        let err = |v: Value| parse_registration(v.to_string().as_bytes()).unwrap_err();
        assert_eq!(err(json!({"redirect_uris": []})).error, "invalid_redirect_uri");
        assert_eq!(err(json!({"redirect_uris": ["http://evil.example/cb"]})).error, "invalid_redirect_uri");
        assert_eq!(
            err(json!({"redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "client_secret_post"}))
                .error,
            "invalid_client_metadata"
        );
        assert_eq!(
            err(json!({"redirect_uris": ["https://a.example/cb"], "grant_types": ["password"]})).error,
            "invalid_client_metadata"
        );
        assert_eq!(
            err(json!({"redirect_uris": ["https://a.example/cb"], "response_types": ["token"]})).error,
            "invalid_client_metadata"
        );
        // Extra grant types a client merely lists are ignored (this is what Claude's metadata document looks like).
        assert!(
            parse_registration(
                json!({"redirect_uris": ["https://a.example/cb"], "response_types": ["code"], "token_endpoint_auth_method": "none",
                    "grant_types": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:jwt-bearer"]})
                .to_string()
                .as_bytes()
            )
            .is_ok()
        );
        assert_eq!(parse_registration(b"[]").unwrap_err().error, "invalid_client_metadata");
        let many: Vec<String> = (0..=MAX_REDIRECT_URIS).map(|i| format!("https://a.example/{i}")).collect();
        assert_eq!(err(json!({"redirect_uris": many})).error, "invalid_redirect_uri");
        let resp = registration_response("cid", "Claude", &redirects(), 5);
        assert_eq!(
            (resp["client_id"].as_str(), resp["token_endpoint_auth_method"].as_str()),
            (Some("cid"), Some("none"))
        );
    }

    #[test]
    fn authorize_requests_are_validated_in_order() {
        let v = validate_authorize(&good_authorize(), &["https://claude.ai/api/mcp/auth_callback".to_owned()], MCP)
            .unwrap();
        assert_eq!((v.state.as_deref(), v.code_challenge.as_str()), (Some("xyz"), CHALLENGE));

        let page = |mut p: HashMap<String, String>, key: &str, val: Option<&str>| {
            match val {
                Some(v) => p.insert(key.to_owned(), v.to_owned()),
                None => p.remove(key),
            };
            validate_authorize(&p, &["https://claude.ai/api/mcp/auth_callback".to_owned()], MCP).unwrap_err()
        };
        // Problems with the client or redirect URI never redirect.
        assert!(matches!(page(good_authorize(), "client_id", None), AuthorizeFailure::Page(_)));
        assert!(matches!(page(good_authorize(), "redirect_uri", None), AuthorizeFailure::Page(_)));
        assert!(matches!(
            page(good_authorize(), "redirect_uri", Some("https://evil.example/cb")),
            AuthorizeFailure::Page(_)
        ));
        // Later problems redirect to the validated URI, keeping state.
        let redirect_error = |f: AuthorizeFailure| match f {
            AuthorizeFailure::Redirect {
                error,
                state,
                ..
            } => (error, state),
            AuthorizeFailure::Page(m) => panic!("expected a redirect, got page {m}"),
        };
        assert_eq!(
            redirect_error(page(good_authorize(), "response_type", Some("token"))),
            ("unsupported_response_type", Some("xyz".to_owned()))
        );
        assert_eq!(redirect_error(page(good_authorize(), "code_challenge_method", Some("plain"))).0, "invalid_request");
        assert_eq!(redirect_error(page(good_authorize(), "code_challenge_method", None)).0, "invalid_request");
        assert_eq!(redirect_error(page(good_authorize(), "code_challenge", Some("short"))).0, "invalid_request");
        assert_eq!(redirect_error(page(good_authorize(), "code_challenge", None)).0, "invalid_request");
        assert_eq!(
            redirect_error(page(good_authorize(), "resource", Some("https://other.example/mcp"))).0,
            "invalid_target"
        );
        assert_eq!(redirect_error(page(good_authorize(), "state", Some(&"s".repeat(600)))).0, "invalid_request");
        // `resource` and `state` are optional.
        let mut p = good_authorize();
        p.remove("resource");
        p.remove("state");
        assert_eq!(
            validate_authorize(&p, &["https://claude.ai/api/mcp/auth_callback".to_owned()], MCP).unwrap().state,
            None
        );
    }

    #[test]
    fn token_requests_are_parsed() {
        let form = parse_form(b"grant_type=authorization_code&code=C&redirect_uri=https%3A%2F%2Fa.example%2Fcb&client_id=cid&code_verifier=V&resource=https%3A%2F%2Frw.example.com%2Fmcp");
        assert_eq!(
            parse_token_request(&form, MCP).unwrap(),
            TokenGrant::AuthorizationCode {
                code: "C".into(),
                redirect_uri: "https://a.example/cb".into(),
                client_id: "cid".into(),
                code_verifier: "V".into()
            }
        );
        let refresh = parse_form(b"grant_type=refresh_token&refresh_token=R");
        assert_eq!(
            parse_token_request(&refresh, MCP).unwrap(),
            TokenGrant::RefreshToken {
                refresh_token: "R".into(),
                client_id: None
            }
        );
        let e = |body: &[u8]| parse_token_request(&parse_form(body), MCP).unwrap_err().error;
        assert_eq!(e(b""), "invalid_request");
        assert_eq!(e(b"grant_type=password"), "unsupported_grant_type");
        assert_eq!(e(b"grant_type=authorization_code&code=C"), "invalid_request");
        assert_eq!(e(b"grant_type=refresh_token"), "invalid_request");
        assert_eq!(
            e(b"grant_type=refresh_token&refresh_token=R&resource=https%3A%2F%2Fother.example%2Fmcp"),
            "invalid_target"
        );
    }

    #[test]
    fn code_redemption_binds_client_redirect_and_pkce() {
        let code = AuthCode {
            user_uuid: "u".into(),
            connection_uuid: "c".into(),
            client_id: "cid".into(),
            redirect_uri: "https://a.example/cb".into(),
            code_challenge: CHALLENGE.into(),
        };
        assert!(check_code_redemption(&code, "cid", "https://a.example/cb", VERIFIER).is_ok());
        for (client, redirect, verifier) in [
            ("other", "https://a.example/cb", VERIFIER),
            ("cid", "https://a.example/other", VERIFIER),
            ("cid", "https://a.example/cb", "wrong-verifier-wrong-verifier-wrong-verifier-1"),
        ] {
            assert_eq!(check_code_redemption(&code, client, redirect, verifier).unwrap_err().error, "invalid_grant");
        }
    }

    #[test]
    fn token_response_shape() {
        let t = token_response("AT", "RT");
        assert_eq!(t["token_type"], "Bearer");
        assert_eq!(t["expires_in"], 3600);
        assert_eq!(
            (t["access_token"].as_str(), t["refresh_token"].as_str(), t["scope"].as_str()),
            (Some("AT"), Some("RT"), Some("mcp"))
        );
        assert_eq!(
            OAuthError::invalid_grant("x").to_json(),
            json!({"error": "invalid_grant", "error_description": "x"})
        );
    }
}
