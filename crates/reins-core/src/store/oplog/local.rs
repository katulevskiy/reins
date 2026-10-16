//! What the sync engine (`crate::live_sync`) keeps next to the op log on this phone:
//!
//! - the **outbox**: this phone's own ops, signed and numbered, until the log brings them back applied;
//! - the **mirror**: per record, the version this phone last saw or wrote and a digest of the store row it matches
//!   (what changed on either side since), and how the store finds the row;
//! - a few **marks** (where Activity was published up to, where the store was brought up to).
//!
//! All of it is this phone's own bookkeeping, like the log itself: never in a snapshot or a checkpoint.

use std::collections::{HashMap, HashSet};

use reins_proto::oplog::Op;
use rusqlite::{Connection, OptionalExtension, params};

use super::{Version, as_i64, as_u64};
use crate::CoreError;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS oplog_outbox (
    counter INTEGER PRIMARY KEY,
    tbl TEXT NOT NULL,
    key TEXT NOT NULL,
    op TEXT NOT NULL,
    sent INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS oplog_mirror (
    tbl TEXT NOT NULL,
    key TEXT NOT NULL,
    epoch INTEGER,
    author TEXT,
    counter INTEGER,
    digest TEXT NOT NULL,
    local TEXT,
    PRIMARY KEY (tbl, key)
);
CREATE TABLE IF NOT EXISTS oplog_local (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// The key a queued op covers when it changes a whole table (stop every bypass).
pub(crate) const WHOLE_TABLE: &str = "*";

/// One of this phone's ops, waiting for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Queued {
    pub counter: u64,
    pub table: String,
    pub key: String,
    pub op: Op,
    /// The server took it (it waits to come back applied).
    pub sent: bool,
}

/// A record as this phone last saw or wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mirrored {
    pub version: Option<Version>,
    /// The digest of the store row it matches; empty for a removed one.
    pub digest: String,
    /// How the store finds the row (JSON), when it has one.
    pub local: Option<String>,
}

fn corrupt() -> CoreError {
    CoreError::storage("corrupt sync outbox")
}

/// The ops waiting, in counter order.
pub(crate) fn outbox(conn: &Connection) -> Result<Vec<Queued>, CoreError> {
    let mut stmt = conn.prepare("SELECT counter, tbl, key, op, sent FROM oplog_outbox ORDER BY counter")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get(4)?))
        })?
        .collect::<Result<Vec<(i64, String, String, String, bool)>, _>>()?;
    rows.into_iter()
        .map(|(counter, table, key, op, sent)| {
            Ok(Queued {
                counter: as_u64(counter),
                table,
                key,
                op: serde_json::from_str(&op).map_err(|_| corrupt())?,
                sent,
            })
        })
        .collect()
}

/// The highest counter queued (0: none).
pub(crate) fn outbox_head(conn: &Connection) -> Result<u64, CoreError> {
    let head: Option<i64> = conn.query_row("SELECT max(counter) FROM oplog_outbox", [], |r| r.get(0))?;
    Ok(head.map_or(0, as_u64))
}

pub(crate) fn queue(conn: &Connection, op: &Op, table: &str, key: &str) -> Result<(), CoreError> {
    let raw = serde_json::to_string(op).map_err(|e| CoreError::storage(e.to_string()))?;
    conn.execute(
        "INSERT INTO oplog_outbox (counter, tbl, key, op, sent) VALUES (?1, ?2, ?3, ?4, 0)",
        params![as_i64(op.counter), table, key, raw],
    )?;
    Ok(())
}

pub(crate) fn mark_sent(conn: &Connection, counters: &[u64]) -> Result<(), CoreError> {
    for c in counters {
        conn.execute("UPDATE oplog_outbox SET sent = 1 WHERE counter = ?1", [as_i64(*c)])?;
    }
    Ok(())
}

/// Takes ops out of the outbox (the server refused them): the records they covered are no longer mirrored, so the
/// next publish compares the store with the log afresh.
pub(crate) fn drop_queued(conn: &Connection, queued: &[Queued]) -> Result<(), CoreError> {
    for q in queued {
        conn.execute("DELETE FROM oplog_outbox WHERE counter = ?1", [as_i64(q.counter)])?;
        conn.execute("DELETE FROM oplog_mirror WHERE tbl = ?1 AND key = ?2", params![q.table, q.key])?;
    }
    Ok(())
}

/// Forgets the ops the log brought back applied (this phone's counter is past them).
pub(crate) fn prune_outbox(conn: &Connection, applied: u64) -> Result<(), CoreError> {
    conn.execute("DELETE FROM oplog_outbox WHERE counter <= ?1", [as_i64(applied)])?;
    Ok(())
}

/// The records an op still waiting covers (`(table, key)`; [`WHOLE_TABLE`] for all of a table).
pub(crate) fn pending_keys(conn: &Connection) -> Result<HashSet<(String, String)>, CoreError> {
    let mut stmt = conn.prepare("SELECT tbl, key FROM oplog_outbox")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

pub(crate) fn mirror(conn: &Connection) -> Result<HashMap<(String, String), Mirrored>, CoreError> {
    let mut stmt = conn.prepare("SELECT tbl, key, epoch, author, counter, digest, local FROM oplog_mirror")?;
    let rows = stmt
        .query_map([], |r| {
            let version =
                match (r.get::<_, Option<i64>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<i64>>(4)?) {
                    (Some(epoch), Some(author), Some(counter)) => Some(Version {
                        epoch: as_u64(epoch),
                        author,
                        counter: as_u64(counter),
                    }),
                    _ => None,
                };
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                Mirrored {
                    version,
                    digest: r.get(5)?,
                    local: r.get(6)?,
                },
            ))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

pub(crate) fn set_mirror(conn: &Connection, table: &str, key: &str, m: &Mirrored) -> Result<(), CoreError> {
    let v = m.version.as_ref();
    conn.execute(
        "INSERT INTO oplog_mirror (tbl, key, epoch, author, counter, digest, local) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (tbl, key) DO UPDATE SET epoch = ?3, author = ?4, counter = ?5, digest = ?6, local = ?7",
        params![
            table,
            key,
            v.map(|v| as_i64(v.epoch)),
            v.map(|v| v.author.clone()),
            v.map(|v| as_i64(v.counter)),
            m.digest,
            m.local
        ],
    )?;
    Ok(())
}

pub(crate) fn mark(conn: &Connection, key: &str) -> Result<Option<String>, CoreError> {
    Ok(conn.query_row("SELECT value FROM oplog_local WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

pub(crate) fn set_mark(conn: &Connection, key: &str, value: &str) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO oplog_local (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}
