//! End-to-end encrypted account sync. The server holds opaque ciphertext and a monotonic CAS revision.
//!
//! Phones never refuse each other's changes: an upload that loses the compare-and-swap (another phone uploaded
//! first) pulls the server's state, merges it with this phone's (`store/merge.rs`) and tries again; a phone that
//! starts with changes the server never got (it was not the approval device, or the app ended before its upload)
//! merges the same way. Sync problems never stop sign-in or the app: at most they leave a notice for a banner.
use crate::store::Pulled;
use crate::{CoreError, engine::Engine};
use data_encoding::BASE64URL_NOPAD;
use reins_proto::account_state::{AccountState, AccountStateUpdate, MAX_CIPHERTEXT_BYTES};
use std::sync::{Arc, atomic::Ordering};
use zeroize::Zeroizing;

/// Uploads that lose the race to another phone's, each followed by a merge, before giving up until the next write.
const ATTEMPTS: usize = 3;

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

/// How an upload ended.
enum Put {
    /// The server took it as this revision.
    Took(u64),
    /// Another phone uploaded first (409).
    Behind,
}

impl Engine {
    /// Brings in what other phones uploaded since this one last synced (merging this phone's own changes) and uploads
    /// the merge. Called when the account opens; a failure leaves the phone with its own data, never locked out.
    pub(crate) async fn restore_account_state(&self) -> Result<(), CoreError> {
        if self.store.owner.is_none() {
            return Ok(());
        }
        // An account with phone roles syncs by the op log (`live_sync.rs`); the snapshot is retired for it.
        if self.holds_roster_now()? {
            return Box::pin(self.live_sync_now()).await;
        }
        let _guard = self.account_sync.lock().await;
        match self.pull_locked().await? {
            // Not the approval device (the server said so): its merge stays here until it is.
            Pulled::Merged if self.not_approval.load(Ordering::Acquire) => Ok(()),
            Pulled::Merged => self.upload_locked().await,
            Pulled::Current | Pulled::Imported | Pulled::Unusable => Ok(()),
        }
    }
    /// The server's state, merged into this phone's. `Current` when the server cannot be reached.
    async fn pull_locked(&self) -> Result<Pulled, CoreError> {
        let session = self.session()?;
        let token = match session.access_token().await {
            Ok(t) => t,
            Err(CoreError::Network {
                ..
            }) => return Ok(Pulled::Current),
            Err(e) => return Err(e),
        };
        let Ok(response) =
            self.http.get(session.server.join("/reins/api/account-state")).bearer_auth(token.as_str()).send().await
        else {
            return Ok(Pulled::Current);
        };
        self.ensure_active()?;
        if response.status().as_u16() == 404 {
            return Ok(Pulled::Current);
        } // Older self-hosted servers retain local-only storage.
        if !response.status().is_success() {
            return Err(CoreError::Server {
                status: response.status().as_u16(),
                reason: "could not sync encrypted account data".to_owned(),
            });
        }
        let state = read_state(response).await?;
        self.ensure_active()?;
        // Only a newer revision is opened. An older one (lost, or replayed) is refused by `pull`, never uploaded over.
        let plain = match &state.ciphertext {
            Some(ciphertext)
                if state.revision > self.store.sync_revision()? && state.revision >= self.store.sync_floor()? =>
            {
                if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
                    return Err(CoreError::storage("account state is too large"));
                }
                let opened = BASE64URL_NOPAD
                    .decode(ciphertext.as_bytes())
                    .map_err(|_| CoreError::storage("invalid encrypted account state"))
                    .and_then(|sealed| self.store.unseal_account(state.revision, &sealed));
                match opened {
                    Ok(plain) => Some(plain),
                    Err(e) => {
                        // Not this account's key, or not for this revision: never imported, never uploaded over.
                        log::warn!("the server's account state does not open: {e}");
                        self.store.note_sync_unreadable()?;
                        return Ok(Pulled::Unusable);
                    }
                }
            }
            _ => None,
        };
        let pulled = self.store.pull(state.revision, plain.as_deref().map(|p| &p[..]))?;
        self.store.flush()?;
        Ok(pulled)
    }
    /// Uploads the account state in the background, so a call that wrote returns as soon as the change is on disk
    /// instead of after a network round trip. Writes in a row share one upload: at most one waits behind a running one.
    /// Failures are retried by the next call that finishes (the local encrypted copy is already durable).
    pub(crate) fn schedule_account_sync(self: &Arc<Self>) {
        if self.store.owner.is_none() || self.synced_writes.load(Ordering::Acquire) == self.store.write_count() {
            return;
        }
        // The server refuses this phone's uploads (it is not the approval device, and holds no roster): its changes
        // stay here, merged on the next pull, instead of a refused upload after every change.
        if self.not_approval.load(Ordering::Acquire) && !self.holds_roster_now().unwrap_or(false) {
            return;
        }
        if self.sync_queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let engine = Arc::clone(self);
        crate::rt::spawn(async move {
            // Wait out an upload already running; writes from here on queue the next one, as this one carries them.
            drop(engine.account_sync.lock().await);
            engine.sync_queued.store(false, Ordering::Release);
            drop(engine.run_account(engine.sync_account_state()).await);
        });
    }
    pub(crate) async fn sync_account_state(&self) -> Result<(), CoreError> {
        if self.store.owner.is_none() {
            return Ok(());
        }
        // Nothing written since the last sync (most calls only read): skip exporting and hashing everything.
        if self.synced_writes.load(Ordering::Acquire) == self.store.write_count() {
            return Ok(());
        }
        if self.holds_roster_now()? {
            return Box::pin(self.live_sync_now()).await;
        }
        if self.not_approval.load(Ordering::Acquire) {
            return Ok(());
        }
        let _guard = self.account_sync.lock().await;
        self.upload_locked().await
    }
    /// Uploads what changed; when another phone uploaded first, pulls and merges its state and tries again.
    async fn upload_locked(&self) -> Result<(), CoreError> {
        for _ in 0..ATTEMPTS {
            self.ensure_active()?;
            let writes = self.store.write_count();
            let Some((revision, plain)) = self.store.pending_upload()? else {
                self.synced_writes.store(writes, Ordering::Release);
                return Ok(());
            };
            match self.put_locked(revision, &plain).await? {
                Put::Took(revision) => {
                    self.store.mark_uploaded(revision, &plain)?;
                    return self.store.flush();
                }
                Put::Behind => {
                    if self.pull_locked().await? == Pulled::Unusable {
                        // Nothing this phone can merge with: uploading over it would lose the other phone's state.
                        return Ok(());
                    }
                }
            }
        }
        Err(CoreError::storage("account data changed on another phone during the upload; it is retried later"))
    }
    async fn put_locked(&self, revision: u64, plain: &[u8]) -> Result<Put, CoreError> {
        let next_revision =
            revision.checked_add(1).ok_or_else(|| CoreError::storage("account state revision exhausted"))?;
        let sealed = self.store.seal_account(next_revision, plain)?;
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
        if response.status().as_u16() == 409 {
            return Ok(Put::Behind);
        }
        if !response.status().is_success() {
            // 403: this phone is not the approval device. Its changes stay here, merged on every pull, until it is; no
            // more uploads are tried until then (`not_approval`).
            if response.status().as_u16() == 403 {
                self.not_approval.store(true, Ordering::Release);
            }
            return Err(CoreError::Server {
                status: response.status().as_u16(),
                reason: "could not sync encrypted account data; the local encrypted copy was kept".to_owned(),
            });
        }
        let state = read_state(response).await?;
        if state.revision != next_revision {
            return Err(CoreError::storage("invalid account state revision"));
        }
        Ok(Put::Took(state.revision))
    }
    /// A sync problem worth a banner (`None`: nothing to show).
    pub fn account_sync_notice(&self) -> Result<Option<String>, CoreError> {
        if self.store.owner.is_none() {
            return Ok(None);
        }
        self.store.sync_notice()
    }
    /// Brings back this phone's own changes that a sync without a merge base set aside (see the notice), merged with
    /// the data as it is now by the usual rules. False when there is nothing to bring back.
    pub fn restore_account_sync_backup(&self) -> Result<bool, CoreError> {
        if self.store.owner.is_none() {
            return Ok(false);
        }
        let restored = self.store.restore_sync_backup()?;
        if restored {
            self.store.clear_sync_notice()?;
        }
        Ok(restored)
    }
    pub fn dismiss_account_sync_notice(&self) -> Result<(), CoreError> {
        if self.store.owner.is_none() {
            return Ok(());
        }
        self.store.clear_sync_notice()
    }
}
