//! In-memory OAuth state (spec §4.1): authorize-page sessions (5 min), single-use
//! authorization codes (60 s, stored only as SHA-256 hashes), fetched client
//! metadata (1 h), fetched client key sets (5 min) and the `jti`s of client assertions
//! already used (until they expire). Nothing here is written to disk; the server runs as one
//! instance, so in memory is enough. No map here refuses when full (see the constants).

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use jsonwebtoken::jwk::JwkSet;
use reins_proto::ids::PairingId;
use tokio::time::Instant;

use super::{
    CIMD_TTL, CODE_TTL, SESSION_TTL,
    oauth::{ASSERTION_SKEW_SECS, AuthCode, AuthMethod, ClientInfo, MAX_ASSERTION_LIFETIME_SECS, ValidAuthorize},
    pairing::PairingClient,
    ttl::TtlMap,
};
use crate::auth::reins::{hash_token, random_token};

// Every map here is shared by all callers, so none of them refuses when full: each drops its oldest entry instead.
// A full map then costs whoever owned the dropped entry a retry (a session or code expires early, a document or key
// set is fetched again, a refetch is allowed sooner), but it never turns away everybody else.

/// Authorize-page sessions; when full the oldest is dropped (its browser shows "Link expired").
pub const MAX_SESSIONS: usize = 5_000;
/// Authorization codes (60 s each); when full the oldest is dropped (its exchange fails, the client starts again).
pub const MAX_CODES: usize = 5_000;
pub const MAX_CLIENT_CACHE: usize = 1_000;
/// Key sets cached, and key sets with a recent refetch mark. Both evict their oldest entry when full: they only
/// save fetches and limit their rate, so dropping one early costs a fetch, never a refusal.
pub const MAX_JWKS_CACHE: usize = 1_000;
/// Client assertion `jti`s remembered. About 10 MB at most; when full the oldest is forgotten (see [`UsedAssertions`]).
pub const MAX_USED_ASSERTIONS: usize = 100_000;
/// A client's key set is fetched again after this long.
pub const JWKS_TTL: Duration = Duration::from_secs(300);
/// An unknown `kid` refetches a key set at most this often (anyone can send one).
pub const JWKS_REFETCH_INTERVAL: Duration = Duration::from_secs(30);
/// A metadata document or key set that could not be fetched is not fetched again for this long (see
/// [`OAuthState::remember_failed_fetch`] for which failures are remembered).
pub const FAILED_FETCH_TTL: Duration = Duration::from_secs(30);
/// Background refreshes of one document start at most this often, whatever became of the last one: a host that is
/// down is asked again soon, but never in a tight loop.
pub const REFRESH_SPACING: Duration = Duration::from_secs(5);
/// After failed background refreshes of a document, the spacing doubles per failure up to this. Far below the
/// grace period: back-off never holds a refresh back long enough to matter, and the grace itself never grows.
pub const MAX_REFRESH_BACKOFF: Duration = Duration::from_secs(60);

/// When the next background refresh of each document may start (per kind and URL), with back-off after failures.
/// Never refuses: when it holds too many documents it forgets those that may start now, then the soonest ones.
pub struct RefreshSchedule {
    capacity: usize,
    /// Key -> (earliest next start, failures in a row).
    next: HashMap<String, (Instant, u32)>,
}

impl RefreshSchedule {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next: HashMap::new(),
        }
    }

    /// Whether a refresh of `key` would be allowed now, without marking it.
    pub fn due(&self, key: &str) -> bool {
        self.next.get(key).is_none_or(|(next, _)| Instant::now() >= *next)
    }

    /// Whether a refresh of `key` may start now; if so, the next one waits at least the current spacing.
    pub fn may_start(&mut self, key: &str) -> bool {
        let now = Instant::now();
        let failures = match self.next.get(key) {
            Some((next, _)) if now < *next => return false,
            Some((_, failures)) => *failures,
            None => 0,
        };
        if self.next.len() >= self.capacity && !self.next.contains_key(key) {
            self.next.retain(|_, (next, _)| now < *next);
            if self.next.len() >= self.capacity
                && let Some(soonest) = self.next.iter().min_by_key(|(_, (next, _))| *next).map(|(k, _)| k.clone())
            {
                self.next.remove(&soonest);
            }
        }
        self.next.insert(key.to_owned(), (now + REFRESH_SPACING, failures));
        true
    }

    /// A refresh of `key` failed: the next waits twice as long as the last (up to [`MAX_REFRESH_BACKOFF`]).
    pub fn failed(&mut self, key: &str) {
        let failures = self.next.get(key).map_or(0, |(_, failures)| *failures).saturating_add(1);
        let backoff = REFRESH_SPACING.saturating_mul(1 << failures.min(10)).min(MAX_REFRESH_BACKOFF);
        self.next.insert(key.to_owned(), (Instant::now() + backoff, failures));
    }

    /// A refresh of `key` worked: back to the plain spacing.
    pub fn succeeded(&mut self, key: &str) {
        if let Some((_, failures)) = self.next.get_mut(key) {
            *failures = 0;
        }
    }

    pub fn purge(&mut self) {
        let now = Instant::now();
        self.next.retain(|_, (next, failures)| now < *next || *failures > 0);
    }
}
/// How long past its freshness a document or key set may still be used, while it is refreshed in the background (or
/// its host is down). A hard limit counted from the last successful fetch: a key the client rotated out stops being
/// accepted at the latest this long after the fresh copy expired, and at once when a refetch no longer lists it.
pub const GRACE: Duration = Duration::from_secs(600);
/// The last good metadata document: fresh for [`CIMD_TTL`], usable for [`GRACE`] more.
pub const CLIENT_LAST_GOOD_TTL: Duration = Duration::from_secs(CIMD_TTL.as_secs() + GRACE.as_secs());
/// The last good key set: fresh for [`JWKS_TTL`], usable for [`GRACE`] more.
pub const JWKS_LAST_GOOD_TTL: Duration = Duration::from_secs(JWKS_TTL.as_secs() + GRACE.as_secs());

/// What was fetched from a URL. Failures are remembered per kind and URL, so a URL fetched as one kind (and failing
/// to parse as it) can never be recorded as failed for the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchKind {
    Metadata,
    KeySet,
}

impl FetchKind {
    fn key(self, url: &str) -> String {
        let kind = match self {
            Self::Metadata => "metadata",
            Self::KeySet => "keys",
        };
        format!("{kind} {url}")
    }
}

/// A remembered failed fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFetch {
    pub why: String,
}
/// A `jti` is remembered for as long as an assertion carrying it can be valid: its longest lifetime plus the skew
/// on both ends.
pub const USED_ASSERTION_TTL: Duration =
    Duration::from_secs(MAX_ASSERTION_LIFETIME_SECS.unsigned_abs() + 2 * ASSERTION_SKEW_SECS.unsigned_abs());

/// The `jti`s of client assertions already used, for [`USED_ASSERTION_TTL`] each.
///
/// This is defence in depth, not what stops a replay: a `jti` is recorded only together with using up a single-use
/// grant of the same client (an authorization code, or a refresh token, which rotates on every use), so a replayed
/// assertion has no grant left to use. What this adds is that an assertion seen once cannot be paired with a second
/// grant of that client while it is still valid. Because it is shared by everybody, it never refuses: when full, the
/// oldest `jti` is forgotten (fail open), so nobody can fill it to lock others out. Entries are 32-byte digests kept
/// in insertion order, which with a single lifetime is also expiry order.
pub struct UsedAssertions {
    ttl: Duration,
    capacity: usize,
    live: HashSet<[u8; 32]>,
    order: VecDeque<(Instant, [u8; 32])>,
}

impl UsedAssertions {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            live: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    fn key(client_id: &str, jti: &str) -> [u8; 32] {
        // Length-prefixed, so no (client, jti) pair can collide with another.
        let input = format!("{}:{client_id}{jti}", client_id.len());
        let digest = ring::digest::digest(&ring::digest::SHA256, input.as_bytes());
        let mut key = [0u8; 32];
        key.copy_from_slice(digest.as_ref());
        key
    }

    /// Records `jti` for `client_id`: `false` if it is already recorded and still live (a replay).
    pub fn record(&mut self, client_id: &str, jti: &str) -> bool {
        let now = Instant::now();
        self.purge_at(now);
        let key = Self::key(client_id, jti);
        if self.live.contains(&key) {
            return false;
        }
        if self.capacity == 0 {
            return true;
        }
        while self.live.len() >= self.capacity {
            let Some((_, oldest)) = self.order.pop_front() else {
                break;
            };
            self.live.remove(&oldest);
        }
        self.live.insert(key);
        self.order.push_back((now + self.ttl, key));
        true
    }

    fn purge_at(&mut self, now: Instant) {
        while let Some((expires, key)) = self.order.front().copied() {
            if now < expires {
                break;
            }
            self.order.pop_front();
            self.live.remove(&key);
        }
    }

    pub fn purge(&mut self) {
        self.purge_at(Instant::now());
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.live.len()
    }
}

/// The pairing started for a session, once the user has submitted their email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedPairing {
    pub id: PairingId,
    /// The number the browser shows.
    pub code: u8,
    /// `None` for decoys (unknown email): nobody can ever approve them.
    pub user_uuid: Option<String>,
    /// The cap refused another pairing. The browser still sees the wait page; nothing is delivered to a phone.
    pub display_only: bool,
}

/// One authorization attempt in a browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthSession {
    pub request: ValidAuthorize,
    pub client: PairingClient,
    /// How the client must authenticate at the token endpoint, as resolved when the flow started.
    pub client_auth: AuthMethod,
    pub pairing: Option<StartedPairing>,
}

pub struct OAuthState {
    sessions: Mutex<TtlMap<String, AuthSession>>,
    codes: Mutex<TtlMap<String, AuthCode>>,
    clients: Mutex<TtlMap<String, ClientInfo>>,
    jwks: Mutex<TtlMap<String, JwkSet>>,
    jwks_refetches: Mutex<TtlMap<String, ()>>,
    refresh_schedule: Mutex<RefreshSchedule>,
    failed_fetches: Mutex<TtlMap<String, FailedFetch>>,
    last_good_clients: Mutex<TtlMap<String, ClientInfo>>,
    last_good_jwks: Mutex<TtlMap<String, JwkSet>>,
    used_assertions: Mutex<UsedAssertions>,
}

impl OAuthState {
    pub fn new() -> Self {
        Self::with_capacity(MAX_SESSIONS, MAX_CODES, MAX_CLIENT_CACHE)
    }

    pub fn with_capacity(sessions: usize, codes: usize, clients: usize) -> Self {
        Self {
            sessions: Mutex::new(TtlMap::new(SESSION_TTL, sessions)),
            codes: Mutex::new(TtlMap::new(CODE_TTL, codes)),
            clients: Mutex::new(TtlMap::new(CIMD_TTL, clients)),
            jwks: Mutex::new(TtlMap::new(JWKS_TTL, MAX_JWKS_CACHE)),
            jwks_refetches: Mutex::new(TtlMap::new(JWKS_REFETCH_INTERVAL, MAX_JWKS_CACHE)),
            refresh_schedule: Mutex::new(RefreshSchedule::new(MAX_JWKS_CACHE)),
            failed_fetches: Mutex::new(TtlMap::new(FAILED_FETCH_TTL, MAX_JWKS_CACHE)),
            last_good_clients: Mutex::new(TtlMap::new(CLIENT_LAST_GOOD_TTL, clients)),
            last_good_jwks: Mutex::new(TtlMap::new(JWKS_LAST_GOOD_TTL, MAX_JWKS_CACHE)),
            used_assertions: Mutex::new(UsedAssertions::new(USED_ASSERTION_TTL, MAX_USED_ASSERTIONS)),
        }
    }

    /// Small limits for the key-set caches and the replay memory (tests).
    #[cfg(test)]
    pub fn with_assertion_limits(key_sets: usize, used_assertions: usize) -> Self {
        let state = Self::new();
        Self {
            jwks: Mutex::new(TtlMap::new(JWKS_TTL, key_sets)),
            jwks_refetches: Mutex::new(TtlMap::new(JWKS_REFETCH_INTERVAL, key_sets)),
            used_assertions: Mutex::new(UsedAssertions::new(USED_ASSERTION_TTL, used_assertions)),
            ..state
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sessions(&self) -> MutexGuard<'_, TtlMap<String, AuthSession>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn codes(&self) -> MutexGuard<'_, TtlMap<String, AuthCode>> {
        self.codes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn clients(&self) -> MutexGuard<'_, TtlMap<String, ClientInfo>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores a new session and returns its unguessable id (dropping the oldest session if there are too many).
    pub fn create_session(&self, session: AuthSession) -> String {
        let id = random_token();
        self.sessions().insert_evicting(id.clone(), session);
        id
    }

    pub fn session(&self, id: &str) -> Option<AuthSession> {
        self.sessions().get(&id.to_owned()).cloned()
    }

    /// Attaches the pairing; only the first submission for a session counts.
    pub fn set_pairing(&self, id: &str, pairing: StartedPairing) -> bool {
        let mut sessions = self.sessions();
        match sessions.get_mut(&id.to_owned()) {
            Some(session) if session.pairing.is_none() => {
                session.pairing = Some(pairing);
                true
            }
            _ => false,
        }
    }

    /// Removes the session; used when the flow ends so a page reload cannot mint a second code.
    pub fn take_session(&self, id: &str) -> Option<AuthSession> {
        self.sessions().remove(&id.to_owned())
    }

    /// Stores `code` and returns the opaque authorization code to hand to the client (dropping the oldest code if
    /// there are too many).
    pub fn issue_code(&self, code: AuthCode) -> String {
        let token = random_token();
        self.codes().insert_evicting(hash_token(&token), code);
        token
    }

    /// Single use: the code is gone after the first call, valid or not.
    pub fn redeem_code(&self, token: &str) -> Option<AuthCode> {
        self.codes().remove(&hash_token(token))
    }

    /// The code, without using it up: the token endpoint checks the request against it first.
    pub fn peek_code(&self, token: &str) -> Option<AuthCode> {
        self.codes().get(&hash_token(token)).cloned()
    }

    pub fn cached_client(&self, client_id: &str) -> Option<ClientInfo> {
        self.clients().get(&client_id.to_owned()).cloned()
    }

    /// Caches a freshly fetched document, and keeps it as the client's last good one.
    pub fn cache_client(&self, client: ClientInfo) {
        // A full cache drops its oldest document, which is fetched again when needed.
        Self::lock(&self.last_good_clients).insert_evicting(client.client_id.clone(), client.clone());
        self.clients().insert_evicting(client.client_id.clone(), client);
    }

    /// The last document fetched successfully for `client_id`, while within [`CLIENT_LAST_GOOD_TTL`] of that fetch.
    pub fn last_good_client(&self, client_id: &str) -> Option<ClientInfo> {
        Self::lock(&self.last_good_clients).get(&client_id.to_owned()).cloned()
    }

    pub fn cached_jwks(&self, jwks_uri: &str) -> Option<JwkSet> {
        Self::lock(&self.jwks).get(&jwks_uri.to_owned()).cloned()
    }

    /// Caches a freshly fetched key set, and keeps it as the URL's last good one.
    pub fn cache_jwks(&self, jwks_uri: &str, set: JwkSet) {
        // A full cache drops its oldest key set, which is fetched again when needed.
        Self::lock(&self.last_good_jwks).insert_evicting(jwks_uri.to_owned(), set.clone());
        Self::lock(&self.jwks).insert_evicting(jwks_uri.to_owned(), set);
    }

    /// The last key set fetched successfully from `jwks_uri`, while within [`JWKS_LAST_GOOD_TTL`] of that fetch.
    pub fn last_good_jwks(&self, jwks_uri: &str) -> Option<JwkSet> {
        Self::lock(&self.last_good_jwks).get(&jwks_uri.to_owned()).cloned()
    }

    /// Why fetching `url` as `kind` failed, if it failed within [`FAILED_FETCH_TTL`]. Keyed by the exact URL the
    /// fetch was made for, and by kind.
    pub fn failed_fetch(&self, kind: FetchKind, url: &str) -> Option<FailedFetch> {
        Self::lock(&self.failed_fetches).get(&kind.key(url)).cloned()
    }

    /// Remembers that an actual fetch of exactly `url`, as `kind`, failed (the oldest such memory makes room when
    /// there are too many). Callers record only failures that do not depend on timing, or failures of a URL that
    /// has a last good result to stand in: a blip of a host never locks out a client that has nothing else.
    pub fn remember_failed_fetch(&self, kind: FetchKind, url: &str, why: &str) {
        let failed = FailedFetch {
            why: why.to_owned(),
        };
        Self::lock(&self.failed_fetches).insert_evicting(kind.key(url), failed);
    }

    /// Whether a background refresh of `url` as `kind` may start now: once per [`REFRESH_SPACING`], or after failed
    /// refreshes once per back-off (doubling, at most [`MAX_REFRESH_BACKOFF`]). Only spacing: a refresh that failed
    /// never stops later ones for long, and never changes how long the last good copy may be used.
    pub fn may_refresh(&self, kind: FetchKind, url: &str) -> bool {
        Self::lock(&self.refresh_schedule).may_start(&kind.key(url))
    }

    /// Whether a background refresh of `url` as `kind` would be allowed now (nothing is marked).
    pub fn refresh_due(&self, kind: FetchKind, url: &str) -> bool {
        Self::lock(&self.refresh_schedule).due(&kind.key(url))
    }

    pub fn refresh_failed(&self, kind: FetchKind, url: &str) {
        Self::lock(&self.refresh_schedule).failed(&kind.key(url));
    }

    pub fn refresh_succeeded(&self, kind: FetchKind, url: &str) {
        Self::lock(&self.refresh_schedule).succeeded(&kind.key(url));
    }

    /// Whether an unknown `kid` from `client_id` may fetch `jwks_uri` again now: once per [`JWKS_REFETCH_INTERVAL`],
    /// per client and URI, so made-up key ids cannot turn this server into a fetcher of the key set, and another
    /// client naming the same `jwks_uri` cannot use up this client's refetch. The oldest mark is dropped when there
    /// are too many, so other clients' marks can never block this one's refetch either.
    pub fn may_refetch_jwks(&self, client_id: &str, jwks_uri: &str) -> bool {
        let mut marks = Self::lock(&self.jwks_refetches);
        let key = format!("{client_id}\n{jwks_uri}");
        if marks.get(&key).is_some() {
            return false;
        }
        marks.insert_evicting(key, ());
        true
    }

    /// Records a verified assertion's `jti` for `client_id`: `false` for a replay while it could still be valid. Call
    /// it only once the request's grant is known to belong to `client_id`, right before the grant is used up.
    pub fn use_assertion(&self, client_id: &str, jti: &str) -> bool {
        Self::lock(&self.used_assertions).record(client_id, jti)
    }

    pub fn purge(&self) {
        self.sessions().purge();
        self.codes().purge();
        self.clients().purge();
        Self::lock(&self.jwks).purge();
        Self::lock(&self.jwks_refetches).purge();
        Self::lock(&self.refresh_schedule).purge();
        Self::lock(&self.failed_fetches).purge();
        Self::lock(&self.last_good_clients).purge();
        Self::lock(&self.last_good_jwks).purge();
        Self::lock(&self.used_assertions).purge();
    }
}

impl Default for OAuthState {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide OAuth state.
pub static OAUTH: LazyLock<OAuthState> = LazyLock::new(OAuthState::new);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::reins::oauth::ClientAuth;

    fn session() -> AuthSession {
        AuthSession {
            request: ValidAuthorize {
                client_id: "cid".into(),
                redirect_uri: "https://a.example/cb".into(),
                code_challenge: "c".repeat(43),
                state: Some("s".into()),
            },
            client: PairingClient {
                client_id: "cid".into(),
                client_name: "Claude".into(),
                client_host: "a.example".into(),
                client_key: None,
            },
            client_auth: AuthMethod::None,
            pairing: None,
        }
    }

    fn pairing(code: u8) -> StartedPairing {
        StartedPairing {
            id: "p1".into(),
            code,
            user_uuid: Some("u1".into()),
            display_only: false,
        }
    }

    fn auth_code() -> AuthCode {
        AuthCode {
            user_uuid: "u1".into(),
            connection_uuid: "c1".into(),
            client_id: "cid".into(),
            redirect_uri: "https://a.example/cb".into(),
            code_challenge: "c".repeat(43),
            client_auth: AuthMethod::PrivateKeyJwt,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn sessions_expire_after_five_minutes_and_take_once() {
        let state = OAuthState::new();
        let id = state.create_session(session());
        assert_eq!(id.len(), 43);
        assert_eq!(state.session(&id).unwrap().client.client_name, "Claude");
        tokio::time::advance(Duration::from_secs(299)).await;
        assert!(state.session(&id).is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(state.session(&id).is_none());
        let id2 = state.create_session(session());
        assert!(state.take_session(&id2).is_some());
        assert!(state.take_session(&id2).is_none());
        assert!(state.session("unknown").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_first_pairing_is_attached() {
        let state = OAuthState::new();
        let id = state.create_session(session());
        assert!(state.set_pairing(&id, pairing(47)));
        assert!(!state.set_pairing(&id, pairing(12)), "resubmitting the form must not restart the pairing");
        assert_eq!(state.session(&id).unwrap().pairing.unwrap().code, 47);
        assert!(!state.set_pairing("unknown", pairing(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn codes_are_single_use_and_expire_after_a_minute() {
        let state = OAuthState::new();
        let code = state.issue_code(auth_code());
        assert_eq!(state.redeem_code(&code).unwrap().connection_uuid, "c1");
        assert!(state.redeem_code(&code).is_none(), "replay");
        let code = state.issue_code(auth_code());
        assert!(state.redeem_code("a-different-guess").is_none());
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(state.redeem_code(&code).is_none(), "expired");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_redemption_attempt_still_burns_a_valid_code() {
        // redeem_code removes the entry before the caller checks PKCE/redirect: a stolen
        // code cannot be retried with a corrected verifier.
        let state = OAuthState::new();
        let code = state.issue_code(auth_code());
        assert!(state.redeem_code(&code).is_some());
        assert!(state.redeem_code(&code).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn capacity_is_bounded_but_never_refuses_newcomers() {
        // A flood of sessions or codes drops the oldest ones; the newest always works.
        let state = OAuthState::with_capacity(2, 2, 1);
        let first = state.create_session(session());
        let sessions: Vec<String> = (0..100).map(|_| state.create_session(session())).collect();
        assert!(state.session(&first).is_none(), "the oldest made room");
        assert!(state.session(&sessions[99]).is_some() && state.session(&sessions[98]).is_some());
        assert!(state.session(&sessions[97]).is_none(), "bounded");
        let first = state.issue_code(auth_code());
        let codes: Vec<String> = (0..100).map(|_| state.issue_code(auth_code())).collect();
        assert!(state.peek_code(&first).is_none());
        assert!(state.redeem_code(&codes[99]).is_some());
        let client = |id: &str| ClientInfo {
            client_id: id.into(),
            client_name: "A".into(),
            redirect_uris: vec!["https://a.example/cb".into()],
            auth: ClientAuth::None,
        };
        state.cache_client(client("https://a.example/1"));
        state.cache_client(client("https://a.example/2"));
        assert!(state.cached_client("https://a.example/2").is_some(), "a full cache takes the newest");
    }

    #[tokio::test(start_paused = true)]
    async fn client_metadata_is_cached_for_an_hour() {
        let state = OAuthState::new();
        let client = ClientInfo {
            client_id: "https://a.example/c.json".into(),
            client_name: "A".into(),
            redirect_uris: vec!["https://a.example/cb".into()],
            auth: ClientAuth::None,
        };
        assert!(state.cached_client(&client.client_id).is_none());
        state.cache_client(client.clone());
        assert_eq!(state.cached_client(&client.client_id), Some(client.clone()));
        tokio::time::advance(Duration::from_secs(3599)).await;
        assert!(state.cached_client(&client.client_id).is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(state.cached_client(&client.client_id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn key_sets_are_cached_briefly_and_refetched_at_most_every_30_seconds() {
        let state = OAuthState::new();
        let uri = "https://a.example/jwks.json";
        assert!(state.cached_jwks(uri).is_none());
        state.cache_jwks(uri, JwkSet::default());
        assert!(state.cached_jwks(uri).is_some());
        tokio::time::advance(JWKS_TTL).await;
        assert!(state.cached_jwks(uri).is_none(), "five minutes at most");
        assert!(state.may_refetch_jwks(A, uri));
        assert!(!state.may_refetch_jwks(A, uri), "a second unknown kid right away does not fetch again");
        assert!(state.may_refetch_jwks(A, "https://b.example/jwks.json"), "per URI");
        tokio::time::advance(JWKS_REFETCH_INTERVAL).await;
        assert!(state.may_refetch_jwks(A, uri));
    }

    /// Background refreshes are spaced, never stopped: the next one may start 5 s later, per kind and URL, and a full
    /// set of marks drops its oldest rather than refusing.
    #[tokio::test(start_paused = true)]
    async fn background_refreshes_are_spaced_but_never_stopped() {
        let state = OAuthState::new();
        let uri = "https://chatgpt.com/oauth/jwks.json";
        assert!(state.may_refresh(FetchKind::KeySet, uri));
        assert!(!state.may_refresh(FetchKind::KeySet, uri));
        assert!(state.may_refresh(FetchKind::Metadata, uri), "per kind");
        tokio::time::advance(REFRESH_SPACING).await;
        assert!(state.may_refresh(FetchKind::KeySet, uri), "tried again 5 s later");
        for i in 0..2 * MAX_JWKS_CACHE {
            state.may_refresh(FetchKind::KeySet, &format!("https://evil-{i}.example/jwks.json"));
        }
        tokio::time::advance(REFRESH_SPACING).await;
        assert!(state.may_refresh(FetchKind::KeySet, uri));
    }

    /// Failed refreshes back off (10 s, 20 s, 40 s, then at most a minute), far below the grace period, and a
    /// working refresh returns to the plain spacing.
    #[tokio::test(start_paused = true)]
    async fn failed_refreshes_back_off_up_to_a_minute() {
        let state = OAuthState::new();
        let uri = "https://slow.example/jwks.json";
        let waits_for = |secs: u64| {
            let state = &state;
            async move {
                assert!(state.may_refresh(FetchKind::KeySet, uri));
                state.refresh_failed(FetchKind::KeySet, uri);
                tokio::time::advance(Duration::from_secs(secs - 1)).await;
                assert!(!state.may_refresh(FetchKind::KeySet, uri), "still backing off before {secs} s");
                tokio::time::advance(Duration::from_secs(1)).await;
            }
        };
        waits_for(10).await;
        waits_for(20).await;
        waits_for(40).await;
        waits_for(60).await;
        waits_for(60).await;
        assert!(MAX_REFRESH_BACKOFF < GRACE);
        assert!(state.may_refresh(FetchKind::KeySet, uri));
        state.refresh_succeeded(FetchKind::KeySet, uri);
        tokio::time::advance(REFRESH_SPACING).await;
        assert!(state.may_refresh(FetchKind::KeySet, uri), "plain spacing again");
    }

    /// Another client naming ChatGPT's `jwks_uri` as its own cannot use up ChatGPT's refetch.
    #[tokio::test(start_paused = true)]
    async fn the_refetch_throttle_is_per_client() {
        let state = OAuthState::new();
        let uri = "https://chatgpt.com/oauth/jwks.json";
        assert!(state.may_refetch_jwks(A, uri));
        assert!(!state.may_refetch_jwks(A, uri));
        assert!(state.may_refetch_jwks(CHATGPT, uri), "ChatGPT's own turn is untouched");
    }

    const A: &str = "https://a.example/c.json";
    const B: &str = "https://b.example/c.json";
    const CHATGPT: &str = "https://chatgpt.com/oauth/client.json";

    #[tokio::test(start_paused = true)]
    async fn an_assertion_id_is_single_use_while_it_can_be_valid() {
        let state = OAuthState::new();
        assert!(state.use_assertion(A, "j1"));
        assert!(!state.use_assertion(A, "j1"), "replay");
        assert!(state.use_assertion(B, "j1"), "per client");
        // Client and jti cannot run into each other: "…c.json" + "xj1" is not "…c.jsonx" + "j1".
        assert!(state.use_assertion(A, "xj1"));
        assert!(state.use_assertion(&format!("{A}x"), "j1"));
        assert_eq!(USED_ASSERTION_TTL, Duration::from_secs(420));
        tokio::time::advance(Duration::from_secs(419)).await;
        assert!(!state.use_assertion(A, "j1"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(state.use_assertion(A, "j1"), "forgotten once it has expired");
    }

    /// Clients controlled by one party that keep live jtis (one or many each) can never make anyone else's
    /// assertion fail: when the memory is full the oldest jti is forgotten, nobody is refused.
    #[tokio::test(start_paused = true)]
    async fn nobody_can_fill_the_replay_memory_to_lock_out_another_client() {
        let state = OAuthState::with_assertion_limits(4, 50);
        assert!(state.use_assertion(CHATGPT, "c0"));
        for i in 0..1000 {
            assert!(state.use_assertion(&format!("https://evil-{i}.example/client.json"), "j"), "{i}");
            assert!(state.use_assertion(A, &format!("flood-{i}")));
        }
        // ChatGPT was quiet all along and holds nothing now: it is still accepted, again and again.
        for i in 1..100 {
            assert!(state.use_assertion(CHATGPT, &format!("c{i}")));
        }
        assert!(!state.use_assertion(CHATGPT, "c99"), "recent jtis are still caught");
        let used = OAuthState::with_assertion_limits(4, 50).used_assertions;
        let mut used = used.into_inner().unwrap();
        for i in 0..1000 {
            used.record(A, &format!("{i}"));
        }
        assert_eq!(used.len(), 50, "bounded");
        tokio::time::advance(USED_ASSERTION_TTL).await;
        used.purge();
        assert_eq!(used.len(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_fetches_are_remembered_briefly_and_never_fill_up() {
        let state = OAuthState::new();
        let url = "https://slow.example/client.json";
        let why = |f: Option<FailedFetch>| f.map(|f| f.why);
        assert!(state.failed_fetch(FetchKind::Metadata, url).is_none());
        state.remember_failed_fetch(FetchKind::Metadata, url, "answered 404");
        assert_eq!(why(state.failed_fetch(FetchKind::Metadata, url)).as_deref(), Some("answered 404"));
        for i in 0..2 * MAX_JWKS_CACHE {
            state.remember_failed_fetch(FetchKind::Metadata, &format!("https://evil-{i}.example/c.json"), "nope");
        }
        state.remember_failed_fetch(FetchKind::Metadata, url, "again");
        assert_eq!(why(state.failed_fetch(FetchKind::Metadata, url)).as_deref(), Some("again"), "still takes new ones");
        tokio::time::advance(FAILED_FETCH_TTL).await;
        assert!(state.failed_fetch(FetchKind::Metadata, url).is_none(), "tried again after 30 s");
    }

    /// A URL fetched as a metadata document (and failing as one) is not failed as a key set, and the other way
    /// round: the failure memory is keyed by kind and exact URL.
    #[tokio::test(start_paused = true)]
    async fn a_failure_of_one_kind_never_marks_the_other() {
        let state = OAuthState::new();
        let jwks = "https://chatgpt.com/oauth/jwks.json";
        state.remember_failed_fetch(FetchKind::Metadata, jwks, "not a client metadata document");
        assert!(state.failed_fetch(FetchKind::KeySet, jwks).is_none());
        assert!(state.failed_fetch(FetchKind::Metadata, "https://chatgpt.com/oauth/jwks.json/").is_none(), "exact URL");
        assert!(state.failed_fetch(FetchKind::Metadata, "https://CHATGPT.com/oauth/jwks.json").is_none(), "exact URL");
    }

    /// The last good document and key set outlive the fresh caches, to stand in when a later fetch fails.
    #[tokio::test(start_paused = true)]
    async fn the_last_good_copy_outlives_the_fresh_cache() {
        let state = OAuthState::new();
        let client = ClientInfo {
            client_id: CHATGPT.into(),
            client_name: "ChatGPT".into(),
            redirect_uris: vec!["https://chatgpt.com/cb".into()],
            auth: ClientAuth::None,
        };
        let uri = "https://chatgpt.com/oauth/jwks.json";
        state.cache_client(client.clone());
        state.cache_jwks(uri, JwkSet::default());
        // Key sets: fresh for 5 minutes, usable for 10 more, then gone (counted from the fetch, never extended).
        tokio::time::advance(JWKS_TTL).await;
        assert!(state.cached_jwks(uri).is_none());
        assert!(state.last_good_jwks(uri).is_some());
        tokio::time::advance(GRACE).await;
        assert!(state.last_good_jwks(uri).is_none(), "15 minutes at most");
        // Documents: fresh for an hour, usable for 10 minutes more.
        tokio::time::advance(Duration::from_mins(45)).await;
        assert!(state.cached_client(CHATGPT).is_none());
        assert_eq!(state.last_good_client(CHATGPT), Some(client));
        tokio::time::advance(GRACE).await;
        assert!(state.last_good_client(CHATGPT).is_none(), "70 minutes at most");
        // A successful refetch replaces both copies: a key it no longer lists is gone at once.
        let old = JwkSet {
            keys: serde_json::from_value(serde_json::json!([{"kty": "oct", "kid": "old", "k": "AA"}])).unwrap(),
        };
        state.cache_jwks(uri, old);
        state.cache_jwks(uri, JwkSet::default());
        assert!(state.last_good_jwks(uri).unwrap().keys.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_refetch_throttle_cannot_be_filled_by_other_key_sets() {
        let state = OAuthState::with_assertion_limits(4, 1);
        let chatgpt = "https://chatgpt.com/oauth/jwks.json";
        assert!(state.may_refetch_jwks(CHATGPT, chatgpt));
        assert!(!state.may_refetch_jwks(CHATGPT, chatgpt), "throttled");
        // Someone else marks far more key sets than there is room for.
        for i in 0..100 {
            assert!(state.may_refetch_jwks(A, &format!("https://evil-{i}.example/jwks.json")));
        }
        tokio::time::advance(JWKS_REFETCH_INTERVAL).await;
        assert!(state.may_refetch_jwks(CHATGPT, chatgpt), "never refused because the marks are full");
        // The key-set cache likewise keeps taking new sets, dropping the oldest.
        for i in 0..10 {
            state.cache_jwks(&format!("https://evil-{i}.example/jwks.json"), JwkSet::default());
        }
        state.cache_jwks(chatgpt, JwkSet::default());
        assert!(state.cached_jwks(chatgpt).is_some());
    }
}
