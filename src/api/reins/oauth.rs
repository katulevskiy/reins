//! OAuth 2.1 authorization-server logic for AI clients (spec §4.5) as pure functions:
//! metadata documents, PKCE, redirect-URI matching, client metadata (CIMD/DCR) and
//! request validation. Handlers in `oauth_routes.rs` only adapt these to HTTP.

use std::collections::HashMap;

use data_encoding::BASE64URL_NOPAD;
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation,
    jwk::{AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ACCESS_TOKEN_SECS, device_flow::DEVICE_CODE_GRANT};
use crate::crypto::ct_eq;

pub const SCOPE: &str = "mcp";
pub const MAX_REDIRECT_URIS: usize = 10;
pub const MAX_REDIRECT_URI_BYTES: usize = 2048;
pub const MAX_STATE_BYTES: usize = 512;
/// CIMD documents larger than this are rejected (Decision 25).
pub const MAX_CLIENT_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_CLIENT_NAME_BYTES: usize = 256;
/// A client's JSON Web Key Set (inline or at `jwks_uri`) larger than this is rejected.
pub const MAX_JWKS_BYTES: usize = 64 * 1024;
/// RFC 7523 §2.2 client assertion type.
pub const JWT_BEARER_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
/// A client assertion may expire at most this far ahead (plus [`ASSERTION_SKEW_SECS`]).
pub const MAX_ASSERTION_LIFETIME_SECS: i64 = 300;
/// Clock skew tolerated on `exp`, `nbf` and `iat`.
pub const ASSERTION_SKEW_SECS: i64 = 60;
pub const MAX_ASSERTION_BYTES: usize = 8 * 1024;
pub const MAX_JTI_BYTES: usize = 256;
/// Smallest RSA modulus accepted for client assertions.
pub const MIN_RSA_BITS: usize = 2048;
/// Signature algorithms accepted for `private_key_jwt` (never `none`, never HMAC).
pub const CLIENT_ASSERTION_ALGS: [Algorithm; 4] =
    [Algorithm::RS256, Algorithm::PS256, Algorithm::ES256, Algorithm::EdDSA];

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
        "resource_name": "Reins"
    })
}

/// RFC 8414 authorization-server metadata.
pub fn authorization_server_metadata(domain: &str) -> Value {
    let iss = issuer(domain);
    json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/reins/oauth/authorize"),
        "token_endpoint": format!("{iss}/reins/oauth/token"),
        "registration_endpoint": format!("{iss}/reins/oauth/register"),
        "revocation_endpoint": format!("{iss}/reins/oauth/revoke"),
        "revocation_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
        "revocation_endpoint_auth_signing_alg_values_supported": assertion_alg_names(),
        "device_authorization_endpoint": format!("{iss}/reins/oauth/device_authorization"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token", DEVICE_CODE_GRANT],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
        "token_endpoint_auth_signing_alg_values_supported": assertion_alg_names(),
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

    /// RFC 6749 §5.2: client authentication failed, was missing, or was of the wrong kind.
    pub fn invalid_client(description: impl Into<String>) -> Self {
        Self::new(401, "invalid_client", description)
    }

    pub fn to_json(&self) -> Value {
        json!({"error": self.error, "error_description": self.description})
    }
}

// ---------------------------------------------------------------------------------------
// Client metadata: CIMD documents and dynamic registration
// ---------------------------------------------------------------------------------------

/// How a client authenticates at the token endpoint. Ordered: a later method is stronger, and the method bound to a
/// code or refresh token is never weakened (`max` of bound and current).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthMethod {
    /// A public client: PKCE, no client authentication.
    None,
    /// RFC 7523 client assertion signed with a key from the client's JWKS.
    PrivateKeyJwt,
}

impl AuthMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PrivateKeyJwt => "private_key_jwt",
        }
    }

    /// The stored form; anything unknown is treated as the strongest method, so a bad row fails closed.
    pub fn from_stored(stored: &str) -> Self {
        match stored {
            "none" => Self::None,
            _ => Self::PrivateKeyJwt,
        }
    }
}

/// Where a `private_key_jwt` client publishes its public keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwksSource {
    /// `jwks_uri`: fetched through the outbound guard and cached briefly.
    Uri(String),
    /// `jwks` in the metadata document itself.
    Inline(JwkSet),
}

/// A client's token endpoint authentication, as its metadata declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAuth {
    None,
    PrivateKeyJwt {
        jwks: JwksSource,
        /// `token_endpoint_auth_signing_alg`, when the client pins one.
        alg: Option<Algorithm>,
    },
}

impl ClientAuth {
    pub const fn method(&self) -> AuthMethod {
        match self {
            Self::None => AuthMethod::None,
            Self::PrivateKeyJwt {
                ..
            } => AuthMethod::PrivateKeyJwt,
        }
    }
}

/// What the server needs to know about an OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub auth: ClientAuth,
}

/// A `client_id` that is a fetchable metadata-document URL (Decision 25).
pub fn is_cimd_client_id(client_id: &str) -> bool {
    Url::parse(client_id)
        .is_ok_and(|u| u.scheme() == "https" && u.host().is_some() && u.path() != "/" && u.fragment().is_none())
}

/// Test mode only (`REINS_TEST_ALLOW_LOOPBACK`): an `http://` metadata document on a loopback host, so the
/// integration tests can serve one from a mock server.
pub fn is_loopback_cimd_client_id(client_id: &str) -> bool {
    Url::parse(client_id)
        .is_ok_and(|u| u.scheme() == "http" && is_loopback_host(&u) && u.path() != "/" && u.fragment().is_none())
}

fn assertion_alg_names() -> Vec<&'static str> {
    CLIENT_ASSERTION_ALGS.iter().map(|a| alg_name(*a)).collect()
}

const fn alg_name(alg: Algorithm) -> &'static str {
    match alg {
        Algorithm::RS256 => "RS256",
        Algorithm::PS256 => "PS256",
        Algorithm::ES256 => "ES256",
        Algorithm::EdDSA => "EdDSA",
        _ => "unsupported",
    }
}

fn assertion_alg(name: &str) -> Option<Algorithm> {
    CLIENT_ASSERTION_ALGS.iter().copied().find(|a| alg_name(*a) == name)
}

/// `jwks_uri`: https (http only on loopback, which the outbound guard refuses outside the test mode), no fragment,
/// no credentials, bounded length. Where it may point is checked again, address by address, when it is fetched.
fn parse_jwks_uri(raw: &str) -> Result<String, String> {
    parse_redirect_uri(raw).map(|_| raw.to_owned()).map_err(|e| e.replace("redirect URI", "jwks_uri"))
}

/// Parses a JWKS document: `{"keys": [...]}` with at least one key.
pub fn parse_jwks(body: &[u8]) -> Result<JwkSet, String> {
    if body.len() > MAX_JWKS_BYTES {
        return Err("the JWKS is too large".to_owned());
    }
    let set: JwkSet = serde_json::from_slice(body).map_err(|e| format!("the JWKS is not valid: {e}"))?;
    if set.keys.is_empty() {
        return Err("the JWKS has no keys".to_owned());
    }
    Ok(set)
}

/// Decides how a client authenticates, from `token_endpoint_auth_method` (its preference) and, as MCP SEP-3149
/// describes, `token_endpoint_auth_methods_supported` (everything it can do). The preference wins when this server
/// supports it; otherwise the strongest supported method the client lists. `private_key_jwt` needs keys
/// (`jwks_uri` or `jwks`, not both) and, if `token_endpoint_auth_signing_alg` is given, an accepted algorithm.
/// `allow_private_key_jwt` is false for dynamic registration, which stays public-only.
fn parse_client_auth(doc: &Value, allow_private_key_jwt: bool) -> Result<ClientAuth, String> {
    let preferred = match doc.get("token_endpoint_auth_method") {
        None | Some(Value::Null) => None,
        Some(Value::String(m)) => Some(m.as_str()),
        Some(_) => return Err("token_endpoint_auth_method must be a string".to_owned()),
    };
    let listed: Option<Vec<&str>> = match doc.get("token_endpoint_auth_methods_supported") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => Some(list.iter().filter_map(Value::as_str).collect()),
        Some(_) => return Err("token_endpoint_auth_methods_supported must be an array".to_owned()),
    };
    let supported = |m: &str| m == "none" || (allow_private_key_jwt && m == "private_key_jwt");
    let lists = |m: &str| listed.as_ref().is_some_and(|l| l.contains(&m));
    let chosen = match preferred {
        Some(m) if supported(m) => m,
        _ if allow_private_key_jwt && lists("private_key_jwt") => "private_key_jwt",
        _ if lists("none") => "none",
        None if listed.is_none() => "none",
        Some("private_key_jwt") => {
            return Err("unsupported token_endpoint_auth_method `private_key_jwt`: dynamic registration accepts \
                        only public clients (`none`); publish a client ID metadata document instead"
                .to_owned());
        }
        Some(other) => return Err(format!("unsupported token_endpoint_auth_method `{other}`")),
        None => return Err("token_endpoint_auth_methods_supported lists no method this server supports".to_owned()),
    };
    if chosen == "none" {
        return Ok(ClientAuth::None);
    }
    let jwks = match (doc.get("jwks_uri"), doc.get("jwks")) {
        (Some(_), Some(_)) => return Err("a client must not declare both jwks_uri and jwks".to_owned()),
        (Some(uri), None) => JwksSource::Uri(parse_jwks_uri(uri.as_str().ok_or("jwks_uri must be a string")?)?),
        (None, Some(inline)) => JwksSource::Inline(parse_jwks(inline.to_string().as_bytes())?),
        (None, None) => return Err("private_key_jwt requires jwks_uri or jwks".to_owned()),
    };
    let alg = match doc.get("token_endpoint_auth_signing_alg") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => {
            Some(assertion_alg(name).ok_or_else(|| format!("unsupported token_endpoint_auth_signing_alg `{name}`"))?)
        }
        Some(_) => return Err("token_endpoint_auth_signing_alg must be a string".to_owned()),
    };
    Ok(ClientAuth::PrivateKeyJwt {
        jwks,
        alg,
    })
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

fn check_client(doc: &Value, allow_private_key_jwt: bool) -> Result<ClientAuth, String> {
    let auth = parse_client_auth(doc, allow_private_key_jwt)?;
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
    Ok(auth)
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
    let auth = check_client(&doc, true)?;
    let redirect_uris = parse_redirect_uris(doc.get("redirect_uris").unwrap_or(&Value::Null))?;
    Ok(ClientInfo {
        client_id: client_id.to_owned(),
        client_name: client_name_of(&doc, &redirect_host(client_id)),
        redirect_uris,
        auth,
    })
}

/// A validated RFC 7591 registration request (public clients only, Decision 24). `private_key_jwt` clients use a
/// client ID metadata document: registering keys would need a stored `jwks_uri` per client and buys nothing over it.
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
    check_client(&doc, false).map_err(|e| bad("invalid_client_metadata", e))?;
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
        return Err(fail("invalid_target", "resource must be the Reins MCP URL"));
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
    /// How the client authenticated when it was resolved at authorize time; the exchange must use at least this.
    pub client_auth: AuthMethod,
}

/// A parsed token request: the grant and the client's RFC 7523 assertion, if it sent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub grant: TokenGrant,
    pub client_assertion: Option<String>,
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
    /// RFC 8628 §3.4: the desktop app polls with its device code.
    DeviceCode {
        device_code: String,
        client_id: String,
    },
}

/// The RFC 7523 client authentication parameters of a token or revocation request: `None` when the client sent
/// none, the assertion when it sent a JWT bearer one. Client secrets and other assertion types are refused.
pub fn parse_client_assertion(form: &HashMap<String, String>) -> Result<Option<String>, OAuthError> {
    let get = |k: &str| form.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if get("client_secret").is_some() {
        return Err(OAuthError::invalid_client("client secrets are not supported"));
    }
    match (get("client_assertion_type"), get("client_assertion")) {
        (None, None) => Ok(None),
        (Some(JWT_BEARER_ASSERTION_TYPE), Some(assertion)) if assertion.len() <= MAX_ASSERTION_BYTES => {
            Ok(Some(assertion.to_owned()))
        }
        (Some(JWT_BEARER_ASSERTION_TYPE), Some(_)) => Err(OAuthError::invalid_client("client_assertion is too long")),
        (Some(other), _) if other != JWT_BEARER_ASSERTION_TYPE => {
            Err(OAuthError::invalid_client(format!("unsupported client_assertion_type `{other}`")))
        }
        _ => Err(OAuthError::invalid_request("client_assertion and client_assertion_type must be sent together")),
    }
}

/// The `sub` of an assertion, unverified: only to know which client claims to speak when the request has no
/// `client_id` (RFC 7523 §3). The assertion is verified against that client before anything is trusted.
fn unverified_subject(assertion: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Sub {
        sub: Option<String>,
    }
    jsonwebtoken::dangerous::insecure_decode::<Sub>(assertion).ok()?.claims.sub.filter(|s| !s.is_empty())
}

pub fn parse_token_request(form: &HashMap<String, String>, mcp_url: &str) -> Result<TokenRequest, OAuthError> {
    let get = |k: &str| form.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let require =
        |k: &str| get(k).map(str::to_owned).ok_or_else(|| OAuthError::invalid_request(format!("missing {k}")));
    if get("resource").is_some_and(|r| canonicalize_resource(r) != mcp_url) {
        return Err(OAuthError::new(400, "invalid_target", "resource must be the Reins MCP URL"));
    }
    let client_assertion = parse_client_assertion(form)?;
    let grant = match get("grant_type") {
        Some("authorization_code") => TokenGrant::AuthorizationCode {
            code: require("code")?,
            redirect_uri: require("redirect_uri")?,
            client_id: match (get("client_id"), client_assertion.as_deref()) {
                (Some(id), _) => id.to_owned(),
                (None, Some(assertion)) => unverified_subject(assertion)
                    .ok_or_else(|| OAuthError::invalid_client("the client assertion names no client"))?,
                (None, None) => return Err(OAuthError::invalid_request("missing client_id")),
            },
            code_verifier: require("code_verifier")?,
        },
        Some("refresh_token") => TokenGrant::RefreshToken {
            refresh_token: require("refresh_token")?,
            client_id: get("client_id").map(str::to_owned),
        },
        Some(DEVICE_CODE_GRANT) => TokenGrant::DeviceCode {
            device_code: require("device_code")?,
            client_id: require("client_id")?,
        },
        Some(other) => {
            return Err(OAuthError::new(400, "unsupported_grant_type", format!("unsupported grant_type `{other}`")));
        }
        None => return Err(OAuthError::invalid_request("missing grant_type")),
    };
    Ok(TokenRequest {
        grant,
        client_assertion,
    })
}

/// A valid RFC 8628 device authorization request: the client, and the desktop app's key when it sent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub client_id: String,
    pub client_key: Option<String>,
}

/// Validates `POST /reins/oauth/device_authorization` (form-encoded): `client_id` is required; `scope`, when given,
/// must be `mcp`; `resource`, when given, the MCP URL; `key_param` (`reins_client_key`), when given, a valid
/// desktop app key: refused rather than dropped, as in the authorize flow.
pub fn parse_device_authorization(
    form: &HashMap<String, String>,
    mcp_url: &str,
    key_param: &str,
) -> Result<DeviceAuthorization, OAuthError> {
    let get = |k: &str| form.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let client_id = get("client_id").ok_or_else(|| OAuthError::invalid_request("missing client_id"))?;
    if get("scope").is_some_and(|scope| scope.split(' ').any(|s| !s.is_empty() && s != SCOPE)) {
        return Err(OAuthError::new(400, "invalid_scope", format!("the only scope is `{SCOPE}`")));
    }
    if get("resource").is_some_and(|r| canonicalize_resource(r) != mcp_url) {
        return Err(OAuthError::new(400, "invalid_target", "resource must be the Reins MCP URL"));
    }
    let client_key = match form.get(key_param) {
        None => None,
        Some(key) if reins_proto::desktop::decode_key(key).is_some() => Some(key.clone()),
        Some(_) => return Err(OAuthError::invalid_request("the desktop app key is not valid")),
    };
    Ok(DeviceAuthorization {
        client_id: client_id.to_owned(),
        client_key,
    })
}

/// The pairing page a computer's QR code links to: `{DOMAIN}/pair`, and with `?code=` the code itself.
pub fn pairing_page_url(domain: &str, user_code: Option<&str>) -> String {
    let page = format!("{}/pair", issuer(domain));
    match (user_code, Url::parse(&page)) {
        (Some(code), Ok(mut url)) => {
            url.query_pairs_mut().append_pair("code", code);
            url.into()
        }
        _ => page,
    }
}

/// The RFC 8628 §3.2 answer, with the number the desktop app shows (`reins_confirm_code`), which the user taps on
/// the phone among three.
pub fn device_authorization_response(
    domain: &str,
    device_code: &str,
    user_code: &str,
    confirm_code: u8,
    expires_in: u64,
    interval: u64,
) -> Value {
    json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": pairing_page_url(domain, None),
        "verification_uri_complete": pairing_page_url(domain, Some(user_code)),
        "expires_in": expires_in,
        "interval": interval,
        "reins_confirm_code": confirm_code
    })
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

// ---------------------------------------------------------------------------------------
// Client authentication: private_key_jwt (RFC 7523 §2.2 and §3, RFC 7521 §4.2)
// ---------------------------------------------------------------------------------------

/// Why a client assertion was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssertionError {
    /// No key in the client's JWKS fits the assertion's `kid` and `alg`: the client may have rotated its keys, so a
    /// JWKS from `jwks_uri` is fetched again (once) before giving up.
    UnknownKey,
    Invalid(String),
}

impl AssertionError {
    pub fn into_oauth(self) -> OAuthError {
        match self {
            Self::UnknownKey => OAuthError::invalid_client("no key of the client matches the client assertion"),
            Self::Invalid(why) => OAuthError::invalid_client(why),
        }
    }
}

/// A client assertion whose signature and claims are good. Its `jti` must still be unused until `exp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAssertion {
    pub jti: String,
    pub exp: i64,
}

/// What a client assertion's `aud` must contain one of: the token endpoint URL or the issuer (RFC 7523 §3).
pub fn assertion_audiences(domain: &str) -> Vec<String> {
    let iss = issuer(domain);
    vec![format!("{iss}/reins/oauth/token"), format!("{iss}/"), iss]
}

#[derive(Deserialize)]
struct AssertionClaims {
    iss: Option<String>,
    sub: Option<String>,
    /// A string or a list of strings.
    aud: Option<Value>,
    jti: Option<String>,
    exp: Option<i64>,
    nbf: Option<i64>,
    iat: Option<i64>,
}

/// The claim checks, on claims either not yet verified (the cheap pre-check) or verified.
fn check_claims(
    claims: AssertionClaims,
    client_id: &str,
    audiences: &[String],
    now: i64,
) -> Result<VerifiedAssertion, AssertionError> {
    let invalid = |why: &str| AssertionError::Invalid(why.to_owned());
    match claims.iss.as_deref() {
        None => return Err(invalid("the client assertion has no iss")),
        Some(iss) if iss != client_id => return Err(invalid("the client assertion's iss is not the client_id")),
        Some(_) => {}
    }
    match claims.sub.as_deref() {
        None => return Err(invalid("the client assertion has no sub")),
        Some(sub) if sub != client_id => return Err(invalid("the client assertion's sub is not the client_id")),
        Some(_) => {}
    }
    let names_us = |aud: &Value| aud.as_str().is_some_and(|a| audiences.iter().any(|ours| ours == a));
    match &claims.aud {
        None => return Err(invalid("the client assertion has no aud")),
        Some(Value::Array(list)) if list.iter().any(names_us) => {}
        Some(aud) if names_us(aud) => {}
        Some(_) => return Err(invalid("the client assertion's aud does not name this server")),
    }
    let exp = claims.exp.ok_or_else(|| invalid("the client assertion has no exp"))?;
    if exp.saturating_add(ASSERTION_SKEW_SECS) <= now {
        return Err(invalid("the client assertion has expired"));
    }
    if exp > now + MAX_ASSERTION_LIFETIME_SECS + ASSERTION_SKEW_SECS {
        return Err(invalid("the client assertion expires too far in the future"));
    }
    if claims.nbf.is_some_and(|nbf| nbf > now + ASSERTION_SKEW_SECS) {
        return Err(invalid("the client assertion is not valid yet"));
    }
    if claims.iat.is_some_and(|iat| iat > now + ASSERTION_SKEW_SECS) {
        return Err(invalid("the client assertion was issued in the future"));
    }
    let jti = claims
        .jti
        .filter(|j| !j.is_empty() && j.len() <= MAX_JTI_BYTES)
        .ok_or_else(|| invalid("the client assertion needs a jti"))?;
    Ok(VerifiedAssertion {
        jti,
        exp,
    })
}

/// Everything about an assertion that needs no key: its size, an accepted algorithm, and all claims (`iss` = `sub` =
/// `client_id`, `aud`, times, `jti`), read without checking the signature. Run first, so an assertion that could never
/// pass costs neither a key-set fetch nor a signature check. [`verify_client_assertion`] checks the claims again once
/// the signature has been verified.
pub fn precheck_client_assertion(
    assertion: &str,
    client_id: &str,
    audiences: &[String],
    now: i64,
) -> Result<(), AssertionError> {
    assertion_header(assertion, None)?;
    let claims = jsonwebtoken::dangerous::insecure_decode::<AssertionClaims>(assertion)
        .map_err(|_| AssertionError::Invalid("the client assertion's claims are not valid".to_owned()))?
        .claims;
    check_claims(claims, client_id, audiences, now).map(|_| ())
}

/// The header's algorithm (one of [`CLIENT_ASSERTION_ALGS`], and the client's pinned one if it has one) and `kid`.
/// `alg: none` does not even parse; HMAC and every other algorithm are refused here.
fn assertion_header(assertion: &str, pinned: Option<Algorithm>) -> Result<(Algorithm, Option<String>), AssertionError> {
    let invalid = |why: &str| AssertionError::Invalid(why.to_owned());
    if assertion.len() > MAX_ASSERTION_BYTES {
        return Err(invalid("the client assertion is too long"));
    }
    let header = jsonwebtoken::decode_header(assertion)
        .map_err(|_| invalid("the client assertion is not a JWT signed with an accepted algorithm"))?;
    if !CLIENT_ASSERTION_ALGS.contains(&header.alg) {
        return Err(AssertionError::Invalid(format!(
            "client assertions signed with {:?} are not accepted",
            header.alg
        )));
    }
    if pinned.is_some_and(|p| p != header.alg) {
        return Err(invalid("the client assertion is not signed with the client's token_endpoint_auth_signing_alg"));
    }
    Ok((header.alg, header.kid))
}

fn key_fits(jwk: &Jwk, alg: Algorithm) -> bool {
    let use_ok = jwk.common.public_key_use.as_ref().is_none_or(|u| *u == PublicKeyUse::Signature);
    let ops_ok = jwk.common.key_operations.as_ref().is_none_or(|ops| ops.contains(&KeyOperations::Verify));
    let alg_ok = jwk.common.key_algorithm.as_ref().is_none_or(|a| {
        matches!(
            (a, alg),
            (KeyAlgorithm::RS256, Algorithm::RS256)
                | (KeyAlgorithm::PS256, Algorithm::PS256)
                | (KeyAlgorithm::ES256, Algorithm::ES256)
                | (KeyAlgorithm::EdDSA, Algorithm::EdDSA)
        )
    });
    let kind_ok = match (&jwk.algorithm, alg) {
        (AlgorithmParameters::RSA(_), Algorithm::RS256 | Algorithm::PS256) => true,
        (AlgorithmParameters::EllipticCurve(p), Algorithm::ES256) => p.curve == EllipticCurve::P256,
        (AlgorithmParameters::OctetKeyPair(p), Algorithm::EdDSA) => p.curve == EllipticCurve::Ed25519,
        _ => false,
    };
    use_ok && ops_ok && alg_ok && kind_ok
}

/// The client key an assertion names: the signing key with its `kid`, or, without a `kid`, the only key that fits.
pub fn select_key<'a>(jwks: &'a JwkSet, alg: Algorithm, kid: Option<&str>) -> Option<&'a Jwk> {
    let mut fitting = jwks.keys.iter().filter(|k| key_fits(k, alg));
    if let Some(kid) = kid {
        return fitting.find(|k| k.common.key_id.as_deref() == Some(kid));
    }
    let only = fitting.next()?;
    fitting.next().is_none().then_some(only)
}

fn rsa_modulus_bits(n: &str) -> usize {
    let bytes = BASE64URL_NOPAD.decode(n.trim_end_matches('=').as_bytes()).unwrap_or_default();
    let significant = bytes.iter().skip_while(|b| **b == 0).count();
    significant * 8
}

fn describe_jwt_error(e: &jsonwebtoken::errors::Error) -> AssertionError {
    use jsonwebtoken::errors::ErrorKind;
    let why = match e.kind() {
        ErrorKind::InvalidSignature => "the client assertion's signature is not valid".to_owned(),
        ErrorKind::InvalidAudience => "the client assertion's aud does not name this server".to_owned(),
        ErrorKind::InvalidIssuer => "the client assertion's iss is not the client_id".to_owned(),
        ErrorKind::InvalidSubject => "the client assertion's sub is not the client_id".to_owned(),
        ErrorKind::MissingRequiredClaim(claim) => format!("the client assertion has no {claim}"),
        _ => "the client assertion is not valid".to_owned(),
    };
    AssertionError::Invalid(why)
}

/// Verifies a client assertion for `client_id` against the client's keys: the signature, `iss` == `sub` ==
/// `client_id`, `aud` naming this server, `exp` present and at most [`MAX_ASSERTION_LIFETIME_SECS`] ahead, `nbf` and
/// `iat` not in the future, a `jti`. Times allow [`ASSERTION_SKEW_SECS`] of skew. The caller burns the `jti`.
pub fn verify_client_assertion(
    assertion: &str,
    client_id: &str,
    jwks: &JwkSet,
    pinned_alg: Option<Algorithm>,
    audiences: &[String],
    now: i64,
) -> Result<VerifiedAssertion, AssertionError> {
    let invalid = |why: &str| AssertionError::Invalid(why.to_owned());
    // Cheap rejects first: no key lookup or signature check for an assertion whose claims could never pass.
    precheck_client_assertion(assertion, client_id, audiences, now)?;
    let (alg, kid) = assertion_header(assertion, pinned_alg)?;
    let jwk = select_key(jwks, alg, kid.as_deref()).ok_or(AssertionError::UnknownKey)?;
    if let AlgorithmParameters::RSA(rsa) = &jwk.algorithm
        && rsa_modulus_bits(&rsa.n) < MIN_RSA_BITS
    {
        return Err(invalid("the client's RSA key is shorter than 2048 bits"));
    }
    let key = DecodingKey::from_jwk(jwk).map_err(|_| invalid("the client's key is not usable"))?;
    let mut validation = Validation::new(alg);
    // Times are checked below against `now`, with this server's own limits.
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.leeway = 0;
    validation.set_required_spec_claims(&["iss", "sub", "aud"]);
    validation.set_issuer(&[client_id]);
    validation.sub = Some(client_id.to_owned());
    validation.set_audience(audiences);
    let claims = jsonwebtoken::decode::<AssertionClaims>(assertion, &key, &validation)
        .map_err(|e| describe_jwt_error(&e))?
        .claims;
    check_claims(claims, client_id, audiences, now)
}

/// The method a client must authenticate with now: the strongest of the one bound to its grant, `private_key_jwt`
/// if this client has ever been seen with it (in a metadata document or a stored grant), and what its current
/// metadata document declares. A client can therefore never drop back to `none`.
pub fn required_method(bound: AuthMethod, seen_with_keys: bool, current: Option<AuthMethod>) -> AuthMethod {
    let seen = if seen_with_keys {
        AuthMethod::PrivateKeyJwt
    } else {
        AuthMethod::None
    };
    bound.max(seen).max(current.unwrap_or(AuthMethod::None))
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
        assert_eq!(asm["token_endpoint"], "https://rw.example.com/reins/oauth/token");
        assert_eq!(asm["revocation_endpoint"], "https://rw.example.com/reins/oauth/revoke");
        assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(asm["token_endpoint_auth_methods_supported"], json!(["none", "private_key_jwt"]));
        assert_eq!(
            asm["token_endpoint_auth_signing_alg_values_supported"],
            json!(["RS256", "PS256", "ES256", "EdDSA"])
        );
        assert_eq!(asm["revocation_endpoint_auth_methods_supported"], json!(["none", "private_key_jwt"]));
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
        assert_eq!(claude.auth, ClientAuth::None);
        assert!(is_cimd_client_id(id));
        assert!(!is_cimd_client_id("https://chatgpt.com/"));
        assert!(!is_cimd_client_id("http://chatgpt.com/x"));
        assert!(!is_cimd_client_id("3f2a-plain-uuid"));
        assert!(is_loopback_cimd_client_id("http://127.0.0.1:9/client.json"));
        assert!(!is_loopback_cimd_client_id("http://a.example/client.json"));
        assert!(!is_loopback_cimd_client_id("https://127.0.0.1/client.json"), "https is the normal path");
    }

    // ChatGPT's documents, fetched 2026-10-15. Production sees the first `client_id` (Caddy's access log of
    // app.reins2fa.com); servers that do not meet ChatGPT's issuer requirements get a per-connector one like the second.
    const CHATGPT_ID: &str = "https://chatgpt.com/oauth/client.json";
    const CHATGPT: &[u8] = br#"{"client_id":"https://chatgpt.com/oauth/client.json","client_uri":"https://chatgpt.com/","redirect_uris":["https://chatgpt.com/connector_platform_oauth_redirect"],"token_endpoint_auth_method":"private_key_jwt","token_endpoint_auth_methods_supported":["none","private_key_jwt"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"client_name":"ChatGPT","logo_uri":"https://persistent.oaistatic.com/sonic/misc/openai-logo.png","token_endpoint_auth_signing_alg":"RS256","jwks_uri":"https://chatgpt.com/oauth/jwks.json"}"#;
    const CHATGPT_CONNECTOR_ID: &str = "https://chatgpt.com/oauth/BpZIpxL-aggt/client.json";
    const CHATGPT_CONNECTOR: &[u8] = br#"{"client_id":"https://chatgpt.com/oauth/BpZIpxL-aggt/client.json","client_uri":"https://chatgpt.com/","redirect_uris":["https://chatgpt.com/connector/oauth/BpZIpxL-aggt"],"token_endpoint_auth_method":"private_key_jwt","token_endpoint_auth_methods_supported":["none","private_key_jwt"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"client_name":"ChatGPT","logo_uri":"https://persistent.oaistatic.com/sonic/misc/openai-logo.png","token_endpoint_auth_signing_alg":"RS256","jwks_uri":"https://chatgpt.com/oauth/jwks.json"}"#;
    const CHATGPT_JWKS: &[u8] = br#"{"keys":[{"kty":"RSA","kid":"cimd-20260428030119","use":"sig","alg":"RS256","n":"y09nMyYX6LhSgS3YmbLOZrFoR8SffxG0kM5gQ5PKpHVMzbAu__-7rf0_Q_pwhVa9vxJzv3cRkGnXKNWKOHDdEXp8YFPVUql4NcDjDdS_0w0uos8gazoa7Td47qVquxOsG3861l8oKEh-E4r5C_6w6Sx0Rl2WEEs2-dmvn3fwH9PkLCQOo4tsNAEnrW_ge2vQE-pFo-kJp5QRRiX2w0YvaMFtIfEvNbdPSJ3xd7NbGrvRt279HrxgDLGUvpbeWrCsp3D7HdR2QZn-9MZp7CHPlMGtFgN9aIB4Guf7qYlRiC3Ja0ZI22jSMct6xrI-90XX4AK2FhiWmYOmjV_2d3vEVw","e":"AQAB"}]}"#;

    #[test]
    fn chatgpts_metadata_document_is_accepted_as_a_private_key_jwt_client() {
        for (id, doc, redirect) in [
            (CHATGPT_ID, CHATGPT, "https://chatgpt.com/connector_platform_oauth_redirect"),
            (CHATGPT_CONNECTOR_ID, CHATGPT_CONNECTOR, "https://chatgpt.com/connector/oauth/BpZIpxL-aggt"),
        ] {
            let info = parse_client_metadata(id, doc).unwrap();
            assert_eq!(info.client_name, "ChatGPT");
            assert_eq!(info.redirect_uris, [redirect]);
            assert_eq!(
                info.auth,
                ClientAuth::PrivateKeyJwt {
                    jwks: JwksSource::Uri("https://chatgpt.com/oauth/jwks.json".into()),
                    alg: Some(Algorithm::RS256),
                }
            );
            assert_eq!(info.auth.method(), AuthMethod::PrivateKeyJwt);
        }
        // Its key set: one RS256 signing key of 2048 bits, found by its kid.
        let jwks = parse_jwks(CHATGPT_JWKS).unwrap();
        let key = select_key(&jwks, Algorithm::RS256, Some("cimd-20260428030119")).unwrap();
        let AlgorithmParameters::RSA(rsa) = &key.algorithm else {
            panic!("RSA")
        };
        assert_eq!(rsa_modulus_bits(&rsa.n), 2048);
        assert!(select_key(&jwks, Algorithm::RS256, Some("other")).is_none());
        assert!(select_key(&jwks, Algorithm::PS256, Some("cimd-20260428030119")).is_none(), "the key says RS256");
        assert!(select_key(&jwks, Algorithm::RS256, None).is_some(), "the only key, without a kid");
    }

    #[test]
    fn private_key_jwt_clients_need_keys_and_an_accepted_algorithm() {
        let id = "https://a.example/client.json";
        let parse = |extra: Value| {
            let mut doc = json!({"client_id": id, "redirect_uris": ["https://a.example/cb"],
                "token_endpoint_auth_method": "private_key_jwt"});
            for (k, v) in extra.as_object().unwrap() {
                doc[k] = v.clone();
            }
            parse_client_metadata(id, doc.to_string().as_bytes()).map(|c| c.auth)
        };
        let jwks: Value = serde_json::from_slice(CHATGPT_JWKS).unwrap();
        // Inline keys work as well as a jwks_uri; without a pinned algorithm any accepted one goes.
        assert_eq!(
            parse(json!({"jwks": jwks})).unwrap(),
            ClientAuth::PrivateKeyJwt {
                jwks: JwksSource::Inline(parse_jwks(CHATGPT_JWKS).unwrap()),
                alg: None
            }
        );
        for alg in ["RS256", "PS256", "ES256", "EdDSA"] {
            let auth = parse(json!({"jwks_uri": "https://a.example/jwks", "token_endpoint_auth_signing_alg": alg}));
            assert!(auth.is_ok(), "{alg}");
        }
        for (extra, why) in [
            (json!({}), "requires jwks_uri or jwks"),
            (json!({"jwks_uri": "https://a.example/jwks", "jwks": jwks}), "not declare both"),
            (json!({"jwks_uri": "http://a.example/jwks"}), "jwks_uri must be https"),
            (json!({"jwks_uri": "https://a.example/jwks#x"}), "fragment"),
            (json!({"jwks_uri": 7}), "must be a string"),
            (json!({"jwks": {"keys": []}}), "no keys"),
            (json!({"jwks": "nope"}), "not valid"),
            (json!({"jwks_uri": "https://a.example/jwks", "token_endpoint_auth_signing_alg": "HS256"}), "HS256"),
            (json!({"jwks_uri": "https://a.example/jwks", "token_endpoint_auth_signing_alg": "none"}), "`none`"),
            (json!({"jwks_uri": "https://a.example/jwks", "token_endpoint_auth_signing_alg": "RS512"}), "RS512"),
        ] {
            let err = parse(extra.clone()).unwrap_err();
            assert!(err.contains(why), "{extra}: {err}");
        }
        // SEP-3149: the listed methods count when the preference is not supported, strongest first.
        let listed = |methods: Value, preferred: Value| {
            let doc = json!({"client_id": id, "redirect_uris": ["https://a.example/cb"],
                "token_endpoint_auth_method": preferred, "token_endpoint_auth_methods_supported": methods,
                "jwks_uri": "https://a.example/jwks"});
            parse_client_metadata(id, doc.to_string().as_bytes()).map(|c| c.auth.method())
        };
        let jwt = Ok(AuthMethod::PrivateKeyJwt);
        assert_eq!(listed(json!(["none", "private_key_jwt"]), json!("private_key_jwt")), jwt);
        assert_eq!(listed(json!(["none", "private_key_jwt"]), json!("none")), Ok(AuthMethod::None), "its preference");
        assert_eq!(listed(json!(["private_key_jwt", "none"]), json!("client_secret_basic")), jwt);
        assert_eq!(listed(json!(["private_key_jwt"]), Value::Null), jwt);
        assert_eq!(listed(json!(["none"]), json!("client_secret_post")), Ok(AuthMethod::None));
        assert!(listed(json!(["client_secret_basic"]), Value::Null).is_err());
        assert!(listed(json!("none"), Value::Null).is_err(), "must be an array");
        assert_eq!(AuthMethod::from_stored("none"), AuthMethod::None);
        assert_eq!(AuthMethod::from_stored("private_key_jwt"), AuthMethod::PrivateKeyJwt);
        assert_eq!(AuthMethod::from_stored("garbage"), AuthMethod::PrivateKeyJwt, "fails closed");
        assert!(AuthMethod::PrivateKeyJwt > AuthMethod::None);
    }

    #[test]
    fn dynamic_registration_accepts_only_public_clients() {
        let ok = json!({"client_name": "Claude", "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"],
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
            "token_endpoint_auth_method": "none", "application_type": "web"});
        let reg = parse_registration(ok.to_string().as_bytes()).unwrap();
        assert_eq!(reg.client_name, "Claude");
        let err = |v: Value| parse_registration(v.to_string().as_bytes()).unwrap_err();
        // private_key_jwt goes through a metadata document, never through registration.
        let jwt =
            err(json!({"redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "private_key_jwt",
            "jwks_uri": "https://a.example/jwks"}));
        assert_eq!(jwt.error, "invalid_client_metadata");
        assert!(jwt.description.contains("client ID metadata document"), "{}", jwt.description);
        // A client that can do both is registered as public.
        assert!(
            parse_registration(
                json!({"redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "private_key_jwt",
                    "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"]})
                .to_string()
                .as_bytes()
            )
            .is_ok()
        );
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
        let grant = |form: &HashMap<String, String>| {
            let request = parse_token_request(form, MCP).unwrap();
            assert_eq!(request.client_assertion, None);
            request.grant
        };
        assert_eq!(
            grant(&form),
            TokenGrant::AuthorizationCode {
                code: "C".into(),
                redirect_uri: "https://a.example/cb".into(),
                client_id: "cid".into(),
                code_verifier: "V".into()
            }
        );
        let refresh = parse_form(b"grant_type=refresh_token&refresh_token=R");
        assert_eq!(
            grant(&refresh),
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
        let device = parse_form(
            b"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=D&client_id=cid",
        );
        assert_eq!(
            grant(&device),
            TokenGrant::DeviceCode {
                device_code: "D".into(),
                client_id: "cid".into()
            }
        );
        assert_eq!(
            e(b"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=D"),
            "invalid_request"
        );
    }

    #[test]
    fn device_authorization_requests_and_answers() {
        let key = reins_proto::desktop::encode_key(&[3u8; 32]);
        let form = |pairs: &[(&str, &str)]| params(pairs);
        let parse = |f: &HashMap<String, String>| parse_device_authorization(f, MCP, "reins_client_key");
        assert_eq!(
            parse(&form(&[("client_id", "cid"), ("scope", "mcp"), ("reins_client_key", &key)])).unwrap(),
            DeviceAuthorization {
                client_id: "cid".into(),
                client_key: Some(key.clone())
            }
        );
        assert_eq!(parse(&form(&[("client_id", "cid"), ("resource", MCP)])).unwrap().client_key, None);
        let error = |f: HashMap<String, String>| parse(&f).unwrap_err().error;
        assert_eq!(error(form(&[])), "invalid_request");
        assert_eq!(error(form(&[("client_id", "cid"), ("scope", "mcp admin")])), "invalid_scope");
        assert_eq!(error(form(&[("client_id", "cid"), ("resource", "https://other.example/mcp")])), "invalid_target");
        assert_eq!(error(form(&[("client_id", "cid"), ("reins_client_key", "nope")])), "invalid_request");

        let answer = device_authorization_response("https://rw.example.com/", "DC", "BCDF-GHJK", 47, 600, 5);
        assert_eq!(answer["verification_uri"], "https://rw.example.com/pair");
        assert_eq!(answer["verification_uri_complete"], "https://rw.example.com/pair?code=BCDF-GHJK");
        assert_eq!((answer["expires_in"].as_u64(), answer["interval"].as_u64()), (Some(600), Some(5)));
        assert_eq!(answer["reins_confirm_code"], 47);
        assert_eq!(
            pairing_page_url("http://127.0.0.1:8000/vw", Some("BCDF-GHJK")),
            "http://127.0.0.1:8000/vw/pair?code=BCDF-GHJK"
        );
        let meta = authorization_server_metadata(DOMAIN);
        assert_eq!(meta["device_authorization_endpoint"], "https://rw.example.com/reins/oauth/device_authorization");
        assert!(meta["grant_types_supported"].as_array().unwrap().iter().any(|g| g == DEVICE_CODE_GRANT));
    }

    #[test]
    fn code_redemption_binds_client_redirect_and_pkce() {
        let code = AuthCode {
            user_uuid: "u".into(),
            connection_uuid: "c".into(),
            client_id: "cid".into(),
            redirect_uri: "https://a.example/cb".into(),
            code_challenge: CHALLENGE.into(),
            client_auth: AuthMethod::None,
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

    // -----------------------------------------------------------------------------------
    // private_key_jwt
    // -----------------------------------------------------------------------------------

    use jsonwebtoken::{EncodingKey, Header};
    use openssl::{
        bn::BigNumContext,
        ec::{EcGroup, EcKey},
        nid::Nid,
        pkey::PKey,
        rsa::Rsa,
    };

    const CLIENT: &str = "https://client.example/oauth/client.json";
    const NOW: i64 = 1_800_000_000;

    /// A client key pair: the public JWK and the private key to sign with.
    struct TestKey {
        jwk: Value,
        signing: EncodingKey,
        alg: Algorithm,
    }

    fn b64(bytes: &[u8]) -> String {
        BASE64URL_NOPAD.encode(bytes)
    }

    fn rsa_key(kid: &str, bits: u32, alg: Algorithm) -> TestKey {
        let rsa = Rsa::generate(bits).unwrap();
        let jwk =
            json!({"kty": "RSA", "kid": kid, "use": "sig", "n": b64(&rsa.n().to_vec()), "e": b64(&rsa.e().to_vec())});
        let pem = PKey::from_rsa(rsa).unwrap().private_key_to_pem_pkcs8().unwrap();
        TestKey {
            jwk,
            signing: EncodingKey::from_rsa_pem(&pem).unwrap(),
            alg,
        }
    }

    fn ec_key(kid: &str) -> TestKey {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let ec = EcKey::generate(&group).unwrap();
        let (mut x, mut y) = (openssl::bn::BigNum::new().unwrap(), openssl::bn::BigNum::new().unwrap());
        ec.public_key().affine_coordinates(&group, &mut x, &mut y, &mut BigNumContext::new().unwrap()).unwrap();
        let jwk = json!({"kty": "EC", "crv": "P-256", "kid": kid,
            "x": b64(&x.to_vec_padded(32).unwrap()), "y": b64(&y.to_vec_padded(32).unwrap())});
        let pem = PKey::from_ec_key(ec).unwrap().private_key_to_pem_pkcs8().unwrap();
        TestKey {
            jwk,
            signing: EncodingKey::from_ec_pem(&pem).unwrap(),
            alg: Algorithm::ES256,
        }
    }

    fn ed_key(kid: &str) -> TestKey {
        let key = PKey::generate_ed25519().unwrap();
        let jwk = json!({"kty": "OKP", "crv": "Ed25519", "kid": kid, "x": b64(&key.raw_public_key().unwrap())});
        TestKey {
            jwk,
            signing: EncodingKey::from_ed_pem(&key.private_key_to_pem_pkcs8().unwrap()).unwrap(),
            alg: Algorithm::EdDSA,
        }
    }

    fn jwks_of(keys: &[&TestKey]) -> JwkSet {
        parse_jwks(json!({"keys": keys.iter().map(|k| k.jwk.clone()).collect::<Vec<_>>()}).to_string().as_bytes())
            .unwrap()
    }

    fn good_claims() -> Value {
        json!({"iss": CLIENT, "sub": CLIENT, "aud": "https://rw.example.com/reins/oauth/token",
            "jti": "jti-1", "iat": NOW, "exp": NOW + 60})
    }

    fn sign(key: &TestKey, kid: Option<&str>, claims: &Value) -> String {
        let mut header = Header::new(key.alg);
        header.kid = kid.map(str::to_owned);
        jsonwebtoken::encode(&header, claims, &key.signing).unwrap()
    }

    fn verify(assertion: &str, jwks: &JwkSet, pinned: Option<Algorithm>) -> Result<VerifiedAssertion, AssertionError> {
        verify_client_assertion(assertion, CLIENT, jwks, pinned, &assertion_audiences(DOMAIN), NOW)
    }

    fn invalid(result: Result<VerifiedAssertion, AssertionError>) -> String {
        match result {
            Err(AssertionError::Invalid(why)) => why,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn valid_assertions_pass_with_every_accepted_algorithm() {
        for key in
            [rsa_key("r", 2048, Algorithm::RS256), rsa_key("p", 2048, Algorithm::PS256), ec_key("e"), ed_key("d")]
        {
            let kid = key.jwk["kid"].as_str().unwrap().to_owned();
            let assertion = sign(&key, Some(&kid), &good_claims());
            let verified =
                verify(&assertion, &jwks_of(&[&key]), None).unwrap_or_else(|e| panic!("{:?}: {e:?}", key.alg));
            assert_eq!(
                verified,
                VerifiedAssertion {
                    jti: "jti-1".into(),
                    exp: NOW + 60
                }
            );
            assert_eq!(verify(&assertion, &jwks_of(&[&key]), Some(key.alg)).map(|v| v.exp), Ok(NOW + 60));
        }
        // aud may name the issuer instead of the token endpoint, or be a list containing either.
        let key = rsa_key("r", 2048, Algorithm::RS256);
        let jwks = jwks_of(&[&key]);
        for aud in [json!(DOMAIN), json!(format!("{DOMAIN}/")), json!(["https://other.example", DOMAIN])] {
            let mut claims = good_claims();
            claims["aud"] = aud.clone();
            assert!(verify(&sign(&key, Some("r"), &claims), &jwks, None).is_ok(), "{aud}");
        }
        // Small clock skew is tolerated.
        let mut claims = good_claims();
        claims["exp"] = json!(NOW - 30);
        claims["iat"] = json!(NOW + 30);
        claims["nbf"] = json!(NOW + 30);
        assert!(verify(&sign(&key, Some("r"), &claims), &jwks, None).is_ok());
        // No kid: fine when exactly one key fits.
        assert!(verify(&sign(&key, None, &good_claims()), &jwks, None).is_ok());
    }

    #[test]
    fn bad_assertions_are_refused() {
        let key = rsa_key("k1", 2048, Algorithm::RS256);
        let jwks = jwks_of(&[&key]);
        let with = |change: &dyn Fn(&mut Value)| {
            let mut claims = good_claims();
            change(&mut claims);
            invalid(verify(&sign(&key, Some("k1"), &claims), &jwks, None))
        };
        // A different key under the same kid: the signature does not verify.
        let impostor = rsa_key("k1", 2048, Algorithm::RS256);
        assert!(invalid(verify(&sign(&impostor, Some("k1"), &good_claims()), &jwks, None)).contains("signature"));
        // A tampered payload.
        let good = sign(&key, Some("k1"), &good_claims());
        let parts: Vec<&str> = good.split('.').collect();
        // A payload that passes every claim check, swapped in under the original signature.
        let mut evil = good_claims();
        evil["jti"] = json!("another-jti");
        let tampered = format!("{}.{}.{}", parts[0], b64(evil.to_string().as_bytes()), parts[2]);
        assert!(invalid(verify(&tampered, &jwks, None)).contains("signature"));

        assert!(with(&|c| c["aud"] = json!("https://other.example/reins/oauth/token")).contains("aud"));
        assert!(with(&|c| c["aud"] = json!("https://rw.example.com/mcp")).contains("aud"));
        assert!(with(&|c| drop(c.as_object_mut().unwrap().remove("aud"))).contains("aud"));
        assert!(with(&|c| c["iss"] = json!("https://evil.example/client.json")).contains("iss"));
        assert!(with(&|c| c["sub"] = json!("https://evil.example/client.json")).contains("sub"));
        assert!(with(&|c| drop(c.as_object_mut().unwrap().remove("sub"))).contains("sub"));
        assert!(with(&|c| c["exp"] = json!(NOW - 61)).contains("expired"));
        assert!(with(&|c| drop(c.as_object_mut().unwrap().remove("exp"))).contains("exp"));
        assert!(with(&|c| c["exp"] = json!(NOW + 3600)).contains("too far"));
        assert!(with(&|c| c["nbf"] = json!(NOW + 120)).contains("not valid yet"));
        assert!(with(&|c| c["iat"] = json!(NOW + 120)).contains("future"));
        assert!(with(&|c| drop(c.as_object_mut().unwrap().remove("jti"))).contains("jti"));
        assert!(with(&|c| c["jti"] = json!("")).contains("jti"));
        assert!(with(&|c| c["jti"] = json!("j".repeat(MAX_JTI_BYTES + 1))).contains("jti"));

        // Signed with an algorithm the client did not pin.
        let ps = rsa_key("k1", 2048, Algorithm::PS256);
        let ps_jwks = jwks_of(&[&ps]);
        let ps_assertion = sign(&ps, Some("k1"), &good_claims());
        assert!(verify(&ps_assertion, &ps_jwks, None).is_ok());
        assert!(
            invalid(verify(&ps_assertion, &ps_jwks, Some(Algorithm::RS256)))
                .contains("token_endpoint_auth_signing_alg")
        );
        // Short RSA keys are refused even with a good signature.
        let weak = rsa_key("w", 1024, Algorithm::RS256);
        assert!(invalid(verify(&sign(&weak, Some("w"), &good_claims()), &jwks_of(&[&weak]), None)).contains("2048"));
        // Garbage and oversized assertions.
        assert!(matches!(verify("not.a.jwt", &jwks, None), Err(AssertionError::Invalid(_))));
        assert!(invalid(verify(&"a".repeat(MAX_ASSERTION_BYTES + 1), &jwks, None)).contains("too long"));
    }

    #[test]
    fn alg_none_and_hmac_assertions_are_refused() {
        let key = rsa_key("k1", 2048, Algorithm::RS256);
        let jwks = jwks_of(&[&key]);
        let claims = b64(good_claims().to_string().as_bytes());
        let none = format!("{}.{claims}.", b64(br#"{"alg":"none","kid":"k1"}"#));
        assert!(matches!(verify(&none, &jwks, None), Err(AssertionError::Invalid(_))));
        let none_sig = format!("{}.{claims}.{}", b64(br#"{"alg":"none"}"#), b64(b"x"));
        assert!(matches!(verify(&none_sig, &jwks, None), Err(AssertionError::Invalid(_))));
        // HS256 with a shared secret, even if the JWKS carries a symmetric key for it.
        let secret = b"a shared secret that must never authenticate a client";
        let oct =
            parse_jwks(json!({"keys": [{"kty": "oct", "kid": "h", "k": b64(secret)}]}).to_string().as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("h".into());
        let hs = jsonwebtoken::encode(&header, &good_claims(), &EncodingKey::from_secret(secret)).unwrap();
        assert!(invalid(verify(&hs, &oct, None)).contains("HS256"));
        // HS256 "signed" with the client's public key bytes (the classic key-confusion attack).
        let n = key.jwk["n"].as_str().unwrap().as_bytes();
        let confused = jsonwebtoken::encode(&header, &good_claims(), &EncodingKey::from_secret(n)).unwrap();
        assert!(invalid(verify(&confused, &jwks, None)).contains("HS256"));
    }

    #[test]
    fn keys_are_chosen_by_kid_type_and_use() {
        let rsa = rsa_key("a", 2048, Algorithm::RS256);
        let other = rsa_key("b", 2048, Algorithm::RS256);
        let ec = ec_key("e");
        let both = jwks_of(&[&rsa, &other, &ec]);
        assert!(verify(&sign(&other, Some("b"), &good_claims()), &both, None).is_ok());
        assert_eq!(verify(&sign(&rsa, Some("zzz"), &good_claims()), &both, None), Err(AssertionError::UnknownKey));
        assert_eq!(
            verify(&sign(&rsa, None, &good_claims()), &both, None),
            Err(AssertionError::UnknownKey),
            "no kid and two RSA keys: ambiguous"
        );
        assert!(verify(&sign(&ec, None, &good_claims()), &both, None).is_ok(), "the only EC key");
        // A key marked for encryption, or for another algorithm, is not used to verify.
        let mut enc = rsa.jwk.clone();
        enc["use"] = json!("enc");
        let enc_set = parse_jwks(json!({"keys": [enc]}).to_string().as_bytes()).unwrap();
        assert_eq!(verify(&sign(&rsa, Some("a"), &good_claims()), &enc_set, None), Err(AssertionError::UnknownKey));
        let mut ps_only = rsa.jwk.clone();
        ps_only["alg"] = json!("PS256");
        let ps_set = parse_jwks(json!({"keys": [ps_only]}).to_string().as_bytes()).unwrap();
        assert_eq!(verify(&sign(&rsa, Some("a"), &good_claims()), &ps_set, None), Err(AssertionError::UnknownKey));
        assert_eq!(AssertionError::UnknownKey.into_oauth().error, "invalid_client");
        assert_eq!(AssertionError::Invalid("x".into()).into_oauth().status, 401);
    }

    #[test]
    fn the_precheck_needs_no_key_and_catches_what_could_never_pass() {
        let key = rsa_key("k1", 2048, Algorithm::RS256);
        let audiences = assertion_audiences(DOMAIN);
        let precheck =
            |claims: &Value| precheck_client_assertion(&sign(&key, Some("k1"), claims), CLIENT, &audiences, NOW);
        assert_eq!(precheck(&good_claims()), Ok(()));
        // Signed by nobody the server knows, and still fine here: the signature is checked later, with the key.
        let stranger = ec_key("unknown");
        assert_eq!(precheck_client_assertion(&sign(&stranger, None, &good_claims()), CLIENT, &audiences, NOW), Ok(()));
        let with = |change: &dyn Fn(&mut Value)| {
            let mut claims = good_claims();
            change(&mut claims);
            match precheck(&claims) {
                Err(AssertionError::Invalid(why)) => why,
                other => panic!("expected Invalid, got {other:?}"),
            }
        };
        assert!(with(&|c| c["exp"] = json!(NOW - 600)).contains("expired"));
        assert!(with(&|c| c["exp"] = json!(NOW + 3600)).contains("too far"));
        assert!(with(&|c| c["aud"] = json!("https://other.example")).contains("aud"));
        assert!(with(&|c| c["aud"] = json!(["https://other.example"])).contains("aud"));
        assert!(with(&|c| c["iss"] = json!("https://evil.example/c.json")).contains("iss"));
        assert!(with(&|c| c["sub"] = json!("https://evil.example/c.json")).contains("sub"));
        assert!(with(&|c| drop(c.as_object_mut().unwrap().remove("jti"))).contains("jti"));
        let none = format!("{}.{}.", b64(br#"{"alg":"none"}"#), b64(good_claims().to_string().as_bytes()));
        assert!(precheck_client_assertion(&none, CLIENT, &audiences, NOW).is_err());
    }

    #[test]
    fn the_required_method_never_drops_to_none() {
        use AuthMethod::{None as Public, PrivateKeyJwt as Keys};
        assert_eq!(required_method(Public, false, None), Public);
        assert_eq!(required_method(Public, false, Some(Public)), Public);
        assert_eq!(required_method(Public, false, Some(Keys)), Keys, "the document says keys now");
        assert_eq!(
            required_method(Public, true, Some(Public)),
            Keys,
            "seen with keys before; the document dropped them"
        );
        assert_eq!(required_method(Public, true, None), Keys, "seen with keys; the document is unavailable");
        assert_eq!(required_method(Keys, false, Some(Public)), Keys, "the grant was issued for keys");
        assert_eq!(required_method(Keys, false, None), Keys);
    }

    #[test]
    fn client_assertion_parameters_are_parsed() {
        let parse = |pairs: &[(&str, &str)]| parse_client_assertion(&params(pairs));
        assert_eq!(parse(&[]), Ok(None));
        assert_eq!(
            parse(&[("client_assertion_type", JWT_BEARER_ASSERTION_TYPE), ("client_assertion", "a.b.c")]),
            Ok(Some("a.b.c".into()))
        );
        let error = |pairs: &[(&str, &str)]| parse(pairs).unwrap_err().error;
        assert_eq!(error(&[("client_assertion", "a.b.c")]), "invalid_request");
        assert_eq!(error(&[("client_assertion_type", JWT_BEARER_ASSERTION_TYPE)]), "invalid_request");
        assert_eq!(error(&[("client_assertion_type", "urn:other"), ("client_assertion", "a.b.c")]), "invalid_client");
        assert_eq!(error(&[("client_secret", "s")]), "invalid_client");
        let long = "a".repeat(MAX_ASSERTION_BYTES + 1);
        assert_eq!(
            error(&[("client_assertion_type", JWT_BEARER_ASSERTION_TYPE), ("client_assertion", long.as_str())]),
            "invalid_client"
        );

        // RFC 7523 §3: with an assertion, client_id may be left out; the assertion's sub names the client.
        let key = ec_key("e");
        let assertion = sign(&key, Some("e"), &good_claims());
        let form = params(&[
            ("grant_type", "authorization_code"),
            ("code", "C"),
            ("redirect_uri", "https://a.example/cb"),
            ("code_verifier", "V"),
            ("client_assertion_type", JWT_BEARER_ASSERTION_TYPE),
            ("client_assertion", &assertion),
        ]);
        let request = parse_token_request(&form, MCP).unwrap();
        assert_eq!(request.client_assertion.as_deref(), Some(assertion.as_str()));
        assert!(matches!(request.grant, TokenGrant::AuthorizationCode { client_id, .. } if client_id == CLIENT));
        let mut no_sub = form;
        no_sub.insert("client_assertion".into(), "e30.e30.sig".into());
        assert_eq!(parse_token_request(&no_sub, MCP).unwrap_err().error, "invalid_client");
    }
}
