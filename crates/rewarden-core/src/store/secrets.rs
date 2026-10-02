//! Secrets of the integrations (a Telegram session, a GitHub token), sealed with the data key.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::CoreError;

fn aad(service: &str, account: &str) -> String {
    format!("secret.{service}.{account}")
}

impl Store {
    pub fn secret_get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, CoreError> {
        let blob: Option<Vec<u8>> = self
            .lock()
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
        self.lock().execute(
            "INSERT INTO secrets (service, account, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT (service, account) DO UPDATE SET value = ?3",
            params![service, account, sealed],
        )?;
        Ok(())
    }

    pub fn secret_delete(&self, service: &str, account: &str) -> Result<(), CoreError> {
        self.lock().execute("DELETE FROM secrets WHERE service = ?1 AND account = ?2", params![service, account])?;
        Ok(())
    }
}

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
        let raw: Vec<u8> = store.lock().query_row("SELECT value FROM secrets", [], |r| r.get(0)).unwrap();
        assert!(!raw.windows(9).any(|w| w == b"ghp_token"), "the token is not stored in the clear");
        // A blob copied to another account does not open there.
        store.lock().execute("UPDATE secrets SET account = 'thief'", []).unwrap();
        assert!(store.secret_get("github", "thief").is_err());
        store.secret_delete("github", "thief").unwrap();
        assert_eq!(store.secret_get("github", "thief").unwrap(), None);
    }
}
