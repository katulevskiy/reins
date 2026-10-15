//! HTTP adapters for the OAuth authorization server (spec §4.5): discovery documents,
//! dynamic registration, the zero-JavaScript authorize flow with phone pairing, and the
//! token endpoint. All decisions live in `oauth.rs`, `pairing.rs` and `oauth_state.rs`.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use reins_proto::{
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
        self, AssertionError, AuthCode, AuthMethod, AuthorizeFailure, ClientAuth, ClientInfo, JwksSource, OAuthError,
        TokenGrant, TokenRequest, VerifiedAssertion, assertion_audiences, canonical_mcp_url, check_code_redemption,
        device_authorization_response, error_redirect, is_cimd_client_id, is_loopback_cimd_client_id, issuer,
        parse_client_assertion, parse_client_metadata, parse_device_authorization, parse_form, parse_jwks,
        parse_registration, parse_token_request, redirect_host, registration_response, success_redirect,
        token_response, validate_authorize, verify_client_assertion,
    },
    oauth_state::{AuthSession, JtiError, OAUTH, StartedPairing},
    outbound, pages,
    pairing::{PairingClient, PairingStatus, sanitize_client_name},
    push,
};
use crate::{
    CONFIG,
    auth::{
        ClientIp,
        reins::{hash_token, issue_access_token, random_token},
    },
    db::{
        DbConn, DbPool,
        models::{ReinsClient, ReinsConnection, ReinsDevice, ReinsRefreshToken, User, UserId},
    },
    ratelimit,
};

const MAX_FORM_BYTES: u64 = 16 * 1024;
const CIMD_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_EMAIL_BYTES: usize = 254;
/// Authorize parameter carrying the desktop app's public key (spec: desktop git proxy, keys and pairing).
pub const CLIENT_KEY_PARAM: &str = "reins_client_key";

pub fn routes() -> Vec<Route> {
    routes![authorize_get, authorize_post, authorize_wait, token, revoke, register, device_authorization]
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

fn error_flow(at: &str, status: Status, title: &str, message: &str) -> Flow {
    Flow::Page(status, pages::error_page(at, title, message))
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

/// Fetches a small JSON document a client names (its Client ID Metadata Document, Decision 25, or its `jwks_uri`):
/// through the outbound guard (https, public addresses only, checked again on every resolved address), no redirects,
/// at most `limit` bytes. Anyone may name any URL here, so it is the same guard as for the phone's requests,
/// independent of `HTTP_REQUEST_BLOCK_NON_GLOBAL_IPS`.
async fn fetch_document(url: &str, limit: usize, what: &str) -> Result<Vec<u8>, String> {
    let mut response = outbound::get_once(url, "application/json", CIMD_TIMEOUT)
        .await
        .map_err(|e| format!("could not fetch the {what}: {e}"))?;
    if response.status().is_redirection() {
        return Err(format!("the {what} URL redirected"));
    }
    if !response.status().is_success() {
        return Err(format!("the {what} answered {}", response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("could not read the {what}: {e}"))? {
        if body.len() + chunk.len() > limit {
            return Err(format!("the {what} is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn fetch_client_metadata(client_id: &str) -> Result<ClientInfo, String> {
    let body = fetch_document(client_id, oauth::MAX_CLIENT_METADATA_BYTES, "client metadata document").await?;
    parse_client_metadata(client_id, &body)
}

/// A client's key set from its `jwks_uri`, fetched under the same guard and cached for a few minutes.
async fn fetch_jwks(jwks_uri: &str) -> Result<jsonwebtoken::jwk::JwkSet, OAuthError> {
    let fetched = fetch_document(jwks_uri, oauth::MAX_JWKS_BYTES, "client's JWKS").await.and_then(|b| parse_jwks(&b));
    let set = fetched.map_err(OAuthError::invalid_client)?;
    OAUTH.cache_jwks(jwks_uri, set.clone());
    Ok(set)
}

/// A `client_id` that names a metadata document (in the test mode also `http://` on loopback, for mock servers).
fn is_metadata_client_id(client_id: &str) -> bool {
    is_cimd_client_id(client_id) || (outbound::allow_loopback() && is_loopback_cimd_client_id(client_id))
}

async fn resolve_client(client_id: &str, conn: &DbConn) -> Result<ClientInfo, String> {
    if is_metadata_client_id(client_id) {
        if let Some(cached) = OAUTH.cached_client(client_id) {
            return Ok(cached);
        }
        let client = fetch_client_metadata(client_id).await?;
        OAUTH.cache_client(client.clone());
        return Ok(client);
    }
    let client = ReinsClient::find(client_id, conn).await.ok_or_else(|| "unknown client_id".to_owned())?;
    Ok(ClientInfo {
        redirect_uris: client.redirect_uri_list(),
        client_id: client.client_id,
        client_name: client.client_name,
        // Dynamic registration accepts public clients only.
        auth: ClientAuth::None,
    })
}

/// RFC 7591 dynamic registration (public clients only).
#[post("/reins/oauth/register", data = "<data>")]
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
        ReinsClient::new(sanitize_client_name(&registration.client_name), &registration.redirect_uris, now_unix());
    if let Err(e) = client.save(&conn).await {
        error!("Reins client registration failed: {e:?}");
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
    format!("{}/reins/oauth/authorize/wait?session={session}", CONFIG.domain_path())
}

/// The optional `reins_client_key` the desktop app adds to the authorize URL. Present means it must be a valid key:
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
#[get("/reins/oauth/authorize?<params..>")]
async fn authorize_get(params: HashMap<String, String>, ip: ClientIp, conn: DbConn) -> Flow {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return error_flow(
            pages::at::AUTHORIZE,
            Status::TooManyRequests,
            "Too many requests",
            "Please wait a moment and try again.",
        );
    }
    let Some(client_id) = params.get("client_id").filter(|c| !c.is_empty()) else {
        return error_flow(pages::at::AUTHORIZE, Status::BadRequest, "Cannot connect", "The request has no client_id.");
    };
    let client = match resolve_client(client_id, &conn).await {
        Ok(c) => c,
        Err(e) => {
            return error_flow(
                pages::at::AUTHORIZE,
                Status::BadRequest,
                "Cannot connect",
                &format!("The client could not be verified: {e}"),
            );
        }
    };
    let request = match validate_authorize(&params, &client.redirect_uris, &mcp_url()) {
        Ok(r) => r,
        Err(AuthorizeFailure::Page(message)) => {
            return error_flow(pages::at::AUTHORIZE, Status::BadRequest, "Cannot connect", &message);
        }
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
        return error_flow(
            pages::at::AUTHORIZE,
            Status::BadRequest,
            "Cannot connect",
            "The desktop app key is not valid.",
        );
    };
    let client_auth = client.auth.method();
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
        client_auth,
        pairing: None,
    };
    match OAUTH.create_session(session) {
        Ok(id) => Flow::Page(Status::Ok, pages::email_form_page(&form.0, &form.1, &id, None, form.2.as_deref())),
        Err(_) => error_flow(
            pages::at::AUTHORIZE,
            Status::ServiceUnavailable,
            "Server busy",
            "Please try again in a few minutes.",
        ),
    }
}

/// The user (and approval device) an email address stands for, if a real pairing is possible.
async fn pairing_target(email: &str, conn: &DbConn) -> Option<(UserId, ReinsDevice)> {
    let user = User::find_by_mail(email, conn).await.filter(|u| u.enabled)?;
    let device = ReinsDevice::find_by_user(&user.uuid, conn).await?;
    Some((user.uuid, device))
}

/// Step 2: start the pairing for the submitted email. Unknown emails get an identical decoy.
#[post("/reins/oauth/authorize", data = "<data>")]
async fn authorize_post(data: Data<'_>, ip: ClientIp, conn: DbConn, pool: &State<DbPool>) -> Flow {
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return error_flow(pages::at::AUTHORIZE, Status::PayloadTooLarge, "Request too large", "Please try again.");
    };
    let form = parse_form(&body);
    let Some(session_id) = form.get("session").filter(|s| !s.is_empty()) else {
        return error_flow(
            pages::at::AUTHORIZE,
            Status::BadRequest,
            "Link expired",
            "Start the connection again from your AI app.",
        );
    };
    let Some(session) = OAUTH.session(session_id) else {
        return error_flow(
            pages::at::AUTHORIZE,
            Status::BadRequest,
            "Link expired",
            "Start the connection again from your AI app.",
        );
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
        return error_flow(
            pages::at::AUTHORIZE,
            Status::ServiceUnavailable,
            "Server busy",
            "Please try again in a few minutes.",
        );
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
#[get("/reins/oauth/authorize/wait?<session>")]
fn authorize_wait(session: &str) -> Flow {
    let expired = || {
        error_flow(pages::at::WAIT, Status::BadRequest, "Link expired", "Start the connection again from your AI app.")
    };
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
                client_auth: session.client_auth,
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
#[post("/reins/oauth/device_authorization", data = "<data>")]
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
        Err(e) => return oauth_error(&OAuthError::invalid_client(e)),
    };
    // The device flow is for public clients: a client with keys signs in through the authorize flow.
    if client.auth.method() != AuthMethod::None {
        return oauth_error(&OAuthError::new(
            400,
            "unauthorized_client",
            "the device flow is only for public clients; this client authenticates with private_key_jwt",
        ));
    }
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
        Poll::SlowDown {
            interval,
        } => Err(pending("slow_down", &format!("polling too often; wait {} s between polls", interval.as_secs()))),
        Poll::Denied => Err(pending("access_denied", "the pairing was denied on the phone")),
        Poll::Expired => Err(pending("expired_token", "the pairing code expired or was used; start again")),
        Poll::WrongClient => Err(OAuthError::invalid_grant("the device code belongs to another client")),
        Poll::Approved {
            user,
            connection,
        } => {
            let user = UserId::from(user);
            let connection = ReinsConnection::find_by_uuid_and_user(&connection.0, &user, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, AuthMethod::None, conn).await
        }
    }
}

// ---------------------------------------------------------------------------------------
// Client authentication (RFC 7523 private_key_jwt)
// ---------------------------------------------------------------------------------------

/// Verifies `assertion` with the client's keys. A key set from `jwks_uri` comes from the cache when it can; an
/// unknown `kid` in a cached set fetches it once more (rate-limited per URI) in case the client rotated its keys.
async fn verify_with_client_keys(
    client_id: &str,
    jwks: &JwksSource,
    alg: Option<jsonwebtoken::Algorithm>,
    assertion: &str,
) -> Result<VerifiedAssertion, OAuthError> {
    let audiences = assertion_audiences(&CONFIG.domain());
    let verify = |set: &jsonwebtoken::jwk::JwkSet| {
        verify_client_assertion(assertion, client_id, set, alg, &audiences, now_unix())
    };
    match jwks {
        JwksSource::Inline(set) => verify(set).map_err(AssertionError::into_oauth),
        JwksSource::Uri(uri) => {
            let (set, fresh) = match OAUTH.cached_jwks(uri) {
                Some(set) => (set, false),
                None => (fetch_jwks(uri).await?, true),
            };
            match verify(&set) {
                Err(AssertionError::UnknownKey) if !fresh && OAUTH.may_refetch_jwks(uri) => {
                    verify(&fetch_jwks(uri).await?).map_err(AssertionError::into_oauth)
                }
                result => result.map_err(AssertionError::into_oauth),
            }
        }
    }
}

/// Client authentication at the token and revocation endpoints. `required` is the method bound to the grant (the
/// authorization code or refresh token): a public client must not send an assertion, and a `private_key_jwt` client
/// must send one that verifies against its current keys, with a `jti` never seen before.
async fn authenticate_client(
    client_id: &str,
    required: AuthMethod,
    assertion: Option<&str>,
    conn: &DbConn,
) -> Result<(), OAuthError> {
    let assertion = match (required, assertion) {
        (AuthMethod::None, None) => return Ok(()),
        (AuthMethod::None, Some(_)) => {
            return Err(OAuthError::invalid_client("this client is public and must not send a client assertion"));
        }
        (AuthMethod::PrivateKeyJwt, None) => {
            return Err(OAuthError::invalid_client(
                "this client authenticates with private_key_jwt: client_assertion is required",
            ));
        }
        (AuthMethod::PrivateKeyJwt, Some(assertion)) => assertion,
    };
    let client = resolve_client(client_id, conn).await.map_err(OAuthError::invalid_client)?;
    let ClientAuth::PrivateKeyJwt {
        jwks,
        alg,
    } = &client.auth
    else {
        return Err(OAuthError::invalid_client("the client does not declare private_key_jwt"));
    };
    let verified = verify_with_client_keys(client_id, jwks, *alg, assertion).await?;
    match OAUTH.use_assertion(client_id, &verified.jti) {
        Ok(()) => Ok(()),
        Err(JtiError::Replayed) => Err(OAuthError::invalid_client("the client assertion was already used")),
        Err(JtiError::Full) => {
            Err(OAuthError::new(503, "temporarily_unavailable", "the server is busy, try again in a few minutes"))
        }
    }
}

// ---------------------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------------------

/// Issues a rotated access/refresh pair for `connection`; the refresh token remembers how the client authenticated.
async fn issue_tokens(
    connection: &ReinsConnection,
    client_auth: AuthMethod,
    conn: &DbConn,
) -> Result<Value, OAuthError> {
    let refresh = random_token();
    let row = ReinsRefreshToken {
        token_hash: hash_token(&refresh),
        connection_uuid: connection.uuid.clone(),
        expires_at: now_unix() + REFRESH_TOKEN_SECS,
        client_auth: client_auth.as_str().to_owned(),
    };
    if let Err(e) = row.save(conn).await {
        error!("Reins refresh token could not be stored: {e:?}");
        return Err(OAuthError::new(500, "server_error", "could not store the refresh token"));
    }
    let access =
        issue_access_token(&connection.user_uuid.to_string(), &connection.uuid, &connection.client_id, &mcp_url());
    Ok(token_response(&access, &refresh))
}

async fn exchange(request: TokenRequest, conn: &DbConn) -> Result<Value, OAuthError> {
    let assertion = request.client_assertion.as_deref();
    match request.grant {
        TokenGrant::AuthorizationCode {
            code,
            redirect_uri,
            client_id,
            code_verifier,
        } => {
            let issued =
                OAUTH.redeem_code(&code).ok_or_else(|| OAuthError::invalid_grant("unknown, used or expired code"))?;
            check_code_redemption(&issued, &client_id, &redirect_uri, &code_verifier)?;
            // The client the code was issued to, authenticating the way it did when it started the flow.
            authenticate_client(&issued.client_id, issued.client_auth, assertion, conn).await?;
            let user = UserId::from(issued.user_uuid.clone());
            let connection = ReinsConnection::find_by_uuid_and_user(&issued.connection_uuid, &user, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, issued.client_auth, conn).await
        }
        TokenGrant::RefreshToken {
            refresh_token,
            client_id,
        } => {
            let invalid = || OAuthError::invalid_grant("the refresh token is invalid or expired");
            let token_hash = hash_token(&refresh_token);
            let row = ReinsRefreshToken::find(&token_hash, conn)
                .await
                .filter(|row| now_unix() < row.expires_at)
                .ok_or_else(invalid)?;
            let connection = ReinsConnection::find_by_uuid(&row.connection_uuid, conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            if client_id.is_some_and(|c| c != connection.client_id) {
                return Err(OAuthError::invalid_grant("the refresh token belongs to another client"));
            }
            // Authenticated before the token is used up, so a request without the client's key cannot burn it.
            let client_auth = AuthMethod::from_stored(&row.client_auth);
            authenticate_client(&connection.client_id, client_auth, assertion, conn).await?;
            ReinsRefreshToken::take(&token_hash, now_unix(), conn).await.ok_or_else(invalid)?;
            issue_tokens(&connection, client_auth, conn).await
        }
        TokenGrant::DeviceCode {
            device_code,
            client_id,
        } => {
            // Only public clients get device codes (`device_authorization`).
            authenticate_client(&client_id, AuthMethod::None, assertion, conn).await?;
            poll_device_code(&device_code, &client_id, conn).await
        }
    }
}

/// RFC 6749 token endpoint (form-encoded, PKCE; public clients and RFC 7523 `private_key_jwt` clients).
#[post("/reins/oauth/token", data = "<data>")]
async fn token(data: Data<'_>, ip: ClientIp, conn: DbConn) -> JsonResponse {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down"}));
    }
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return oauth_error(&OAuthError::invalid_request("request body too large"));
    };
    let request = match parse_token_request(&parse_form(&body), &mcp_url()) {
        Ok(r) => r,
        Err(e) => return oauth_error(&e),
    };
    match exchange(request, &conn).await {
        Ok(tokens) => JsonResponse(Status::Ok, tokens),
        Err(e) => oauth_error(&e),
    }
}

/// RFC 7009 token revocation: `token` (a refresh or an access token of this server) ends its connection, so every
/// token of that grant dies with it (the phone's list of connections drops it too). `client_id`, when given, must be
/// the connection's client. Always 200, also for a token that is unknown, expired or already revoked (RFC 7009 2.2),
/// so the answer tells nobody whether a guessed token existed.
///
/// Client authentication: a refresh token of a `private_key_jwt` client is revoked only with a valid client
/// assertion of that client; any assertion that is sent is verified (a public client sending one is refused), and a
/// failure is 401 `invalid_client` with nothing revoked. An access token alone is enough for public clients, as before.
#[post("/reins/oauth/revoke", data = "<data>")]
async fn revoke(data: Data<'_>, ip: ClientIp, conn: DbConn) -> JsonResponse {
    if ratelimit::check_limit_unauthenticated(&ip.ip).is_err() {
        return JsonResponse(Status::TooManyRequests, json!({"error": "slow_down"}));
    }
    let Some(body) = read_limited(data, MAX_FORM_BYTES).await else {
        return oauth_error(&OAuthError::invalid_request("request body too large"));
    };
    let form = parse_form(&body);
    let Some(token) = form.get("token").map(String::as_str).filter(|t| !t.is_empty() && t.len() <= 4096) else {
        return oauth_error(&OAuthError::invalid_request("`token` is required"));
    };
    let assertion = match parse_client_assertion(&form) {
        Ok(a) => a,
        Err(e) => return oauth_error(&e),
    };
    let (connection, bound) = match ReinsRefreshToken::find(&hash_token(token), &conn).await {
        Some(row) => (
            ReinsConnection::find_by_uuid(&row.connection_uuid, &conn).await,
            AuthMethod::from_stored(&row.client_auth),
        ),
        None => (super::mcp_routes::authenticate(Some(&format!("Bearer {token}")), &conn).await.ok(), AuthMethod::None),
    };
    if let Some(connection) = connection {
        let client_ok = form.get("client_id").is_none_or(|c| *c == connection.client_id);
        if client_ok {
            // A sent assertion is always checked: as `private_key_jwt` for the connection's client.
            let required = if assertion.is_some() {
                AuthMethod::PrivateKeyJwt
            } else {
                bound
            };
            if let Err(e) = authenticate_client(&connection.client_id, required, assertion.as_deref(), &conn).await {
                return oauth_error(&e);
            }
            match connection.delete(&conn).await {
                Ok(()) => info!("Reins connection {} revoked by its client", connection.uuid),
                Err(e) => {
                    warn!("Could not revoke Reins connection: {e:?}");
                    return JsonResponse(Status::ServiceUnavailable, json!({"error": "temporarily_unavailable"}));
                }
            }
        }
    }
    JsonResponse(Status::Ok, json!({}))
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
        assert!(loc.ends_with("/reins/oauth/authorize/wait?session=abc"), "{loc}");
    }
}
