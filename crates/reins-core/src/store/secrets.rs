//! Secrets of the integrations (a Telegram session, a GitHub token), sealed with the data key.

use data_encoding::BASE64URL_NOPAD;
use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

use super::Store;
use crate::CoreError;

fn aad(service: &str, account: &str) -> String {
    format!("secret.{service}.{account}")
}

impl Store {
    pub fn secret_get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, CoreError> {
        let blob: Option<Vec<u8>> = self
            .lock()?
            .query_row(
                "SELECT value FROM secrets WHERE service = ?1 AND account = ?2",
                params![service, account],
                |r| r.get(0),
            )
            .optional()?;
        blob.map(|b| self.unseal(&aad(service, account), &b)).transpose()
    }

    pub fn secret_put(&self, service: &str, account: &str, value: &[u8]) -> Result<(), CoreError> {
        let sealed = self.seal(&aad(service, account), value)?;
        self.lock()?.execute(
            "INSERT INTO secrets (service, account, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT (service, account) DO UPDATE SET value = ?3",
            params![service, account, sealed],
        )?;
        Ok(())
    }

    pub fn secret_delete(&self, service: &str, account: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM secrets WHERE service = ?1 AND account = ?2", params![service, account])?;
        Ok(())
    }

    /// This phone's device key ([`reins_proto::device::DEVICE_KEY_HEADER`]), base64url: 32 random bytes made the
    /// first time and kept sealed. Lost with the data key, it is made again, and the server then wants a proof from
    /// this phone before it approves again, as from any other.
    pub fn device_key(&self) -> Result<String, CoreError> {
        if let Some(control) = &self.control {
            return control.device_key();
        }
        let (service, account) = DEVICE_KEY;
        match self.secret_get(service, account)? {
            Some(raw) if raw.len() == 32 => return Ok(BASE64URL_NOPAD.encode(&raw)),
            Some(_) => self.secret_delete(service, account)?,
            None => {}
        }
        let fresh = Zeroizing::new(crate::crypto::random_bytes::<32>()?);
        let sealed = self.seal(&aad(service, account), &fresh[..])?;
        // Of two first calls at once, the first key stays.
        self.lock()?.execute(
            "INSERT INTO secrets (service, account, value) VALUES (?1, ?2, ?3) ON CONFLICT (service, account) DO NOTHING",
            params![service, account, sealed],
        )?;
        let raw =
            self.secret_get(service, account)?.ok_or_else(|| CoreError::storage("the device key was not kept"))?;
        Ok(BASE64URL_NOPAD.encode(&raw))
    }

    /// This phone's inbox key: the X25519 secret that opens what the desktop app seals to the phone (a secret to save
    /// with `reins vault add`). Made the first time, kept sealed, and never copied to another phone of the account: a
    /// new phone has a new key, which the computer then asks the user to check.
    pub fn inbox_key(&self) -> Result<Zeroizing<[u8; 32]>, CoreError> {
        let (service, account) = INBOX_KEY;
        if let Some(raw) = self.secret_get(service, account)?
            && let Ok(key) = <[u8; 32]>::try_from(raw.as_slice())
        {
            return Ok(Zeroizing::new(key));
        }
        self.secret_delete(service, account)?;
        let fresh = Zeroizing::new(crate::crypto::random_bytes::<32>()?);
        let sealed = self.seal(&aad(service, account), &fresh[..])?;
        // Of two first calls at once, the first key stays.
        self.lock()?.execute(
            "INSERT INTO secrets (service, account, value) VALUES (?1, ?2, ?3) ON CONFLICT (service, account) DO NOTHING",
            params![service, account, sealed],
        )?;
        let raw = Zeroizing::new(
            self.secret_get(service, account)?.ok_or_else(|| CoreError::storage("the inbox key was not kept"))?,
        );
        <[u8; 32]>::try_from(raw.as_slice())
            .map(Zeroizing::new)
            .map_err(|_| CoreError::storage("the inbox key was not kept"))
    }
}

/// Where the device key is kept (service, account).
const DEVICE_KEY: (&str, &str) = ("reins.device-key", "this");
/// Where the inbox key is kept (service, account).
pub(crate) const INBOX_KEY: (&str, &str) = ("reins.inbox-key", "this");

#[cfg(test)]
mod tests {
    use crate::store::tests::open;

    #[test]
    fn secrets_are_sealed_per_service_and_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.secret_put("github", "octocat", b"ghp_token").unwrap();
        assert_eq!(store.secret_get("github", "octocat").unwrap().as_deref(), Some(b"ghp_token".as_slice()));
        assert_eq!(store.secret_get("github", "other").unwrap(), None);
        let raw: Vec<u8> = store.lock().unwrap().query_row("SELECT value FROM secrets", [], |r| r.get(0)).unwrap();
        assert!(!raw.windows(9).any(|w| w == b"ghp_token"), "the token is not stored in the clear");
        // A blob copied to another account does not open there.
        store.lock().unwrap().execute("UPDATE secrets SET account = 'thief'", []).unwrap();
        assert!(store.secret_get("github", "thief").is_err());
        store.secret_delete("github", "thief").unwrap();
        assert_eq!(store.secret_get("github", "thief").unwrap(), None);
    }

    #[test]
    fn the_device_key_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let key = open(dir.path()).device_key().unwrap();
        assert!(reins_proto::device::device_key_hash(&key).is_some(), "32 bytes, base64url");
        assert_eq!(open(dir.path()).device_key().unwrap(), key, "the same after a restart");
        let other = tempfile::tempdir().unwrap();
        assert_ne!(open(other.path()).device_key().unwrap(), key);
    }

    #[test]
    fn the_inbox_key_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let key = open(dir.path()).inbox_key().unwrap();
        assert_eq!(*open(dir.path()).inbox_key().unwrap(), *key, "the same after a restart");
        let other = tempfile::tempdir().unwrap();
        assert_ne!(*open(other.path()).inbox_key().unwrap(), *key);
    }
}
