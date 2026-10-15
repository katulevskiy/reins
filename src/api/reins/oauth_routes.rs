//! HTTP adapters for the OAuth authorization server (spec §4.5): discovery documents,
//! dynamic registration, the zero-JavaScript authorize flow with phone pairing, and the
//! token endpoint. All decisions live in `oauth.rs`, `pairing.rs` and `oauth_state.rs`.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
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
        parse_registration, parse_token_request, precheck_client_assertion, redirect_host, registration_response,
        success_redirect, token_response, validate_authorize, verify_client_assertion,
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
        reins::{hash_token, issue_access_token, random_token},
    },
    db::{
        DbConn, DbPool,
        models::{ReinsClient, ReinsConnection, ReinsDevice, ReinsJwtClient, ReinsRefreshToken, User, UserId},
    },
    ratelimit,
};

const MAX_FORM_BYTES: u64 = 16 * 1024;
/// The whole fetch of a client's metadata document or key set: connecting, the answer and its body.
const CIMD_TIMEOUT: Duration = Duration::from_secs(5);
/// Client documents fetched at once, all clients together; a request that finds them all busy waits this long.
const MAX_CONCURRENT_FETCHES: usize = 16;
const FETCH_SLOT_WAIT: Duration = Duration::from_secs(2);
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

/// Why a client document could not be had.
enum FetchError {
    /// The document is unusable (unreachable, an error status, too large, invalid): remembered for a short while.
    Failed(String),
    /// Too many fetches are running already; nothing is remembered, the next request tries again.
    Busy,
}

impl FetchError {
    fn describe(self) -> String {
        match self {
            Self::Failed(why) => why,
            Self::Busy => "the server is busy fetching other client documents; try again in a moment".to_owned(),
        }
    }
}

/// Fetches running at once across all clients, and how long a request waits for a turn.
static FETCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_CONCURRENT_FETCHES);
/// One fetch per URL at a time: concurrent requests for the same document wait for that fetch and use its result.
static FETCHES_IN_FLIGHT: LazyLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);

/// The turn to fetch one URL; dropping it lets the next request for that URL go (and forgets the URL when nobody
/// else is waiting).
struct FetchTurn {
    url: String,
    _turn: tokio::sync::OwnedMutexGuard<()>,
}

impl Drop for FetchTurn {
    fn drop(&mut self) {
        let mut in_flight = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Two references: the map's and the one inside our own guard. More means somebody is waiting for it.
        if in_flight.get(&self.url).is_some_and(|lock| Arc::strong_count(lock) <= 2) {
            in_flight.remove(&self.url);
        }
    }
}

async fn fetch_turn(url: &str) -> FetchTurn {
    let lock = {
        let mut in_flight = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(in_flight.entry(url.to_owned()).or_default())
    };
    FetchTurn {
        url: url.to_owned(),
        _turn: lock.lock_owned().await,
    }
}

/// Fetches a small JSON document a client names (its Client ID Metadata Document, Decision 25, or its `jwks_uri`):
/// through the outbound guard (https, public addresses only, checked again on every resolved address), no redirects,
/// at most `limit` bytes, [`CIMD_TIMEOUT`] in all, and at most [`MAX_CONCURRENT_FETCHES`] at once. Anyone may name
/// any URL here, so it is the same guard as for the phone's requests, independent of
/// `HTTP_REQUEST_BLOCK_NON_GLOBAL_IPS`. Callers hold no database connection while this runs.
async fn fetch_document(url: &str, limit: usize, what: &str) -> Result<Vec<u8>, FetchError> {
    let Ok(Ok(_slot)) = tokio::time::timeout(FETCH_SLOT_WAIT, FETCH_SLOTS.acquire()).await else {
        return Err(FetchError::Busy);
    };
    let failed = |why: String| FetchError::Failed(why);
    let mut response = outbound::get_once(url, "application/json", CIMD_TIMEOUT)
        .await
        .map_err(|e| failed(format!("could not fetch the {what}: {e}")))?;
    if response.status().is_redirection() {
        return Err(failed(format!("the {what} URL redirected")));
    }
    if !response.status().is_success() {
        return Err(failed(format!("the {what} answered {}", response.status())));
    }
    let mut body = Vec::new();
    let read = async {
        while let Some(chunk) = response.chunk().await.map_err(|e| failed(format!("could not read the {what}: {e}")))? {
            if body.len() + chunk.len() > limit {
                return Err(failed(format!("the {what} is too large")));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(())
    };
    // get_once's timeout covers the whole response, the body too; this bound is only belt and braces.
    tokio::time::timeout(CIMD_TIMEOUT, read).await.map_err(|_| failed(format!("the {what} took too long")))??;
    Ok(body)
}

/// A client's metadata document: from the cache, from a failure remembered within the last seconds, or fetched
/// (once per URL at a time). A document that declares `private_key_jwt` marks its client for good.
async fn client_metadata(client_id: &str, pool: &DbPool) -> Result<ClientInfo, String> {
    let known = || match (OAUTH.cached_client(client_id), OAUTH.failed_fetch(client_id)) {
        (Some(client), _) => Some(Ok(client)),
        (None, Some(why)) => Some(Err(why)),
        (None, None) => None,
    };
    if let Some(known) = known() {
        return known;
    }
    let _turn = fetch_turn(client_id).await;
    if let Some(known) = known() {
        return known;
    }
    let fetched = match fetch_document(client_id, oauth::MAX_CLIENT_METADATA_BYTES, "client metadata document").await {
        Ok(body) => parse_client_metadata(client_id, &body),
        Err(FetchError::Busy) => return Err(FetchError::Busy.describe()),
        Err(FetchError::Failed(why)) => Err(why),
    };
    match fetched {
        Ok(client) => {
            if client.auth.method() == AuthMethod::PrivateKeyJwt {
                mark_seen_with_keys(client_id, pool).await;
            }
            OAUTH.cache_client(client.clone());
            Ok(client)
        }
        Err(why) => {
            OAUTH.remember_failed_fetch(client_id, &why);
            Err(why)
        }
    }
}

/// A client's key set from its `jwks_uri`: cached for a few minutes, fetched once per URL at a time, failures
/// remembered briefly. `force` skips the cache (an unknown `kid`; the caller rate-limits that per URI).
async fn client_jwks(jwks_uri: &str, force: bool) -> Result<jsonwebtoken::jwk::JwkSet, OAuthError> {
    let known = |force: bool| match (OAUTH.cached_jwks(jwks_uri).filter(|_| !force), OAUTH.failed_fetch(jwks_uri)) {
        (Some(set), _) => Some(Ok(set)),
        (None, Some(why)) => Some(Err(OAuthError::invalid_client(why))),
        (None, None) => None,
    };
    if let Some(known) = known(force) {
        return known;
    }
    let _turn = fetch_turn(jwks_uri).await;
    // Whoever waited for the same fetch takes its result rather than fetching again.
    if !force && let Some(known) = known(false) {
        return known;
    }
    let fetched = match fetch_document(jwks_uri, oauth::MAX_JWKS_BYTES, "client's JWKS").await {
        Ok(body) => parse_jwks(&body),
        Err(FetchError::Busy) => {
            return Err(OAuthError::new(503, "temporarily_unavailable", FetchError::Busy.describe()));
        }
        Err(FetchError::Failed(why)) => Err(why),
    };
    match fetched {
        Ok(set) => {
            OAUTH.cache_jwks(jwks_uri, set.clone());
            Ok(set)
        }
        Err(why) => {
            OAUTH.remember_failed_fetch(jwks_uri, &why);
            Err(OAuthError::invalid_client(why))
        }
    }
}

/// A `client_id` that names a metadata document (in the test mode also `http://` on loopback, for mock servers).
fn is_metadata_client_id(client_id: &str) -> bool {
    is_cimd_client_id(client_id) || (outbound::allow_loopback() && is_loopback_cimd_client_id(client_id))
}

/// A short-lived database connection: handlers that may fetch from the network take the pool, not a connection, and
/// hold a connection only around their queries, so slow remote hosts can never tie up the database.
async fn db(pool: &DbPool) -> Result<DbConn, OAuthError> {
    pool.get()
        .await
        .map_err(|_| OAuthError::new(503, "temporarily_unavailable", "the server is busy, try again in a moment"))
}

async fn resolve_client(client_id: &str, pool: &DbPool) -> Result<ClientInfo, String> {
    if is_metadata_client_id(client_id) {
        return client_metadata(client_id, pool).await;
    }
    let conn = db(pool).await.map_err(|e| e.description)?;
    let client = ReinsClient::find(client_id, &conn).await.ok_or_else(|| "unknown client_id".to_owned())?;
    Ok(ClientInfo {
        redirect_uris: client.redirect_uri_list(),
        client_id: client.client_id,
        client_name: client.client_name,
        // Dynamic registration accepts public clients only.
        auth: ClientAuth::None,
    })
}

/// Whether `client_id` has ever been seen with `private_key_jwt` (its document, or a stored grant).
async fn seen_with_keys(client_id: &str, pool: &DbPool) -> Result<bool, OAuthError> {
    if !is_metadata_client_id(client_id) {
        return Ok(false); // registered clients are public-only
    }
    let conn = db(pool).await?;
    ReinsJwtClient::seen(client_id, &conn)
        .await
        .map_err(|_| OAuthError::new(503, "temporarily_unavailable", "the server is busy, try again in a moment"))
}

async fn mark_seen_with_keys(client_id: &str, pool: &DbPool) {
    let marked = match db(pool).await {
        Ok(conn) => ReinsJwtClient::mark(client_id, now_unix(), &conn).await.map_err(|e| format!("{e:?}")),
        Err(e) => Err(e.description),
    };
    if let Err(e) = marked {
        // The grant still records the method, and the next fetch of the document marks the client again.
        warn!("Could not record that an OAuth client uses private_key_jwt: {e}");
    }
}

/// The method `client_id` must authenticate with now ([`oauth::required_method`]) and its current metadata
/// document, if it has one. A document that cannot be fetched counts as unknown: it can then only raise the method
/// (through what was seen before), never lower it, and a `private_key_jwt` client without its document is refused.
async fn method_for(
    client_id: &str,
    bound: AuthMethod,
    pool: &DbPool,
) -> Result<(AuthMethod, Option<ClientInfo>), OAuthError> {
    let seen = seen_with_keys(client_id, pool).await?;
    let current = if is_metadata_client_id(client_id) {
        match client_metadata(client_id, pool).await {
            Ok(client) => Some(client),
            Err(why) if bound == AuthMethod::None && !seen => {
                warn!("Could not check the metadata document of a public OAuth client: {why}");
                None
            }
            Err(why) => return Err(OAuthError::invalid_client(why)),
        }
    } else {
        None
    };
    let method = oauth::required_method(bound, seen, current.as_ref().map(|c| c.auth.method()));
    Ok((method, current))
}

/// RFC 7591 dynamic registration (public clients only).
#[post("/reins/oauth/register", data = "<data>")]
async fn register(data: Data<'_>, ip: ClientIp, pool: &State<DbPool>) -> JsonResponse {
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
    // The database connection is taken only now, after the rate limits.
    let conn = match db(pool).await {
        Ok(conn) => conn,
        Err(e) => return oauth_error(&e),
    };
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
async fn authorize_get(params: HashMap<String, String>, ip: ClientIp, pool: &State<DbPool>) -> Flow {
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
    let cannot_connect =
        |message: &str| error_flow(pages::at::AUTHORIZE, Status::BadRequest, "Cannot connect", message);
    let client = match resolve_client(client_id, pool).await {
        Ok(c) => c,
        Err(e) => return cannot_connect(&format!("The client could not be verified: {e}")),
    };
    // A client once seen with private_key_jwt never starts a flow without it again.
    let client_auth = match seen_with_keys(&client.client_id, pool).await {
        Ok(seen) => oauth::required_method(AuthMethod::None, seen, Some(client.auth.method())),
        Err(e) => return error_flow(pages::at::AUTHORIZE, Status::ServiceUnavailable, "Server busy", &e.description),
    };
    if client_auth != client.auth.method() {
        return cannot_connect(
            "The client could not be verified: it used private_key_jwt before, and its metadata document no longer \
             declares it.",
        );
    }
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
    let id = OAUTH.create_session(session);
    Flow::Page(Status::Ok, pages::email_form_page(&form.0, &form.1, &id, None, form.2.as_deref()))
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
            let token = OAUTH.issue_code(code);
            Flow::Redirect(success_redirect(
                &session.request.redirect_uri,
                &token,
                session.request.state.as_deref(),
                &iss,
            ))
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
async fn device_authorization(data: Data<'_>, ip: ClientIp, pool: &State<DbPool>) -> JsonResponse {
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
    let client = match resolve_client(&request.client_id, pool).await {
        Ok(c) => c,
        Err(e) => return oauth_error(&OAuthError::invalid_client(e)),
    };
    // The device flow is for public clients: a client with keys (now or ever) signs in through the authorize flow.
    let method = match seen_with_keys(&client.client_id, pool).await {
        Ok(seen) => oauth::required_method(AuthMethod::None, seen, Some(client.auth.method())),
        Err(e) => return oauth_error(&e),
    };
    if method != AuthMethod::None {
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
async fn poll_device_code(device_code: &str, client_id: &str, pool: &DbPool) -> Result<Value, OAuthError> {
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
            // A client that has taken up private_key_jwt since it asked for the code gets no tokens this way.
            let (method, _) = method_for(client_id, AuthMethod::None, pool).await?;
            if method != AuthMethod::None {
                return Err(OAuthError::invalid_client("this client authenticates with private_key_jwt"));
            }
            let user = UserId::from(user);
            let conn = db(pool).await?;
            let connection = ReinsConnection::find_by_uuid_and_user(&connection.0, &user, &conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, AuthMethod::None, &conn).await
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
            let was_cached = OAUTH.cached_jwks(uri).is_some();
            match verify(&client_jwks(uri, false).await?) {
                Err(AssertionError::UnknownKey) if was_cached && OAUTH.may_refetch_jwks(uri) => {
                    verify(&client_jwks(uri, true).await?).map_err(AssertionError::into_oauth)
                }
                result => result.map_err(AssertionError::into_oauth),
            }
        }
    }
}

/// Client authentication at the token and revocation endpoints, step 1. The grant (authorization code or refresh
/// token) has been looked up and belongs to `client_id`; `bound` is the method it was issued for. The client must
/// use [`method_for`]: the strongest of that, of any method it was ever seen with, and of its current document, so
/// it can never drop back to `none`. A public client must not send an assertion; a `private_key_jwt` client must send
/// one that verifies against its current keys. Cheapest first: the assertion's claims (no network, no signature),
/// then the method (database, perhaps the document), then the signature (perhaps the key set).
///
/// Returns the method used, which is what the next grant is bound to. The `jti` is not recorded here:
/// [`record_assertion`] does that right before the grant is used, so assertions that come without a valid grant of
/// their own client leave no trace.
async fn verify_client(
    client_id: &str,
    bound: AuthMethod,
    assertion: Option<&str>,
    pool: &DbPool,
) -> Result<(AuthMethod, Option<VerifiedAssertion>), OAuthError> {
    if let Some(assertion) = assertion {
        let audiences = assertion_audiences(&CONFIG.domain());
        precheck_client_assertion(assertion, client_id, &audiences, now_unix()).map_err(AssertionError::into_oauth)?;
    }
    let (method, current) = method_for(client_id, bound, pool).await?;
    let assertion = match (method, assertion) {
        (AuthMethod::None, None) => return Ok((method, None)),
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
    let Some(ClientInfo {
        auth: ClientAuth::PrivateKeyJwt {
            jwks,
            alg,
        },
        ..
    }) = &current
    else {
        return Err(OAuthError::invalid_client(
            "this client must use private_key_jwt, and its metadata document does not declare it (any more)",
        ));
    };
    let verified = verify_with_client_keys(client_id, jwks, *alg, assertion).await?;
    Ok((method, Some(verified)))
}

/// Step 2, right before the grant is used up: burns the verified assertion's `jti`; a replay fails. The grant itself
/// is single-use (codes, and refresh tokens, which rotate), which is what stops an assertion from working twice;
/// the `jti` memory also keeps one assertion from serving two grants.
fn record_assertion(client_id: &str, verified: Option<&VerifiedAssertion>) -> Result<(), OAuthError> {
    match verified {
        Some(verified) if !OAUTH.use_assertion(client_id, &verified.jti) => {
            Err(OAuthError::invalid_client("the client assertion was already used"))
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------------------

/// Issues a rotated access/refresh pair for `connection`; the refresh token remembers how the client authenticated
/// (and a `private_key_jwt` grant marks its client for good).
async fn issue_tokens(
    connection: &ReinsConnection,
    client_auth: AuthMethod,
    conn: &DbConn,
) -> Result<Value, OAuthError> {
    if client_auth == AuthMethod::PrivateKeyJwt
        && let Err(e) = ReinsJwtClient::mark(&connection.client_id, now_unix(), conn).await
    {
        // The refresh token below records the method too, which counts as having seen it.
        warn!("Could not record that an OAuth client uses private_key_jwt: {e:?}");
    }
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

/// Order matters for every grant: find the grant without using it up (unknown grants end here, before anything is
/// fetched or recorded), check that it belongs to the requesting client, verify the client, and only then burn the
/// assertion's `jti` and use up the grant. Database connections are held only around queries, never while a client
/// document or key set is fetched.
async fn exchange(request: TokenRequest, pool: &DbPool) -> Result<Value, OAuthError> {
    let assertion = request.client_assertion.as_deref();
    match request.grant {
        TokenGrant::AuthorizationCode {
            code,
            redirect_uri,
            client_id,
            code_verifier,
        } => {
            let unknown = || OAuthError::invalid_grant("unknown, used or expired code");
            let issued = OAUTH.peek_code(&code).ok_or_else(unknown)?;
            if let Err(e) = check_code_redemption(&issued, &client_id, &redirect_uri, &code_verifier) {
                // A wrong client, redirect URI or verifier still burns the code: it cannot be retried.
                OAUTH.redeem_code(&code);
                return Err(e);
            }
            // The client the code was issued to, authenticating at least the way it did when it started the flow. A
            // failure here leaves the code alone: whoever lacks the client's key can neither use it nor take it away.
            let (method, verified) = verify_client(&issued.client_id, issued.client_auth, assertion, pool).await?;
            record_assertion(&issued.client_id, verified.as_ref())?;
            let issued = OAUTH.redeem_code(&code).ok_or_else(unknown)?;
            let user = UserId::from(issued.user_uuid.clone());
            let conn = db(pool).await?;
            let connection = ReinsConnection::find_by_uuid_and_user(&issued.connection_uuid, &user, &conn)
                .await
                .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
            issue_tokens(&connection, method, &conn).await
        }
        TokenGrant::RefreshToken {
            refresh_token,
            client_id,
        } => {
            let invalid = || OAuthError::invalid_grant("the refresh token is invalid or expired");
            let token_hash = hash_token(&refresh_token);
            let (row, connection) = {
                let conn = db(pool).await?;
                let row = ReinsRefreshToken::find(&token_hash, &conn)
                    .await
                    .filter(|row| now_unix() < row.expires_at)
                    .ok_or_else(invalid)?;
                let connection = ReinsConnection::find_by_uuid(&row.connection_uuid, &conn)
                    .await
                    .ok_or_else(|| OAuthError::invalid_grant("the connection no longer exists"))?;
                (row, connection)
            };
            if client_id.is_some_and(|c| c != connection.client_id) {
                return Err(OAuthError::invalid_grant("the refresh token belongs to another client"));
            }
            // Authenticated before the token is used up, so a request without the client's key cannot burn it.
            let bound = AuthMethod::from_stored(&row.client_auth);
            let (method, verified) = verify_client(&connection.client_id, bound, assertion, pool).await?;
            record_assertion(&connection.client_id, verified.as_ref())?;
            let conn = db(pool).await?;
            ReinsRefreshToken::take(&token_hash, now_unix(), &conn).await.ok_or_else(invalid)?;
            issue_tokens(&connection, method, &conn).await
        }
        TokenGrant::DeviceCode {
            device_code,
            client_id,
        } => {
            // Only public clients get device codes (`device_authorization`): an assertion is refused unchecked.
            if assertion.is_some() {
                return Err(OAuthError::invalid_client("this client is public and must not send a client assertion"));
            }
            poll_device_code(&device_code, &client_id, pool).await
        }
    }
}

/// RFC 6749 token endpoint (form-encoded, PKCE; public clients and RFC 7523 `private_key_jwt` clients). The per-address
/// limit comes first, before any database connection or fetch.
#[post("/reins/oauth/token", data = "<data>")]
async fn token(data: Data<'_>, ip: ClientIp, pool: &State<DbPool>) -> JsonResponse {
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
    match exchange(request, pool).await {
        Ok(tokens) => JsonResponse(Status::Ok, tokens),
        Err(e) => oauth_error(&e),
    }
}

/// The connection a token to revoke stands for, and the method its grant was bound to (`none` for an access token).
async fn revocation_target(token: &str, pool: &DbPool) -> Result<Option<(ReinsConnection, AuthMethod)>, OAuthError> {
    let conn = db(pool).await?;
    Ok(match ReinsRefreshToken::find(&hash_token(token), &conn).await {
        Some(row) => ReinsConnection::find_by_uuid(&row.connection_uuid, &conn)
            .await
            .map(|connection| (connection, AuthMethod::from_stored(&row.client_auth))),
        None => super::mcp_routes::authenticate(Some(&format!("Bearer {token}")), &conn)
            .await
            .ok()
            .map(|connection| (connection, AuthMethod::None)),
    })
}

/// RFC 7009 token revocation: `token` (a refresh or an access token of this server) ends its connection, so every
/// token of that grant dies with it (the phone's list of connections drops it too). `client_id`, when given, must be
/// the connection's client. Always 200, also for a token that is unknown, expired or already revoked (RFC 7009 2.2),
/// so the answer tells nobody whether a guessed token existed.
///
/// Client authentication follows the same rule as the token endpoint: a client that uses `private_key_jwt` (now, in
/// the token's grant, or ever before) revokes only with a valid assertion, whichever of its tokens it names; a public
/// client needs nothing but the token, and must not send an assertion. Any assertion that is sent is verified. A
/// failure is 401 `invalid_client` with nothing revoked. An unknown token, or one of another client, is answered 200
/// before any assertion is looked at, so it records and fetches nothing.
#[post("/reins/oauth/revoke", data = "<data>")]
async fn revoke(data: Data<'_>, ip: ClientIp, pool: &State<DbPool>) -> JsonResponse {
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
    let (connection, bound) = match revocation_target(token, pool).await {
        Ok(Some(target)) => target,
        Ok(None) => return JsonResponse(Status::Ok, json!({})),
        Err(e) => return oauth_error(&e),
    };
    if form.get("client_id").is_some_and(|c| *c != connection.client_id) {
        return JsonResponse(Status::Ok, json!({}));
    }
    // A sent assertion is always checked, as `private_key_jwt` for the connection's client.
    let bound = if assertion.is_some() {
        bound.max(AuthMethod::PrivateKeyJwt)
    } else {
        bound
    };
    let authenticated = match verify_client(&connection.client_id, bound, assertion.as_deref(), pool).await {
        // The token exists and is this client's: only now is the assertion's jti burnt.
        Ok((_, verified)) => record_assertion(&connection.client_id, verified.as_ref()),
        Err(e) => Err(e),
    };
    if let Err(e) = authenticated {
        return oauth_error(&e);
    }
    let deleted = match db(pool).await {
        Ok(conn) => connection.delete(&conn).await.map_err(|e| format!("{e:?}")),
        Err(e) => Err(e.description),
    };
    match deleted {
        Ok(()) => info!("Reins connection {} revoked by its client", connection.uuid),
        Err(e) => {
            warn!("Could not revoke Reins connection: {e}");
            return JsonResponse(Status::ServiceUnavailable, json!({"error": "temporarily_unavailable"}));
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
