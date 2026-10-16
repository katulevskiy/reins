//! Checkpoints (the primary's sealed, signed snapshot of the op-log store at a `seq`) and heartbeats (its signed
//! "I applied up to here").
//!
//! A checkpoint's body is sealed with the newest view key, with its signed envelope (account, epoch, author, `seq`,
//! applied hash, time) as associated data, so the body opens only as the checkpoint its signature covers. Only the
//! primary of the newest roster entry this phone verified, signing at that entry's epoch, is taken: a replaced primary
//! writes no checkpoint anyone loads. Loading one compares its applied hash with this phone's own at that `seq` (a
//! difference is a fork), and its state digest ([`super::state_digest`]) with what it holds once loaded.
//!
//! Only a heartbeat from the current primary at the current epoch counts: for liveness, and as proof of a fork (an
//! applied hash other than this phone's). Another primary's that differs is a warning, never a halt.

use reins_proto::canon::{Canon, decode_payload, encode_payload, hex32, parse_hex32, parse_sig};
use reins_proto::oplog::{Checkpoint, Heartbeat, Table};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::{
    Context, Halt, LogState, RECORD_COLUMNS, Version, applied_at, as_i64, as_u64, fork, record_from_row, record_of,
    save_state, state, state_digest, table_name,
};
use crate::CoreError;
use crate::roles::keys::{SigningKey, contexts, ed25519_verify, sealed_kid};

#[derive(Serialize, Deserialize)]
pub(super) struct RecordRow {
    table: Table,
    pub(super) key: String,
    value: Option<serde_json::Value>,
    version: Version,
    seq: u64,
    at: i64,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ProposalRow {
    key: String,
    author: String,
    value: Option<serde_json::Value>,
    seq: u64,
    received_at: i64,
}

#[derive(Serialize, Deserialize)]
pub(super) struct AuthorRow {
    author: String,
    sign_pub: String,
    counter: u64,
    gap_from: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct HeldRow {
    author: String,
    sign_pub: String,
    counter: u64,
    seq: u64,
    op: String,
}

/// What a checkpoint's body holds: the whole store at `seq`, and its state digest.
#[derive(Serialize, Deserialize)]
pub(super) struct Body {
    pub(super) v: u32,
    pub(super) seq: u64,
    pub(super) applied_hash: String,
    pub(super) log_epoch: u64,
    /// [`state_digest`] of the store this body holds, hex.
    pub(super) state_hash: String,
    pub(super) records: Vec<RecordRow>,
    pub(super) proposals: Vec<ProposalRow>,
    pub(super) authors: Vec<AuthorRow>,
    #[serde(default)]
    pub(super) held: Vec<HeldRow>,
}

/// The associated data a checkpoint's body is sealed with: its signed envelope but the body.
fn body_aad(cp: &Checkpoint) -> Result<Vec<u8>, CoreError> {
    let applied = parse_hex32(&cp.applied_hash).ok_or_else(|| CoreError::invalid("invalid checkpoint"))?;
    let mut c = Canon::new(contexts::CHECKPOINT_BODY);
    c.u32(cp.v).str(&cp.account).u64(cp.epoch).str(&cp.author).u64(cp.seq).key(&applied).i64(cp.at);
    Ok(c.finish())
}

fn json_of(value: Option<String>, what: &str) -> Result<Option<serde_json::Value>, CoreError> {
    value.map(|v| serde_json::from_str(&v)).transpose().map_err(|_| CoreError::storage(format!("corrupt {what}")))
}

pub(super) fn dump(conn: &Connection, s: &LogState) -> Result<Body, CoreError> {
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
                value: json_of(value, "proposal")?,
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
    let mut stmt =
        conn.prepare("SELECT author, sign_pub, counter, seq, op FROM oplog_held ORDER BY author, counter")?;
    let held = stmt
        .query_map([], |r| {
            Ok(HeldRow {
                author: r.get(0)?,
                sign_pub: r.get(1)?,
                counter: as_u64(r.get(2)?),
                seq: as_u64(r.get(3)?),
                op: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Body {
        v: reins_proto::PROTOCOL_VERSION,
        seq: s.seq,
        applied_hash: hex32(&s.applied_hash),
        log_epoch: s.log_epoch,
        state_hash: hex32(&state_digest(conn)?),
        records,
        proposals,
        authors,
        held,
    })
}

/// The primary's checkpoint of everything up to the last applied op, signed by `signer` as `author` at `epoch`.
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
    sign_body(ctx, signer, author, epoch, &dump(conn, &s)?)
}

/// `body` sealed and signed as a checkpoint.
pub(super) fn sign_body(
    ctx: &Context<'_>,
    signer: &SigningKey,
    author: &str,
    epoch: u64,
    body: &Body,
) -> Result<Checkpoint, CoreError> {
    let plain = serde_json::to_vec(body).map_err(|e| CoreError::storage(e.to_string()))?;
    let mut cp = Checkpoint {
        v: reins_proto::PROTOCOL_VERSION,
        account: ctx.account.to_owned(),
        epoch,
        author: author.to_owned(),
        seq: body.seq,
        applied_hash: body.applied_hash.clone(),
        at: ctx.now,
        body: String::new(),
        sig: String::new(),
    };
    cp.body = encode_payload(&ctx.keyring.seal(&body_aad(&cp)?, &plain)?);
    cp.sig = signer.sign_b64(&cp.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
    Ok(cp)
}

/// What [`load`] made of a checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointCheck {
    /// It was ahead of this phone: the store is the checkpoint's now.
    Loaded,
    /// This phone is at or past it, and saw the same history up to it (at its very `seq`, the same state too).
    Matches,
    /// This phone is past it and no longer keeps its hash at that `seq`: nothing to compare.
    Unknown,
}

/// The signing key of the primary of the newest roster entry this phone verified, when `author` at `epoch` is it:
/// the only signer whose checkpoints are loaded and whose heartbeats count.
fn current_primary_key(ctx: &Context<'_>, epoch: u64, author: &str) -> Result<Option<[u8; 32]>, Halt> {
    let current = ctx.roster.current();
    if epoch > current.epoch {
        return Err(Halt::UnknownEpoch(epoch));
    }
    Ok((epoch == current.epoch && current.primary.device_id == author)
        .then(|| current.primary.sign_key().ok())
        .flatten())
}

/// The key of the primary this phone's recovery replaced, when `author` at `epoch` is it at the epoch right before
/// that recovery entry (the only checkpoint a fresh recovered phone may start from, as untrusted).
fn predecessor_key(ctx: &Context<'_>, epoch: u64, author: &str) -> Option<[u8; 32]> {
    let entries = &ctx.roster.entries;
    let current = ctx.roster.current();
    let i = entries.iter().rposition(|e| {
        e.reason == reins_proto::roster::RosterReason::Recovery && e.primary.same_keys(&current.primary)
    })?;
    let before = entries.get(i.checked_sub(1)?)?;
    (before.epoch == epoch && before.primary.device_id == author).then(|| before.primary.sign_key().ok()).flatten()
}

/// The signing key of the primary at `epoch` (any epoch this phone verified), for a heartbeat's fork evidence.
fn primary_key_at(ctx: &Context<'_>, epoch: u64, author: &str) -> Result<Option<[u8; 32]>, Halt> {
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
    load_from(conn, ctx, cp, Start::Current)
}

/// [`load`] for a phone with no log state at all that verifies the current primary's history: a checkpoint signed with
/// the current primary's key at an earlier epoch of its tenure (it was primary, with that key, at every epoch since) is
/// taken too, so that a roster change the primary wrote no checkpoint after does not lock a recovery out. What it loads
/// is untrusted all the same (the caller tightens it).
pub fn load_tenure(conn: &mut Connection, ctx: &Context<'_>, cp: &Checkpoint) -> Result<CheckpointCheck, Halt> {
    load_from(conn, ctx, cp, Start::Tenure)
}

/// Which checkpoints a load takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    /// The current primary's, at the current epoch.
    Current,
    /// On a phone with nothing: also the current primary's from an earlier epoch of its tenure.
    Tenure,
    /// On a phone with nothing that recovered: also the replaced primary's, right before the recovery.
    Bootstrap,
}

/// The current primary's key, when `author` at `epoch` is it and it was primary with that key at every epoch since.
fn tenure_key(ctx: &Context<'_>, epoch: u64, author: &str) -> Option<[u8; 32]> {
    let current = ctx.roster.current();
    let since: Vec<_> = ctx.roster.entries.iter().filter(|e| e.epoch >= epoch).collect();
    let held =
        since.first().is_some_and(|e| e.epoch == epoch) && since.iter().all(|e| e.primary.same_keys(&current.primary));
    (held && current.primary.device_id == author).then(|| current.primary.sign_key().ok()).flatten()
}

/// [`load`] for a primary that has no log state at all (a fresh phone that recovered the account) and so can verify
/// no checkpoint of the current epoch, which it would write itself: it takes one signed by the primary of the
/// checkpoint's epoch (any epoch it verified). A replaced primary's key could sign one; only a phone that has nothing
/// else uses it, and it then writes its own checkpoint.
pub fn load_bootstrap(conn: &mut Connection, ctx: &Context<'_>, cp: &Checkpoint) -> Result<CheckpointCheck, Halt> {
    load_from(conn, ctx, cp, Start::Bootstrap)
}

fn load_from(conn: &mut Connection, ctx: &Context<'_>, cp: &Checkpoint, start: Start) -> Result<CheckpointCheck, Halt> {
    let bad = |why: &str| Halt::Core(CoreError::invalid(format!("The checkpoint is refused: {why}.")));
    cp.validate().map_err(|e| bad(&e.to_string()))?;
    if cp.account != ctx.account {
        return Err(bad("it is another account's"));
    }
    let s0 = state(conn)?;
    let fresh = s0.seq == 0
        && s0.forked.is_none()
        && conn.query_row("SELECT count(*) FROM oplog_records", [], |r| r.get::<_, i64>(0))? == 0;
    let key = match current_primary_key(ctx, cp.epoch, &cp.author)? {
        Some(key) => key,
        None if start == Start::Bootstrap && fresh => predecessor_key(ctx, cp.epoch, &cp.author)
            .ok_or_else(|| bad("it is not the replaced primary's own, at the epoch right before the recovery"))?,
        None if start == Start::Tenure && fresh => tenure_key(ctx, cp.epoch, &cp.author)
            .ok_or_else(|| bad("it is not the current primary's own, from its tenure"))?,
        None => return Err(bad("it is not from the current primary at the current epoch")),
    };
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
    if cp.seq < s.seq {
        return match applied_at(conn, cp.seq)? {
            Some(mine) if mine == applied => Ok(CheckpointCheck::Matches),
            Some(_) => Err(fork(conn, &format!("the checkpoint at seq {} saw another history", cp.seq))),
            None => Ok(CheckpointCheck::Unknown),
        };
    }
    if cp.epoch < s.log_epoch {
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
    let digest = parse_hex32(&body.state_hash).ok_or_else(|| bad("its state hash is malformed"))?;
    if cp.seq == s.seq {
        // The same seq: the same history, and so the same state, or they forked.
        if s.applied_hash != applied {
            return Err(fork(conn, &format!("the checkpoint at seq {} saw another history", cp.seq)));
        }
        if state_digest(conn)? != digest {
            return Err(fork(conn, &format!("the checkpoint at seq {} holds another state", cp.seq)));
        }
        return Ok(CheckpointCheck::Matches);
    }
    let tx = conn.transaction()?;
    tx.execute_batch(
        "DELETE FROM oplog_records; DELETE FROM oplog_proposals; DELETE FROM oplog_authors; DELETE FROM oplog_held;",
    )?;
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
    for h in &body.held {
        tx.execute(
            "INSERT OR REPLACE INTO oplog_held (author, sign_pub, counter, seq, op) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![h.author, h.sign_pub, as_i64(h.counter), as_i64(h.seq), h.op],
        )?;
    }
    // The state the body holds must be the one its digest names (else nothing is kept: the transaction goes).
    if state_digest(&tx)? != digest {
        return Err(bad("its records are not the state it names"));
    }
    let loaded = LogState {
        seq: cp.seq,
        applied_hash: applied,
        log_epoch: body.log_epoch.max(cp.epoch).max(s.log_epoch),
        forked: None,
    };
    save_state(&tx, &loaded)?;
    tx.execute("DELETE FROM oplog_seqs", [])?;
    // No op hash for the checkpoint's seq (''): a later copy of that op is a duplicate, never a fork.
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
    /// Not from the current primary at the current epoch, and nothing it says differs from this phone: no news.
    Stale,
    /// Not from the current primary at the current epoch, and it names another history at a `seq` this phone applied.
    /// A warning for the user ("your devices may disagree"), never a halt and never kept: a replaced primary's key
    /// (a stolen old phone) must not stop every phone.
    Disagrees,
}

/// Checks a heartbeat: signed by the primary of its epoch (as this phone verified the roster). From the current primary
/// at the current epoch, an applied hash other than this phone's at that `seq` is a fork, and applying stops for good.
/// From anyone else (a replaced primary, an older epoch) it is only [`HeartbeatCheck::Disagrees`].
pub fn check_heartbeat(conn: &Connection, ctx: &Context<'_>, hb: &Heartbeat) -> Result<HeartbeatCheck, Halt> {
    let bad = |why: &str| Halt::Core(CoreError::invalid(format!("The heartbeat is refused: {why}.")));
    hb.validate().map_err(|e| bad(&e.to_string()))?;
    if hb.account != ctx.account {
        return Err(bad("it is another account's"));
    }
    let key = primary_key_at(ctx, hb.epoch, &hb.author)?.ok_or_else(|| bad("its author was not the primary"))?;
    let message = hb.signing_bytes().map_err(|e| bad(&e.to_string()))?;
    let sig = parse_sig(&hb.sig).ok_or_else(|| bad("not a signature"))?;
    if !ed25519_verify(&key, &message, &sig) {
        return Err(bad("the primary's signature does not verify"));
    }
    let s = state(conn)?;
    if let Some(why) = &s.forked {
        return Err(Halt::Forked(why.clone()));
    }
    let current = current_primary_key(ctx, hb.epoch, &hb.author)?.is_some() && hb.epoch >= s.log_epoch;
    let theirs = parse_hex32(&hb.applied_hash).ok_or_else(|| bad("invalid applied hash"))?;
    let compared = if hb.head_seq > s.seq {
        HeartbeatCheck::Behind
    } else {
        match applied_at(conn, hb.head_seq)? {
            Some(mine) if mine == theirs => HeartbeatCheck::Matches,
            Some(_) if current => {
                return Err(fork(conn, &format!("the primary saw another history up to seq {}", hb.head_seq)));
            }
            Some(_) => return Ok(HeartbeatCheck::Disagrees),
            None => HeartbeatCheck::Unknown,
        }
    };
    Ok(if current {
        compared
    } else {
        HeartbeatCheck::Stale
    })
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
        last_counter: Some(super::author_last_counter(conn, author, &signer.public_b64())?),
        sig: String::new(),
    };
    hb.sig = signer.sign_b64(&hb.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
    Ok(hb)
}
