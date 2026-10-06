//! The public keys of the paired Reins desktop apps, per connection. A key is not secret; what matters is that it
//! is the one the user compared when pairing, so it is pinned then and only ever replaced by another pairing.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::CoreError;

impl Store {
    pub fn pin_desktop_key(&self, connection_id: &str, public_key: &str, now: i64) -> Result<(), CoreError> {
        self.lock()?.execute(
            "INSERT INTO desktop_keys (connection_id, public_key, pinned_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (connection_id) DO UPDATE SET public_key = ?2, pinned_at = ?3",
            params![connection_id, public_key, now],
        )?;
        Ok(())
    }

    pub fn desktop_key(&self, connection_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .lock()?
            .query_row("SELECT public_key FROM desktop_keys WHERE connection_id = ?1", params![connection_id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn remove_desktop_key(&self, connection_id: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM desktop_keys WHERE connection_id = ?1", params![connection_id])?;
        Ok(())
    }

    /// Signing out: the connections may be gone by the next sign-in, and a key must never outlive its pairing.
    pub fn clear_desktop_keys(&self) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM desktop_keys", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::tests::open;

    #[test]
    fn keys_are_pinned_per_connection_replaced_by_a_new_pairing_and_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert_eq!(store.desktop_key("c1").unwrap(), None);
        store.pin_desktop_key("c1", "key-a", 1).unwrap();
        store.pin_desktop_key("c2", "key-b", 1).unwrap();
        store.pin_desktop_key("c1", "key-c", 2).unwrap();
        assert_eq!(store.desktop_key("c1").unwrap().as_deref(), Some("key-c"));
        store.remove_desktop_key("c1").unwrap();
        assert_eq!(store.desktop_key("c1").unwrap(), None);
        assert_eq!(store.desktop_key("c2").unwrap().as_deref(), Some("key-b"));
        store.clear_desktop_keys().unwrap();
        assert_eq!(store.desktop_key("c2").unwrap(), None);
    }
}
