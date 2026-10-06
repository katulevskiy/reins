//! Items parked for the user, and ids already answered.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::{CoreError, PendingKind};

/// The server forgets pending requests and pairings after 600 s (contracts §A).
pub const PENDING_TTL_SECS: i64 = 600;
/// Answered ids are remembered this long so duplicates are ignored.
pub const HANDLED_RETENTION_SECS: i64 = 86_400;

/// A parked item with its decrypted payload (JSON written by the handler).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRow {
    pub id: String,
    pub kind: PendingKind,
    /// Server creation time.
    pub created_at: i64,
    /// Local time the item was parked.
    pub parked_at: i64,
    pub payload: Vec<u8>,
}

fn aad(id: &str) -> String {
    format!("pending.payload:{id}")
}

fn kind_str(kind: PendingKind) -> &'static str {
    match kind {
        PendingKind::Request => "request",
        PendingKind::Pairing => "pairing",
        PendingKind::Blob => "blob",
        PendingKind::Join => "join",
    }
}

impl Store {
    /// Parks an item. Returns false (and changes nothing) when `id` is already parked.
    pub fn park(
        &self,
        id: &str,
        kind: PendingKind,
        created_at: i64,
        now: i64,
        payload: &[u8],
    ) -> Result<bool, CoreError> {
        let sealed = self.seal(&aad(id), payload)?;
        let inserted = self.lock()?.execute(
            "INSERT OR IGNORE INTO pending (id, kind, created_at, parked_at, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, kind_str(kind), created_at, now, sealed],
        )?;
        Ok(inserted == 1)
    }

    /// One unexpired parked item.
    pub fn pending_item(&self, id: &str, now: i64) -> Result<Option<PendingRow>, CoreError> {
        Ok(self.pending_rows(now)?.into_iter().find(|r| r.id == id))
    }

    /// Unexpired parked items, newest first. Expired and undecryptable rows are deleted.
    pub fn pending_rows(&self, now: i64) -> Result<Vec<PendingRow>, CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM pending WHERE parked_at <= ?1", params![now - PENDING_TTL_SECS])?;
        let mut stmt =
            conn.prepare("SELECT id, kind, created_at, parked_at, payload FROM pending ORDER BY created_at DESC, id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut out = Vec::with_capacity(rows.len());
        for (id, kind, created_at, parked_at, sealed) in rows {
            let kind = match kind.as_str() {
                "pairing" => PendingKind::Pairing,
                "blob" => PendingKind::Blob,
                "join" => PendingKind::Join,
                _ => PendingKind::Request,
            };
            match self.unseal(&aad(&id), &sealed) {
                Ok(payload) => out.push(PendingRow {
                    id,
                    kind,
                    created_at,
                    parked_at,
                    payload,
                }),
                Err(_) => {
                    conn.execute("DELETE FROM pending WHERE id = ?1", params![id])?;
                }
            }
        }
        Ok(out)
    }

    /// Removes a parked item. Returns whether it existed.
    pub fn remove_pending(&self, id: &str) -> Result<bool, CoreError> {
        Ok(self.lock()?.execute("DELETE FROM pending WHERE id = ?1", params![id])? == 1)
    }

    pub fn delete_all_pending(&self) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM pending", [])?;
        Ok(())
    }

    /// Remembers that `id` was answered.
    pub fn mark_handled(&self, id: &str, now: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM handled WHERE at <= ?1", params![now - HANDLED_RETENTION_SECS])?;
        conn.execute("INSERT OR REPLACE INTO handled (id, at) VALUES (?1, ?2)", params![id, now])?;
        Ok(())
    }

    /// True when `id` is parked or was already answered.
    pub fn is_known(&self, id: &str) -> Result<bool, CoreError> {
        let conn = self.lock()?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM pending WHERE id = ?1 UNION ALL SELECT 1 FROM handled WHERE id = ?1 LIMIT 1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::open;

    #[test]
    fn park_dedupes_orders_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert!(store.park("r1", PendingKind::Request, 100, 1_000, b"one").unwrap());
        assert!(!store.park("r1", PendingKind::Request, 100, 1_000, b"again").unwrap());
        assert!(store.park("p1", PendingKind::Pairing, 200, 1_100, b"two").unwrap());
        let rows = store.pending_rows(1_100).unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["p1", "r1"]);
        assert_eq!(rows[1].payload, b"one");
        assert_eq!(rows[0].kind, PendingKind::Pairing);
        assert!(store.pending_item("r1", 1_599).unwrap().is_some());
        assert!(store.pending_item("r1", 1_600).unwrap().is_none(), "expired 600 s after parking");
        assert!(store.pending_item("p1", 1_600).unwrap().is_some());
        assert!(store.park("b1", PendingKind::Blob, 300, 1_100, b"three").unwrap());
        assert_eq!(store.pending_item("b1", 1_100).unwrap().unwrap().kind, PendingKind::Blob);
    }

    #[test]
    fn handled_and_known() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert!(!store.is_known("r1").unwrap());
        store.park("r1", PendingKind::Request, 1, 10, b"x").unwrap();
        assert!(store.is_known("r1").unwrap());
        assert!(store.remove_pending("r1").unwrap());
        assert!(!store.remove_pending("r1").unwrap());
        assert!(!store.is_known("r1").unwrap());
        store.mark_handled("r1", 10).unwrap();
        assert!(store.is_known("r1").unwrap());
        store.mark_handled("r2", 10 + HANDLED_RETENTION_SECS).unwrap();
        assert!(!store.is_known("r1").unwrap(), "old handled ids are pruned");
    }

    #[test]
    fn payload_is_bound_to_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.park("a", PendingKind::Request, 1, 10, b"secret-a").unwrap();
        store.park("b", PendingKind::Request, 1, 10, b"secret-b").unwrap();
        let conn = store.lock().unwrap();
        conn.execute("UPDATE pending SET payload = (SELECT payload FROM pending WHERE id = 'a') WHERE id = 'b'", [])
            .unwrap();
        drop(conn);
        let rows = store.pending_rows(10).unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["a"], "swapped blob is dropped");
    }
}
