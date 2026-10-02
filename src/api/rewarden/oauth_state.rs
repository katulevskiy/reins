//! In-memory OAuth state (spec §4.1): authorize-page sessions (5 min), single-use
//! authorization codes (60 s, stored only as SHA-256 hashes) and fetched client
//! metadata (1 h). Nothing here is written to disk.

use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use rewarden_proto::ids::PairingId;

use super::{
    CIMD_TTL, CODE_TTL, SESSION_TTL,
    oauth::{AuthCode, ClientInfo, ValidAuthorize},
    pairing::PairingClient,
    ttl::{Full, TtlMap},
};
use crate::auth::rewarden::{hash_token, random_token};

pub const MAX_SESSIONS: usize = 5_000;
pub const MAX_CODES: usize = 5_000;
pub const MAX_CLIENT_CACHE: usize = 1_000;

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
    pub pairing: Option<StartedPairing>,
}

pub struct OAuthState {
    sessions: Mutex<TtlMap<String, AuthSession>>,
    codes: Mutex<TtlMap<String, AuthCode>>,
    clients: Mutex<TtlMap<String, ClientInfo>>,
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
        }
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

    pub fn purge(&self) {
        self.sessions().purge();
        self.codes().purge();
        self.clients().purge();
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
    use std::time::Duration;

    use super::*;

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
        };
        assert!(state.cached_client(&client.client_id).is_none());
        state.cache_client(client.clone());
        assert_eq!(state.cached_client(&client.client_id), Some(client.clone()));
        tokio::time::advance(Duration::from_secs(3599)).await;
        assert!(state.cached_client(&client.client_id).is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(state.cached_client(&client.client_id).is_none());
    }
}
