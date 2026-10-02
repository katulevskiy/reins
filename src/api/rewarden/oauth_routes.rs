//! HTTP adapters for the OAuth authorization server (spec §4.5): discovery documents,
//! dynamic registration, the zero-JavaScript authorize flow with phone pairing, and the
//! token endpoint. All decisions live in `oauth.rs`, `pairing.rs` and `oauth_state.rs`.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use rewarden_proto::{
    desktop,
    pairing::{PushKind, PushMessage},
};
use rocket::{
    Data, Request, Route, State,
    data::ToByteUnit,
    http::{ContentType, Status},
    response::{Redirect, Responder, Response, content::RawHtml, status::Custom},
};
use serde_json::{Value, json};

use super::{
    HUB, REFRESH_TOKEN_SECS,
    device_flow::{DEVICE_GRANT_TTL, DEVICE_GRANTS, POLL_INTERVAL, Poll},
    limits::{self, rate_limited_text, retry_secs},
    now_unix,
    oauth::{
        self, AuthCode, AuthorizeFailure, ClientInfo, OAuthError, TokenGrant, canonical_mcp_url, check_code_redemption,
        device_authorization_response, error_redirect, is_cimd_client_id, issuer, parse_client_metadata,
        parse_device_authorization, parse_form, parse_registration, parse_token_request, redirect_host,
        registration_response, success_redirect, token_response, validate_authorize,
    },
    oauth_state::{AuthSession, OAUTH, StartedPairing},
    outbound, pages,
    pairing::{PairingClient, PairingStatus, sanitize_client_name},
    push,
};
use crate::{
    CONFIG,
    auth::{
        ClientIp,
        rewarden::{hash_token, issue_access_token, random_token},
    },
    db::{
        DbConn, DbPool,
        models::{RewardenClient, RewardenConnection, RewardenDevice, RewardenRefreshToken, User, UserId},
    },
    ratelimit,
};

const MAX_FORM_BYTES: u64 = 16 * 1024;
const CIMD_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_EMAIL_BYTES: usize = 254;
/// Authorize parameter carrying the desktop app's public key (spec: desktop git proxy, keys and pairing).
pub const CLIENT_KEY_PARAM: &str = "rewarden_client_key";

pub fn routes() -> Vec<Route> {
    routes![authorize_get, authorize_post, authorize_wait, token, register, device_authorization]
}

pub fn well_known_routes() -> Vec<Route> {
    routes![prm_root, prm_suffix, as_root, as_suffix]
}

// ---------------------------------------------------------------------------------------
// Responders
// ---------------------------------------------------------------------------------------

/// A JSON body with an explicit status and `Cache-Control: no-store` (RFC 6749 §5.1).
pub struct JsonResponse(pub Status, pub Value);

impl<'r> Responder<'r, 'static> for JsonResponse {
    fn respond_to(self, _: &'r Request<'_>) -> Result<Response<'static>, Status> {
        let body = self.1.to_string();
        Response::build()
            .status(self.0)
            .header(ContentType::JSON)
            .raw_header("Cache-Control", "no-store")
            .raw_header("Pragma", "no-cache")
            .sized_body(body.len(), Cursor::new(body))
            .ok()
    }
}

/// Either an HTML page or a redirect (the authorize flow ends both ways).
pub enum Flow {
    Page(Status, String),
    Redirect(String),
}

impl<'r> Responder<'r, 'static> for Flow {
    fn respond_to(self, req: &'r Request<'_>) -> Result<Response<'static>, Status> {
        match self {
            Self::Page(status, html) => Custom(status, RawHtml(html)).respond_to(req),
            Self::Redirect(url) => Redirect::to(url).respond_to(req),
        }
    }
}

fn oauth_error(e: &OAuthError) -> JsonResponse {
    JsonResponse(Status::from_code(e.status).unwrap_or(Status::BadRequest), e.to_json())
}

fn error_flow(status: Status, title: &str, message: &str) -> Flow {
    Flow::Page(status, pages::error_page(title, message))
}

// ---------------------------------------------------------------------------------------
// Discovery documents
// ---------------------------------------------------------------------------------------

/// The path-inserted well-known suffixes (RFC 8414 §3.1, RFC 9728 §3.1) that name this server.
pub fn well_known_suffix_ok(suffix: &str, domain_path: &str, resource_path: &str) -> bool {
    let prefix = domain_path.trim_matches('/');
    let expected =
        [prefix, resource_path.trim_matches('/')].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>();
    suffix.trim_matches('/') == expected.join("/")
}

fn document(suffix: &Path, resource_path: &str, doc: Value) -> JsonResponse {
    if well_known_suffix_ok(&suffix.to_string_lossy(), &CONFIG.domain_path(), resource_path) {
        JsonResponse(Status::Ok, doc)
    } else {
        JsonResponse(Status::NotFound, json!({"error": "not_found"}))
    }
}

#[get("/.well-known/oauth-protected-resource")]
fn prm_root() -> JsonResponse {
    JsonResponse(Status::Ok, oauth::protected_resource_metadata(&CONFIG.domain()))
}

#[get("/.well-known/oauth-protected-resource/<suffix..>")]
#[expect(clippy::needless_pass_by_value, reason = "Rocket's `<suffix..>` guard yields an owned PathBuf")]
fn prm_suffix(suffix: PathBuf) -> JsonResponse {
    document(&suffix, "mcp", oauth::protected_resource_metadata(&CONFIG.domain()))
}

#[get("/.well-known/oauth-authorization-server")]
fn as_root() -> JsonResponse {
    JsonResponse(Status::Ok, oauth::authorization_server_metadata(&CONFIG.domain()))
}

#[get("/.well-known/oauth-authorization-server/<suffix..>")]
#[expect(clippy::needless_pass_by_value, reason = "Rocket's `<suffix..>` guard yields an owned PathBuf")]
fn as_suffix(suffix: PathBuf) -> JsonResponse {
    document(&suffix, "", oauth::authorization_server_metadata(&CONFIG.domain()))
}

// ---------------------------------------------------------------------------------------
// Clients
// ---------------------------------------------------------------------------------------

async fn read_limited(data: Data<'_>, limit: u64) -> Option<Vec<u8>> {
    let bytes = data.open(limit.bytes()).into_bytes().await.ok()?;
    bytes.is_complete().then(|| bytes.into_inner())
}

/// Fetches a Client ID Metadata Document (Decision 25): through the outbound guard (https, public addresses only, checked
/// again on every resolved address), no redirects, size-capped. Anyone may name any URL here, so it is the same guard
/// as for the phone's requests, independent of `HTTP_REQUEST_BLOCK_NON_GLOBAL_IPS`.
async fn fetch_client_metadata(client_id: &str) -> Result<ClientInfo, String> {
    let mut response = outbound::get_once(client_id, "application/json", CIMD_TIMEOUT)
        .await
        .map_err(|e| format!("could not fetch the client metadata document: {e}"))?;
    if response.status().is_redirection() {
        return Err("the client metadata document URL redirected".to_owned());
    }
    if !response.status().is_success() {
        return Err(format!("the client metadata document answered {}", response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("could not read the client metadata: {e}"))? {
        if body.len() + chunk.len() > oauth::MAX_CLIENT_METADATA_BYTES {
            return Err("the client metadata document is too large".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    parse_client_metadata(client_id, &body)
}

async fn resolve_client(client_id: &str, conn: &DbConn) -> Result<ClientInfo, String> {
    if is_cimd_client_id(client_id) {
        if let Some(cached) = OAUTH.cached_client(client_id) {
            return Ok(cached);
        }
        let client = fetch_client_metadata(client_id).await?;
        OAUTH.cache_client(client.clone());
        return Ok(client);
    }
    let client = RewardenClient::find(client_id, conn).await.ok_or_else(|| "unknown client_id".to_owned())?;
    Ok(ClientInfo {
        redirect_uris: client.redirect_uri_list(),
        client_id: client.client_id,
        client_name: client.client_name,
    })
}

/// RFC 7591 dynamic registration (public clients only).
#[post("/rewarden/oauth/register", data = "<data>")]
async fn register(data: Data<'_>, ip: ClientIp, conn: DbConn) -> JsonResponse {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down"}));
    }
    // Every registration is a row that stays: a tighter budget than the shared unauthenticated one.
    if let Err(wait) = limits::REGISTRATIONS.check(&ip.ip) {
        let description = rate_limited_text("too many client registrations from this address", wait);
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down", "error_description": description}));
    }
    let Some(body) = read_limited(data, oauth::MAX_CLIENT_METADATA_BYTES as u64 + 1).await else {
        return oauth_error(&OAuthError::new(400, "invalid_client_metadata", "registration request is too large"));
    };
    let registration = match parse_registration(&body) {
        Ok(r) => r,
        Err(e) => return oauth_error(&e),
    };
    let client =
        RewardenClient::new(sanitize_client_name(&registration.client_name), &registration.redirect_uris, now_unix());
    if let Err(e) = client.save(&conn).await {
        error!("Rewarden client registration failed: {e:?}");
        return JsonResponse(Status::InternalServerError, json!({"error": "server_error"}));
    }
    JsonResponse(
        Status::Created,
        registration_response(&client.client_id, &client.client_name, &registration.redirect_uris, client.created_at),
    )
}

// ---------------------------------------------------------------------------------------
// Authorize flow
// ---------------------------------------------------------------------------------------

fn mcp_url() -> String {
    canonical_mcp_url(&CONFIG.domain())
}

fn wait_location(session: &str) -> String {
    format!("{}/rewarden/oauth/authorize/wait?session={session}", CONFIG.domain_path())
}

/// The optional `rewarden_client_key` the desktop app adds to the authorize URL. Present means it must be a valid key:
/// a malformed one is refused rather than dropped, so the phone never pairs the app without the key it expects.
fn desktop_client_key(params: &HashMap<String, String>) -> Result<Option<String>, ()> {
    match params.get(CLIENT_KEY_PARAM) {
        None => Ok(None),
        Some(key) if desktop::decode_key(key).is_some() => Ok(Some(key.clone())),
        Some(_) => Err(()),
    }
}

fn key_fingerprint_of(client: &PairingClient) -> Option<String> {
    client.client_key.as_deref().and_then(desktop::key_fingerprint)
}

/// Step 1: validate the request and ask for the account email.
#[get("/rewarden/oauth/authorize?<params..>")]
async fn authorize_get(params: HashMap<String, String>, ip: ClientIp, conn: DbConn) -> Flow {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return error_flow(Status::TooManyRequests, "Too many requests", "Please wait a moment and try again.");
    }
    let Some(client_id) = params.get("client_id").filter(|c| !c.is_empty()) else {
        return error_flow(Status::BadRequest, "Cannot connect", "The request has no client_id.");
    };
    let client = match resolve_client(client_id, &conn).await {
        Ok(c) => c,
        Err(e) => {
            return error_flow(Status::BadRequest, "Cannot connect", &format!("The client could not be verified: {e}"));
        }
    };
    let request = match validate_authorize(&params, &client.redirect_uris, &mcp_url()) {
        Ok(r) => r,
        Err(AuthorizeFailure::Page(message)) => return error_flow(Status::BadRequest, "Cannot connect", &message),
        Err(AuthorizeFailure::Redirect {
            redirect_uri,
            state,
            error,
            description,
        }) => {
            return Flow::Redirect(error_redirect(
                &redirect_uri,
                error,
                &description,
                state.as_deref(),
                &issuer(&CONFIG.domain()),
            ));
        }
    };
    let Ok(client_key) = desktop_client_key(&params) else {
        return error_flow(Status::BadRequest, "Cannot connect", "The desktop app key is not valid.");
    };
    let pairing_client = PairingClient {
        client_id: client.client_id,
        client_name: sanitize_client_name(&client.client_name),
        client_host: redirect_host(&request.redirect_uri),
        client_key,
    };
    let form =
        (pairing_client.client_name.clone(), pairing_client.client_host.clone(), key_fingerprint_of(&pairing_client));
    let session = AuthSession {
        request,
        client: pairing_client,
        pairing: None,
    };
    match OAUTH.create_session(session) {
        Ok(id) => Flow::Page(Status::Ok, pages::email_form_page(&form.0, &form.1, &id, None, form.2.as_deref())),
        Err(_) => error_flow(Status::ServiceUnavailable, "Server busy", "Please try again in a few minutes."),
    }
}

/// The user (and approval device) an email address stands for, if a real pairing is possible.
async fn pairing_target(email: &str, conn: &DbConn) -> Option<(UserId, RewardenDevice)> {
    let user = User::find_by_mail(email, conn).await.filter(|u| u.enabled)?;
    let device = RewardenDevice::find_by_user(&user.uuid, conn).await?;
    Some((user.uuid, device))
}

/// Step 2: start the pairing for the submitted email. Unknown emails get an identical decoy.
#[post("/rewarden/oauth/authorize", data = "<data>")]
async fn authorize_post(data: Data<'_>, ip: ClientIp, conn: DbConn, pool: &State<DbPool>) -> Flow {
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return error_flow(Status::PayloadTooLarge, "Request too large", "Please try again.");
    };
    let form = parse_form(&body);
    let Some(session_id) = form.get("session").filter(|s| !s.is_empty()) else {
        return error_flow(Status::BadRequest, "Link expired", "Start the connection again from your AI app.");
    };
    let Some(session) = OAUTH.session(session_id) else {
        return error_flow(Status::BadRequest, "Link expired", "Start the connection again from your AI app.");
    };
    let form_page = |status: Status, error: &str| {
        Flow::Page(
            status,
            pages::email_form_page(
                &session.client.client_name,
                &session.client.client_host,
                session_id,
                Some(error),
                key_fingerprint_of(&session.client).as_deref(),
            ),
        )
    };
    if ratelimit::check_limit_login(&ip.ip).is_err() {
        return form_page(Status::TooManyRequests, "Too many attempts. Please wait a minute and try again.");
    }
    if session.pairing.is_some() {
        return Flow::Redirect(wait_location(session_id));
    }
    let email = form.get("email").map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if email.is_empty() || email.len() > MAX_EMAIL_BYTES || !email.contains('@') {
        return form_page(Status::BadRequest, "Enter a valid email address.");
    }
    // Each pairing pushes to the account's phone. Counted per email whether or not it has an account, so the limit
    // tells nothing about which emails exist.
    if let Err(wait) = limits::PAIRINGS.check(&email) {
        let message =
            format!("Too many connection requests for this account. Please try again in {} s.", retry_secs(wait));
        return form_page(Status::TooManyRequests, &message);
    }
    let now = now_unix();
    let target = pairing_target(&email, &conn).await;
    let started = match &target {
        Some((user_uuid, _)) => HUB.pairings.start(&user_uuid.to_string(), session.client.clone(), now),
        None => HUB.pairings.start_decoy(session.client.clone(), now),
    };
    let Ok((request, code)) = started else {
        return error_flow(Status::ServiceUnavailable, "Server busy", "Please try again in a few minutes.");
    };
    if let Some((user_uuid, device)) = &target {
        push::spawn_push(
            pool.inner().clone(),
            user_uuid.clone(),
            device.fcm_token.clone(),
            PushMessage {
                t: PushKind::Pair,
                id: request.id.0.clone(),
            },
        );
    }
    OAUTH.set_pairing(
        session_id,
        StartedPairing {
            id: request.id,
            code,
            user_uuid: target.map(|(user_uuid, _)| user_uuid.to_string()),
        },
    );
    Flow::Redirect(wait_location(session_id))
}

/// Step 3: show the code; once the phone has answered, hand the result back to the client.
#[get("/rewarden/oauth/authorize/wait?<session>")]
fn authorize_wait(session: &str) -> Flow {
    let expired = || error_flow(Status::BadRequest, "Link expired", "Start the connection again from your AI app.");
    let Some(current) = OAUTH.session(session) else {
        return expired();
    };
    let Some(pairing) = current.pairing.clone() else {
        return expired();
    };
    let iss = issuer(&CONFIG.domain());
    let deny = |description: &str| {
        Flow::Redirect(error_redirect(
            &current.request.redirect_uri,
            "access_denied",
            description,
            current.request.state.as_deref(),
            &iss,
        ))
    };
    match HUB.pairings.status(&pairing.id) {
        Some(PairingStatus::Waiting) => {
            let fingerprint = key_fingerprint_of(&current.client);
            Flow::Page(Status::Ok, pages::wait_page(&current.client.client_name, pairing.code, fingerprint.as_deref()))
        }
        Some(PairingStatus::Approved {
            connection_id,
        }) => {
            // Only the request that removes the session may mint the (single) code.
            let Some(session) = OAUTH.take_session(session) else {
                return expired();
            };
            let Some(user_uuid) = pairing.user_uuid else {
                return deny("The request was not approved.");
            };
            let code = AuthCode {
                user_uuid,
                connection_uuid: connection_id.0,
                client_id: session.request.client_id.clone(),
                redirect_uri: session.request.redirect_uri.clone(),
                code_challenge: session.request.code_challenge.clone(),
            };
            match OAUTH.issue_code(code) {
                Ok(token) => Flow::Redirect(success_redirect(
                    &session.request.redirect_uri,
                    &token,
                    session.request.state.as_deref(),
                    &iss,
                )),
                Err(_) => Flow::Redirect(error_redirect(
                    &session.request.redirect_uri,
                    "server_error",
                    "The server is busy, please try again.",
                    session.request.state.as_deref(),
                    &iss,
                )),
            }
        }
        Some(PairingStatus::Rejected) => {
            OAUTH.take_session(session);
            deny("The request was denied on the phone.")
        }
        None => {
            OAUTH.take_session(session);
            deny("The request timed out.")
        }
    }
}

// ---------------------------------------------------------------------------------------
// Device authorization (RFC 8628): the desktop app shows a QR code, the phone scans it
// ---------------------------------------------------------------------------------------

/// RFC 8628 §3.1-3.2: a grant for the desktop app (or any public client), which the phone of an account claims by
/// scanning its code (`device_api`, A6b). Answers with the codes to show and how often to poll the token endpoint.
#[post("/rewarden/oauth/device_authorization", data = "<data>")]
async fn device_authorization(data: Data<'_>, ip: ClientIp, conn: DbConn) -> JsonResponse {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down"}));
    }
    if let Err(wait) = limits::DEVICE_AUTHORIZATIONS.check(&ip.ip) {
        let description = rate_limited_text("too many pairing codes for this address", wait);
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down", "error_description": description}));
    }
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return oauth_error(&OAuthError::invalid_request("request body too large"));
    };
    let request = match parse_device_authorization(&parse_form(&body), &mcp_url(), CLIENT_KEY_PARAM) {
        Ok(r) => r,
        Err(e) => return oauth_error(&e),
    };
    let client = match resolve_client(&request.client_id, &conn).await {
        Ok(c) => c,
        Err(e) => return oauth_error(&OAuthError::new(401, "invalid_client", e)),
    };
    // What the phone shows under the name, as for the authorize flow: the host the client registered.
    let client_host = client.redirect_uris.first().map(|uri| redirect_host(uri)).unwrap_or_default();
    let pairing_client = PairingClient {
        client_id: client.client_id,
        client_name: sanitize_client_name(&client.client_name),
        client_host,
        client_key: request.client_key,
    };
    match DEVICE_GRANTS.start(pairing_client) {
        Ok(started) => JsonResponse(
            Status::Ok,
            device_authorization_response(
                &CONFIG.domain(),
                &started.device_code,
                &started.user_code,
                started.confirm_code,
                DEVICE_GRANT_TTL.as_secs(),
                POLL_INTERVAL.as_secs(),
            ),
        ),
        Err(_) => JsonResponse(
            Status::ServiceUnavailable,
            json!({"error": "temporarily_unavailable", "error_description": "the server is busy, try again soon"}),
        ),
    }
}

/// RFC 8628 §3.5: one poll of the desktop app.
async fn poll_device_code(device_code: &str, client_id: &str, conn: &DbConn) -> Result<Value, OAuthError> {
    let pending = |error: &'static str, description: &str| OAuthError::new(400, error, description);
    match DEVICE_GRANTS.poll(device_code, client_id, &HUB.pairings) {
        Poll::Pending => Err(pending("authorization_pending", "waiting for the phone to approve")),
        Poll::SlowDown => Err(pending("slow_down", "polling too often; wait 5 seconds more between polls")),
        Poll::Denied => Err(pending("access_denied", "the pairing was denied on the phone")),
        Poll::Expired => Err(pending("expired_token", "the pairing code expired or was used; start again")),
        Poll::WrongClient => Err(OAuthError::invalid_grant("the device code belongs to another client")),
        Poll::Approved {
            user,
            connection,
        } => {
            let user = UserId::from(user);
            let connection = RewardenConnection::find_by_uuid_and_user(&connection.0, &user, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, conn).await
        }
    }
}

// ---------------------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------------------

/// Issues a rotated access/refresh pair for `connection`.
async fn issue_tokens(connection: &RewardenConnection, conn: &DbConn) -> Result<Value, OAuthError> {
    let refresh = random_token();
    let row = RewardenRefreshToken {
        token_hash: hash_token(&refresh),
        connection_uuid: connection.uuid.clone(),
        expires_at: now_unix() + REFRESH_TOKEN_SECS,
    };
    if let Err(e) = row.save(conn).await {
        error!("Rewarden refresh token could not be stored: {e:?}");
        return Err(OAuthError::new(500, "server_error", "could not store the refresh token"));
    }
    let access =
        issue_access_token(&connection.user_uuid.to_string(), &connection.uuid, &connection.client_id, &mcp_url());
    Ok(token_response(&access, &refresh))
}

async fn exchange(grant: TokenGrant, conn: &DbConn) -> Result<Value, OAuthError> {
    match grant {
        TokenGrant::AuthorizationCode {
            code,
            redirect_uri,
            client_id,
            code_verifier,
        } => {
            let issued =
                OAUTH.redeem_code(&code).ok_or_else(|| OAuthError::invalid_grant("unknown, used or expired code"))?;
            check_code_redemption(&issued, &client_id, &redirect_uri, &code_verifier)?;
            let user = UserId::from(issued.user_uuid.clone());
            let connection = RewardenConnection::find_by_uuid_and_user(&issued.connection_uuid, &user, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, conn).await
        }
        TokenGrant::RefreshToken {
            refresh_token,
            client_id,
        } => {
            let row = RewardenRefreshToken::take(&hash_token(&refresh_token), now_unix(), conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the refresh token is invalid or expired"))?;
            let connection = RewardenConnection::find_by_uuid(&row.connection_uuid, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            if client_id.is_some_and(|c| c != connection.client_id) {
                return Err(OAuthError::invalid_grant("the refresh token belongs to another client"));
            }
            issue_tokens(&connection, conn).await
        }
        TokenGrant::DeviceCode {
            device_code,
            client_id,
        } => poll_device_code(&device_code, &client_id, conn).await,
    }
}

/// RFC 6749 token endpoint (form-encoded, public clients, PKCE).
#[post("/rewarden/oauth/token", data = "<data>")]
async fn token(data: Data<'_>, ip: ClientIp, conn: DbConn) -> JsonResponse {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down"}));
    }
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return oauth_error(&OAuthError::invalid_request("request body too large"));
    };
    let grant = match parse_token_request(&parse_form(&body), &mcp_url()) {
        Ok(g) => g,
        Err(e) => return oauth_error(&e),
    };
    match exchange(grant, &conn).await {
        Ok(tokens) => JsonResponse(Status::Ok, tokens),
        Err(e) => oauth_error(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_inserted_well_known_suffixes() {
        // Server at the domain root.
        assert!(well_known_suffix_ok("mcp", "", "mcp"));
        assert!(well_known_suffix_ok("/mcp/", "", "mcp"));
        assert!(!well_known_suffix_ok("other", "", "mcp"));
        assert!(!well_known_suffix_ok("mcp/extra", "", "mcp"));
        assert!(well_known_suffix_ok("", "", ""));
        // Server under a path prefix.
        assert!(well_known_suffix_ok("vw/mcp", "/vw", "mcp"));
        assert!(!well_known_suffix_ok("mcp", "/vw", "mcp"));
        assert!(well_known_suffix_ok("vw", "/vw", ""));
        assert!(well_known_suffix_ok("vw", "/vw/", ""));
        assert!(!well_known_suffix_ok("", "/vw", ""));
    }

    #[test]
    fn the_desktop_key_is_optional_but_must_be_valid_when_given() {
        let key = desktop::encode_key(&[9u8; 32]);
        let params = |v: Option<&str>| {
            let mut p = HashMap::from([("client_id".to_owned(), "c".to_owned())]);
            if let Some(v) = v {
                p.insert(CLIENT_KEY_PARAM.to_owned(), v.to_owned());
            }
            p
        };
        assert_eq!(desktop_client_key(&params(None)), Ok(None));
        assert_eq!(desktop_client_key(&params(Some(&key))), Ok(Some(key.clone())));
        let padded = format!("{key}=");
        let short = desktop::encode_key(&[9u8; 32])[..40].to_owned();
        for bad in ["", "not a key", short.as_str(), padded.as_str()] {
            assert_eq!(desktop_client_key(&params(Some(bad))), Err(()), "{bad:?}");
        }
    }

    #[test]
    fn wait_location_keeps_the_domain_path() {
        let loc = wait_location("abc");
        assert!(loc.ends_with("/rewarden/oauth/authorize/wait?session=abc"), "{loc}");
    }
}
