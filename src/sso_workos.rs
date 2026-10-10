//! WorkOS AuthKit as the SSO provider (`SSO_PROVIDER=workos`, or an `SSO_AUTHORITY` on `api.workos.com`).
//!
//! AuthKit publishes an OpenID discovery document, but it is not one a generic OpenID Connect client can use: its
//! `issuer` names another client id than the one in its URL, it has no `userinfo_endpoint`, and its token endpoint
//! (`/user_management/authenticate`) takes the client secret in a JSON body and answers with the user itself instead
//! of an `id_token`. So this talks to the User Management API directly:
//!
//! - authorize: `GET {base}/user_management/authorize?client_id&redirect_uri&response_type=code&state&provider=authkit`
//!   (plus `code_challenge` with PKCE, and `SSO_AUTHORIZE_EXTRA_PARAMS`, such as `screen_hint=sign-up`).
//! - code: `POST {base}/user_management/authenticate {grant_type: authorization_code, client_id, client_secret, code,
//!   code_verifier}`; the answer carries `user {id, email, email_verified}` and an access token whose `sid` names the
//!   WorkOS session (kept so that a revoked session signs the device out, see `api::reins::workos_sync`).
//! - refresh: the same endpoint with `grant_type: refresh_token` (only used with `SSO_AUTH_ONLY_NOT_SESSION=false`).
//! - delete: `DELETE {base}/user_management/users/<user id>` with the API key, when a user deletes their account in
//!   the app (`api::reins::account_delete`), so the identity cannot sign in to a new, empty account by itself.
//!
//! The answer comes straight from WorkOS over TLS, authenticated with the client secret, so its `user` is the
//! identity; there is no id token to verify. `{base}` is `SSO_AUTHORITY` without `/user_management/<client id>`.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::{
    CONFIG,
    api::ApiResult,
    db::models::{OIDCAuthenticatedUser, SsoAuth},
    http_client::get_reqwest_client_builder,
    sso::{OIDCCode, OIDCCodeChallenge, OIDCCodeVerifier, OIDCIdentifier, OIDCState},
    sso_client::RefreshTokenResponse,
};

/// The path segment that separates the API base from the client id in a WorkOS authority.
const USER_MANAGEMENT: &str = "/user_management/";

/// Whether SSO goes through WorkOS: `SSO_PROVIDER=workos`, or (when `SSO_PROVIDER` is empty) an `SSO_AUTHORITY` on
/// `api.workos.com`.
pub fn enabled() -> bool {
    provider_is_workos(&CONFIG.sso_provider(), &CONFIG.sso_authority())
}

pub fn provider_is_workos(provider: &str, authority: &str) -> bool {
    match provider.trim().to_ascii_lowercase().as_str() {
        "workos" => true,
        "" => Url::parse(authority).is_ok_and(|u| u.host_str() == Some("api.workos.com")),
        _ => false,
    }
}

/// `https://api.workos.com` for `https://api.workos.com/user_management/client_...`.
pub fn api_base_of(authority: &str) -> Option<String> {
    let at = authority.find(USER_MANAGEMENT)?;
    let client = &authority[at + USER_MANAGEMENT.len()..];
    (!client.is_empty() && !client.contains('/')).then(|| authority[..at].trim_end_matches('/').to_owned())
}

/// The WorkOS API base of this server's `SSO_AUTHORITY`.
pub fn api_base() -> ApiResult<String> {
    let Some(base) = api_base_of(&CONFIG.sso_authority()) else {
        err!("SSO_AUTHORITY must be https://api.workos.com/user_management/<client id> for WorkOS")
    };
    Ok(base)
}

/// The fixed signed-out landing page must be registered in WorkOS's allowed sign-out URIs.
/// Never put API keys or app tokens in this URL, or accept a caller-provided return URI.
pub fn logout_url(session_id: &str) -> ApiResult<String> {
    let Ok(mut url) = Url::parse(&format!("{}/user_management/sessions/logout", api_base()?)) else {
        err!("Invalid WorkOS logout URL");
    };
    url.query_pairs_mut()
        .append_pair("session_id", session_id)
        .append_pair("return_to", &format!("{}/reins/signed-out", CONFIG.domain()));
    Ok(url.into())
}

/// The key the User Management and Events APIs take: `REINS_WORKOS_API_KEY`, else `SSO_CLIENT_SECRET` (WorkOS
/// uses the API key as the client secret).
pub fn api_key() -> String {
    CONFIG.reins_workos_api_key().filter(|k| !k.is_empty()).unwrap_or_else(|| CONFIG.sso_client_secret())
}

pub fn http_client() -> ApiResult<reqwest::Client> {
    match get_reqwest_client_builder(false).redirect(reqwest::redirect::Policy::none()).build() {
        Ok(client) => Ok(client),
        Err(e) => err!(format!("Failed to build the WorkOS http client: {e}")),
    }
}

/// The AuthKit authorize URL for one sign-in, and the `SsoAuth` row that remembers it. `provider=authkit` unless
/// `SSO_AUTHORIZE_EXTRA_PARAMS` picks a provider, connection or organization itself.
pub fn authorize_url(
    state: OIDCState,
    client_challenge: OIDCCodeChallenge,
    redirect_uri: String,
    binding_hash: Option<String>,
) -> ApiResult<(Url, SsoAuth)> {
    let base64_state = data_encoding::BASE64.encode(state.to_string().as_bytes());
    let mut url = match Url::parse(&format!("{}{USER_MANAGEMENT}authorize", api_base()?)) {
        Ok(url) => url,
        Err(e) => err!(format!("Invalid WorkOS authorize URL: {e}")),
    };
    let extra = CONFIG.sso_authorize_extra_params_vec();
    let picks_provider =
        extra.iter().any(|(k, _)| matches!(k.as_str(), "provider" | "connection_id" | "organization_id"));
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &CONFIG.sso_client_id())
            .append_pair("redirect_uri", CONFIG.sso_redirect_url()?.as_str())
            .append_pair("response_type", "code")
            .append_pair("state", &base64_state);
        if !picks_provider {
            q.append_pair("provider", "authkit");
        }
        if CONFIG.sso_pkce() {
            q.append_pair("code_challenge", client_challenge.as_ref()).append_pair("code_challenge_method", "S256");
        }
        for (k, v) in &extra {
            q.append_pair(k, v);
        }
    }
    // WorkOS has no nonce (there is no id token); the column is required, so it holds a random value.
    let nonce = data_encoding::HEXLOWER.encode(&crate::crypto::get_random_bytes::<16>());
    Ok((url, SsoAuth::new(state, client_challenge, nonce, redirect_uri, binding_hash)))
}

#[derive(Debug, Deserialize)]
struct WorkosUser {
    id: String,
    email: String,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthenticateResponse {
    user: WorkosUser,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Set when a WorkOS dashboard user impersonates the user.
    #[serde(default)]
    impersonator: Option<Value>,
    #[serde(default)]
    authentication_method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// The unverified claims of a WorkOS access token that matter here.
#[derive(Debug, Default, Deserialize)]
struct AccessClaims {
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
}

fn access_claims(token: &str) -> AccessClaims {
    // Read, not verified: the token came straight from WorkOS's token endpoint over TLS.
    jsonwebtoken::dangerous::insecure_decode::<AccessClaims>(token).map(|t| t.claims).unwrap_or_default()
}

fn expires_in(claims: &AccessClaims) -> Option<Duration> {
    let secs = claims.exp? - chrono::Utc::now().timestamp();
    u64::try_from(secs).ok().map(Duration::from_secs)
}

/// What WorkOS said when it refused, for the log and the error.
fn refusal(status: reqwest::StatusCode, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let pick = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let code = pick("error").or_else(|| pick("code")).unwrap_or_else(|| status.as_u16().to_string());
    match pick("error_description").or_else(|| pick("message")) {
        Some(msg) => format!("{code}: {msg}"),
        None => code,
    }
}

async fn authenticate(body: &Value) -> ApiResult<String> {
    let url = format!("{}{USER_MANAGEMENT}authenticate", api_base()?);
    let response = match http_client()?.post(url).json(body).send().await {
        Ok(r) => r,
        Err(e) => err!(format!("Failed to contact WorkOS: {e}")),
    };
    let status = response.status();
    let text = match response.text().await {
        Ok(t) => t,
        Err(e) => err!(format!("Failed to read the WorkOS answer: {e}")),
    };
    if CONFIG.sso_debug_tokens() {
        debug!("WorkOS authenticate answer {text}");
    }
    if !status.is_success() {
        err!(format!("WorkOS refused the sign-in ({})", refusal(status, &text)))
    }
    Ok(text)
}

/// The OIDC identifier of a WorkOS user: `SSO_AUTHORITY/<user id>`, stable across sign-ins.
pub fn identifier(user_id: &str) -> OIDCIdentifier {
    OIDCIdentifier::new(&CONFIG.sso_authority(), user_id)
}

/// Exchanges the code AuthKit handed back for the user it signed in.
pub async fn exchange_code(code: &OIDCCode, verifier: OIDCCodeVerifier) -> ApiResult<OIDCAuthenticatedUser> {
    let mut body = json!({
        "grant_type": "authorization_code",
        "client_id": CONFIG.sso_client_id(),
        "client_secret": CONFIG.sso_client_secret(),
        "code": code.to_string(),
    });
    if CONFIG.sso_pkce() {
        body["code_verifier"] = Value::String(verifier.to_string());
    }
    let text = authenticate(&body).await?;
    parse_authenticated(&text, CONFIG.reins_enabled() && CONFIG.reins_workos_require_passkey(), CONFIG.reins_enabled())
}

fn parse_authenticated(text: &str, require_passkey: bool, require_session: bool) -> ApiResult<OIDCAuthenticatedUser> {
    let auth: AuthenticateResponse = match serde_json::from_str(text) {
        Ok(a) => a,
        Err(e) => err!(format!("Unexpected WorkOS authenticate answer: {e}")),
    };
    if auth.impersonator.as_ref().is_some_and(|i| !i.is_null()) {
        err!("A WorkOS impersonation session cannot sign in to Reins")
    }
    if require_passkey && auth.authentication_method.as_deref() != Some("Passkey") {
        err!("A passkey is required. Create a passkey in the WorkOS sign-up screen, then sign in with your passkey.")
    }
    let claims = access_claims(&auth.access_token);
    if (require_passkey || require_session) && claims.sid.as_deref().is_none_or(str::is_empty) {
        err!("WorkOS did not return a session identifier; please sign in again")
    }
    let name = [auth.user.first_name.as_deref(), auth.user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(OIDCAuthenticatedUser {
        refresh_token: auth.refresh_token,
        expires_in: expires_in(&claims),
        access_token: auth.access_token,
        identifier: identifier(&auth.user.id),
        email: auth.user.email.to_lowercase(),
        email_verified: auth.user.email_verified,
        user_name: (!name.is_empty()).then_some(name),
        session_id: claims.sid,
    })
}

/// The WorkOS user id in an identifier this server made ([`identifier`]); `None` for another provider's.
pub fn user_id_of(identifier: &str) -> Option<&str> {
    user_id_in(identifier, &CONFIG.sso_authority())
}

fn user_id_in<'a>(identifier: &'a str, authority: &str) -> Option<&'a str> {
    let id = identifier.strip_prefix(authority)?.strip_prefix('/')?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).then_some(id)
}

/// Deletes a WorkOS user (`DELETE {base}/user_management/users/<id>`), which also ends its sessions. A user WorkOS no
/// longer has counts as deleted, so a retry after a lost answer succeeds.
pub async fn delete_user(user_id: &str) -> ApiResult<()> {
    let mut url = match Url::parse(&format!("{}/user_management/users", api_base()?)) {
        Ok(url) => url,
        Err(e) => err!(format!("Invalid WorkOS users URL: {e}")),
    };
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.push(user_id);
    }
    let response = match http_client()?.delete(url).bearer_auth(api_key()).send().await {
        Ok(r) => r,
        Err(e) => err!(format!("Failed to contact WorkOS: {e}")),
    };
    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    err!(format!("WorkOS refused to delete the user ({})", refusal(status, &text)))
}

/// Ends a WorkOS session (`POST {base}/user_management/sessions/revoke`): its refresh token stops working and the
/// browser cookie of that sign-in no longer signs anyone in silently. A session WorkOS no longer has counts as ended.
pub async fn revoke_session(session_id: &str) -> ApiResult<()> {
    let url = format!("{}/user_management/sessions/revoke", api_base()?);
    let response =
        match http_client()?.post(url).bearer_auth(api_key()).json(&json!({"session_id": session_id})).send().await {
            Ok(r) => r,
            Err(e) => err!(format!("Failed to contact WorkOS: {e}")),
        };
    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    err!(format!("WorkOS refused to end the session ({})", refusal(status, &text)))
}

/// A new access token (and the rotated refresh token) for a WorkOS session.
pub async fn exchange_refresh_token(refresh_token: String) -> ApiResult<RefreshTokenResponse> {
    let body = json!({
        "grant_type": "refresh_token",
        "client_id": CONFIG.sso_client_id(),
        "client_secret": CONFIG.sso_client_secret(),
        "refresh_token": refresh_token,
    });
    let text = authenticate(&body).await?;
    let r: RefreshResponse = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => err!(format!("Unexpected WorkOS refresh answer: {e}")),
    };
    let expires = expires_in(&access_claims(&r.access_token));
    Ok((r.refresh_token, r.access_token, expires))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_workos_authorities() {
        assert!(provider_is_workos("", "https://api.workos.com/user_management/client_01ABC"));
        assert!(provider_is_workos("WorkOS", "http://127.0.0.1:9000/user_management/client_01ABC"));
        assert!(!provider_is_workos("", "https://auth.example.com/realms/reins"));
        assert!(!provider_is_workos("oidc", "https://api.workos.com/user_management/client_01ABC"));
    }

    #[test]
    fn api_base_strips_the_client() {
        assert_eq!(
            api_base_of("https://api.workos.com/user_management/client_01ABC").as_deref(),
            Some("https://api.workos.com")
        );
        assert_eq!(api_base_of("http://127.0.0.1:9000/user_management/c1").as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(api_base_of("https://api.workos.com/user_management/"), None);
        assert_eq!(api_base_of("https://api.workos.com/sso/c1"), None);
    }

    #[test]
    fn user_ids_come_only_from_this_authoritys_identifiers() {
        let authority = "https://api.workos.com/user_management/client_01ABC";
        assert_eq!(user_id_in(&format!("{authority}/user_01XYZ"), authority), Some("user_01XYZ"));
        assert_eq!(user_id_in("https://auth.example.com/realms/reins/user_01XYZ", authority), None);
        assert_eq!(user_id_in(&format!("{authority}user_01XYZ"), authority), None);
        assert_eq!(user_id_in(&format!("{authority}/"), authority), None);
        assert_eq!(user_id_in(&format!("{authority}/../users/other"), authority), None);
        assert_eq!(user_id_in(&format!("{authority}/user_01?x=1"), authority), None);
    }

    fn token(claims: &Value) -> String {
        let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.encode(v.to_string().as_bytes());
        format!("{}.{}.sig", b64(&json!({"alg": "RS256", "kid": "k"})), b64(claims))
    }

    #[test]
    fn reads_the_user_and_session_of_an_authenticate_answer() {
        let exp = chrono::Utc::now().timestamp() + 300;
        let answer = json!({
            "user": {"object": "user", "id": "user_01X", "email": "Me@Example.com", "email_verified": true,
                     "first_name": "Ada", "last_name": "Lovelace"},
            "access_token": token(&json!({"sub": "user_01X", "sid": "session_01S", "exp": exp})),
            "refresh_token": "rt-1",
            "authentication_method": "GoogleOAuth",
        });
        let user = parse_authenticated(&answer.to_string(), false, false).unwrap();
        assert_eq!(user.email, "me@example.com");
        assert_eq!(user.email_verified, Some(true));
        assert_eq!(user.user_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(user.session_id.as_deref(), Some("session_01S"));
        assert_eq!(user.refresh_token.as_deref(), Some("rt-1"));
        assert!(user.identifier.ends_with("/user_01X"));
        let secs = user.expires_in.unwrap().as_secs();
        assert!((290..=300).contains(&secs), "{secs}");
    }

    #[test]
    fn impersonation_and_odd_answers_are_refused() {
        let answer = json!({
            "user": {"id": "user_01X", "email": "me@example.com", "email_verified": true},
            "access_token": "opaque",
            "impersonator": {"email": "admin@example.com", "reason": "support"},
        });
        assert!(parse_authenticated(&answer.to_string(), false, false).is_err());
        assert!(parse_authenticated("{\"access_token\": \"x\"}", false, false).is_err());
        // An opaque access token still signs in; it has no session id or expiry.
        let answer = json!({"user": {"id": "u", "email": "a@b.c"}, "access_token": "opaque"});
        let user = parse_authenticated(&answer.to_string(), false, false).unwrap();
        assert_eq!((user.session_id, user.expires_in, user.email_verified), (None, None, None));
    }

    #[test]
    fn passkey_policy_refuses_other_and_missing_authentication_methods() {
        let mut answer = json!({"user": {"id": "u", "email": "a@b.c", "email_verified": true},
                                "access_token": "opaque"});
        assert!(parse_authenticated(&answer.to_string(), true, true).is_err());
        for method in ["Password", "MagicAuth", "GoogleOAuth", "passkey"] {
            answer["authentication_method"] = json!(method);
            assert!(parse_authenticated(&answer.to_string(), true, true).is_err(), "{method}");
            assert!(parse_authenticated(&answer.to_string(), false, false).is_ok());
        }
        answer["authentication_method"] = json!("Passkey");
        assert!(parse_authenticated(&answer.to_string(), true, true).is_err(), "revocation needs a WorkOS session id");
        answer["access_token"] = json!(token(&json!({"sid": "session_passkey"})));
        assert!(parse_authenticated(&answer.to_string(), true, true).is_ok());
    }

    #[test]
    fn workos_login_methods_keep_a_revocable_reins_session() {
        let mut answer = json!({
            "user": {"id": "u", "email": "a@b.c", "email_verified": true},
            "access_token": token(&json!({"sid": "session_authkit"})),
        });
        for method in ["Password", "MagicAuth", "GoogleOAuth", "GitHubOAuth", "MicrosoftOAuth", "AppleOAuth", "Passkey"]
        {
            answer["authentication_method"] = json!(method);
            let user = parse_authenticated(&answer.to_string(), false, true).unwrap();
            assert_eq!(user.session_id.as_deref(), Some("session_authkit"), "{method}");
            assert_eq!(user.email_verified, Some(true));
        }
        // Email/social sign-in needs the same session revocation support as passkeys.
        for claims in [json!({}), json!({"sid": ""})] {
            answer["access_token"] = json!(token(&claims));
            assert!(parse_authenticated(&answer.to_string(), false, true).is_err());
        }
        answer["access_token"] = json!(token(&json!({"sid": "session_authkit"})));
        answer["impersonator"] = json!({"email": "admin@example.com"});
        assert!(parse_authenticated(&answer.to_string(), false, true).is_err());
    }

    #[test]
    fn refusals_read_both_error_shapes() {
        let s = reqwest::StatusCode::BAD_REQUEST;
        assert_eq!(
            refusal(s, r#"{"error":"invalid_grant","error_description":"The code has expired."}"#),
            "invalid_grant: The code has expired."
        );
        assert_eq!(refusal(s, r#"{"code":"entity_not_found","message":"Nope"}"#), "entity_not_found: Nope");
        assert_eq!(refusal(s, "<html>"), "400");
    }
}
