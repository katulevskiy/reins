//! The public keys of the paired Reins desktop apps, per connection. A key is not secret; what matters is that it
//! is the one the user compared when pairing, so it is pinned then and only ever replaced by another pairing.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::CoreError;

/// Where the name a desktop app was paired with is kept (`meta`, so that it travels with the account like its key).
const LABEL_KEY: &str = "desktop.label.";

impl Store {
    /// Pins a desktop app's key to its connection, with the name the user saw when pairing it.
    pub fn pin_desktop_key(
        &self,
        connection_id: &str,
        public_key: &str,
        label: &str,
        now: i64,
    ) -> Result<(), CoreError> {
        self.lock()?.execute(
            "INSERT INTO desktop_keys (connection_id, public_key, pinned_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (connection_id) DO UPDATE SET public_key = ?2, pinned_at = ?3",
            params![connection_id, public_key, now],
        )?;
        self.meta_set(&format!("{LABEL_KEY}{connection_id}"), label)
    }

    pub fn desktop_key(&self, connection_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .lock()?
            .query_row("SELECT public_key FROM desktop_keys WHERE connection_id = ?1", params![connection_id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// The name a desktop app was paired with and when, as this phone kept them: the server's name for the
    /// connection is the server's to say.
    pub fn desktop_pairing(&self, connection_id: &str) -> Result<Option<(String, i64)>, CoreError> {
        let pinned_at: Option<i64> = self
            .lock()?
            .query_row("SELECT pinned_at FROM desktop_keys WHERE connection_id = ?1", params![connection_id], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(at) = pinned_at else {
            return Ok(None);
        };
        Ok(Some((self.meta_get(&format!("{LABEL_KEY}{connection_id}"))?.unwrap_or_default(), at)))
    }

    /// The connections of every paired desktop app.
    pub fn desktop_connections(&self) -> Result<Vec<String>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT connection_id FROM desktop_keys ORDER BY pinned_at")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn remove_desktop_key(&self, connection_id: &str) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM desktop_keys WHERE connection_id = ?1", params![connection_id])?;
        conn.execute("DELETE FROM meta WHERE key = ?1", params![format!("app.{LABEL_KEY}{connection_id}")])?;
        Ok(())
    }

    /// Signing out: the connections may be gone by the next sign-in, and a key must never outlive its pairing.
    pub fn clear_desktop_keys(&self) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM desktop_keys", [])?;
        conn.execute("DELETE FROM meta WHERE key LIKE ?1", params![format!("app.{LABEL_KEY}%")])?;
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
        store.pin_desktop_key("c1", "key-a", "Laptop", 1).unwrap();
        store.pin_desktop_key("c2", "key-b", "Desk", 1).unwrap();
        store.pin_desktop_key("c1", "key-c", "Work laptop", 2).unwrap();
        assert_eq!(store.desktop_key("c1").unwrap().as_deref(), Some("key-c"));
        assert_eq!(store.desktop_pairing("c1").unwrap(), Some(("Work laptop".to_owned(), 2)));
        assert_eq!(store.desktop_connections().unwrap(), ["c2", "c1"]);
        store.remove_desktop_key("c1").unwrap();
        assert_eq!(store.desktop_key("c1").unwrap(), None);
        assert_eq!(store.desktop_pairing("c1").unwrap(), None);
        assert_eq!(store.meta_get("desktop.label.c1").unwrap(), None);
        assert_eq!(store.desktop_key("c2").unwrap().as_deref(), Some("key-b"));
        store.clear_desktop_keys().unwrap();
        assert_eq!(store.desktop_key("c2").unwrap(), None);
        assert_eq!(store.meta_get("desktop.label.c2").unwrap(), None);
        assert!(store.desktop_connections().unwrap().is_empty());
    }
}
