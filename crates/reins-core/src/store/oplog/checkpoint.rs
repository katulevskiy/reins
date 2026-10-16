//! Checkpoints (the primary's sealed, signed snapshot of the op-log store at a `seq`) and heartbeats (its signed
//! "I applied up to here").
//!
//! A checkpoint's body is sealed with the newest view key, with its signed envelope (account, epoch, author, `seq`,
//! applied hash, time) as associated data, so the body opens only as the checkpoint its signature covers. Loading one
//! checks the primary's signature at its epoch against the verified roster, refuses one older than the log already
//! showed (epoch floor), and compares its applied hash with this phone's own at that `seq`: a difference is a fork.

use reins_proto::canon::{Canon, decode_payload, encode_payload, hex32, parse_hex32, parse_sig};
use reins_proto::oplog::{Checkpoint, Heartbeat, Table};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::{
    Context, Halt, LogState, RECORD_COLUMNS, Version, applied_at, as_i64, as_u64, fork, record_from_row, record_of,
    save_state, state, table_name,
};
use crate::CoreError;
use crate::roles::keys::{SigningKey, contexts, ed25519_verify, sealed_kid};
use crate::store::AUDIT_RETENTION;

#[derive(Serialize, Deserialize)]
struct RecordRow {
    table: Table,
    key: String,
    value: Option<serde_json::Value>,
    version: Version,
    seq: u64,
    at: i64,
}

#[derive(Serialize, Deserialize)]
struct ProposalRow {
    key: String,
    author: String,
    value: Option<serde_json::Value>,
    seq: u64,
    received_at: i64,
}

#[derive(Serialize, Deserialize)]
struct AuthorRow {
    author: String,
    sign_pub: String,
    counter: u64,
    gap_from: Option<u64>,
}

/// What a checkpoint's body holds: the whole store at `seq`.
#[derive(Serialize, Deserialize)]
struct Body {
    v: u32,
    seq: u64,
    applied_hash: String,
    log_epoch: u64,
    records: Vec<RecordRow>,
    proposals: Vec<ProposalRow>,
    authors: Vec<AuthorRow>,
}

/// The associated data a checkpoint's body is sealed with: its signed envelope but the body.
fn body_aad(cp: &Checkpoint) -> Result<Vec<u8>, CoreError> {
    let applied = parse_hex32(&cp.applied_hash).ok_or_else(|| CoreError::invalid("invalid checkpoint"))?;
    let mut c = Canon::new(contexts::CHECKPOINT_BODY);
    c.u32(cp.v).str(&cp.account).u64(cp.epoch).str(&cp.author).u64(cp.seq).key(&applied).i64(cp.at);
    Ok(c.finish())
}

/// Keeps the newest [`AUDIT_RETENTION`] Activity entries (by the op that wrote them).
fn trim_audit(conn: &Connection) -> Result<(), CoreError> {
    conn.execute(
        "DELETE FROM oplog_records WHERE tbl = ?1 AND key NOT IN
             (SELECT key FROM oplog_records WHERE tbl = ?1 ORDER BY seq DESC LIMIT ?2)",
        params![table_name(Table::Audit), AUDIT_RETENTION],
    )?;
    Ok(())
}

fn dump(conn: &Connection, s: &LogState) -> Result<Body, CoreError> {
    let mut stmt = conn.prepare(&format!("SELECT {RECORD_COLUMNS} FROM oplog_records ORDER BY tbl, key"))?;
    let raws = stmt.query_map([], record_from_row)?.collect::<Result<Vec<_>, _>>()?;
    let records = raws
        .into_iter()
        .map(|raw| {
            let r = record_of(raw)?;
            Ok(RecordRow {
                table: r.table,
                key: r.key,
                value: r.value,
                version: r.version,
                seq: r.seq,
                at: r.at,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let mut stmt = conn.prepare("SELECT key, author, value, seq, received_at FROM oplog_proposals ORDER BY key")?;
    let proposals = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<Vec<(String, String, Option<String>, i64, i64)>, _>>()?
        .into_iter()
        .map(|(key, author, value, seq, received_at)| {
            Ok(ProposalRow {
                key,
                author,
                value: value
                    .map(|v| serde_json::from_str(&v))
                    .transpose()
                    .map_err(|_| CoreError::storage("corrupt proposal"))?,
                seq: as_u64(seq),
                received_at,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let mut stmt = conn.prepare("SELECT author, sign_pub, counter, gap_from FROM oplog_authors ORDER BY author")?;
    let authors = stmt
        .query_map([], |r| {
            Ok(AuthorRow {
                author: r.get(0)?,
                sign_pub: r.get(1)?,
                counter: as_u64(r.get(2)?),
                gap_from: r.get::<_, Option<i64>>(3)?.map(as_u64),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Body {
        v: reins_proto::PROTOCOL_VERSION,
        seq: s.seq,
        applied_hash: hex32(&s.applied_hash),
        log_epoch: s.log_epoch,
        records,
        proposals,
        authors,
    })
}

/// The primary's checkpoint of everything up to the last applied op (Activity trimmed to its retention first),
/// signed by `signer` as `author` at `epoch`.
pub fn produce(
    conn: &Connection,
    ctx: &Context<'_>,
    signer: &SigningKey,
    author: &str,
    epoch: u64,
) -> Result<Checkpoint, CoreError> {
    let s = state(conn)?;
    if s.forked.is_some() {
        return Err(CoreError::invalid("the account's history forked; no checkpoint is made"));
    }
    trim_audit(conn)?;
    let body = serde_json::to_vec(&dump(conn, &s)?).map_err(|e| CoreError::storage(e.to_string()))?;
    let mut cp = Checkpoint {
        v: reins_proto::PROTOCOL_VERSION,
        account: ctx.account.to_owned(),
        epoch,
        author: author.to_owned(),
        seq: s.seq,
        applied_hash: hex32(&s.applied_hash),
        at: ctx.now,
        body: String::new(),
        sig: String::new(),
    };
    cp.body = encode_payload(&ctx.keyring.seal(&body_aad(&cp)?, &body)?);
    cp.sig = signer.sign_b64(&cp.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
    Ok(cp)
}

/// What [`load`] made of a checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointCheck {
    /// It was ahead of this phone: the store is the checkpoint's now.
    Loaded,
    /// This phone is at or past it, and saw the same history up to it.
    Matches,
    /// This phone is past it and no longer keeps its hash at that `seq`: nothing to compare.
    Unknown,
}

/// The primary that signed at `epoch` (the roster entry this phone verified), or why it cannot be checked.
fn primary_key(ctx: &Context<'_>, epoch: u64, author: &str) -> Result<Option<[u8; 32]>, Halt> {
    if epoch > ctx.roster.epoch() {
        return Err(Halt::UnknownEpoch(epoch));
    }
    let Some(entry) = ctx.roster.at(epoch) else {
        return Err(Halt::Unverifiable(epoch));
    };
    Ok((entry.primary.device_id == author).then(|| entry.primary.sign_key().ok()).flatten())
}

/// Checks a checkpoint (from the server) and, when it is ahead of this phone, loads it in place of the store.
pub fn load(conn: &mut Connection, ctx: &Context<'_>, cp: &Checkpoint) -> Result<CheckpointCheck, Halt> {
    let bad = |why: &str| Halt::Core(CoreError::invalid(format!("The checkpoint is refused: {why}.")));
    cp.validate().map_err(|e| bad(&e.to_string()))?;
    if cp.account != ctx.account {
        return Err(bad("it is another account's"));
    }
    let key = primary_key(ctx, cp.epoch, &cp.author)?.ok_or_else(|| bad("its author was not the primary"))?;
    let message = cp.signing_bytes().map_err(|e| bad(&e.to_string()))?;
    let sig = parse_sig(&cp.sig).ok_or_else(|| bad("not a signature"))?;
    if !ed25519_verify(&key, &message, &sig) {
        return Err(bad("the primary's signature does not verify"));
    }
    let s = state(conn)?;
    if let Some(why) = &s.forked {
        return Err(Halt::Forked(why.clone()));
    }
    let applied = parse_hex32(&cp.applied_hash).ok_or_else(|| bad("invalid applied hash"))?;
    if cp.seq <= s.seq {
        return match applied_at(conn, cp.seq)? {
            Some(mine) if mine == applied => Ok(CheckpointCheck::Matches),
            Some(_) => Err(fork(conn, &format!("the checkpoint at seq {} saw another history", cp.seq))),
            None => Ok(CheckpointCheck::Unknown),
        };
    }
    if cp.epoch < s.log_epoch {
        // Ahead in seq, behind in epoch: the log already showed a later roster than its signer's.
        return Err(Halt::Rollback);
    }
    let sealed = decode_payload(&cp.body).ok_or_else(|| bad("its body is malformed"))?;
    let kid = sealed_kid(&sealed).unwrap_or(0);
    if ctx.keyring.get(kid).is_none() {
        return Err(Halt::MissingKey(kid));
    }
    let plain = ctx.keyring.open(&body_aad(cp)?, &sealed).map_err(|_| bad("its body does not open"))?;
    let body: Body = serde_json::from_slice(&plain).map_err(|_| bad("its body is malformed"))?;
    if body.v != reins_proto::PROTOCOL_VERSION || body.seq != cp.seq || body.applied_hash != cp.applied_hash {
        return Err(bad("its body is for another seq"));
    }
    let tx = conn.transaction()?;
    tx.execute_batch("DELETE FROM oplog_records; DELETE FROM oplog_proposals; DELETE FROM oplog_authors;")?;
    for r in &body.records {
        let value = r.value.as_ref().map(serde_json::to_string).transpose().map_err(|e| bad(&e.to_string()))?;
        tx.execute(
            "INSERT OR REPLACE INTO oplog_records (tbl, key, value, epoch, author, counter, seq, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                table_name(r.table),
                r.key,
                value,
                as_i64(r.version.epoch),
                r.version.author,
                as_i64(r.version.counter),
                as_i64(r.seq),
                r.at
            ],
        )?;
    }
    for p in &body.proposals {
        let value = p.value.as_ref().map(serde_json::to_string).transpose().map_err(|e| bad(&e.to_string()))?;
        tx.execute(
            "INSERT OR REPLACE INTO oplog_proposals (key, author, value, seq, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![p.key, p.author, value, as_i64(p.seq), p.received_at],
        )?;
    }
    for a in &body.authors {
        tx.execute(
            "INSERT OR REPLACE INTO oplog_authors (author, sign_pub, counter, gap_from) VALUES (?1, ?2, ?3, ?4)",
            params![a.author, a.sign_pub, as_i64(a.counter), a.gap_from.map(as_i64)],
        )?;
    }
    let loaded = LogState {
        seq: cp.seq,
        applied_hash: applied,
        log_epoch: body.log_epoch.max(cp.epoch).max(s.log_epoch),
        forked: None,
    };
    save_state(&tx, &loaded)?;
    tx.execute("DELETE FROM oplog_seqs", [])?;
    tx.execute(
        "INSERT INTO oplog_seqs (seq, op_hash, applied_hash) VALUES (?1, '', ?2)",
        params![as_i64(cp.seq), cp.applied_hash],
    )?;
    tx.commit()?;
    Ok(CheckpointCheck::Loaded)
}

/// What a heartbeat says about this phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeartbeatCheck {
    /// The primary applied what this phone did up to its head: the same history.
    Matches,
    /// The primary is ahead of this phone: fetch more.
    Behind,
    /// This phone no longer keeps its hash at that `seq`.
    Unknown,
    /// Signed at an epoch older than the log already showed: ignored.
    Stale,
}

/// Checks the primary's heartbeat: signed by the primary of its epoch (as this phone verified the roster), and, at
/// a `seq` this phone applied, the same applied hash (else the history forked, and applying stops).
pub fn check_heartbeat(conn: &Connection, ctx: &Context<'_>, hb: &Heartbeat) -> Result<HeartbeatCheck, Halt> {
    let bad = |why: &str| Halt::Core(CoreError::invalid(format!("The heartbeat is refused: {why}.")));
    hb.validate().map_err(|e| bad(&e.to_string()))?;
    if hb.account != ctx.account {
        return Err(bad("it is another account's"));
    }
    let key = primary_key(ctx, hb.epoch, &hb.author)?.ok_or_else(|| bad("its author was not the primary"))?;
    let message = hb.signing_bytes().map_err(|e| bad(&e.to_string()))?;
    let sig = parse_sig(&hb.sig).ok_or_else(|| bad("not a signature"))?;
    if !ed25519_verify(&key, &message, &sig) {
        return Err(bad("the primary's signature does not verify"));
    }
    let s = state(conn)?;
    if let Some(why) = &s.forked {
        return Err(Halt::Forked(why.clone()));
    }
    if hb.epoch < s.log_epoch {
        return Ok(HeartbeatCheck::Stale);
    }
    if hb.head_seq > s.seq {
        return Ok(HeartbeatCheck::Behind);
    }
    let theirs = parse_hex32(&hb.applied_hash).ok_or_else(|| bad("invalid applied hash"))?;
    match applied_at(conn, hb.head_seq)? {
        Some(mine) if mine == theirs => Ok(HeartbeatCheck::Matches),
        Some(_) => Err(fork(conn, &format!("the primary saw another history up to seq {}", hb.head_seq))),
        None => Ok(HeartbeatCheck::Unknown),
    }
}

/// The primary's heartbeat for where this phone's log stands.
pub fn heartbeat(
    conn: &Connection,
    ctx: &Context<'_>,
    signer: &SigningKey,
    author: &str,
    epoch: u64,
) -> Result<Heartbeat, CoreError> {
    let s = state(conn)?;
    let mut hb = Heartbeat {
        v: reins_proto::PROTOCOL_VERSION,
        account: ctx.account.to_owned(),
        epoch,
        author: author.to_owned(),
        head_seq: s.seq,
        applied_hash: hex32(&s.applied_hash),
        t: ctx.now,
        sig: String::new(),
    };
    hb.sig = signer.sign_b64(&hb.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
    Ok(hb)
}
