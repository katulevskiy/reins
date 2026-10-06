//! The user's own Vaultwarden vault, the one Reins is signed in to. What an AI can do with it is decided by the
//! phone: names, usernames and addresses are listed; every secret (a password, a one-time code, a note, a card number,
//! a private key) is handed over one field at a time and asked for each time; changes (new items, edits, the trash,
//! folders, Sends) are previewed and approved. The vault key is kept sealed on the phone after the master password was
//! entered once; the master password itself is not kept.
//!
//! The areas: `items` (items, folders, trash, archive), `sends` (Sends and the generator), `totp` (one-time codes).

#![allow(dead_code, reason = "helpers shared by the tool areas, used as the areas grow")]

use std::sync::{Arc, Mutex, PoisonError};

use reins_proto::connector::{ConnectorCall, VAULT};
use reqwest::Method;
use serde_json::Value;
use zeroize::Zeroizing;

use super::{Connector, Item, Preview};
use crate::crypto::{self, VaultKey};
use crate::session::Session;
use crate::store::Store;
use crate::types::GmailStatus;
use crate::vault::VaultClient;
use crate::{CoreError, text};

mod items;
mod sends;
mod totp;

pub use items::desktop::{secret_release_view, ssh_sign_view};
pub use totp::totp_code;

/// The signed-in Reins session, shared with the engine.
pub type SessionSlot = Arc<Mutex<Option<Arc<Session>>>>;

/// The resource of things that are not in a folder: `RESOURCE` alone, and every item's path starts with it.
const RESOURCE: &str = "vault";

pub struct Vault {
    slot: SessionSlot,
    store: Arc<Store>,
}

/// What an operation works on: the session, the vault key and the vault as the server sent it (`/api/sync`).
pub(super) struct Loaded {
    pub session: Arc<Session>,
    pub key: VaultKey,
    /// `profile`, `folders`, `ciphers`, `sends`, `collections`, ... exactly as the server sent them.
    pub sync: Value,
}

impl Vault {
    pub fn new(slot: SessionSlot, store: Arc<Store>) -> Self {
        Self {
            slot,
            store,
        }
    }

    pub(super) fn session(&self) -> Result<Arc<Session>, CoreError> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or(CoreError::NotLoggedIn)
    }

    /// The key that decrypts this vault, from what was sealed when the master password was entered.
    pub(super) fn key(&self, account: &str) -> Result<VaultKey, CoreError> {
        let raw = self
            .store
            .secret_get(VAULT, account)?
            .ok_or_else(|| CoreError::needs_attention("Enter the master password again to unlock the vault"))?;
        VaultKey::from_bytes(&raw)
    }

    /// The session, the key and the vault, freshly read.
    pub(super) async fn load(&self, account: &str) -> Result<Loaded, CoreError> {
        let session = self.session()?;
        let key = self.key(account)?;
        let sync = self.api(&session, Method::GET, "/api/sync?excludeDomains=true", None).await?;
        Ok(Loaded {
            session,
            key,
            sync,
        })
    }

    /// The vault as typed structures (logins), for the parts that read only that.
    fn typed(sync: &Value) -> Result<items::Sync, CoreError> {
        serde_json::from_value(sync.clone()).map_err(|_| CoreError::Network {
            reason: "invalid vault response".to_owned(),
        })
    }

    /// A call to the server this phone is signed in to, as the signed-in user; a 401 gets one fresh token. `path`
    /// starts with `/api/`. The answer is the JSON body (`Null` when empty). Errors carry the server's own message
    /// (short, one line): "Send not found", "The field Name is required".
    pub(super) async fn api(
        &self,
        session: &Session,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, CoreError> {
        let mut retried = false;
        loop {
            let token = session.access_token().await?;
            let mut req = session.http.request(method.clone(), session.server.join(path)).bearer_auth(token.as_str());
            if let Some(body) = body {
                req = req.json(body);
            }
            let response = req.send().await?;
            if !session.is_active() {
                return Err(CoreError::NotLoggedIn);
            }
            let status = response.status();
            if status.as_u16() == 401 && !retried {
                retried = true;
                session.invalidate().await;
                continue;
            }
            let bytes = Zeroizing::new(response.bytes().await?.to_vec());
            if !status.is_success() {
                let message = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["message"].as_str().or_else(|| v["Message"].as_str()).map(str::to_owned))
                    .map(|m| text::truncate_chars(&text::one_line(&m), 200))
                    .filter(|m| !m.is_empty());
                return Err(match (status.as_u16(), message) {
                    (401, _) => CoreError::NotLoggedIn,
                    (400 | 404 | 409 | 422, Some(m)) => CoreError::service(format!("The vault refused: {m}")),
                    (404, None) => CoreError::service("The vault does not have that."),
                    (code, _) => CoreError::Server {
                        status: code,
                        reason: "the vault could not do that".to_owned(),
                    },
                });
            }
            return Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null));
        }
    }

    /// Like [`Vault::api`] with a multipart form (a file for a Send or an attachment).
    pub(super) async fn api_form(
        &self,
        session: &Session,
        path: &str,
        form: impl Fn() -> reqwest::multipart::Form,
    ) -> Result<Value, CoreError> {
        let mut retried = false;
        loop {
            let token = session.access_token().await?;
            let response = session
                .http
                .post(session.server.join(path))
                .bearer_auth(token.as_str())
                .multipart(form())
                .send()
                .await?;
            if !session.is_active() {
                return Err(CoreError::NotLoggedIn);
            }
            let status = response.status();
            if status.as_u16() == 401 && !retried {
                retried = true;
                session.invalidate().await;
                continue;
            }
            let bytes = response.bytes().await?;
            if !status.is_success() {
                let message = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v["message"].as_str().map(|m| text::truncate_chars(&text::one_line(m), 200)));
                return Err(CoreError::service(format!(
                    "The vault refused the file{}",
                    message.map(|m| format!(": {m}")).unwrap_or_default()
                )));
            }
            return Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null));
        }
    }

    /// Unlocks the vault with the master password and keeps the vault key, sealed. Returns the account (the email
    /// this phone is signed in with).
    pub async fn unlock(&self, master_password: &str) -> Result<String, CoreError> {
        let session = self.session()?;
        let email = session.email();
        let kdf = VaultClient::new(&session.http, &session.server).prelogin(&email).await?;
        let (password, salt) = (Zeroizing::new(master_password.to_owned()), email.clone());
        let stretched =
            tokio::task::spawn_blocking(move || crypto::master_key(&password, &salt, kdf).map(|k| k.stretch()))
                .await
                .map_err(|_| CoreError::storage("key derivation was interrupted"))??;
        let sync = self.api(&session, Method::GET, "/api/sync?excludeDomains=true", None).await?;
        let wrapped = sync["profile"]["key"]
            .as_str()
            .ok_or_else(|| CoreError::invalid("this account has no vault key"))?
            .to_owned();
        let user_key =
            stretched.decrypt(&wrapped).map_err(|_| CoreError::invalid("That is not the master password."))?;
        let key = VaultKey::from_bytes(&user_key)?;
        self.store.secret_put(VAULT, &email, &key.to_bytes())?;
        Ok(email)
    }
}

#[async_trait::async_trait]
impl Connector for Vault {
    fn service(&self) -> &'static str {
        VAULT
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if let Some(result) = items::desktop::fetch(self, account, call).await {
            return result;
        }
        if let Some(result) = items::fetch(self, account, call).await {
            return result;
        }
        if let Some(result) = sends::fetch(self, account, call).await {
            return result;
        }
        Err(CoreError::service(format!("the vault cannot {}", call.op)))
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if let Some(result) = items::desktop::preview(self, account, call).await {
            return result;
        }
        if let Some(result) = items::preview(self, account, call).await {
            return result;
        }
        if let Some(result) = sends::preview(self, account, call).await {
            return result;
        }
        Err(CoreError::service(format!("the vault cannot {}", call.op)))
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        if let Some(result) = items::desktop::perform(self, account, call).await {
            return result;
        }
        if let Some(result) = items::perform(self, account, call).await {
            return result;
        }
        if let Some(result) = sends::perform(self, account, call).await {
            return result;
        }
        Err(CoreError::service(format!("the vault cannot {}", call.op)))
    }

    async fn status(&self, account: &str) -> GmailStatus {
        let signed_in = self.session().is_ok_and(|s| s.email() == account);
        if signed_in && self.store.secret_get(VAULT, account).ok().flatten().is_some() {
            GmailStatus::Ready
        } else {
            GmailStatus::NeedsConsent
        }
    }

    /// For the vault the "token" is the master password.
    async fn sign_in_token(&self, master_password: &str) -> Result<String, CoreError> {
        self.unlock(master_password).await
    }

    async fn forget(&self, account: &str) {
        self.store.secret_delete(VAULT, account).ok();
    }
}
