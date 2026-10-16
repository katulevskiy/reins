//! The op-log store (phone roles, item 4): the account's records as the signed op log builds them, per record, with
//! nothing taken from the server on trust.
//!
//! **Applying an op** ([`apply`]), in the server's `seq` order, one at a time:
//!
//! 1. *Order.* `seq` must be the next one: a lower one is a duplicate (the same op as before, else a fork), a higher
//!    one means the server withheld something ([`Halt::Gap`]). A phone that saw a fork applies nothing more.
//! 2. *Roster.* The op's epoch must be one this phone verified ([`Halt::UnknownEpoch`], [`Halt::Unverifiable`]). Its
//!    author is authorized by [`reins_proto::oplog::authorize_op`] against the roster as it stood at the op's epoch,
//!    or, for a late op (signed at an epoch older than one the log already showed), against the current roster, where
//!    only the brake is left to a replaced primary.
//! 3. *Signature* by the author's key in that roster; *counter* above the author's last (a gap is flagged).
//! 4. *Body* opened with the view key it names and the signed envelope as associated data; [`check_change`] must allow
//!    the change for the op's class.
//! 5. *Rules* per table ([`rules`]).
//!
//! An op refused at steps 2 to 5 is skipped but still counted: every phone hashes every op the server sequenced
//! ([`applied_hash_step`]), so the primary's heartbeats and checkpoints can tell whether two phones saw one history.
//!
//! **Versions.** A record's version is the op that last wrote it: `(epoch, author, counter)`. A put says which version
//! it replaces (`prev`, [`RecordPut`]): a writer that saw the record as it is replaces it (last writer wins, exact,
//! no clocks); any other put met a change it did not see (a viewer's brake, a late op, another primary) and goes
//! through the table's join, which can only tighten security records.
//!
//! **Floors.** The applied `seq`, the highest epoch the log showed (`log_epoch`, also refusing an older checkpoint),
//! each author's counter, and the applied hash per `seq` ([`check_heartbeat`], [`checkpoint::load`]).
//!
//! **Tombstones** stay until every member acknowledged them, or [`TOMBSTONE_HORIZON_SECS`] ([`gc`]).
//! **Proposals** live in their own table, keyed `<author>/<id>`, until resolved, withdrawn or expired.

pub mod checkpoint;
mod rules;
#[cfg(test)]
mod tests;

use reins_proto::canon::{decode_payload, encode_payload, hex32, parse_hex32, parse_sig};
use reins_proto::oplog::{
    ChangeRefusal, GENESIS_APPLIED_HASH, Op, OpBody, OpClass, OpRefusal, SyncedOp, Table, applied_hash_step,
    authorize_op, body_associated_data, check_change,
};
use reins_proto::roster::Member;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::TOMBSTONE_HORIZON_SECS;
use crate::CoreError;
use crate::roles::LocalRoster;
use crate::roles::keys::{SigningKey, ViewKeyring, ed25519_verify, sealed_kid};

pub use checkpoint::{CheckpointCheck, HeartbeatCheck, check_heartbeat};

/// How long a proposal waits for the primary (seconds).
pub const PROPOSAL_TTL_SECS: i64 = 7 * 86_400;
/// Applied hashes kept per `seq` behind the head (to compare heartbeats and checkpoints with).
const KEPT_SEQS: u64 = 4_096;

/// The tables of the op-log store.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS oplog_records (
    tbl TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT,
    epoch INTEGER NOT NULL,
    author TEXT NOT NULL,
    counter INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (tbl, key)
);
CREATE TABLE IF NOT EXISTS oplog_proposals (
    key TEXT PRIMARY KEY,
    author TEXT NOT NULL,
    value TEXT,
    seq INTEGER NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS oplog_authors (
    author TEXT NOT NULL,
    sign_pub TEXT NOT NULL,
    counter INTEGER NOT NULL,
    gap_from INTEGER,
    PRIMARY KEY (author, sign_pub)
);
CREATE TABLE IF NOT EXISTS oplog_seqs (
    seq INTEGER PRIMARY KEY,
    op_hash TEXT NOT NULL,
    applied_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS oplog_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    seq INTEGER NOT NULL,
    applied_hash TEXT NOT NULL,
    log_epoch INTEGER NOT NULL,
    forked TEXT
);
";

/// Which op wrote a record's current version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: u64,
    pub author: String,
    pub counter: u64,
}

impl Version {
    pub fn of(op: &Op) -> Self {
        Self {
            epoch: op.epoch,
            author: op.author.clone(),
            counter: op.counter,
        }
    }
}

/// The value of a `put` op: the row, and the version of the record its writer replaced (`None`: a new record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPut {
    #[serde(default)]
    pub prev: Option<Version>,
    pub row: Json,
}

/// One record as the log built it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub table: Table,
    pub key: String,
    /// `None`: removed (a tombstone).
    pub value: Option<Json>,
    pub version: Version,
    /// The op that wrote this version.
    pub seq: u64,
    /// When this phone applied it (unix seconds).
    pub at: i64,
}

/// A viewer's proposal waiting for the primary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// `<author>/<id>`.
    pub key: String,
    pub author: String,
    pub value: Json,
    pub seq: u64,
    pub received_at: i64,
}

/// Where the log stands on this phone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogState {
    pub seq: u64,
    pub applied_hash: [u8; 32],
    /// The highest roster epoch the log showed (ops and checkpoints).
    pub log_epoch: u64,
    /// Why this phone stopped applying (a fork), if it did.
    pub forked: Option<String>,
}

/// What happened to one op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Its change is in the records.
    Applied,
    /// Valid, and counted, but the record kept another value (a removal beat it, a stale edit, an immutable entry).
    Superseded,
    /// Counted, not applied.
    Refused(Refusal),
    /// The op at this `seq` was applied already.
    Duplicate,
}

/// Why an op the server sequenced was not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Invalid(String),
    NotAuthorized(OpRefusal),
    BadSignature,
    /// The author used this counter (or a later one) already.
    CounterReplay,
    /// The body does not open under its envelope.
    Unopenable,
    Change(ChangeRefusal),
}

/// Why applying stops (nothing of the op is kept; the caller fixes what is missing and tries again).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Halt {
    /// The server skipped ops (it withheld something): wait for them.
    #[error("the log skips from seq {expected} to {got}")]
    Gap {
        expected: u64,
        got: u64,
    },
    /// An op or checkpoint names an epoch after the roster this phone verified: fetch the roster.
    #[error("the log is at roster epoch {0}, after the one this phone verified")]
    UnknownEpoch(u64),
    /// It names an epoch before the roster entry this phone trusts first: load a newer checkpoint.
    #[error("the log goes back to roster epoch {0}, before what this phone can verify")]
    Unverifiable(u64),
    /// Sealed with a view key this phone does not hold (yet).
    #[error("the log is sealed with a view key this phone lacks ({0})")]
    MissingKey(u64),
    /// The server showed this phone two histories.
    #[error("the account's history forked: {0}")]
    Forked(String),
    /// A checkpoint or heartbeat older than what this phone applied.
    #[error("the server sent an older state than this phone has (a rollback)")]
    Rollback,
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<rusqlite::Error> for Halt {
    fn from(e: rusqlite::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<Halt> for CoreError {
    fn from(h: Halt) -> Self {
        match h {
            Halt::Core(e) => e,
            other => Self::invalid(other.to_string()),
        }
    }
}

/// What applying (and checking) ops needs: the account, the roster this phone verified and its view keys.
pub struct Context<'a> {
    pub account: &'a str,
    pub roster: &'a LocalRoster,
    pub keyring: &'a ViewKeyring,
    /// Unix seconds.
    pub now: i64,
}

fn table_name(table: Table) -> String {
    serde_json::to_value(table).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn table_of(name: &str) -> Option<Table> {
    serde_json::from_value(Json::String(name.to_owned())).ok()
}

fn as_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn as_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// Where the log stands (all zeros before the first op or checkpoint).
pub fn state(conn: &Connection) -> Result<LogState, CoreError> {
    let row: Option<(i64, String, i64, Option<String>)> = conn
        .query_row("SELECT seq, applied_hash, log_epoch, forked FROM oplog_state WHERE id = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    Ok(match row {
        Some((seq, hash, log_epoch, forked)) => LogState {
            seq: as_u64(seq),
            applied_hash: parse_hex32(&hash).ok_or_else(|| CoreError::storage("corrupt op-log state"))?,
            log_epoch: as_u64(log_epoch),
            forked,
        },
        None => LogState {
            seq: 0,
            applied_hash: GENESIS_APPLIED_HASH,
            log_epoch: 0,
            forked: None,
        },
    })
}

fn save_state(conn: &Connection, s: &LogState) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO oplog_state (id, seq, applied_hash, log_epoch, forked) VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET seq = ?1, applied_hash = ?2, log_epoch = ?3, forked = ?4",
        params![as_i64(s.seq), hex32(&s.applied_hash), as_i64(s.log_epoch), s.forked],
    )?;
    Ok(())
}

/// Stops applying for good on this phone: the server showed it two histories.
fn fork(conn: &Connection, why: &str) -> Halt {
    match state(conn).and_then(|mut s| {
        s.forked = Some(why.to_owned());
        save_state(conn, &s)
    }) {
        Ok(()) => Halt::Forked(why.to_owned()),
        Err(e) => Halt::Core(e),
    }
}

/// The applied hash this phone computed after `seq`, if it still keeps it.
pub(crate) fn applied_at(conn: &Connection, seq: u64) -> Result<Option<[u8; 32]>, CoreError> {
    let s = state(conn)?;
    if seq == s.seq {
        return Ok(Some(s.applied_hash));
    }
    let kept: Option<String> = conn
        .query_row("SELECT applied_hash FROM oplog_seqs WHERE seq = ?1", params![as_i64(seq)], |r| r.get(0))
        .optional()?;
    Ok(kept.and_then(|h| parse_hex32(&h)))
}

fn record_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRecord> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
}

type RawRecord = (String, String, Option<String>, i64, String, i64, i64, i64);

fn record_of(raw: RawRecord) -> Result<Record, CoreError> {
    let (tbl, key, value, epoch, author, counter, seq, at) = raw;
    Ok(Record {
        table: table_of(&tbl).ok_or_else(|| CoreError::storage("corrupt op-log record"))?,
        key,
        value: value
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|_| CoreError::storage("corrupt op-log record"))?,
        version: Version {
            epoch: as_u64(epoch),
            author,
            counter: as_u64(counter),
        },
        seq: as_u64(seq),
        at,
    })
}

const RECORD_COLUMNS: &str = "tbl, key, value, epoch, author, counter, seq, at";

/// One record, a tombstone included.
pub fn record(conn: &Connection, table: Table, key: &str) -> Result<Option<Record>, CoreError> {
    let raw = conn
        .query_row(
            &format!("SELECT {RECORD_COLUMNS} FROM oplog_records WHERE tbl = ?1 AND key = ?2"),
            params![table_name(table), key],
            record_from_row,
        )
        .optional()?;
    raw.map(record_of).transpose()
}

/// The live records of `table`, by key.
pub fn records(conn: &Connection, table: Table) -> Result<Vec<Record>, CoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM oplog_records WHERE tbl = ?1 AND value IS NOT NULL ORDER BY key"
    ))?;
    let raws = stmt.query_map(params![table_name(table)], record_from_row)?.collect::<Result<Vec<_>, _>>()?;
    raws.into_iter().map(record_of).collect()
}

/// The proposals waiting for the primary (not resolved, withdrawn or expired), oldest first.
pub fn proposals(conn: &Connection, now: i64) -> Result<Vec<Proposal>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT key, author, value, seq, received_at FROM oplog_proposals
             WHERE value IS NOT NULL AND received_at > ?1 ORDER BY seq",
    )?;
    let rows = stmt
        .query_map(params![now - PROPOSAL_TTL_SECS], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get(4)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(key, author, value, seq, received_at)| {
            Ok(Proposal {
                key,
                author,
                value: serde_json::from_str(&value).map_err(|_| CoreError::storage("corrupt proposal"))?,
                seq: as_u64(seq),
                received_at,
            })
        })
        .collect()
}

/// Authors whose counters skipped (something of theirs was withheld or refused by the server): author, signing key,
/// first missing counter.
pub fn counter_gaps(conn: &Connection) -> Result<Vec<(String, String, u64)>, CoreError> {
    let mut stmt = conn
        .prepare("SELECT author, sign_pub, gap_from FROM oplog_authors WHERE gap_from IS NOT NULL ORDER BY author")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().map(|(a, k, g)| (a, k, as_u64(g))).collect())
}

/// Who may have written `op`, judged against the roster as it stood at the op's epoch, or against the current one
/// for a late op (`op.epoch` before `log_epoch`).
fn authorize<'a>(op: &Op, ctx: &'a Context<'_>, log_epoch: u64) -> Result<Result<&'a Member, Refusal>, Halt> {
    let entries = &ctx.roster.entries;
    let first = entries.first().map_or(0, |e| e.epoch);
    if op.epoch > ctx.roster.epoch() {
        return Err(Halt::UnknownEpoch(op.epoch));
    }
    if op.epoch < first {
        return Err(Halt::Unverifiable(op.epoch));
    }
    let view = if op.epoch < log_epoch {
        &entries[..]
    } else {
        let end = entries.iter().position(|e| e.epoch == op.epoch).map_or(entries.len(), |i| i + 1);
        &entries[..end]
    };
    Ok(authorize_op(op, view).map_err(Refusal::NotAuthorized))
}

/// Applies the op the server sequenced at `synced.seq` (see the module docs).
pub fn apply(conn: &mut Connection, ctx: &Context<'_>, synced: &SyncedOp) -> Result<Outcome, Halt> {
    let tx = conn.transaction()?;
    match apply_in(&tx, ctx, synced) {
        Ok(outcome) => {
            tx.commit()?;
            Ok(outcome)
        }
        // A fork is recorded before anything of the op is written: kept, so applying stays stopped.
        Err(Halt::Forked(why)) => {
            tx.commit()?;
            Err(Halt::Forked(why))
        }
        Err(halt) => Err(halt),
    }
}

/// Applies ops in order until one halts; the outcomes of those applied, and the halt if one did.
pub fn apply_all(conn: &mut Connection, ctx: &Context<'_>, ops: &[SyncedOp]) -> (Vec<Outcome>, Option<Halt>) {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match apply(conn, ctx, op) {
            Ok(o) => out.push(o),
            Err(h) => return (out, Some(h)),
        }
    }
    (out, None)
}

fn apply_in(tx: &Transaction<'_>, ctx: &Context<'_>, synced: &SyncedOp) -> Result<Outcome, Halt> {
    let mut s = state(tx)?;
    if let Some(why) = &s.forked {
        return Err(Halt::Forked(why.clone()));
    }
    let op = &synced.op;
    let Ok(op_hash) = op.hash() else {
        // An op the server's own checks would refuse: it cannot be hashed into the history either.
        return Err(fork(tx, &format!("the op at seq {} is malformed", synced.seq)));
    };
    if synced.seq <= s.seq {
        let kept: Option<String> = tx
            .query_row("SELECT op_hash FROM oplog_seqs WHERE seq = ?1", params![as_i64(synced.seq)], |r| r.get(0))
            .optional()?;
        return match kept {
            Some(h) if h != hex32(&op_hash) => Err(fork(tx, &format!("two different ops at seq {}", synced.seq))),
            _ => Ok(Outcome::Duplicate),
        };
    }
    if synced.seq != s.seq + 1 {
        return Err(Halt::Gap {
            expected: s.seq + 1,
            got: synced.seq,
        });
    }
    let outcome = judge_and_apply(tx, ctx, op, synced.seq, &mut s)?;
    let applied = applied_hash_step(&s.applied_hash, synced.seq, op).map_err(|e| CoreError::invalid(e.to_string()))?;
    s.seq = synced.seq;
    s.applied_hash = applied;
    save_state(tx, &s)?;
    tx.execute(
        "INSERT OR REPLACE INTO oplog_seqs (seq, op_hash, applied_hash) VALUES (?1, ?2, ?3)",
        params![as_i64(synced.seq), hex32(&op_hash), hex32(&applied)],
    )?;
    tx.execute("DELETE FROM oplog_seqs WHERE seq < ?1", params![as_i64(synced.seq.saturating_sub(KEPT_SEQS))])?;
    Ok(outcome)
}

/// Steps 2 to 5 of the module docs for one op; `s.log_epoch` rises with every authentic op.
fn judge_and_apply(
    tx: &Transaction<'_>,
    ctx: &Context<'_>,
    op: &Op,
    seq: u64,
    s: &mut LogState,
) -> Result<Outcome, Halt> {
    if let Err(e) = op.validate() {
        return Ok(Outcome::Refused(Refusal::Invalid(e.to_string())));
    }
    let signer = match authorize(op, ctx, s.log_epoch)? {
        Ok(member) => member,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let verified = match (signer.sign_key(), op.signing_bytes(), parse_sig(&op.sig)) {
        (Ok(key), Ok(message), Some(sig)) => ed25519_verify(&key, &message, &sig),
        _ => false,
    };
    if !verified {
        return Ok(Outcome::Refused(Refusal::BadSignature));
    }
    // Authentic from here: the author's counter and the log's epoch move.
    let last: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT counter, gap_from FROM oplog_authors WHERE author = ?1 AND sign_pub = ?2",
            params![op.author, signer.sign_pub],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (last_counter, gap) = last.map_or((0, None), |(c, g)| (as_u64(c), g.map(as_u64)));
    if op.counter <= last_counter {
        return Ok(Outcome::Refused(Refusal::CounterReplay));
    }
    let gap = gap.or_else(|| (op.counter > last_counter + 1).then_some(last_counter + 1));
    tx.execute(
        "INSERT INTO oplog_authors (author, sign_pub, counter, gap_from) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (author, sign_pub) DO UPDATE SET counter = ?3, gap_from = ?4",
        params![op.author, signer.sign_pub, as_i64(op.counter), gap.map(as_i64)],
    )?;
    s.log_epoch = s.log_epoch.max(op.epoch);
    let Some(sealed) = decode_payload(&op.body) else {
        return Ok(Outcome::Refused(Refusal::Unopenable));
    };
    let kid = sealed_kid(&sealed).unwrap_or(0);
    if ctx.keyring.get(kid).is_none() {
        return Err(Halt::MissingKey(kid));
    }
    let Ok(plain) = ctx.keyring.open(&body_associated_data(op), &sealed) else {
        return Ok(Outcome::Refused(Refusal::Unopenable));
    };
    let body: OpBody = match serde_json::from_slice(&plain) {
        Ok(body) => body,
        Err(e) => return Ok(Outcome::Refused(Refusal::Invalid(format!("body: {e}")))),
    };
    if let Err(refusal) = check_change(op, &body) {
        return Ok(Outcome::Refused(Refusal::Change(refusal)));
    }
    rules::apply_change(tx, op, &body.change, seq, ctx.now).map_err(Halt::from)
}

/// Drops tombstones every member has passed (`min_acked_seq`, from the server's sync answer) or that are older than
/// [`TOMBSTONE_HORIZON_SECS`], resolved proposals the same way, and proposals past [`PROPOSAL_TTL_SECS`] (they become
/// resolved first, so a late copy cannot bring them back). Returns how many rows went.
pub fn gc(conn: &Connection, min_acked_seq: u64, now: i64) -> Result<usize, CoreError> {
    let horizon = now - TOMBSTONE_HORIZON_SECS;
    let mut n = conn.execute(
        "DELETE FROM oplog_records WHERE value IS NULL AND (seq <= ?1 OR at < ?2)",
        params![as_i64(min_acked_seq), horizon],
    )?;
    conn.execute(
        "UPDATE oplog_proposals SET value = NULL WHERE value IS NOT NULL AND received_at <= ?1",
        params![now - PROPOSAL_TTL_SECS],
    )?;
    n += conn.execute(
        "DELETE FROM oplog_proposals WHERE value IS NULL AND (seq <= ?1 OR received_at < ?2)",
        params![as_i64(min_acked_seq), horizon],
    )?;
    Ok(n)
}

impl super::Store {
    /// Runs `work` on this store's op-log tables (in the account's sealed database, with everything else).
    pub fn oplog<T>(&self, work: impl FnOnce(&mut Connection) -> T) -> Result<T, CoreError> {
        let mut conn = self.lock()?;
        Ok(work(&mut conn))
    }
}

/// An op of this phone's (`author`, signing key `signer`): `body` sealed with the newest view key under the signed
/// envelope as associated data, then signed. The caller numbers its ops (`counter`) and picks the class.
#[expect(clippy::too_many_arguments, reason = "an op's envelope, field by field")]
pub fn seal_op(
    account: &str,
    keyring: &ViewKeyring,
    signer: &SigningKey,
    author: &str,
    epoch: u64,
    counter: u64,
    class: OpClass,
    notify: bool,
    body: &OpBody,
) -> Result<Op, CoreError> {
    let mut op = Op {
        v: reins_proto::PROTOCOL_VERSION,
        account: account.to_owned(),
        epoch,
        author: author.to_owned(),
        counter,
        class,
        notify,
        body: String::new(),
        sig: String::new(),
    };
    let plain = serde_json::to_vec(body).map_err(|e| CoreError::storage(e.to_string()))?;
    op.body = encode_payload(&keyring.seal(&body_associated_data(&op), &plain)?);
    op.sig = signer.sign_b64(&op.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
    Ok(op)
}
