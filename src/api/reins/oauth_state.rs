//! In-memory OAuth state (spec §4.1): authorize-page sessions (5 min), single-use
//! authorization codes (60 s, stored only as SHA-256 hashes), fetched client
//! metadata (1 h), fetched client key sets (5 min) and the `jti`s of client assertions
//! already used (until they expire). Nothing here is written to disk; the server runs
//! as one instance, so in memory is enough for replay protection.

use std::{
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use jsonwebtoken::jwk::JwkSet;
use reins_proto::ids::PairingId;

use super::{
    CIMD_TTL, CODE_TTL, SESSION_TTL,
    oauth::{ASSERTION_SKEW_SECS, AuthCode, AuthMethod, ClientInfo, MAX_ASSERTION_LIFETIME_SECS, ValidAuthorize},
    pairing::PairingClient,
    ttl::{Full, TtlMap},
};
use crate::auth::reins::{hash_token, random_token};

pub const MAX_SESSIONS: usize = 5_000;
pub const MAX_CODES: usize = 5_000;
pub const MAX_CLIENT_CACHE: usize = 1_000;
pub const MAX_JWKS_CACHE: usize = 1_000;
/// Client assertions remembered at once; when full, new assertions are refused until old ones expire (fail closed).
pub const MAX_USED_ASSERTIONS: usize = 50_000;
/// A client's key set is fetched again after this long.
pub const JWKS_TTL: Duration = Duration::from_secs(300);
/// An unknown `kid` refetches a key set at most this often (anyone can send one).
pub const JWKS_REFETCH_INTERVAL: Duration = Duration::from_secs(30);
/// A `jti` is remembered for as long as an assertion carrying it can be valid: its longest lifetime plus the skew
/// on both ends.
pub const USED_ASSERTION_TTL: Duration =
    Duration::from_secs(MAX_ASSERTION_LIFETIME_SECS.unsigned_abs() + 2 * ASSERTION_SKEW_SECS.unsigned_abs());

/// Why a verified assertion's `jti` could not be recorded.
#[derive(Debug, PartialEq, Eq)]
pub enum JtiError {
    Replayed,
    Full,
}

/// The pairing started for a session, once the user has submitted their email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedPairing {
    pub id: PairingId,
    /// The number the browser shows.
    pub code: u8,
    /// `None` for decoys (unknown email): nobody can ever approve them.
    pub user_uuid: Option<String>,
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
    used_assertions: Mutex<TtlMap<String, ()>>,
}

impl OAuthState {
    pub fn new() -> Self {
        Self::with_capacity(MAX_SESSIONS, MAX_CODES, MAX_CLIENT_CACHE)
    }

    pub fn with_capacity(sessions: usize, codes: usize, clients: usize) -> Self {
        Self::with_assertion_capacity(sessions, codes, clients, MAX_USED_ASSERTIONS)
    }

    pub fn with_assertion_capacity(sessions: usize, codes: usize, clients: usize, assertions: usize) -> Self {
        Self {
            sessions: Mutex::new(TtlMap::new(SESSION_TTL, sessions)),
            codes: Mutex::new(TtlMap::new(CODE_TTL, codes)),
            clients: Mutex::new(TtlMap::new(CIMD_TTL, clients)),
            jwks: Mutex::new(TtlMap::new(JWKS_TTL, MAX_JWKS_CACHE)),
            jwks_refetches: Mutex::new(TtlMap::new(JWKS_REFETCH_INTERVAL, MAX_JWKS_CACHE)),
            used_assertions: Mutex::new(TtlMap::new(USED_ASSERTION_TTL, assertions)),
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

    /// Stores a new session and returns its unguessable id.
    pub fn create_session(&self, session: AuthSession) -> Result<String, Full> {
        let id = random_token();
        self.sessions().insert(id.clone(), session)?;
        Ok(id)
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

    /// Stores `code` and returns the opaque authorization code to hand to the client.
    pub fn issue_code(&self, code: AuthCode) -> Result<String, Full> {
        let token = random_token();
        self.codes().insert(hash_token(&token), code)?;
        Ok(token)
    }

    /// Single use: the code is gone after the first call, valid or not.
    pub fn redeem_code(&self, token: &str) -> Option<AuthCode> {
        self.codes().remove(&hash_token(token))
    }

    pub fn cached_client(&self, client_id: &str) -> Option<ClientInfo> {
        self.clients().get(&client_id.to_owned()).cloned()
    }

    pub fn cache_client(&self, client: ClientInfo) {
        // A full cache just means the next request fetches again.
        self.clients().insert(client.client_id.clone(), client).ok();
    }

    pub fn cached_jwks(&self, jwks_uri: &str) -> Option<JwkSet> {
        Self::lock(&self.jwks).get(&jwks_uri.to_owned()).cloned()
    }

    pub fn cache_jwks(&self, jwks_uri: &str, set: JwkSet) {
        // A full cache just means the next assertion fetches again.
        Self::lock(&self.jwks).insert(jwks_uri.to_owned(), set).ok();
    }

    /// Whether an unknown `kid` may fetch `jwks_uri` again now: once per [`JWKS_REFETCH_INTERVAL`] and URI, so
    /// made-up key ids cannot turn this server into a fetcher of the client's key set.
    pub fn may_refetch_jwks(&self, jwks_uri: &str) -> bool {
        let mut marks = Self::lock(&self.jwks_refetches);
        let key = jwks_uri.to_owned();
        if marks.get(&key).is_some() {
            return false;
        }
        marks.insert(key, ()).is_ok()
    }

    /// Records a verified assertion's `jti` for `client_id`; a second use while it could still be valid fails.
    pub fn use_assertion(&self, client_id: &str, jti: &str) -> Result<(), JtiError> {
        let key = hash_token(&format!("{client_id}\n{jti}"));
        let mut used = Self::lock(&self.used_assertions);
        if used.get(&key).is_some() {
            return Err(JtiError::Replayed);
        }
        used.insert(key, ()).map_err(|Full| JtiError::Full)
    }

    pub fn purge(&self) {
        self.sessions().purge();
        self.codes().purge();
        self.clients().purge();
        Self::lock(&self.jwks).purge();
        Self::lock(&self.jwks_refetches).purge();
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
        let id = state.create_session(session()).unwrap();
        assert_eq!(id.len(), 43);
        assert_eq!(state.session(&id).unwrap().client.client_name, "Claude");
        tokio::time::advance(Duration::from_secs(299)).await;
        assert!(state.session(&id).is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(state.session(&id).is_none());
        let id2 = state.create_session(session()).unwrap();
        assert!(state.take_session(&id2).is_some());
        assert!(state.take_session(&id2).is_none());
        assert!(state.session("unknown").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_first_pairing_is_attached() {
        let state = OAuthState::new();
        let id = state.create_session(session()).unwrap();
        assert!(state.set_pairing(&id, pairing(47)));
        assert!(!state.set_pairing(&id, pairing(12)), "resubmitting the form must not restart the pairing");
        assert_eq!(state.session(&id).unwrap().pairing.unwrap().code, 47);
        assert!(!state.set_pairing("unknown", pairing(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn codes_are_single_use_and_expire_after_a_minute() {
        let state = OAuthState::new();
        let code = state.issue_code(auth_code()).unwrap();
        assert_eq!(state.redeem_code(&code).unwrap().connection_uuid, "c1");
        assert!(state.redeem_code(&code).is_none(), "replay");
        let code = state.issue_code(auth_code()).unwrap();
        assert!(state.redeem_code("a-different-guess").is_none());
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(state.redeem_code(&code).is_none(), "expired");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_redemption_attempt_still_burns_a_valid_code() {
        // redeem_code removes the entry before the caller checks PKCE/redirect: a stolen
        // code cannot be retried with a corrected verifier.
        let state = OAuthState::new();
        let code = state.issue_code(auth_code()).unwrap();
        assert!(state.redeem_code(&code).is_some());
        assert!(state.redeem_code(&code).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn capacity_is_bounded() {
        let state = OAuthState::with_capacity(1, 1, 1);
        state.create_session(session()).unwrap();
        assert!(state.create_session(session()).is_err());
        state.issue_code(auth_code()).unwrap();
        assert!(state.issue_code(auth_code()).is_err());
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
        assert!(state.may_refetch_jwks(uri));
        assert!(!state.may_refetch_jwks(uri), "a second unknown kid right away does not fetch again");
        assert!(state.may_refetch_jwks("https://b.example/jwks.json"), "per URI");
        tokio::time::advance(JWKS_REFETCH_INTERVAL).await;
        assert!(state.may_refetch_jwks(uri));
    }

    #[tokio::test(start_paused = true)]
    async fn an_assertion_id_is_single_use_while_it_can_be_valid() {
        let state = OAuthState::with_assertion_capacity(1, 1, 1, 2);
        assert_eq!(state.use_assertion("https://a.example/c.json", "j1"), Ok(()));
        assert_eq!(state.use_assertion("https://a.example/c.json", "j1"), Err(JtiError::Replayed));
        assert_eq!(state.use_assertion("https://b.example/c.json", "j1"), Ok(()), "per client");
        assert_eq!(state.use_assertion("https://a.example/c.json", "j2"), Err(JtiError::Full), "full fails closed");
        assert_eq!(USED_ASSERTION_TTL, Duration::from_secs(420));
        tokio::time::advance(Duration::from_secs(419)).await;
        assert_eq!(state.use_assertion("https://a.example/c.json", "j1"), Err(JtiError::Replayed));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(state.use_assertion("https://a.example/c.json", "j1"), Ok(()), "forgotten once it has expired");
    }
}
