//! The live Vaultwarden session: an access token that refreshes itself (rotating the
//! stored refresh token) and an authenticated-call helper that retries once on 401.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::CoreError;
use crate::http::ServerUrl;
use crate::store::{Store, StoredSession};
use crate::vault::{Tokens, VaultClient};

/// Access tokens are renewed this long before they expire.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

struct TokenState {
    access: Option<(Zeroizing<String>, Instant)>,
    refresh: Zeroizing<String>,
}

pub struct Session {
    pub server: ServerUrl,
    email: std::sync::Mutex<String>,
    pub http: reqwest::Client,
    store: Arc<Store>,
    state: Mutex<TokenState>,
    /// This phone's device key, sent with every phone-API call (`None`: the store could not make one; the server then
    /// does not take this phone for the approval device).
    device_key: Option<String>,
    /// The master password hash of the password this session signed in or unlocked with, kept until the phone is
    /// the approval device: the proof `PUT /device` wants when another device approves for the account.
    proof: std::sync::Mutex<Option<Zeroizing<String>>>,
}

impl Session {
    /// A session right after login (tokens known) or after a restart (`tokens = None`,
    /// only the stored refresh token).
    pub fn new(
        http: reqwest::Client,
        store: Arc<Store>,
        server: ServerUrl,
        email: String,
        refresh: Zeroizing<String>,
        access: Option<(Zeroizing<String>, i64)>,
    ) -> Self {
        if let Some((token, _)) = &access {
            remember_account_id(&store, &server, &email, token);
        }
        let access =
            access.map(|(token, secs)| (token, Instant::now() + Duration::from_secs(u64::try_from(secs).unwrap_or(0))));
        let device_key = store.device_key().map_err(|e| log::warn!("no device key: {e}")).ok();
        Self {
            server,
            email: std::sync::Mutex::new(email),
            http,
            store,
            state: Mutex::new(TokenState {
                access,
                refresh,
            }),
            device_key,
            proof: std::sync::Mutex::default(),
        }
    }

    /// The value of the device key header ([`reins_proto::device::DEVICE_KEY_HEADER`]).
    pub fn device_key(&self) -> Option<&str> {
        self.device_key.as_deref()
    }

    pub fn email(&self) -> String {
        self.email.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn proof_slot(&self) -> std::sync::MutexGuard<'_, Option<Zeroizing<String>>> {
        self.proof.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keeps the master password hash of the password just used, for taking the approval role.
    pub(crate) fn keep_proof(&self, master_password_hash: Zeroizing<String>) {
        *self.proof_slot() = Some(master_password_hash);
    }

    pub(crate) fn proof(&self) -> Option<Zeroizing<String>> {
        self.proof_slot().clone()
    }

    pub(crate) fn forget_proof(&self) {
        *self.proof_slot() = None;
    }

    pub fn from_login(
        http: reqwest::Client,
        store: Arc<Store>,
        server: ServerUrl,
        email: String,
        tokens: Tokens,
    ) -> Self {
        Self::new(http, store, server, email, tokens.refresh_token, Some((tokens.access_token, tokens.expires_in)))
    }

    /// A valid access token, refreshing when missing or about to expire.
    pub async fn access_token(&self) -> Result<Zeroizing<String>, CoreError> {
        let mut state = self.state.lock().await;
        let cached = state
            .access
            .as_ref()
            .and_then(|(token, expires)| (Instant::now() + EXPIRY_MARGIN < *expires).then(|| token.clone()));
        if let Some(token) = cached {
            return Ok(token);
        }
        self.refresh_locked(&mut state).await
    }

    /// The subject from a token authenticated by this server, remembered across restarts. This only identifies
    /// locally encrypted secrets; it never authorizes a server call. Reading recovery keys must also work offline.
    pub(crate) async fn account_user_id(&self) -> Result<String, CoreError> {
        let key = account_id_key(&self.server, &self.email());
        if let Some(id) = self.store.meta_get(&key)? {
            return Ok(id);
        }
        let token = self.access_token().await?;
        let (id, _) = crate::sso::access_claims(&token)?;
        self.store.meta_set(&key, &id)?;
        Ok(id)
    }

    /// Verified account metadata piggybacks on the authenticated pending poll, so email updates do not wait for
    /// token expiry or add HTTP requests. Only random-secret accounts can follow email changes without KDF changes.
    pub(crate) async fn adopt_account_email(&self, email: &str) -> Result<(), CoreError> {
        let email = email.trim().to_lowercase();
        if email == self.email() {
            return Ok(());
        }
        if email.len() > 254 || !email.contains('@') || email.chars().any(char::is_control) {
            return Err(CoreError::invalid("the server returned an invalid account email"));
        }
        let id = self.account_user_id().await?;
        if self.store.secret_get(crate::sso::SECRET_SERVICE, &id)?.is_none() {
            return Ok(());
        }
        // Serialize with token refresh so the latest rotating refresh token is persisted with the new email.
        let state = self.state.lock().await;
        let previous = self.email();
        let saved = StoredSession {
            server_url: self.server.as_str().to_owned(),
            email: email.clone(),
            refresh_token: state.refresh.clone(),
        };
        self.store.rename_session_account(
            reins_proto::connector::VAULT,
            &previous,
            &saved,
            &account_id_key(&self.server, &email),
            &id,
        )?;
        *self.email.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = email;
        Ok(())
    }

    /// Forgets the cached access token (the server said 401) so the next call refreshes.
    pub async fn invalidate(&self) {
        self.state.lock().await.access = None;
    }

    async fn refresh_locked(&self, state: &mut TokenState) -> Result<Zeroizing<String>, CoreError> {
        let tokens = match VaultClient::new(&self.http, &self.server).refresh(&state.refresh).await {
            Ok(tokens) => tokens,
            Err(CoreError::NotLoggedIn) => {
                // The refresh token itself was rejected: the session is over.
                if let Err(e) = self.store.clear_session() {
                    log::warn!("could not clear the rejected session: {e}");
                }
                return Err(CoreError::NotLoggedIn);
            }
            Err(e) => return Err(e),
        };
        state.refresh.clone_from(&tokens.refresh_token);
        let mut email = self.email();
        let mut saved_with_rename = false;
        if let Ok((id, updated_email)) = crate::sso::access_claims(&tokens.access_token) {
            let known_id = self.store.meta_get(&account_id_key(&self.server, &email))?;
            if known_id.as_ref().is_some_and(|known| known != &id) {
                self.store.clear_session()?;
                return Err(CoreError::NotLoggedIn);
            }
            if updated_email != email && self.store.secret_get(crate::sso::SECRET_SERVICE, &id)?.is_some() {
                let saved = StoredSession {
                    server_url: self.server.as_str().to_owned(),
                    email: updated_email.clone(),
                    refresh_token: tokens.refresh_token.clone(),
                };
                match self.store.rename_session_account(
                    reins_proto::connector::VAULT,
                    &email,
                    &saved,
                    &account_id_key(&self.server, &updated_email),
                    &id,
                ) {
                    Ok(()) => {
                        email = updated_email;
                        self.email.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone_from(&email);
                        saved_with_rename = true;
                    }
                    Err(error) => log::warn!("could not update the vault account name: {error}"),
                }
            }
        }
        remember_account_id(&self.store, &self.server, &email, &tokens.access_token);
        // A failed rename rolls back the old alias. Still persist the rotating token with that unchanged email.
        if !saved_with_rename {
            let saved = StoredSession {
                server_url: self.server.as_str().to_owned(),
                email,
                refresh_token: tokens.refresh_token,
            };
            if let Err(e) = self.store.save_session(&saved) {
                log::warn!("could not persist the rotated refresh token: {e}");
            }
        }
        let expires = Instant::now() + Duration::from_secs(u64::try_from(tokens.expires_in).unwrap_or(0));
        state.access = Some((tokens.access_token.clone(), expires));
        Ok(tokens.access_token)
    }
}

fn account_id_key(server: &ServerUrl, email: &str) -> String {
    // A JSON tuple is unambiguous even for unusual self-hosted paths or email addresses.
    format!("session.account-id:{}", serde_json::json!([server.as_str(), email]))
}

fn remember_account_id(store: &Store, server: &ServerUrl, email: &str, token: &str) {
    if let Ok((id, _)) = crate::sso::access_claims(token)
        && let Err(error) = store.meta_set(&account_id_key(server, email), &id)
    {
        log::warn!("could not remember the session account id: {error}");
    }
}

/// Runs a phone-API call with a fresh token; a 401 invalidates the token and retries once.
/// Evaluates to `Result<T, ApiFailure>` (`$body` is an expression producing the call's future).
macro_rules! api_call {
    ($session:expr, |$api:ident| $body:expr) => {{
        let session: &$crate::session::Session = $session;
        let mut retried = false;
        loop {
            let outcome = match session.access_token().await {
                Ok(token) => {
                    let $api = $crate::phone_api::PhoneApi::new(&session.http, &session.server, &token)
                        .with_device_key(session.device_key());
                    let result = $body.await;
                    result
                }
                Err(e) => Err($crate::phone_api::ApiFailure::Core(e)),
            };
            if matches!(outcome, Err($crate::phone_api::ApiFailure::Unauthorized)) && !retried {
                retried = true;
                session.invalidate().await;
                continue;
            }
            break outcome;
        }
    }};
}
pub(crate) use api_call;

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn authenticated_subject_survives_restart_without_a_token_refresh() {
        use data_encoding::BASE64URL_NOPAD;
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#);
        let token = format!("h.{payload}.s");
        let (store, session) = session_with(&server, dir.path(), Some((&token, 3600)));
        assert_eq!(session.account_user_id().await.unwrap(), "account-1");
        let restored = Session::new(
            client().unwrap(),
            Arc::clone(&store),
            session.server.clone(),
            session.email(),
            Zeroizing::new("REFRESH-0".to_owned()),
            None,
        );
        assert_eq!(restored.account_user_id().await.unwrap(), "account-1");
        assert!(server.received_requests().await.unwrap().is_empty(), "no network needed for local secret identity");
        assert!(store.meta_get(&account_id_key(&session.server, "other@example.com")).unwrap().is_none());
        let other = ServerUrl::parse("https://other.example.com").unwrap();
        assert!(store.meta_get(&account_id_key(&other, &session.email())).unwrap().is_none());
    }
    use crate::http::client;
    use crate::phone_api::ApiFailure;
    use crate::store::tests::open;

    fn token_response(access: &str, refresh: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({"access_token": access, "refresh_token": refresh, "expires_in": 3600}))
    }

    fn session_with(server: &MockServer, dir: &std::path::Path, access: Option<(&str, i64)>) -> (Arc<Store>, Session) {
        let store = Arc::new(open(dir));
        let url = ServerUrl::parse(&server.uri()).unwrap();
        store
            .save_session(&StoredSession {
                server_url: url.as_str().to_owned(),
                email: "me@example.com".to_owned(),
                refresh_token: Zeroizing::new("REFRESH-0".to_owned()),
            })
            .unwrap();
        let session = Session::new(
            client().unwrap(),
            Arc::clone(&store),
            url,
            "me@example.com".to_owned(),
            Zeroizing::new("REFRESH-0".to_owned()),
            access.map(|(t, s)| (Zeroizing::new(t.to_owned()), s)),
        );
        (store, session)
    }

    #[tokio::test]
    async fn pending_metadata_updates_passwordless_email_without_another_http_request() {
        use data_encoding::BASE64URL_NOPAD;
        let server = MockServer::start().await;
        let payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#);
        let dir = tempfile::tempdir().unwrap();
        let (store, session) = session_with(&server, dir.path(), Some((&format!("h.{payload}.s"), 3600)));
        store.secret_put(crate::sso::SECRET_SERVICE, "account-1", &[7; 32]).unwrap();
        store.secret_put("vault", "me@example.com", b"vault-key").unwrap();
        store.add_account("vault", "me@example.com", 1).unwrap();
        session.adopt_account_email("Updated@Example.com").await.unwrap();
        assert_eq!(session.email(), "updated@example.com");
        assert_eq!(store.load_session().unwrap().unwrap().email, "updated@example.com");
        assert_eq!(store.secret_get("vault", "updated@example.com").unwrap().unwrap(), b"vault-key");
        assert_eq!(session.account_user_id().await.unwrap(), "account-1");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pending_metadata_keeps_password_account_kdf_identity_unchanged() {
        use data_encoding::BASE64URL_NOPAD;
        let server = MockServer::start().await;
        let payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#);
        let dir = tempfile::tempdir().unwrap();
        let (_, session) = session_with(&server, dir.path(), Some((&format!("h.{payload}.s"), 3600)));
        session.adopt_account_email("updated@example.com").await.unwrap();
        assert_eq!(session.email(), "me@example.com");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refreshed_email_updates_live_session_and_reseals_the_vault_alias() {
        use data_encoding::BASE64URL_NOPAD;
        let server = MockServer::start().await;
        let old_payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#);
        let new_payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"updated@example.com"}"#);
        let (old, new) = (format!("h.{old_payload}.s"), format!("h.{new_payload}.s"));
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(token_response(&new, "REFRESH-NEW"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (store, session) = session_with(&server, dir.path(), Some((&old, 3600)));
        store.secret_put(crate::sso::SECRET_SERVICE, "account-1", &[7; 32]).unwrap();
        store.secret_put("vault", "me@example.com", b"vault-key").unwrap();
        store.add_account("vault", "me@example.com", 123).unwrap();
        store.add_account("gmail", "me@example.com", 124).unwrap();
        session.invalidate().await;
        session.access_token().await.unwrap();
        assert_eq!(session.email(), "updated@example.com");
        assert_eq!(store.load_session().unwrap().unwrap().email, "updated@example.com");
        assert_eq!(store.secret_get("vault", "updated@example.com").unwrap().unwrap(), b"vault-key");
        assert!(store.secret_get("vault", "me@example.com").unwrap().is_none());
        let accounts = store.accounts().unwrap();
        assert!(
            accounts.iter().any(|a| a.service == "vault" && a.account == "updated@example.com" && a.added_at == 123)
        );
        assert!(accounts.iter().any(|a| a.service == "gmail" && a.account == "me@example.com"));
        let restarted = Session::new(
            client().unwrap(),
            Arc::clone(&store),
            session.server.clone(),
            session.email(),
            Zeroizing::new("REFRESH-NEW".to_owned()),
            None,
        );
        assert_eq!(restarted.account_user_id().await.unwrap(), "account-1");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "restarted recovery id remains offline");
    }

    #[tokio::test]
    async fn refresh_cannot_switch_the_session_to_another_account() {
        use data_encoding::BASE64URL_NOPAD;
        let server = MockServer::start().await;
        let old_payload = BASE64URL_NOPAD.encode(br#"{"sub":"account-1","email":"me@example.com"}"#);
        let new_payload = BASE64URL_NOPAD.encode(br#"{"sub":"another-account","email":"other@example.com"}"#);
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(token_response(&format!("h.{new_payload}.s"), "REFRESH-NEW"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (store, session) = session_with(&server, dir.path(), Some((&format!("h.{old_payload}.s"), 3600)));
        session.invalidate().await;
        assert_eq!(session.access_token().await.unwrap_err(), CoreError::NotLoggedIn);
        assert!(store.load_session().unwrap().is_none());
        assert_eq!(session.email(), "me@example.com");
    }

    #[tokio::test]
    async fn a_fresh_token_is_reused_and_a_stale_one_refreshes_and_rotates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .and(body_string_contains("refresh_token=REFRESH-0"))
            .respond_with(token_response("ACCESS-1", "REFRESH-1"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (store, session) = session_with(&server, dir.path(), Some(("ACCESS-0", 3600)));
        assert_eq!(session.access_token().await.unwrap().as_str(), "ACCESS-0", "still valid");
        session.invalidate().await;
        assert_eq!(session.access_token().await.unwrap().as_str(), "ACCESS-1");
        assert_eq!(session.access_token().await.unwrap().as_str(), "ACCESS-1", "cached after refresh");
        assert_eq!(store.load_session().unwrap().unwrap().refresh_token.as_str(), "REFRESH-1", "rotation persisted");
    }

    #[tokio::test]
    async fn tokens_close_to_expiry_are_renewed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(token_response("ACCESS-N", "REFRESH-N"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (_, session) = session_with(&server, dir.path(), Some(("ACCESS-0", 30)));
        assert_eq!(session.access_token().await.unwrap().as_str(), "ACCESS-N", "30 s left is inside the 60 s margin");
    }

    #[tokio::test]
    async fn a_rejected_refresh_token_ends_the_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (store, session) = session_with(&server, dir.path(), None);
        assert_eq!(session.access_token().await.unwrap_err(), CoreError::NotLoggedIn);
        assert!(store.load_session().unwrap().is_none(), "stored session cleared");
    }

    #[tokio::test]
    async fn api_call_retries_once_on_401_with_a_new_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(token_response("ACCESS-NEW", "REFRESH-NEW"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/connections"))
            .and(header("authorization", "Bearer ACCESS-NEW"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": []})))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/connections"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (_, session) = session_with(&server, dir.path(), Some(("ACCESS-OLD", 3600)));
        let list = api_call!(&session, |api| api.connections()).unwrap();
        assert!(list.connections.is_empty());
    }

    #[tokio::test]
    async fn a_second_401_is_reported_not_looped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/identity/connect/token"))
            .respond_with(token_response("A2", "R2"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/connections"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (_, session) = session_with(&server, dir.path(), Some(("A1", 3600)));
        let result = api_call!(&session, |api| api.connections());
        assert_eq!(result.unwrap_err(), ApiFailure::Unauthorized);
    }
}
