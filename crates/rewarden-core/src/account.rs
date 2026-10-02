//! The engine's side of passwordless sign-in and the keyless vault (see [`crate::sso`]): finishing the browser
//! sign-in, making or opening the account's keys, and the recovery code.

use std::sync::Arc;

use rewarden_proto::connector::VAULT;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::CoreError;
use crate::crypto::{self, VaultKey};
use crate::engine::Engine;
use crate::http::ServerUrl;
use crate::session::Session;
use crate::sso::{self, AccountSecret, SECRET_SERVICE};
use crate::store::StoredSession;
use crate::types::{AccountKeys, SessionInfo, SsoOutcome, SsoStart};
use crate::vault::VaultClient;

fn interrupted(_: tokio::task::JoinError) -> CoreError {
    CoreError::storage("key derivation was interrupted")
}

impl Engine {
    /// Starts a sign-in through the server's SSO: the URL to open in the browser session, and the state and verifier
    /// to hand back to [`Engine::sso_finish`].
    pub fn sso_begin(&self, server_url: &str) -> Result<SsoStart, CoreError> {
        let server = ServerUrl::parse(server_url)?;
        let start = sso::begin(&server)?;
        Ok(SsoStart {
            url: start.url,
            callback_scheme: sso::CALLBACK_SCHEME.to_owned(),
            state: start.state,
            verifier: start.verifier.to_string(),
        })
    }

    /// Finishes the sign-in with the URL the browser came back with, keeps the session like `login`, and then makes
    /// the account's keys (a new account), opens them with the secret this phone keeps, or reports that they are
    /// locked (another phone, or the recovery code, has the secret).
    pub async fn sso_finish(
        &self,
        server_url: &str,
        callback_url: &str,
        state: &str,
        verifier: Zeroizing<String>,
    ) -> Result<SsoOutcome, CoreError> {
        let server = ServerUrl::parse(server_url)?;
        let code = sso::callback_code(callback_url, state)?;
        let device_id = self.store.device_id()?;
        let signed_in = sso::exchange(&self.http, &server, &code, &verifier, &device_id).await?;
        let access = signed_in.tokens.access_token.clone();
        let email = signed_in.email.clone();
        self.store.save_session(&StoredSession {
            server_url: server.as_str().to_owned(),
            email: email.clone(),
            refresh_token: signed_in.tokens.refresh_token.clone(),
        })?;
        let session = Session::from_login(
            self.http.clone(),
            Arc::clone(&self.store),
            server.clone(),
            email.clone(),
            signed_in.tokens,
        );
        self.set_session(Some(Arc::new(session)));
        let info = SessionInfo {
            server_url: server.as_str().to_owned(),
            email: email.clone(),
        };

        let keys = match signed_in.key {
            None => {
                self.create_keys(&server, &access, &signed_in.user_id, &email).await?;
                AccountKeys::Created
            }
            Some(wrapped) => match self.secret_of(&signed_in.user_id)? {
                Some(secret) => {
                    let opened = tokio::task::spawn_blocking(move || secret.open_user_key(&wrapped))
                        .await
                        .map_err(interrupted)?;
                    match opened {
                        Ok(key) => {
                            self.keep_vault_key(&email, &key)?;
                            AccountKeys::Unlocked
                        }
                        // A secret from an earlier account of the same id would be odd; it does not open this one.
                        Err(_) => AccountKeys::Locked,
                    }
                }
                None if self.store.secret_get(VAULT, &email)?.is_some() => AccountKeys::Unlocked,
                None => AccountKeys::Locked,
            },
        };
        Ok(SsoOutcome {
            session: info,
            keys,
        })
    }

    /// A new account's keys, protected by a fresh account secret that this phone keeps.
    async fn create_keys(&self, server: &ServerUrl, access: &str, user_id: &str, email: &str) -> Result<(), CoreError> {
        let secret = AccountSecret::generate()?;
        let raw = Zeroizing::new(secret.as_bytes().to_vec());
        let keys = tokio::task::spawn_blocking(move || sso::new_keys(&secret)).await.map_err(interrupted)??;
        // Kept before the server has the keys: a crash in between leaves a secret for keys that do not exist yet,
        // never keys whose secret is lost.
        self.store.secret_put(SECRET_SERVICE, user_id, &raw)?;
        sso::set_keys(&self.http, server, access, &keys).await?;
        self.keep_vault_key(email, &keys.user_key)?;
        Ok(())
    }

    /// Keeps the vault key sealed, as unlocking the vault with a master password does, and lists the vault as a
    /// connected integration: an account without a master password never asks for one.
    fn keep_vault_key(&self, email: &str, key: &VaultKey) -> Result<(), CoreError> {
        self.store.secret_put(VAULT, email, &key.to_bytes())?;
        self.register_account(VAULT, email).map(drop)
    }

    fn secret_of(&self, user_id: &str) -> Result<Option<AccountSecret>, CoreError> {
        self.store.secret_get(SECRET_SERVICE, user_id)?.map(|raw| AccountSecret::from_bytes(&raw)).transpose()
    }

    /// The signed-in account's server id and its wrapped user key (`profile.key`, `None` before it has keys).
    async fn account_profile(&self, session: &Session) -> Result<(String, Option<String>), CoreError> {
        let token = session.access_token().await?;
        let resp =
            session.http.get(session.server.join("/api/accounts/profile")).bearer_auth(token.as_str()).send().await?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err(CoreError::NotLoggedIn);
        }
        if !status.is_success() {
            return Err(CoreError::Server {
                status: status.as_u16(),
                reason: "could not read the account".to_owned(),
            });
        }
        let profile: Value = resp.json().await?;
        let user_id = profile["id"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| CoreError::Network {
            reason: "invalid account response".to_owned(),
        })?;
        let key = profile["key"].as_str().filter(|k| !k.is_empty());
        Ok((user_id.to_owned(), key.map(str::to_owned)))
    }

    /// Whether this phone can open the signed-in account's vault.
    pub async fn account_keys(&self) -> Result<AccountKeys, CoreError> {
        let session = self.session()?;
        if self.store.secret_get(VAULT, &session.email)?.is_some() {
            return Ok(AccountKeys::Unlocked);
        }
        let (user_id, key) = self.account_profile(&session).await?;
        let (Some(wrapped), Some(secret)) = (key, self.secret_of(&user_id)?) else {
            return Ok(AccountKeys::Locked);
        };
        match tokio::task::spawn_blocking(move || secret.open_user_key(&wrapped)).await.map_err(interrupted)? {
            Ok(key) => {
                self.keep_vault_key(&session.email, &key)?;
                Ok(AccountKeys::Unlocked)
            }
            Err(_) => Ok(AccountKeys::Locked),
        }
    }

    /// Opens the account's keys with its recovery code (kept from then on, like on the phone that made it), or with
    /// the master password of an account that has one.
    pub async fn unlock_account(&self, code_or_password: Zeroizing<String>) -> Result<(), CoreError> {
        let session = self.session()?;
        let (user_id, wrapped) = self.account_profile(&session).await?;
        let wrapped = wrapped.ok_or_else(|| CoreError::invalid("This account has no keys yet. Sign in again."))?;
        let email = session.email.clone();
        if let Some(secret) = AccountSecret::from_recovery_code(&code_or_password) {
            let raw = Zeroizing::new(secret.as_bytes().to_vec());
            let key =
                tokio::task::spawn_blocking(move || secret.open_user_key(&wrapped)).await.map_err(interrupted)??;
            self.store.secret_put(SECRET_SERVICE, &user_id, &raw)?;
            self.keep_vault_key(&email, &key)?;
            return Ok(());
        }
        let kdf = VaultClient::new(&session.http, &session.server).prelogin(&email).await?;
        let salt = email.clone();
        let key = tokio::task::spawn_blocking(move || -> Result<VaultKey, CoreError> {
            let stretched = crypto::master_key(&code_or_password, &salt, kdf)?.stretch();
            let raw = stretched
                .decrypt(&wrapped)
                .map_err(|_| CoreError::invalid("That is neither the recovery code nor the master password."))?;
            VaultKey::from_bytes(&raw)
        })
        .await
        .map_err(interrupted)??;
        self.keep_vault_key(&email, &key)?;
        Ok(())
    }

    /// The signed-in account's recovery code, when this phone keeps its secret (the app asks for biometrics first).
    pub async fn account_recovery_code(&self) -> Result<String, CoreError> {
        let session = self.session()?;
        let token = session.access_token().await?;
        let (user_id, _) = sso::access_claims(&token)?;
        let secret = self.secret_of(&user_id)?.ok_or_else(|| {
            CoreError::invalid("This account has no recovery code: it was made with a master password.")
        })?;
        Ok(secret.recovery_code().to_string())
    }

    /// The account secret this phone keeps for the signed-in account, for sealing to a new phone (`None`: an account
    /// with a master password).
    pub(crate) async fn signed_in_secret(&self) -> Result<Option<AccountSecret>, CoreError> {
        let session = self.session()?;
        let token = session.access_token().await?;
        let (user_id, _) = sso::access_claims(&token)?;
        self.secret_of(&user_id)
    }

    /// Keeps a secret another phone sealed to this one, after checking that it opens the account.
    pub(crate) async fn adopt_secret(&self, secret: AccountSecret) -> Result<(), CoreError> {
        let session = self.session()?;
        let (user_id, wrapped) = self.account_profile(&session).await?;
        let wrapped = wrapped.ok_or_else(|| CoreError::invalid("This account has no keys yet."))?;
        let raw = Zeroizing::new(secret.as_bytes().to_vec());
        let key = tokio::task::spawn_blocking(move || secret.open_user_key(&wrapped)).await.map_err(interrupted)??;
        self.store.secret_put(SECRET_SERVICE, &user_id, &raw)?;
        self.keep_vault_key(&session.email, &key)?;
        Ok(())
    }
}
