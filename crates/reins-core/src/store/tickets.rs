//! Requests that became tickets (Decisions v2, 03): their deadlines and what this phone decided. A parked request
//! that is a ticket lives until its deadline instead of the relay's ten minutes ([`super::Store::pending_rows`]).

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::CoreError;

/// One ticket as this phone knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalTicket {
    pub id: String,
    pub connection_id: String,
    /// The request as the server sealed it (JSON): what an approval is revalidated against.
    pub request: String,
    /// The request digest the server stored (and this phone checked against the opened body).
    pub digest: String,
    /// `on_approve` or `on_collect`.
    pub execution: String,
    /// How long an approval stays usable (03 §6).
    pub valid_secs: i64,
    /// As the server last said, or as this phone moved it.
    pub state: String,
    pub deferred_at: i64,
    pub deadline: i64,
    /// The "Why?" this phone asked.
    pub question: Option<String>,
    /// The agent's answer (shown labelled as the agent's).
    pub reply: Option<String>,
    pub stale: Option<String>,
    /// When this phone approved it, and until when the approval can be collected.
    pub approved_at: Option<i64>,
    pub valid_until: Option<i64>,
    /// This phone signed the expiry the server recorded (for webhooks).
    pub expiry_signed: bool,
    /// What the owner released when approving a read (the message ids picked): a collect releases only these, of
    /// what the read finds then. JSON array.
    pub approved_ids: Option<String>,
}

/// Approved tickets are kept this long after their validity, so a late collect is answered "expired" rather than
/// "unknown".
const KEEP_AFTER_SECS: i64 = 86_400;

impl Store {
    pub(crate) fn ticket(&self, id: &str) -> Result<Option<LocalTicket>, CoreError> {
        let conn = self.read()?;
        Ok(conn
            .query_row(
                "SELECT id, digest, execution, state, deferred_at, deadline, question, reply, stale, approved_at, \
                 valid_until, expiry_signed, connection_id, request, valid_secs, approved_ids FROM tickets WHERE id = ?1",
                params![id],
                |r| {
                    Ok(LocalTicket {
                        id: r.get(0)?,
                        digest: r.get(1)?,
                        execution: r.get(2)?,
                        state: r.get(3)?,
                        deferred_at: r.get(4)?,
                        deadline: r.get(5)?,
                        question: r.get(6)?,
                        reply: r.get(7)?,
                        stale: r.get(8)?,
                        approved_at: r.get(9)?,
                        valid_until: r.get(10)?,
                        expiry_signed: r.get::<_, i64>(11)? != 0,
                        connection_id: r.get(12)?,
                        request: r.get(13)?,
                        valid_secs: r.get(14)?,
                        approved_ids: r.get(15)?,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn save_ticket(&self, t: &LocalTicket, now: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM tickets WHERE MAX(deadline, COALESCE(valid_until, 0)) < ?1",
            params![now - KEEP_AFTER_SECS],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO tickets (id, digest, execution, state, deferred_at, deadline, question, reply, \
             stale, approved_at, valid_until, expiry_signed, connection_id, request, valid_secs, approved_ids) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                t.id,
                t.digest,
                t.execution,
                t.state,
                t.deferred_at,
                t.deadline,
                t.question,
                t.reply,
                t.stale,
                t.approved_at,
                t.valid_until,
                i64::from(t.expiry_signed),
                t.connection_id,
                t.request,
                t.valid_secs,
                t.approved_ids
            ],
        )?;
        Ok(())
    }

    /// Keeps a request exactly as the server relayed it (a day): what its ticket's digest is over, whatever this
    /// phone resolves in its own copy.
    pub(crate) fn remember_received(&self, id: &str, request: &str, now: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM received WHERE at < ?1", params![now - 86_400])?;
        conn.execute(
            "INSERT OR REPLACE INTO received (id, request, at) VALUES (?1, ?2, ?3)",
            params![id, request, now],
        )?;
        Ok(())
    }

    pub(crate) fn received(&self, id: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .read()?
            .query_row("SELECT request FROM received WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?)
    }

    pub(crate) fn remove_ticket(&self, id: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM tickets WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::tests::open;
    use crate::types::PendingKind;

    use super::*;

    #[test]
    fn a_ticket_outlives_the_relays_ten_minutes_until_its_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.park("r1", PendingKind::Request, 1_000, 1_000, b"one").unwrap();
        store.park("r2", PendingKind::Request, 1_000, 1_000, b"two").unwrap();
        let ticket = LocalTicket {
            id: "r1".to_owned(),
            digest: "d".to_owned(),
            execution: "on_approve".to_owned(),
            state: "deferred".to_owned(),
            deferred_at: 1_100,
            deadline: 15_000,
            ..LocalTicket::default()
        };
        store.save_ticket(&ticket, 1_100).unwrap();
        let ids: Vec<String> = store.pending_rows(5_000).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["r1"], "r2 expired with the relay; the ticket waits");
        assert!(store.pending_rows(15_000).unwrap().is_empty(), "until its deadline");
        assert_eq!(store.ticket("r1").unwrap(), Some(ticket));
        store.remove_ticket("r1").unwrap();
        assert_eq!(store.ticket("r1").unwrap(), None);
    }
}
