//! End-to-end encrypted account sync. The server holds opaque ciphertext and a monotonic CAS revision.
use crate::{CoreError, engine::Engine};
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use reins_proto::account_state::{AccountState, AccountStateUpdate, MAX_CIPHERTEXT_BYTES};
use ring::digest;
use zeroize::Zeroizing;
const REVISION: &str = "account-state.revision";
const SYNCED: &str = "account-state.synced";
fn hash(bytes: &[u8]) -> String {
    HEXLOWER.encode(digest::digest(&digest::SHA256, bytes).as_ref())
}
async fn read_state(mut response: reqwest::Response) -> Result<AccountState, CoreError> {
    let limit = MAX_CIPHERTEXT_BYTES + 1024;
    if response.content_length().is_some_and(|size| size > limit as u64) {
        return Err(CoreError::storage("account state is too large"));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(CoreError::storage("account state is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| CoreError::storage("invalid account state response"))
}
impl Engine {
    pub(crate) async fn restore_account_state(&self) -> Result<(), CoreError> {
        if self.store.owner.is_none() {
            return Ok(());
        }
        let _guard = self.account_sync.lock().await;
        self.restore_state_locked().await
    }
    async fn restore_state_locked(&self) -> Result<(), CoreError> {
        let session = self.session()?;
        let token = match session.access_token().await {
            Ok(t) => t,
            Err(CoreError::Network {
                ..
            }) => return Ok(()),
            Err(e) => return Err(e),
        };
        let Ok(response) =
            self.http.get(session.server.join("/reins/api/account-state")).bearer_auth(token.as_str()).send().await
        else {
            return Ok(());
        };
        self.ensure_active()?;
        if response.status().as_u16() == 404 {
            return Ok(());
        } // Older self-hosted servers retain local-only storage.
        if !response.status().is_success() {
            return Err(CoreError::Server {
                status: response.status().as_u16(),
                reason: "could not sync encrypted account data".to_owned(),
            });
        }
        let state = read_state(response).await?;
        self.ensure_active()?;
        let local_revision = self.store.meta_get(REVISION)?.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        if state.revision < local_revision {
            return Err(CoreError::storage("the server returned an older account state"));
        }
        if state.revision == local_revision {
            return Ok(());
        }
        let local = self.store.export_account()?;
        let previously_synced = self.store.meta_get(SYNCED)?;
        if (local_revision > 0 && previously_synced.as_deref() != Some(&hash(&local)))
            || (local_revision == 0 && previously_synced.is_none() && crate::store::Store::has_account_rows(&local)?)
        {
            return Err(CoreError::needs_attention(
                "This account has unsynced changes on this phone and newer changes on another phone. Its local data was preserved.",
            ));
        }
        if let Some(ciphertext) = state.ciphertext {
            if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
                return Err(CoreError::storage("account state is too large"));
            }
            let sealed = BASE64URL_NOPAD
                .decode(ciphertext.as_bytes())
                .map_err(|_| CoreError::storage("invalid encrypted account state"))?;
            let plain = self.store.unseal_account(state.revision, &sealed)?;
            self.store.import_account(&plain)?;
            self.store.meta_set(SYNCED, &hash(&self.store.export_account()?))?;
        }
        self.store.meta_set(REVISION, &state.revision.to_string())?;
        self.store.flush()
    }
    pub(crate) async fn sync_account_state(&self) -> Result<(), CoreError> {
        if self.store.owner.is_none() {
            return Ok(());
        }
        let _guard = self.account_sync.lock().await;
        self.ensure_active()?;
        let plain = self.store.export_account()?;
        let fingerprint = hash(&plain);
        if self.store.meta_get(SYNCED)?.as_deref() == Some(&fingerprint) {
            return Ok(());
        }
        let revision = self.store.meta_get(REVISION)?.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let next_revision =
            revision.checked_add(1).ok_or_else(|| CoreError::storage("account state revision exhausted"))?;
        let sealed = self.store.seal_account(next_revision, &plain)?;
        let ciphertext = BASE64URL_NOPAD.encode(&sealed);
        if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
            return Err(CoreError::storage("account state is too large to sync"));
        }
        let session = self.session()?;
        let token = session.access_token().await?;
        let mut request = self.http.put(session.server.join("/reins/api/account-state")).bearer_auth(token.as_str());
        if let Some(key) = session.device_key() {
            request = request.header(reins_proto::device::DEVICE_KEY_HEADER, key);
        }
        let response = request
            .json(&AccountStateUpdate {
                revision,
                ciphertext,
            })
            .send()
            .await?;
        self.ensure_active()?;
        if !response.status().is_success() {
            return Err(CoreError::Server {
                status: response.status().as_u16(),
                reason: "could not sync encrypted account data; the local encrypted copy was kept".to_owned(),
            });
        }
        let state = read_state(response).await?;
        if state.revision != next_revision {
            return Err(CoreError::storage("invalid account state revision"));
        }
        self.store.meta_set(REVISION, &state.revision.to_string())?;
        self.store.meta_set(SYNCED, &fingerprint)?;
        self.store.flush()
    }
}
