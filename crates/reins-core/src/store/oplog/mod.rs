//! The op-log store (phone roles, item 4): the account's records as the signed op log builds them, per record, with
//! nothing taken from the server on trust.
//!
//! **Applying an op** ([`apply`]), in the server's `seq` order, one at a time:
//!
//! 1. *Order.* `seq` must be the next one: a lower one is a duplicate (the same op as before, else a fork), a higher
//!    one means the server withheld something ([`Halt::Gap`]). A phone that saw a fork applies nothing more.
//! 2. *Signature.* The op's epoch must be one this phone verified ([`Halt::UnknownEpoch`], [`Halt::Unverifiable`]),
//!    its author a member then, and its signature that member's.
//! 3. *Counter.* The next of its author's: a used one is a replay; a later one waits ([`Outcome::Held`]) for those
//!    before it, so a reordered log loses nothing (a brake least of all), and goes ahead, the gap flagged, after
//!    [`HOLD_SEQS`] ops. Authentic ops take their counter whatever comes next.
//! 4. *Role.* [`reins_proto::oplog::authorize_op`] against the newest roster this phone verified: a replaced primary
//!    keeps only the brake, a removed phone nothing. The outgoing primary's history is judged as it stood before it was
//!    replaced: its ops up to the `primary_counter` the handover or recovery entry names.
//! 5. *Body* opened with the view key it names and the signed envelope as associated data; [`check_change`] must allow
//!    the change for the op's class. Then the *rules* per table ([`rules`]).
//!
//! An op refused at steps 2 to 5 is skipped but still counted: every phone hashes every op the server sequenced
//! ([`applied_hash_step`]), so the primary's heartbeats and checkpoints can tell whether two phones saw one history.
//! An op that cannot even be hashed halts applying ([`Halt::Malformed`]) without stopping it for good.
//!
//! **Versions.** A record's version is the op that last wrote it: `(epoch, author, counter)`. A put says which version
//! it replaces (`prev`, [`RecordPut`]): a writer that saw the record as it is replaces it (last writer wins, exact,
//! no clocks); any other put met a change it did not see (a viewer's brake, a late op, another primary) and goes
//! through the table's join, which can only tighten security records.
//!
//! **Floors.** The applied `seq`, the highest epoch the log showed (`log_epoch`, also refusing an older checkpoint),
//! each author's counter, and the applied hash per `seq` ([`check_heartbeat`], [`checkpoint::load`]).
//!
//! **Housekeeping** is part of applying, the same on every phone at the same `seq` ([`state_digest`] covers it): a
//! removed grant, account, secret, MCP server, desktop key or target is remembered for good; other tombstones go
//! [`TOMBSTONE_KEEP_SEQS`] ops later; Activity keeps its newest [`AUDIT_RETENTION`] entries.
//! **Proposals** live in their own table, keyed `<author>/<id>`, until resolved or withdrawn (remembered, so they never
//! come back); one older than [`PROPOSAL_TTL_SECS`] is no longer shown.

pub mod checkpoint;
mod rules;
#[cfg(test)]
mod tests;

use reins_proto::canon::{decode_payload, encode_payload, hex32, parse_hex32, parse_sig};
use reins_proto::oplog::{
    ChangeRefusal, GENESIS_APPLIED_HASH, Op, OpBody, OpClass, OpRefusal, SyncedOp, Table, applied_hash_step,
    authorize_op, body_associated_data, check_change,
};
use reins_proto::roster::{Member, RosterReason};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::AUDIT_RETENTION;
use crate::CoreError;
use crate::roles::LocalRoster;
use crate::roles::keys::{SigningKey, ViewKeyring, ed25519_verify, sealed_kid};

pub use checkpoint::{CheckpointCheck, HeartbeatCheck, check_heartbeat};

/// How long a proposal waits for the primary (seconds).
pub const PROPOSAL_TTL_SECS: i64 = 7 * 86_400;
/// Applied hashes kept per `seq` behind the head (to compare heartbeats and checkpoints with).
const KEPT_SEQS: u64 = 4_096;
/// How many ops an op waits for an earlier counter of its author before it is applied anyway (with the gap flagged).
pub const HOLD_SEQS: u64 = 1_000;
/// How many ops a tombstone of a record whose removal is no security decision is kept after the removal.
pub const TOMBSTONE_KEEP_SEQS: u64 = 10_000;
/// The tables whose removals are remembered for good (`super::GUARDED`'s records).
const GUARDED_TABLES: [Table; 6] =
    [Table::Grants, Table::Accounts, Table::Secrets, Table::McpServers, Table::DesktopKeys, Table::Targets];

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
CREATE INDEX IF NOT EXISTS oplog_records_by_seq ON oplog_records (tbl, seq);
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
CREATE TABLE IF NOT EXISTS oplog_held (
    author TEXT NOT NULL,
    sign_pub TEXT NOT NULL,
    counter INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    op TEXT NOT NULL,
    PRIMARY KEY (author, sign_pub, counter)
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
    /// Authentic, counted, and waiting for an earlier op of its author ([`HOLD_SEQS`]).
    Held,
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
    /// The op at this `seq` cannot even be hashed (its body is not base64url): the server must serve another.
    #[error("the op at seq {0} is malformed")]
    Malformed(u64),
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

/// The member who signed `op` (as the roster stood at its epoch), whatever it may write.
fn signer_of<'a>(op: &Op, ctx: &'a Context<'_>) -> Result<Result<&'a Member, Refusal>, Halt> {
    let first = ctx.roster.entries.first().map_or(0, |e| e.epoch);
    if op.epoch > ctx.roster.epoch() {
        return Err(Halt::UnknownEpoch(op.epoch));
    }
    if op.epoch < first {
        return Err(Halt::Unverifiable(op.epoch));
    }
    if op.account != ctx.account {
        return Ok(Err(Refusal::NotAuthorized(OpRefusal::WrongAccount)));
    }
    let at = ctx.roster.at(op.epoch).ok_or(Halt::Unverifiable(op.epoch))?;
    Ok(at.role_of(&op.author).map(|(_, m)| m).ok_or(Refusal::NotAuthorized(OpRefusal::NotAMember)))
}

/// Whether `op`'s author may make it. An op is judged against the newest roster this phone verified ([`authorize_op`]
/// over the whole chain: a replaced primary keeps only the brake, a removed phone nothing), unless it is history the
/// entry that replaced its author vouched for: an op of the outgoing primary whose counter is at most the
/// `primary_counter` of the `handover` or `recovery` entry that replaced it is judged as the roster stood before.
fn authorize(op: &Op, ctx: &Context<'_>) -> Result<(), Refusal> {
    let entries = &ctx.roster.entries;
    let view = if let Some(at) = ctx.roster.at(op.epoch)
        && at.primary.device_id == op.author
        && let Some(i) = entries.iter().position(|e| e.epoch > op.epoch && !e.primary.same_keys(&at.primary))
        && matches!(entries[i].reason, RosterReason::Handover | RosterReason::Recovery)
        && entries[i].primary_counter.is_some_and(|last| op.counter <= last)
    {
        &entries[..i]
    } else {
        &entries[..]
    };
    authorize_op(op, view).map(|_| ()).map_err(Refusal::NotAuthorized)
}

/// The last counter of `member`'s ops this phone took (0: none): what a handover or recovery entry names as the
/// outgoing primary's `primary_counter`.
pub fn last_counter(conn: &Connection, member: &Member) -> Result<u64, CoreError> {
    author_counter(conn, &member.device_id, &member.sign_pub)
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
        // An op the server's own checks would refuse: it cannot be hashed into the history. Nothing is kept; the
        // server must serve another (this phone does not stop for good over one op).
        return Err(Halt::Malformed(synced.seq));
    };
    if synced.seq <= s.seq {
        let kept: Option<String> = tx
            .query_row("SELECT op_hash FROM oplog_seqs WHERE seq = ?1", params![as_i64(synced.seq)], |r| r.get(0))
            .optional()?;
        // A seq a checkpoint covered has no op hash here ('').
        return match kept {
            Some(h) if !h.is_empty() && h != hex32(&op_hash) => {
                Err(fork(tx, &format!("two different ops at seq {}", synced.seq)))
            }
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
    // Held ops whose wait is over apply now, in counter order (the gap stays flagged).
    release_held(tx, ctx, synced.seq)?;
    prune(tx, synced.seq)?;
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
    let signer = match signer_of(op, ctx)? {
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
    // Authentic from here: the log's epoch moves, and the author's counter.
    s.log_epoch = s.log_epoch.max(op.epoch);
    // A view key this phone lacks halts here, whenever the op's turn comes.
    if let Some(kid) = decode_payload(&op.body).and_then(|sealed| sealed_kid(&sealed))
        && ctx.keyring.get(kid).is_none()
    {
        return Err(Halt::MissingKey(kid));
    }
    let last = author_counter(tx, &op.author, &signer.sign_pub)?;
    if op.counter <= last {
        return Ok(Outcome::Refused(Refusal::CounterReplay));
    }
    if op.counter > last + 1 {
        // Something of this author's came before it and is not here yet (the server reordered or withheld it): the
        // op waits for it, so that nothing (a brake least of all) is lost to a counter that moved on.
        let raw = serde_json::to_string(op).map_err(|e| CoreError::storage(e.to_string()))?;
        tx.execute(
            "INSERT OR REPLACE INTO oplog_held (author, sign_pub, counter, seq, op) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![op.author, signer.sign_pub, as_i64(op.counter), as_i64(seq), raw],
        )?;
        return Ok(Outcome::Held);
    }
    set_counter(tx, &op.author, &signer.sign_pub, op.counter)?;
    let outcome = take_turn(tx, ctx, op, seq)?;
    // The ops of this author that waited for this one follow it.
    drain_held(tx, ctx, &op.author, &signer.sign_pub)?;
    Ok(outcome)
}

fn author_counter(conn: &Connection, author: &str, sign_pub: &str) -> Result<u64, CoreError> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT counter FROM oplog_authors WHERE author = ?1 AND sign_pub = ?2",
            params![author, sign_pub],
            |r| r.get(0),
        )
        .optional()?;
    Ok(last.map_or(0, as_u64))
}

fn set_counter(tx: &Transaction<'_>, author: &str, sign_pub: &str, counter: u64) -> Result<(), CoreError> {
    tx.execute(
        "INSERT INTO oplog_authors (author, sign_pub, counter) VALUES (?1, ?2, ?3)
             ON CONFLICT (author, sign_pub) DO UPDATE SET counter = ?3",
        params![author, sign_pub, as_i64(counter)],
    )?;
    Ok(())
}

fn flag_gap(tx: &Transaction<'_>, author: &str, sign_pub: &str, from: u64) -> Result<(), CoreError> {
    tx.execute(
        "INSERT INTO oplog_authors (author, sign_pub, counter, gap_from) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT (author, sign_pub) DO UPDATE SET gap_from = coalesce(gap_from, ?3)",
        params![author, sign_pub, as_i64(from)],
    )?;
    Ok(())
}

/// Steps 4 and 5 for an authentic op whose turn it is: may its author make it; open the body, check the change, apply.
fn take_turn(tx: &Transaction<'_>, ctx: &Context<'_>, op: &Op, seq: u64) -> Result<Outcome, Halt> {
    if let Err(refusal) = authorize(op, ctx) {
        return Ok(Outcome::Refused(refusal));
    }
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

fn held_op(raw: &str) -> Result<Op, CoreError> {
    serde_json::from_str(raw).map_err(|_| CoreError::storage("corrupt held op"))
}

/// Applies `author`'s held ops whose turn has come (each the next counter), in counter order.
fn drain_held(tx: &Transaction<'_>, ctx: &Context<'_>, author: &str, sign_pub: &str) -> Result<(), Halt> {
    loop {
        let next = author_counter(tx, author, sign_pub)? + 1;
        let held: Option<(i64, String)> = tx
            .query_row(
                "SELECT seq, op FROM oplog_held WHERE author = ?1 AND sign_pub = ?2 AND counter = ?3",
                params![author, sign_pub, as_i64(next)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((seq, raw)) = held else {
            return Ok(());
        };
        tx.execute(
            "DELETE FROM oplog_held WHERE author = ?1 AND sign_pub = ?2 AND counter = ?3",
            params![author, sign_pub, as_i64(next)],
        )?;
        set_counter(tx, author, sign_pub, next)?;
        take_turn(tx, ctx, &held_op(&raw)?, as_u64(seq))?;
    }
}

/// Ops held longer than [`HOLD_SEQS`] (the missing ones never came): applied anyway, in counter order, with the gap
/// flagged ([`counter_gaps`]). The same seq releases them on every phone.
fn release_held(tx: &Transaction<'_>, ctx: &Context<'_>, seq: u64) -> Result<(), Halt> {
    let mut stmt = tx.prepare(
        "SELECT author, sign_pub, counter, seq, op FROM oplog_held WHERE seq + ?1 <= ?2 ORDER BY author, sign_pub, counter",
    )?;
    let due = stmt
        .query_map(params![as_i64(HOLD_SEQS), as_i64(seq)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (author, sign_pub, counter, at, raw) in due {
        tx.execute(
            "DELETE FROM oplog_held WHERE author = ?1 AND sign_pub = ?2 AND counter = ?3",
            params![author, sign_pub, counter],
        )?;
        let last = author_counter(tx, &author, &sign_pub)?;
        if as_u64(counter) <= last {
            continue;
        }
        // The ops before it never came: a gap, for good.
        flag_gap(tx, &author, &sign_pub, last + 1)?;
        set_counter(tx, &author, &sign_pub, as_u64(counter))?;
        take_turn(tx, ctx, &held_op(&raw)?, as_u64(at))?;
        drain_held(tx, ctx, &author, &sign_pub)?;
    }
    Ok(())
}

/// Deterministic housekeeping after each op (the same on every phone at the same seq, so their states stay alike):
/// - tombstones of records whose removal is no security decision (settings, profiles, memory, preferences) go
///   [`TOMBSTONE_KEEP_SEQS`] ops after the removal; a removed grant, account, secret, MCP server, desktop key or target
///   is remembered for good, and a resolved proposal too, so none can come back;
/// - Activity keeps its newest [`AUDIT_RETENTION`] entries (by the op that wrote them).
fn prune(tx: &Transaction<'_>, seq: u64) -> Result<(), CoreError> {
    let guarded = GUARDED_TABLES.iter().map(|t| format!("'{}'", table_name(*t))).collect::<Vec<_>>().join(",");
    tx.execute(
        &format!("DELETE FROM oplog_records WHERE value IS NULL AND tbl NOT IN ({guarded}) AND seq + ?1 <= ?2"),
        params![as_i64(TOMBSTONE_KEEP_SEQS), as_i64(seq)],
    )?;
    tx.execute(
        "DELETE FROM oplog_records WHERE tbl = ?1 AND seq <
             (SELECT seq FROM oplog_records WHERE tbl = ?1 ORDER BY seq DESC LIMIT 1 OFFSET ?2)",
        params![table_name(Table::Audit), AUDIT_RETENTION - 1],
    )?;
    Ok(())
}

/// A digest of the store's state as the log built it (records with their versions and tombstones, proposals and their
/// resolutions, authors' counters, held ops), without this phone's own times: two phones that applied the same ops
/// have the same digest. Checkpoints carry it ([`checkpoint`]).
pub fn state_digest(conn: &Connection) -> Result<[u8; 32], CoreError> {
    use std::fmt::Write as _;
    let mut text = String::new();
    let mut stmt = conn.prepare(
        "SELECT tbl, key, coalesce(value, ''), value IS NULL, epoch, author, counter, seq FROM oplog_records ORDER BY tbl, key",
    )?;
    for row in stmt.query_map([], |r| {
        Ok(serde_json::json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?
        ]))
    })? {
        writeln!(text, "r{}", row?).map_err(|e| CoreError::storage(e.to_string()))?;
    }
    let mut stmt =
        conn.prepare("SELECT key, author, coalesce(value, ''), value IS NULL, seq FROM oplog_proposals ORDER BY key")?;
    for row in stmt.query_map([], |r| {
        Ok(serde_json::json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
            r.get::<_, i64>(4)?
        ]))
    })? {
        writeln!(text, "p{}", row?).map_err(|e| CoreError::storage(e.to_string()))?;
    }
    let mut stmt = conn.prepare(
        "SELECT author, sign_pub, counter, coalesce(gap_from, 0) FROM oplog_authors ORDER BY author, sign_pub",
    )?;
    for row in stmt.query_map([], |r| {
        Ok(serde_json::json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?
        ]))
    })? {
        writeln!(text, "a{}", row?).map_err(|e| CoreError::storage(e.to_string()))?;
    }
    let mut stmt =
        conn.prepare("SELECT author, sign_pub, counter, seq, op FROM oplog_held ORDER BY author, sign_pub, counter")?;
    for row in stmt.query_map([], |r| {
        Ok(serde_json::json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?
        ]))
    })? {
        writeln!(text, "h{}", row?).map_err(|e| CoreError::storage(e.to_string()))?;
    }
    Ok(reins_proto::canon::sha256(text.as_bytes()))
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
