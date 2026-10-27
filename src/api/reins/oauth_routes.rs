//! HTTP adapters for the OAuth authorization server (spec §4.5): discovery documents,
//! dynamic registration, the zero-JavaScript authorize flow with phone pairing, and the
//! token endpoint. All decisions live in `oauth.rs`, `pairing.rs` and `oauth_state.rs`.

use std::{
    collections::HashMap,
    io::Cursor,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};

use reins_proto::{
    desktop,
    ids::PairingId,
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
    device_flow::{DEVICE_GRANT_TTL, DEVICE_GRANTS, POLL_INTERVAL, Poll, StartError as GrantStart},
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
    oauth_state::{AuthSession, FetchKind, OAUTH, StartedPairing},
    outbound, pages,
    pairing::{
        PairingClient, PairingStatus, StartError as PairingStart, email_cap_key, generate_choices, sanitize_client_name,
    },
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
    util::get_uuid,
};

const MAX_FORM_BYTES: u64 = 16 * 1024;
/// The whole fetch of a client's metadata document or key set: connecting, the answer and its body.
const CIMD_TIMEOUT: Duration = Duration::from_secs(4);
/// Client documents fetched at once from one site (registrable domain, roughly; or one address): a site that is
/// slow, or many requests naming it, can only ever hold this many fetches.
const FETCHES_PER_SITE: usize = 2;
/// Client documents fetched at once over all sites: the global bound.
const MAX_CONCURRENT_FETCHES: usize = 32;
/// How long a request queues for its turns (site and global together) before it is refused, alone; nothing is
/// remembered, it can try again. Queued requests hold no database connection.
const FETCH_QUEUE_WAIT: Duration = Duration::from_secs(2);
/// Fetches at once that refresh documents the server already holds (known clients), in a pool of their own: other
/// fetches, however many, never take these turns. Fair (first come, first served), so a refresh always gets a turn.
const MAX_CONCURRENT_REFRESHES: usize = 4;
/// How long a background refresh waits for a refresh turn (nobody waits on it but the refresh itself).
const REFRESH_QUEUE_WAIT: Duration = Duration::from_secs(10);
/// Of the reserved refresh turns, how many one site may hold at once.
const REFRESHES_PER_SITE: usize = 1;
/// Keys a turn map holds at once; beyond it, idle ones are dropped first (a map never refuses).
const MAX_FETCH_SITES: usize = 4096;
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
    /// The URL's own answer makes it unusable, whenever it is asked: refused address, a redirect, a client error
    /// status, too large, not a valid document. Remembered for 30 s for this kind and exact URL.
    Definitive(String),
    /// Timing or load: the host did not answer in time, the connection broke, a server error or "slow down", no
    /// turn, or the request's address is over its fetch budget. Not remembered for a request's own fetch.
    Transient(String),
}

impl FetchError {
    fn why(&self) -> &str {
        match self {
            Self::Definitive(why) | Self::Transient(why) => why,
        }
    }
}

/// The global bound on fetches at once.
static FETCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_CONCURRENT_FETCHES);
/// The reserved pool for refreshes of known clients (see [`MAX_CONCURRENT_REFRESHES`]).
static REFRESH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_CONCURRENT_REFRESHES);
/// One fetch per URL at a time: concurrent requests for the same document wait for that fetch and use its result.
static FETCHES_IN_FLIGHT: LazyLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);
type TurnMap = LazyLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>;

/// Request fetch turns per site, so one site's slowness, or an attacker's many names under one domain, holds only its
/// own ([`FETCHES_PER_SITE`]).
static SITE_TURNS: TurnMap = LazyLock::new(Default::default);
/// Refresh turns per site, inside the reserved refresh pool ([`REFRESHES_PER_SITE`]).
static REFRESH_SITE_TURNS: TurnMap = LazyLock::new(Default::default);

/// The ICANN public suffix of `host` by the Public Suffix List (compiled into the `psl` crate). Private-section
/// suffixes (`github.io`, `s3.amazonaws.com`) are looked through: they are run by one company, so their tenants are
/// counted as that company's site, and many free subdomains of one platform never multiply anyone's turns.
fn icann_suffix(host: &str) -> &str {
    let mut candidate = host;
    loop {
        let Some(found) = psl::suffix(candidate.as_bytes()) else {
            return candidate.rsplit('.').next().unwrap_or(candidate);
        };
        let found_len = found.trim().as_bytes().len();
        let suffix = &candidate[candidate.len().saturating_sub(found_len)..];
        match (found.typ(), suffix.split_once('.')) {
            (Some(psl::Type::Private), Some((_, parent))) => candidate = parent,
            _ => return suffix,
        }
    }
}

/// The network an address is grouped by: IPv4 by /24, IPv6 by /48.
fn address_network(address: IpAddr) -> String {
    match address.to_canonical() {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            format!("{a}.{b}.{c}.0/24")
        }
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[6..].fill(0);
            format!("{}/48", std::net::Ipv6Addr::from(octets))
        }
    }
}

/// The site a URL's fetches count against, [`FETCHES_PER_SITE`] at once: its registrable domain under the ICANN
/// section of the Public Suffix List, or the full host when it has none (a bare suffix). There is no cap per public
/// suffix: sites under a busy suffix such as `com` never share turns, so an attacker's `.com` sites cannot starve
/// anyone else's; the per-address budget and the global bound limit how many sites one party can keep busy.
fn fetch_site(url: &str) -> String {
    let Some(host) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)) else {
        return String::new();
    };
    let host = host.trim_end_matches('.');
    let address = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>();
    if let Ok(address) = address {
        // Addresses only reach a fetch in the test mode (documents must be named by a domain); grouped by network
        // anyway, so rotating through one network's addresses never counts as many sites.
        return address_network(address);
    }
    let suffix = icann_suffix(host);
    match host.strip_suffix(suffix).and_then(|rest| rest.strip_suffix('.')) {
        Some(rest) if !rest.is_empty() => {
            let label = rest.rsplit('.').next().unwrap_or(rest);
            format!("{label}.{suffix}")
        }
        _ => host.to_owned(),
    }
}

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

fn url_lock(url: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut in_flight = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(in_flight.entry(url.to_owned()).or_default())
}

/// The turn to fetch `url`, waiting at most for one fetch of it plus the queue: `None` if it does not come.
async fn fetch_turn(url: &str) -> Option<FetchTurn> {
    let turn = tokio::time::timeout(CIMD_TIMEOUT + FETCH_QUEUE_WAIT, url_lock(url).lock_owned()).await.ok()?;
    Some(FetchTurn {
        url: url.to_owned(),
        _turn: turn,
    })
}

/// The turn to fetch `url` if nobody is fetching it right now (for background refreshes).
fn try_fetch_turn(url: &str) -> Option<FetchTurn> {
    let turn = url_lock(url).try_lock_owned().ok()?;
    Some(FetchTurn {
        url: url.to_owned(),
        _turn: turn,
    })
}

/// One turn in a per-key turn map (per site, per suffix). It holds its own reference to the key's turns from the start,
/// so whatever way it ends (done, timed out, or the request gone mid-wait), dropping it drops that reference and then
/// forgets the key if nobody else holds or waits for a turn there.
struct Turn {
    map: &'static TurnMap,
    key: String,
    turns: Option<Arc<tokio::sync::Semaphore>>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Turn {
    fn new(map: &'static TurnMap, key: String, per_key: usize) -> Self {
        let turns = {
            let mut keys = map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if keys.len() >= MAX_FETCH_SITES && !keys.contains_key(&key) {
                forget_idle_keys(&mut keys);
            }
            Arc::clone(keys.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(per_key))))
        };
        Self {
            map,
            key,
            turns: Some(turns),
            permit: None,
        }
    }

    /// Waits until `deadline` for a turn; `false` if none came.
    async fn acquire(&mut self, deadline: tokio::time::Instant) -> bool {
        let Some(turns) = &self.turns else {
            return false;
        };
        self.permit =
            tokio::time::timeout_at(deadline, Arc::clone(turns).acquire_owned()).await.ok().and_then(Result::ok);
        self.permit.is_some()
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        drop(self.permit.take());
        drop(self.turns.take());
        let mut keys = self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if keys.get(&self.key).is_some_and(|turns| Arc::strong_count(turns) == 1) {
            keys.remove(&self.key);
        }
    }
}

/// Drops keys nobody holds or waits for a turn at (only the map refers to them).
fn forget_idle_keys(keys: &mut HashMap<String, Arc<tokio::sync::Semaphore>>) {
    keys.retain(|_, turns| Arc::strong_count(turns) > 1);
}

/// Housekeeping: forgets idle sites and suffixes, whatever left them behind.
pub fn purge_fetch_sites() {
    for map in [&SITE_TURNS, &REFRESH_SITE_TURNS] {
        forget_idle_keys(&mut map.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    }
}

/// Which turns a fetch takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchPool {
    /// A request's own fetch: its site's turns and the global ones, queueing at most [`FETCH_QUEUE_WAIT`].
    Request,
    /// A background refresh of a document the server holds: the reserved refresh turns only.
    Refresh,
}

/// Fetches a small JSON document a client names (its Client ID Metadata Document, Decision 25, or its `jwks_uri`):
/// through the outbound guard (https, public addresses only, checked again on every resolved address), no redirects,
/// at most `limit` bytes and [`CIMD_TIMEOUT`] in all. It queues at most [`FETCH_QUEUE_WAIT`] for a turn of its site
/// ([`FETCHES_PER_SITE`]) and of all sites ([`MAX_CONCURRENT_FETCHES`]). Anyone may name any URL here, so it is the
/// same guard as for the phone's requests, independent of `HTTP_REQUEST_BLOCK_NON_GLOBAL_IPS`. Callers hold no
/// database connection while this runs.
async fn fetch_document(url: &str, limit: usize, what: &str, pool: FetchPool) -> Result<Vec<u8>, FetchError> {
    let busy =
        || FetchError::Transient("the server is busy fetching client documents; try again in a moment".to_owned());
    let site = fetch_site(url);
    let (_turn, _slot) = match pool {
        FetchPool::Request => {
            let deadline = tokio::time::Instant::now() + FETCH_QUEUE_WAIT;
            let mut site_turn = Turn::new(&SITE_TURNS, site, FETCHES_PER_SITE);
            if !site_turn.acquire(deadline).await {
                return Err(busy());
            }
            let Ok(Ok(slot)) = tokio::time::timeout_at(deadline, FETCH_SLOTS.acquire()).await else {
                return Err(busy());
            };
            (site_turn, slot)
        }
        FetchPool::Refresh => {
            let deadline = tokio::time::Instant::now() + REFRESH_QUEUE_WAIT;
            let mut site_turn = Turn::new(&REFRESH_SITE_TURNS, site, REFRESHES_PER_SITE);
            if !site_turn.acquire(deadline).await {
                return Err(busy());
            }
            let Ok(Ok(slot)) = tokio::time::timeout_at(deadline, REFRESH_SLOTS.acquire()).await else {
                return Err(busy());
            };
            (site_turn, slot)
        }
    };
    let definitive = |why: String| FetchError::Definitive(why);
    let transient = |why: String| FetchError::Transient(why);
    let mut response = match outbound::get_once(url, "application/json", CIMD_TIMEOUT).await {
        Ok(response) => response,
        Err(e @ outbound::OutboundError::Refused(_)) => {
            return Err(definitive(format!("could not fetch the {what}: {e}")));
        }
        Err(e @ outbound::OutboundError::Failed(_)) => {
            return Err(transient(format!("could not fetch the {what}: {e}")));
        }
    };
    let status = response.status();
    if status.is_redirection() {
        return Err(definitive(format!("the {what} URL redirected")));
    }
    if status.is_server_error() || status.as_u16() == 429 || status.as_u16() == 408 {
        return Err(transient(format!("the {what} answered {status}")));
    }
    if !status.is_success() {
        return Err(definitive(format!("the {what} answered {status}")));
    }
    let mut body = Vec::new();
    let read = async {
        while let Some(chunk) =
            response.chunk().await.map_err(|e| transient(format!("could not read the {what}: {e}")))?
        {
            if body.len() + chunk.len() > limit {
                return Err(definitive(format!("the {what} is too large")));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(())
    };
    // get_once's timeout covers the whole response, the body too; this bound is only belt and braces.
    tokio::time::timeout(CIMD_TIMEOUT, read).await.map_err(|_| transient(format!("the {what} took too long")))??;
    Ok(body)
}

/// Who pays for a fetch: [`limits::client_network`] of the requesting address.
fn fetch_payer(ip: IpAddr) -> IpAddr {
    limits::client_network(ip)
}

/// Charges one fetch to the requesting address ([`limits::CLIENT_DOCUMENT_FETCHES`], see [`fetch_payer`]).
fn charge_fetch(ip: IpAddr) -> Result<(), FetchError> {
    limits::CLIENT_DOCUMENT_FETCHES.check(&fetch_payer(ip)).map_err(|wait| {
        FetchError::Transient(rate_limited_text("too many client documents fetched for this address", wait))
    })
}

/// Fetches and parses a metadata document now, and caches it (and its client's `private_key_jwt`, for good). A
/// definitive failure is remembered for this exact URL as a metadata document.
async fn fetch_metadata_now(client_id: &str, pool: &DbPool, turns: FetchPool) -> Result<ClientInfo, FetchError> {
    let fetched = fetch_document(client_id, oauth::MAX_CLIENT_METADATA_BYTES, "client metadata document", turns)
        .await
        .and_then(|body| parse_client_metadata(client_id, &body).map_err(FetchError::Definitive));
    let client = match fetched {
        Ok(client) => client,
        Err(error) => {
            if let FetchError::Definitive(why) = &error {
                OAUTH.remember_failed_fetch(FetchKind::Metadata, client_id, why);
            }
            return Err(error);
        }
    };
    if client.auth.method() == AuthMethod::PrivateKeyJwt {
        mark_seen_with_keys(client_id, pool).await.map_err(FetchError::Transient)?;
    }
    OAUTH.cache_client(client.clone());
    Ok(client)
}

/// Whether the requesting address may start one more background refresh ([`limits::CLIENT_DOCUMENT_REFRESHES`]).
/// Over it, nothing is refused: the request is served from what the server holds, without a refresh.
fn may_trigger_refresh(ip: IpAddr) -> bool {
    limits::CLIENT_DOCUMENT_REFRESHES.check(&fetch_payer(ip)).is_ok()
}

/// Starts a background refresh of a known client's metadata document: in the reserved refresh pool (one turn per
/// site), uncharged to the fetch budget, at most one per URL at a time, spaced per URL with back-off after failures,
/// and limited per requesting address. A failed refresh changes nothing else: the last good copy goes on standing in
/// until its grace ends (a hard limit, never extended), and only a definitive failure is remembered, as for any fetch.
fn refresh_metadata_in_background(client_id: &str, pool: &DbPool, ip: IpAddr) {
    // Due first (no mark), then the requester's limit, then the mark: a requester over its limit leaves no mark that
    // would hold back the next requester's refresh.
    if !OAUTH.refresh_due(FetchKind::Metadata, client_id)
        || !may_trigger_refresh(ip)
        || !OAUTH.may_refresh(FetchKind::Metadata, client_id)
    {
        return;
    }
    let Some(turn) = try_fetch_turn(client_id) else {
        return;
    };
    let (client_id, pool) = (client_id.to_owned(), pool.clone());
    tokio::spawn(async move {
        let _turn = turn;
        match fetch_metadata_now(&client_id, &pool, FetchPool::Refresh).await {
            Ok(_) => OAUTH.refresh_succeeded(FetchKind::Metadata, &client_id),
            Err(error) => {
                OAUTH.refresh_failed(FetchKind::Metadata, &client_id);
                warn!("Could not refresh a client metadata document: {}", error.why());
            }
        }
    });
}

/// A client's metadata document. A known client (its document fetched within the last hour) is served from the
/// cache, or within the grace period (10 minutes) past that from its last good copy while a background refresh runs: it never waits for a
/// fetch and is never charged. Otherwise the document is fetched once (per URL at a time), charged to `ip`. Errors
/// refuse the request.
async fn client_metadata(client_id: &str, pool: &DbPool, ip: IpAddr) -> Result<ClientInfo, String> {
    let held = || {
        OAUTH
            .cached_client(client_id)
            .map(|client| (client, true))
            .or_else(|| OAUTH.last_good_client(client_id).map(|client| (client, false)))
    };
    if let Some((client, fresh)) = held() {
        if !fresh {
            refresh_metadata_in_background(client_id, pool, ip);
        }
        return Ok(client);
    }
    if let Some(failed) = OAUTH.failed_fetch(FetchKind::Metadata, client_id) {
        return Err(failed.why);
    }
    let Some(_turn) = fetch_turn(client_id).await else {
        return Err("the server is busy fetching client documents; try again in a moment".to_owned());
    };
    if let Some((client, _)) = held() {
        return Ok(client);
    }
    if let Some(failed) = OAUTH.failed_fetch(FetchKind::Metadata, client_id) {
        return Err(failed.why);
    }
    charge_fetch(ip).map_err(|e| e.why().to_owned())?;
    fetch_metadata_now(client_id, pool, FetchPool::Request).await.map_err(|e| e.why().to_owned())
}

/// Fetches and parses a key set now, and caches it (replacing the last good copy, so a key it no longer lists is
/// gone at once). A definitive failure is remembered for this exact URL as a key set.
async fn fetch_jwks_now(jwks_uri: &str, turns: FetchPool) -> Result<jsonwebtoken::jwk::JwkSet, FetchError> {
    let fetched = fetch_document(jwks_uri, oauth::MAX_JWKS_BYTES, "client's JWKS", turns)
        .await
        .and_then(|body| parse_jwks(&body).map_err(FetchError::Definitive));
    match fetched {
        Ok(set) => {
            OAUTH.cache_jwks(jwks_uri, set.clone());
            Ok(set)
        }
        Err(error) => {
            if let FetchError::Definitive(why) = &error {
                OAUTH.remember_failed_fetch(FetchKind::KeySet, jwks_uri, why);
            }
            Err(error)
        }
    }
}

/// Starts a background refresh of a key set the server holds, like [`refresh_metadata_in_background`].
fn refresh_jwks_in_background(jwks_uri: &str, ip: IpAddr) {
    if !OAUTH.refresh_due(FetchKind::KeySet, jwks_uri)
        || !may_trigger_refresh(ip)
        || !OAUTH.may_refresh(FetchKind::KeySet, jwks_uri)
    {
        return;
    }
    let Some(turn) = try_fetch_turn(jwks_uri) else {
        return;
    };
    let jwks_uri = jwks_uri.to_owned();
    tokio::spawn(async move {
        let _turn = turn;
        match fetch_jwks_now(&jwks_uri, FetchPool::Refresh).await {
            Ok(_) => OAUTH.refresh_succeeded(FetchKind::KeySet, &jwks_uri),
            Err(error) => {
                OAUTH.refresh_failed(FetchKind::KeySet, &jwks_uri);
                warn!("Could not refresh a client's JWKS: {}", error.why());
            }
        }
    });
}

fn jwks_error(error: &FetchError) -> OAuthError {
    match error {
        FetchError::Definitive(why) => OAuthError::invalid_client(why.clone()),
        FetchError::Transient(why) => OAuthError::new(503, "temporarily_unavailable", why.clone()),
    }
}

/// A client's key set from its `jwks_uri`, served like [`client_metadata`]: the cache, or within the grace period (10 minutes) its last
/// good copy while a background refresh runs, or one fetch charged to `ip`. `force` (an unknown `kid`, rate-limited
/// per client and URI by the caller) always fetches, charged to `ip`, and never falls back to an older copy.
async fn client_jwks(jwks_uri: &str, force: bool, ip: IpAddr) -> Result<jsonwebtoken::jwk::JwkSet, OAuthError> {
    let held = || {
        OAUTH
            .cached_jwks(jwks_uri)
            .map(|set| (set, true))
            .or_else(|| OAUTH.last_good_jwks(jwks_uri).map(|set| (set, false)))
    };
    if !force && let Some((set, fresh)) = held() {
        if !fresh {
            refresh_jwks_in_background(jwks_uri, ip);
        }
        return Ok(set);
    }
    if let Some(failed) = OAUTH.failed_fetch(FetchKind::KeySet, jwks_uri) {
        return Err(OAuthError::invalid_client(failed.why));
    }
    let Some(_turn) = fetch_turn(jwks_uri).await else {
        return Err(jwks_error(&FetchError::Transient("the server is busy fetching client documents".to_owned())));
    };
    // Whoever waited for the same fetch takes its result rather than fetching again.
    if !force && let Some((set, _)) = held() {
        return Ok(set);
    }
    charge_fetch(ip).map_err(|e| jwks_error(&e))?;
    fetch_jwks_now(jwks_uri, FetchPool::Request).await.map_err(|e| jwks_error(&e))
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

async fn resolve_client(client_id: &str, pool: &DbPool, ip: IpAddr) -> Result<ClientInfo, String> {
    if oauth::is_ip_literal_https_url(client_id) {
        return Err(
            "a client metadata document at an IP address is not allowed; it must be named by a domain".to_owned()
        );
    }
    if is_metadata_client_id(client_id) {
        return client_metadata(client_id, pool, ip).await;
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

/// Records that `client_id` declared `private_key_jwt`. A failure fails the request (its document is not used), so a
/// client is never handled as one whose keys were not seen just because the record could not be written.
async fn mark_seen_with_keys(client_id: &str, pool: &DbPool) -> Result<(), String> {
    let conn = db(pool).await.map_err(|e| e.description)?;
    ReinsJwtClient::mark(client_id, now_unix(), &conn).await.map_err(|e| {
        warn!("Could not record that an OAuth client uses private_key_jwt: {e:?}");
        "the server could not record the client's authentication method; try again in a moment".to_owned()
    })
}

/// The method `client_id` must authenticate with now ([`oauth::required_method`]) and its current metadata
/// document, if it has one. Fails closed: if what was seen before or the current document cannot be determined (a
/// database error, a document that cannot be fetched and has no last good copy), the request is refused, never
/// treated as a public client's.
async fn method_for(
    client_id: &str,
    bound: AuthMethod,
    pool: &DbPool,
    ip: IpAddr,
) -> Result<(AuthMethod, Option<ClientInfo>), OAuthError> {
    let seen = seen_with_keys(client_id, pool).await?;
    let current = if is_metadata_client_id(client_id) {
        let client = client_metadata(client_id, pool, ip).await.map_err(|why| {
            OAuthError::new(
                503,
                "temporarily_unavailable",
                format!("could not check the client's metadata document, try again later: {why}"),
            )
        })?;
        Some(client)
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

/// The cap refused another pairing. The browser still gets the wait page, and nothing is stored or pushed, so the
/// page is the same for a full account and for an email that has none.
fn display_only_wait(session_id: &str) -> Flow {
    let (code, _) = generate_choices();
    OAUTH.set_pairing(
        session_id,
        StartedPairing {
            id: PairingId(get_uuid()),
            code,
            user_uuid: None,
            display_only: true,
        },
    );
    Flow::Redirect(wait_location(session_id))
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
    let client = match resolve_client(client_id, pool, ip.ip).await {
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
    // The cap is decided before the account lookup, and it is the same key for a real pairing and a decoy. A network
    // that already holds its share is refused here, for every email, still with the ordinary wait page.
    let network = limits::client_network(ip.ip).to_string();
    let cap_key = email_cap_key(&email);
    if HUB.pairings.capped_for(&cap_key, &network) {
        return display_only_wait(session_id);
    }
    let target = pairing_target(&email, &conn).await;
    let user = target.as_ref().map(|(user_uuid, _)| user_uuid.to_string());
    let started = HUB.pairings.start_email(&cap_key, &network, user.as_deref(), session.client.clone(), now);
    let (request, code) = match started {
        Ok(started) => started,
        // Same page as a decoy that was stored: a probe cannot tell which emails exist, or which are full.
        Err(PairingStart::Party) => return display_only_wait(session_id),
        Err(PairingStart::Busy) => {
            return error_flow(
                pages::at::AUTHORIZE,
                Status::ServiceUnavailable,
                "Server busy",
                "Please try again in a few minutes.",
            );
        }
    };
    if let Some((user_uuid, device)) = &target {
        push::spawn_push(
            pool.inner().clone(),
            user_uuid.clone(),
            device.fcm_token.clone(),
            PushMessage {
                t: PushKind::Pair,
                id: request.id.0.clone(),
                p: None,
            },
        );
    }
    OAUTH.set_pairing(
        session_id,
        StartedPairing {
            id: request.id,
            code,
            user_uuid: user,
            display_only: false,
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
    let show_wait = |code: u8| {
        let fingerprint = key_fingerprint_of(&current.client);
        Flow::Page(Status::Ok, pages::wait_page(&current.client.client_name, code, fingerprint.as_deref()))
    };
    match HUB.pairings.status(&pairing.id) {
        Some(PairingStatus::Waiting) => show_wait(pairing.code),
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
            // A refused cap never created a pairing. Keep showing the wait page for the life of the session: an
            // immediate timeout would tell it apart from a pairing that is still open. An expired decoy is not
            // display-only, so it still ends here.
            if pairing.display_only {
                return show_wait(pairing.code);
            }
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
    let client = match resolve_client(&request.client_id, pool, ip.ip).await {
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
    match DEVICE_GRANTS.start(pairing_client, &limits::client_network(ip.ip).to_string()) {
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
        Err(GrantStart::Party) => {
            let description = "too many pairing codes for this address; try again when one expires";
            JsonResponse(Status::TooManyRequests, json!({"error": "slow_down", "error_description": description}))
        }
        Err(GrantStart::Busy) => JsonResponse(
            Status::ServiceUnavailable,
            json!({"error": "temporarily_unavailable", "error_description": "the server is busy, try again soon"}),
        ),
    }
}

/// RFC 8628 §3.5: one poll of the desktop app.
async fn poll_device_code(device_code: &str, client_id: &str, pool: &DbPool, ip: IpAddr) -> Result<Value, OAuthError> {
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
            let (method, _) = method_for(client_id, AuthMethod::None, pool, ip).await?;
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
    ip: IpAddr,
) -> Result<VerifiedAssertion, OAuthError> {
    let audiences = assertion_audiences(&CONFIG.domain());
    let verify = |set: &jsonwebtoken::jwk::JwkSet| {
        verify_client_assertion(assertion, client_id, set, alg, &audiences, now_unix())
    };
    match jwks {
        JwksSource::Inline(set) => verify(set).map_err(AssertionError::into_oauth),
        JwksSource::Uri(uri) => {
            let was_cached = OAUTH.cached_jwks(uri).is_some();
            match verify(&client_jwks(uri, false, ip).await?) {
                Err(AssertionError::UnknownKey) if was_cached && OAUTH.may_refetch_jwks(client_id, uri) => {
                    verify(&client_jwks(uri, true, ip).await?).map_err(AssertionError::into_oauth)
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
    ip: IpAddr,
) -> Result<(AuthMethod, Option<VerifiedAssertion>), OAuthError> {
    if let Some(assertion) = assertion {
        let audiences = assertion_audiences(&CONFIG.domain());
        precheck_client_assertion(assertion, client_id, &audiences, now_unix()).map_err(AssertionError::into_oauth)?;
    }
    let (method, current) = method_for(client_id, bound, pool, ip).await?;
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
    let verified = verify_with_client_keys(client_id, jwks, *alg, assertion, ip).await?;
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
async fn exchange(request: TokenRequest, pool: &DbPool, ip: IpAddr) -> Result<Value, OAuthError> {
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
            let (method, verified) = verify_client(&issued.client_id, issued.client_auth, assertion, pool, ip).await?;
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
            let (method, verified) = verify_client(&connection.client_id, bound, assertion, pool, ip).await?;
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
            poll_device_code(&device_code, &client_id, pool, ip).await
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
    match exchange(request, pool, ip.ip).await {
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
    let authenticated = match verify_client(&connection.client_id, bound, assertion.as_deref(), pool, ip.ip).await {
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
    fn fetches_are_paid_per_address_and_per_ipv6_network() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(fetch_payer(ip("203.0.113.7")), ip("203.0.113.7"));
        assert_eq!(fetch_payer(ip("2001:db8:1:2:aaaa::1")), ip("2001:db8:1:2::"));
        assert_eq!(fetch_payer(ip("2001:db8:1:2:ffff:ffff:ffff:ffff")), ip("2001:db8:1:2::"), "one /64 is one payer");
        assert_ne!(fetch_payer(ip("2001:db8:1:3::1")), fetch_payer(ip("2001:db8:1:2::1")));
        assert_eq!(fetch_payer(ip("::ffff:203.0.113.7")), ip("203.0.113.7"), "IPv4-mapped is the IPv4 address");
    }

    #[test]
    fn fetch_sites_are_icann_registrable_domains() {
        let site = fetch_site;
        assert_eq!(site("https://chatgpt.com/oauth/client.json"), "chatgpt.com");
        assert_eq!(site("https://a.b.c.chatgpt.com/x"), "chatgpt.com", "subdomains are one site");
        assert_eq!(site("https://x.evil.COM./x"), "evil.com");
        assert_eq!(site("https://login.example.co.uk/x"), "example.co.uk");
        assert_eq!(icann_suffix("login.example.co.uk"), "co.uk");
        assert_eq!(site("https://a.b.example.com.au/x"), "example.com.au");
        // The old two-letter heuristic split `abc.de` into one site per subdomain.
        assert_eq!(site("https://one.abc.de/x"), "abc.de");
        assert_eq!(site("https://two.abc.de/x"), "abc.de");
        // Private-section suffixes are looked through: all of a platform's tenants are the platform's one site, so
        // free subdomains never multiply an attacker's turns.
        assert_eq!(site("https://alice.github.io/x"), "github.io");
        assert_eq!(site("https://alice.github.io/x"), site("https://bob.github.io/x"));
        assert_eq!(site("https://bucket.s3.amazonaws.com/x"), "amazonaws.com");
        assert_eq!(icann_suffix("alice.github.io"), "io");
        // No cap per suffix: two .com sites never share turns.
        assert_ne!(site("https://chatgpt.com/x"), site("https://evil.com/x"));
        // Names the list does not know count under their last label; no registrable domain: the full host.
        assert_eq!(site("https://a.b.evil.example/x"), "evil.example");
        assert_eq!(site("https://co.uk/x"), "co.uk");
        // Addresses (test mode only; documents must be named by a domain) are grouped by network.
        assert_eq!(site("https://203.0.113.7/x"), "203.0.113.0/24");
        assert_eq!(site("https://203.0.113.200/x"), site("https://203.0.113.7/x"));
        assert_ne!(site("https://203.0.114.7/x"), site("https://203.0.113.7/x"));
        assert_eq!(site("https://[2001:db8:1:2::1]/x"), "2001:db8:1::/48");
        assert_eq!(site("https://[2001:db8:1:ffff::9]/x"), site("https://[2001:db8:1:2::1]/x"));
        assert_ne!(site("https://chatgpt.com/x"), site("https://chatgpt.com.evil.com/x"));
    }

    fn known(map: &'static TurnMap, key: &str) -> bool {
        map.lock().unwrap().contains_key(key)
    }

    /// A key's entry goes once nobody holds or waits for its turns, whether the turns were used, timed out, or the
    /// request went away mid-wait.
    #[tokio::test]
    async fn turns_are_forgotten_however_they_end() {
        let soon = || tokio::time::Instant::now() + Duration::from_millis(50);
        let site = "turns-end.example";
        let turn = || Turn::new(&SITE_TURNS, site.to_owned(), FETCHES_PER_SITE);
        {
            let mut used = turn();
            assert!(used.acquire(soon()).await);
            assert!(known(&SITE_TURNS, site));
        }
        assert!(!known(&SITE_TURNS, site), "used");

        let (mut a, mut b) = (turn(), turn());
        assert!(a.acquire(soon()).await && b.acquire(soon()).await);
        let mut third = turn();
        assert!(!third.acquire(soon()).await, "two turns per site");
        drop(third);
        assert!(known(&SITE_TURNS, site), "still held");
        // A request that goes away while it waits.
        let waiting = async {
            let mut waiter = turn();
            waiter.acquire(tokio::time::Instant::now() + Duration::from_secs(30)).await
        };
        assert!(tokio::time::timeout(Duration::from_millis(50), waiting).await.is_err());
        drop((a, b));
        assert!(!known(&SITE_TURNS, site), "timed out and abandoned waiters leave nothing behind");
    }

    #[test]
    fn idle_keys_are_purged_and_the_maps_are_capped() {
        {
            let mut sites = SITE_TURNS.lock().unwrap();
            for i in 0..MAX_FETCH_SITES {
                sites.insert(format!("idle-{i}.example"), Arc::new(tokio::sync::Semaphore::new(FETCHES_PER_SITE)));
            }
        }
        let turn = Turn::new(&SITE_TURNS, "newcomer.example".to_owned(), FETCHES_PER_SITE);
        assert!(!known(&SITE_TURNS, "idle-0.example"), "a full map drops idle keys to make room");
        assert!(known(&SITE_TURNS, "newcomer.example"));
        drop(turn);
        for map in [&SITE_TURNS, &REFRESH_SITE_TURNS] {
            map.lock().unwrap().insert("left-behind".to_owned(), Arc::new(tokio::sync::Semaphore::new(1)));
        }
        purge_fetch_sites();
        for map in [&SITE_TURNS, &REFRESH_SITE_TURNS] {
            assert!(!known(map, "left-behind"));
        }
    }

    /// Refreshes of known clients have turns of their own: all request turns taken does not stop them, and within
    /// the refresh pool one site holds one turn, so one site's refreshes cannot take the pool either.
    #[tokio::test]
    async fn refreshes_have_a_reserved_pool_shared_per_site() {
        let soon = || tokio::time::Instant::now() + Duration::from_millis(50);
        let all = FETCH_SLOTS.acquire_many(u32::try_from(MAX_CONCURRENT_FETCHES).unwrap()).await.unwrap();
        assert!(FETCH_SLOTS.try_acquire().is_err());
        let refresh = REFRESH_SLOTS.try_acquire();
        assert!(refresh.is_ok());
        drop((all, refresh));
        let mut first = Turn::new(&REFRESH_SITE_TURNS, "slow.example".to_owned(), REFRESHES_PER_SITE);
        assert!(first.acquire(soon()).await);
        let mut second = Turn::new(&REFRESH_SITE_TURNS, "slow.example".to_owned(), REFRESHES_PER_SITE);
        assert!(!second.acquire(soon()).await, "one refresh turn per site");
        let mut chatgpt = Turn::new(&REFRESH_SITE_TURNS, "chatgpt.com".to_owned(), REFRESHES_PER_SITE);
        assert!(chatgpt.acquire(soon()).await);
    }

    /// A background refresh that fails records no failure (the last good copy keeps standing in for the rest of its
    /// grace) and only backs the next attempt off, briefly.
    #[tokio::test]
    async fn a_failed_background_refresh_records_no_failure() {
        let url = "https://does-not-exist.invalid/oauth/jwks.json";
        let ip: IpAddr = "192.0.2.10".parse().unwrap();
        refresh_jwks_in_background(url, ip);
        assert!(!OAUTH.may_refresh(FetchKind::KeySet, url), "attempts are spaced");
        for _ in 0..200 {
            if !FETCHES_IN_FLIGHT.lock().unwrap().contains_key(url) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!FETCHES_IN_FLIGHT.lock().unwrap().contains_key(url), "the refresh ended");
        assert!(OAUTH.failed_fetch(FetchKind::KeySet, url).is_none());
        assert!(
            OAUTH.cached_jwks(url).is_none() && OAUTH.last_good_jwks(url).is_none(),
            "nothing stretched or made up"
        );
    }

    #[test]
    fn refresh_starts_are_limited_per_requesting_address() {
        let ip: IpAddr = "192.0.2.77".parse().unwrap();
        let started = (0..50).filter(|_| may_trigger_refresh(ip)).count();
        assert!((10..=11).contains(&started), "{started}");
        assert!(may_trigger_refresh("192.0.2.78".parse().unwrap()), "another address has its own");
    }

    /// An address over its refresh limit that names someone else's client starts nothing and marks nothing: the
    /// client's own next request still refreshes at once.
    #[test]
    fn a_requester_over_its_limit_cannot_hold_back_anothers_refresh() {
        let attacker: IpAddr = "192.0.2.99".parse().unwrap();
        while may_trigger_refresh(attacker) {}
        let url = "https://refresh-order.example/jwks.json";
        refresh_jwks_in_background(url, attacker);
        assert!(OAUTH.refresh_due(FetchKind::KeySet, url), "no mark left behind");
        assert!(!FETCHES_IN_FLIGHT.lock().unwrap().contains_key(url), "nothing started");
    }

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
