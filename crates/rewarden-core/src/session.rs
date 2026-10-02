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
    pub email: String,
    pub http: reqwest::Client,
    store: Arc<Store>,
    state: Mutex<TokenState>,
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
        let access =
            access.map(|(token, secs)| (token, Instant::now() + Duration::from_secs(u64::try_from(secs).unwrap_or(0))));
        Self {
            server,
            email,
            http,
            store,
            state: Mutex::new(TokenState {
                access,
                refresh,
            }),
        }
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
        // Vaultwarden rotates refresh tokens: persist the new one before using the new access token.
        let saved = StoredSession {
            server_url: self.server.as_str().to_owned(),
            email: self.email.clone(),
            refresh_token: tokens.refresh_token,
        };
        if let Err(e) = self.store.save_session(&saved) {
            log::warn!("could not persist the rotated refresh token: {e}");
        }
        let expires = Instant::now() + Duration::from_secs(u64::try_from(tokens.expires_in).unwrap_or(0));
        state.access = Some((tokens.access_token.clone(), expires));
        Ok(tokens.access_token)
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
                    let $api = $crate::phone_api::PhoneApi::new(&session.http, &session.server, &token);
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
            .and(path("/rewarden/api/connections"))
            .and(header("authorization", "Bearer ACCESS-NEW"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": []})))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rewarden/api/connections"))
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
            .and(path("/rewarden/api/connections"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (_, session) = session_with(&server, dir.path(), Some(("A1", 3600)));
        let result = api_call!(&session, |api| api.connections());
        assert_eq!(result.unwrap_err(), ApiFailure::Unauthorized);
    }
}
