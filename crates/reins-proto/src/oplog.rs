//! Live sync between an account's phones: a per-account log of signed, sealed ops, checkpoints and the primary's
//! heartbeat. The server stores ciphertext only and checks who may write what ([`authorize_op`]); phones verify the
//! same rules, then apply each op per record.
//!
//! - **Ops** ([`Op`]): one change of one record ([`OpBody`]), sealed with the view key (`body`, opaque to the server),
//!   signed by its author's device key. Each author numbers its ops 1, 2, 3, ... (`counter`); the server refuses a
//!   counter that does not rise and assigns each accepted op the account's next `seq`.
//!
//! Who may write what is checked twice. The server, which cannot open bodies, checks the signed envelope against the
//! roster ([`authorize_op`]: the author's role comes from the roster, never from the op). Every phone checks it again,
//! then opens the body (sealed with the envelope as associated data, [`body_associated_data`]) and refuses a change the
//! op's signed class does not allow ([`check_change`]): a restrictive op carries only a typed [`Restriction`], which
//! can only tighten.
//! - **Checkpoints** ([`Checkpoint`]): the primary's sealed snapshot of everything up to `seq`, with
//!   [`applied_hash`](applied_hash_step) of the ops it contains. The server keeps the latest two and drops the ops up
//!   to the older one.
//! - **Heartbeats** ([`Heartbeat`]): the primary says, about every [`HEARTBEAT_EVERY_SECS`], what it has applied, so a
//!   viewer can tell "live" from "last seen" and a server that withholds ops shows.
//!
//! Endpoints (server, `/reins/api`): `POST /ops` ([`OpBatch`] → [`OpsAppended`]), `GET /sync?after=&seen=&wait=`
//! ([`SyncResponse`]), `POST /ack` ([`Ack`]), `PUT`/`GET /checkpoint` ([`Checkpoint`]), `POST /heartbeat`
//! ([`Heartbeat`]).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ValidationError,
    canon::{Canon, contexts, decode_payload, parse_hex32, parse_sig, sha256},
    device::valid_device_id,
    join::JoinRequest,
    pairing::PairingRequest,
    relay::RelayRequest,
    roster::{HandoverRequest, MAX_ACCOUNT_CHARS, Member, RosterEntry, previous_primary},
    validate::invalid,
};

/// Largest op body (decoded bytes), for the primary's ops; see [`OpClass::max_body_bytes`].
pub const MAX_OP_BODY_BYTES: usize = 64 * 1024;
/// Largest body of a proposal (a connection, a credential or a vault item, sealed).
pub const MAX_PROPOSAL_BODY_BYTES: usize = 16 * 1024;
/// Largest body of a restrictive op (a typed [`Restriction`], sealed).
pub const MAX_RESTRICTIVE_BODY_BYTES: usize = 1024;
/// Ops in one `POST /ops`.
pub const MAX_OPS_PER_BATCH: usize = 100;
/// Ops in one `GET /sync` answer; `more` says when there are others after them.
pub const MAX_SYNC_OPS: usize = 500;
/// Largest checkpoint body (decoded bytes). Base64 and JSON make it about 11 MiB, under the 16 MiB `max_allowed_packet`
/// MariaDB and older MySQL servers default to.
pub const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
/// How often the primary posts a heartbeat while it polls (seconds); the server takes at most one per
/// [`MIN_HEARTBEAT_SECS`].
pub const HEARTBEAT_EVERY_SECS: i64 = 30;
pub const MIN_HEARTBEAT_SECS: i64 = 10;
/// A viewer shows "Live" while the primary's latest heartbeat is younger than this (seconds).
pub const LIVE_SECS: i64 = 120;
/// How far a heartbeat's time may be from the server's (seconds).
pub const HEARTBEAT_SKEW_SECS: i64 = 300;
/// The running hash before any op.
pub const GENESIS_APPLIED_HASH: [u8; 32] = [0; 32];

/// What an op may do, and so who may write it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpClass {
    /// A mode, grant, rule, setting or record change, or a removal: the primary only.
    Primary,
    /// A suggestion the primary keeps or discards (a viewer adding a connection): any member; inert until resolved.
    Proposal,
    /// A change that can only tighten (Lockdown, stop Bypass, revoke), applied by the restrictive join: any member of
    /// the current roster, and the primary before the current one (its late ops).
    Restrictive,
}

impl OpClass {
    /// The largest body of this class (decoded bytes): only the primary writes large records.
    #[must_use]
    pub const fn max_body_bytes(self) -> usize {
        match self {
            Self::Primary => MAX_OP_BODY_BYTES,
            Self::Proposal => MAX_PROPOSAL_BODY_BYTES,
            Self::Restrictive => MAX_RESTRICTIVE_BODY_BYTES,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Proposal => "proposal",
            Self::Restrictive => "restrictive",
        }
    }
}

/// One signed op.
///
/// Signed message ([`crate::canon`], context `reins/op/v1`): `v` (u32), `account` (string), `epoch` (u64), `author`
/// (string), `counter` (u64), `class` (string), `notify` (bool), SHA-256 of the decoded `body` (32 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub v: u32,
    /// The server's id of the account.
    pub account: String,
    /// The roster epoch the author signed at.
    pub epoch: u64,
    /// The author's device id (a member at `epoch`).
    pub author: String,
    /// 1 for the author's first op, then +1 (the server only requires that it rises).
    pub counter: u64,
    pub class: OpClass,
    /// Viewers get an alert push (a mode change, Bypass on or off, a new connection or credential), else a silent one.
    #[serde(default)]
    pub notify: bool,
    /// The change sealed with the view key, base64url without padding; at most [`MAX_OP_BODY_BYTES`] decoded.
    pub body: String,
    /// Ed25519 by the author's `sign_pub` at `epoch`, base64url.
    pub sig: String,
}

fn check_account(account: &str) -> Result<(), ValidationError> {
    if account.is_empty() || account.chars().count() > MAX_ACCOUNT_CHARS || account.chars().any(char::is_control) {
        return Err(invalid("account", format!("must be 1 to {MAX_ACCOUNT_CHARS} characters")));
    }
    Ok(())
}

fn check_author(author: &str) -> Result<(), ValidationError> {
    if valid_device_id(author) {
        Ok(())
    } else {
        Err(invalid("author", "must be a device id"))
    }
}

fn body_hash(field: &'static str, body: &str, max: usize) -> Result<[u8; 32], ValidationError> {
    let raw = decode_payload(body).ok_or_else(|| invalid(field, "must be base64url without padding"))?;
    if raw.is_empty() || raw.len() > max {
        return Err(invalid(field, format!("must be 1 to {max} bytes")));
    }
    Ok(sha256(&raw))
}

impl Op {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        check_account(&self.account)?;
        check_author(&self.author)?;
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        if self.counter == 0 {
            return Err(invalid("counter", "starts at 1"));
        }
        body_hash("body", &self.body, self.class.max_body_bytes())?;
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(())
    }

    /// The bytes the author signs (see the type's docs).
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        let mut c = Canon::new(contexts::OP);
        c.u32(self.v)
            .str(&self.account)
            .u64(self.epoch)
            .str(&self.author)
            .u64(self.counter)
            .str(self.class.as_str())
            .bool(self.notify)
            .key(&body_hash("body", &self.body, MAX_OP_BODY_BYTES)?);
        Ok(c.finish())
    }

    /// SHA-256 of [`Op::signing_bytes`]: what [`applied_hash_step`] chains.
    pub fn hash(&self) -> Result<[u8; 32], ValidationError> {
        Ok(sha256(&self.signing_bytes()?))
    }
}

/// One step of the running hash of applied ops, in `seq` order from [`GENESIS_APPLIED_HASH`]: SHA-256 over context
/// `reins/applied/v1`, the previous hash (32 bytes), `seq` (u64) and [`Op::hash`] (32 bytes). Checkpoints and
/// heartbeats carry the result; two phones (or a phone and a checkpoint) with the same `seq` and different hashes saw
/// different histories.
pub fn applied_hash_step(prev: &[u8; 32], seq: u64, op: &Op) -> Result<[u8; 32], ValidationError> {
    let mut c = Canon::new(contexts::APPLIED);
    c.key(prev).u64(seq).key(&op.hash()?);
    Ok(sha256(&c.finish()))
}

/// Why an op may not be written by its author.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum OpRefusal {
    #[error("the op is for another account")]
    WrongAccount,
    /// The roster has no entry at the op's epoch (not yet, or never).
    #[error("the roster has no entry at the op's epoch")]
    UnknownEpoch,
    /// The author is not in the roster at the op's epoch, or not now with the same key.
    #[error("the author is not a member of the roster")]
    NotAMember,
    #[error("only the primary writes this class of op")]
    NotPrimary,
}

/// Who may write `op` given the roster `entries` (ascending by epoch, ending with the current entry):
///
/// - it names an epoch the roster has, and its author is a member there (whose `sign_pub` must have signed it);
/// - `primary`: the author is primary at that epoch and is the current primary, with the same key;
/// - `proposal`: the author is a current member with the same key;
/// - `restrictive`: the author is a current member with the same key (viewers may pull the brake), or it was primary
///   at that epoch and is the primary before the current one ([`previous_primary`]): a replaced primary's late ops
///   can only tighten.
///
/// Returns the member whose key must verify the signature.
pub fn authorize_op<'a>(op: &Op, entries: &'a [RosterEntry]) -> Result<&'a Member, OpRefusal> {
    let current = entries.last().ok_or(OpRefusal::UnknownEpoch)?;
    if op.account != current.account {
        return Err(OpRefusal::WrongAccount);
    }
    let at = entries.iter().rev().find(|e| e.epoch == op.epoch).ok_or(OpRefusal::UnknownEpoch)?;
    let (role_then, signer) = at.role_of(&op.author).ok_or(OpRefusal::NotAMember)?;
    let now = current.member(&op.author).filter(|m| m.sign_pub == signer.sign_pub);
    let primary_then = role_then == crate::roster::Role::Primary;
    match op.class {
        OpClass::Primary => {
            if primary_then && current.primary.device_id == op.author && current.primary.sign_pub == signer.sign_pub {
                Ok(signer)
            } else if now.is_some() {
                Err(OpRefusal::NotPrimary)
            } else {
                Err(OpRefusal::NotAMember)
            }
        }
        OpClass::Proposal => now.map(|_| signer).ok_or(OpRefusal::NotAMember),
        OpClass::Restrictive => {
            let late = primary_then && previous_primary(entries).is_some_and(|p| p.same_keys(signer));
            if now.is_some() || late {
                Ok(signer)
            } else {
                Err(OpRefusal::NotAMember)
            }
        }
    }
}

/// The tables an op may change (the per-table apply rules live with the phones' store).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Table {
    Grants,
    AutopilotSettings,
    Audit,
    AutopilotMemory,
    Profiles,
    Targets,
    Accounts,
    McpServers,
    DesktopKeys,
    ConnectionPrefs,
    Secrets,
    Meta,
    /// A viewer's suggestions (a connection, a credential, a vault item), keyed `<author device id>/<id>`.
    Proposals,
}

/// The only changes a restrictive op may make: each can only tighten, whatever state it meets, so applying one late,
/// twice or out of order never loosens anything. No free-form put is restrictive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "restriction", rename_all = "snake_case")]
pub enum Restriction {
    /// Revoke one grant (revoked wins over any concurrent edit).
    RevokeGrant {
        grant_id: String,
    },
    /// Autopilot to Lockdown (Lockdown wins over every other mode).
    Lockdown,
    /// End a running Bypass now (a stopped Bypass stays stopped).
    StopBypass,
}

/// What one op does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Put {
        table: Table,
        key: String,
        value: serde_json::Value,
    },
    Remove {
        table: Table,
        key: String,
    },
    Restrict(Restriction),
}

/// The plaintext of an op's `body`, sealed with the view key with [`body_associated_data`] as the AEAD's associated
/// data. The server never sees it; phones open it and refuse the op unless [`check_change`] allows its change for its
/// signed class and author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpBody {
    pub v: u32,
    pub change: Change,
}

/// Longest record key (characters).
pub const MAX_RECORD_KEY_CHARS: usize = 256;

/// The associated data an op body is sealed with: context `reins/op-body/v1`, `account` (string), `epoch` (u64),
/// `author` (string), `counter` (u64), `class` (string), `notify` (bool). The op's signature covers the sealed body
/// (by its SHA-256) and these fields; the seal ties the plaintext, and so its table, key and kind of change, to them:
/// a body opened under any other envelope fails authentication.
#[must_use]
pub fn body_associated_data(op: &Op) -> Vec<u8> {
    let mut c = Canon::new(contexts::OP_BODY);
    c.u32(op.v).str(&op.account).u64(op.epoch).str(&op.author).u64(op.counter).str(op.class.as_str()).bool(op.notify);
    c.finish()
}

/// Why an opened op is not applied.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ChangeRefusal {
    #[error("unsupported op body version")]
    Version,
    #[error("a restrictive op only carries a restriction, and a restriction only travels as a restrictive op")]
    NotRestrictive,
    #[error("a proposal only puts or withdraws its author's own proposal")]
    NotOwnProposal,
    #[error("the primary only resolves proposals (removes them); it never writes one")]
    PrimaryProposal,
    #[error("a record key must be 1 to 256 characters without control characters")]
    BadKey,
}

fn check_key(key: &str) -> Result<(), ChangeRefusal> {
    if key.is_empty() || key.chars().count() > MAX_RECORD_KEY_CHARS || key.chars().any(char::is_control) {
        Err(ChangeRefusal::BadKey)
    } else {
        Ok(())
    }
}

/// Whether an op of `op.class` by `op.author` (already authorized by [`authorize_op`] against the roster, and its
/// body opened with [`body_associated_data`]) may make `body.change`:
///
/// - `restrictive` carries exactly a [`Restriction`], and a restriction travels only as `restrictive`: a viewer cannot
///   label a put or a removal "restrictive" to have it applied;
/// - `proposal` puts or removes a [`Table::Proposals`] record keyed `<author>/...`, nothing else;
/// - `primary` may change every table, except that it never writes a proposal (it removes one to resolve it).
pub fn check_change(op: &Op, body: &OpBody) -> Result<(), ChangeRefusal> {
    if body.v != crate::PROTOCOL_VERSION {
        return Err(ChangeRefusal::Version);
    }
    match (&body.change, op.class) {
        (Change::Restrict(r), OpClass::Restrictive) => match r {
            Restriction::RevokeGrant {
                grant_id,
            } => check_key(grant_id),
            Restriction::Lockdown | Restriction::StopBypass => Ok(()),
        },
        (Change::Restrict(_), _) | (_, OpClass::Restrictive) => Err(ChangeRefusal::NotRestrictive),
        (
            Change::Put {
                table,
                key,
                ..
            }
            | Change::Remove {
                table,
                key,
            },
            OpClass::Proposal,
        ) => {
            check_key(key)?;
            let own = key.strip_prefix(op.author.as_str()).is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'));
            if *table == Table::Proposals && own {
                Ok(())
            } else {
                Err(ChangeRefusal::NotOwnProposal)
            }
        }
        (
            Change::Put {
                table: Table::Proposals,
                ..
            },
            OpClass::Primary,
        ) => Err(ChangeRefusal::PrimaryProposal),
        (
            Change::Put {
                key,
                ..
            }
            | Change::Remove {
                key,
                ..
            },
            OpClass::Primary,
        ) => check_key(key),
    }
}

/// `POST /reins/api/ops`: ops of the calling device, in counter order. All are stored or none (an op already stored,
/// byte for byte, counts as stored: a retry after a lost answer).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpBatch {
    pub ops: Vec<Op>,
}

/// What `POST /reins/api/ops` answers: each op's `seq`, in the batch's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpsAppended {
    pub seqs: Vec<u64>,
    pub head_seq: u64,
}

/// An op as the log serves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedOp {
    pub seq: u64,
    pub op: Op,
}

/// A sealed snapshot of the account's records after the op at `seq`, signed by the primary.
///
/// Signed message (context `reins/checkpoint/v1`): `v` (u32), `account` (string), `epoch` (u64), `author`
/// (string), `seq` (u64), `applied_hash` (32 bytes), `at` (i64), SHA-256 of the decoded `body` (32 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub v: u32,
    pub account: String,
    pub epoch: u64,
    /// The primary that wrote it.
    pub author: String,
    /// The last op it contains; 0 for the state before any op.
    pub seq: u64,
    /// [`applied_hash_step`] after the op at `seq`, hex.
    pub applied_hash: String,
    pub at: i64,
    /// The snapshot sealed with the view key, base64url; at most [`MAX_CHECKPOINT_BYTES`] decoded.
    pub body: String,
    pub sig: String,
}

impl Checkpoint {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        check_account(&self.account)?;
        check_author(&self.author)?;
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        parse_hex32(&self.applied_hash).ok_or_else(|| invalid("applied_hash", "must be 64 lowercase hex digits"))?;
        body_hash("body", &self.body, MAX_CHECKPOINT_BYTES)?;
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        let applied = parse_hex32(&self.applied_hash).ok_or_else(|| invalid("applied_hash", "not a hash"))?;
        let mut c = Canon::new(contexts::CHECKPOINT);
        c.u32(self.v)
            .str(&self.account)
            .u64(self.epoch)
            .str(&self.author)
            .u64(self.seq)
            .key(&applied)
            .i64(self.at)
            .key(&body_hash("body", &self.body, MAX_CHECKPOINT_BYTES)?);
        Ok(c.finish())
    }
}

/// The primary's signed "I have applied everything up to `head_seq`".
///
/// Signed message (context `reins/heartbeat/v1`): `v` (u32), `account` (string), `epoch` (u64), `author` (string),
/// `head_seq` (u64), `applied_hash` (32 bytes), `t` (i64).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub v: u32,
    pub account: String,
    pub epoch: u64,
    pub author: String,
    pub head_seq: u64,
    /// [`applied_hash_step`] after the op at `head_seq`, hex.
    pub applied_hash: String,
    /// Unix seconds, the primary's clock.
    pub t: i64,
    pub sig: String,
}

impl Heartbeat {
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::check_version(self.v)?;
        check_account(&self.account)?;
        check_author(&self.author)?;
        if self.epoch == 0 {
            return Err(invalid("epoch", "starts at 1"));
        }
        parse_hex32(&self.applied_hash).ok_or_else(|| invalid("applied_hash", "must be 64 lowercase hex digits"))?;
        parse_sig(&self.sig).ok_or_else(|| invalid("sig", "must be 64 bytes, base64url without padding"))?;
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ValidationError> {
        let applied = parse_hex32(&self.applied_hash).ok_or_else(|| invalid("applied_hash", "not a hash"))?;
        let mut c = Canon::new(contexts::HEARTBEAT);
        c.u32(self.v).str(&self.account).u64(self.epoch).str(&self.author).u64(self.head_seq).key(&applied).i64(self.t);
        Ok(c.finish())
    }
}

/// What waits for the primary, as a viewer sees it (the same cards, without the buttons). Viewers looking never
/// mark anything delivered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    #[serde(default)]
    pub requests: Vec<RelayRequest>,
    #[serde(default)]
    pub pairings: Vec<PairingRequest>,
    #[serde(default)]
    pub joins: Vec<JoinRequest>,
    #[serde(default)]
    pub handovers: Vec<HandoverRequest>,
}

/// `GET /reins/api/sync?after=<seq>&seen=<cursor>&wait=<s>` (any member). Answers at once when there are ops after
/// `after` or `cursor` differs from `seen`; otherwise waits up to `wait` (at most 25) seconds for either.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResponse {
    /// The current roster epoch (fetch `GET /roster` when it is newer than the one this device verified).
    pub epoch: u64,
    /// The highest `seq` assigned.
    pub head_seq: u64,
    /// Ops up to this `seq` are gone (dropped after a checkpoint): a device behind it loads the checkpoint.
    pub floor_seq: u64,
    /// Ops after `after`, ascending, at most [`MAX_SYNC_OPS`].
    pub ops: Vec<SyncedOp>,
    /// More ops follow the last one returned.
    pub more: bool,
    /// The `seq` of the latest checkpoint.
    #[serde(default)]
    pub checkpoint_seq: Option<u64>,
    /// The primary's latest heartbeat.
    #[serde(default)]
    pub heartbeat: Option<Heartbeat>,
    /// The lowest `seq` every current member acknowledged ([`Ack`]): the server's own bookkeeping (which ops it may
    /// drop behind a checkpoint). Phones take nothing from it: what they forget is decided by the log alone.
    pub min_acked_seq: u64,
    pub waiting: Waiting,
    /// Requests, pairings and joins answered recently (ids).
    #[serde(default)]
    pub answered: Vec<String>,
    /// Pass back as `seen`: it changes with the epoch, the heartbeat, the waiting items and the answered ids.
    pub cursor: String,
}

/// `POST /reins/api/roster/recovery-log`: a phone about to recover (no member yet) reads the log it will vouch for,
/// with the same proof as the recovery wrap.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryLogRequest {
    pub master_password_hash: String,
    /// Ops after this `seq`.
    #[serde(default)]
    pub after: u64,
}

impl std::fmt::Debug for RecoveryLogRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryLogRequest").field("after", &self.after).finish_non_exhaustive()
    }
}

/// What `POST /reins/api/roster/recovery-log` answers: the latest checkpoint and the ops after `after` (from the
/// floor on), at most [`MAX_SYNC_OPS`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryLog {
    #[serde(default)]
    pub checkpoint: Option<Checkpoint>,
    pub floor_seq: u64,
    pub head_seq: u64,
    pub ops: Vec<SyncedOp>,
    pub more: bool,
}

/// `POST /reins/api/ack`: this device applied everything up to `seq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    pub seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        canon::{encode_payload, encode_sig, hex32},
        roster::{
            RosterReason,
            tests::{fake_sign, genesis, member, next},
        },
    };

    fn op(author: &str, epoch: u64, class: OpClass, counter: u64) -> Op {
        Op {
            v: 1,
            account: "acct".to_owned(),
            epoch,
            author: author.to_owned(),
            counter,
            class,
            notify: false,
            body: encode_payload(b"sealed"),
            sig: encode_sig(&[0; 64]),
        }
    }

    /// p primary with viewer v (epoch 2), v and w (3); handover to v (4); enroll y (5).
    fn chain() -> Vec<RosterEntry> {
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], 50);
        let e3 = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], 50);
        let e4 = next(&e3, RosterReason::Handover, member("v", 2), vec![member("w", 3), member("p", 1)], 50);
        let e5 =
            next(&e4, RosterReason::Enroll, member("v", 2), vec![member("w", 3), member("p", 1), member("y", 5)], 50);
        vec![e1, e2, e3, e4, e5]
    }

    fn body(change: Change) -> OpBody {
        OpBody {
            v: 1,
            change,
        }
    }

    fn put(table: Table, key: &str) -> Change {
        Change::Put {
            table,
            key: key.to_owned(),
            value: serde_json::json!({"mode": "bypass"}),
        }
    }

    #[test]
    fn a_permissive_change_labelled_restrictive_is_refused() {
        use OpClass::{Primary, Proposal, Restrictive};
        let viewer = |class| op("w", 5, class, 1);
        // A viewer's put (Bypass on, a new grant) as "restrictive": the roster allows a viewer's restrictive op, the
        // change does not.
        assert_eq!(
            check_change(&viewer(Restrictive), &body(put(Table::AutopilotSettings, "mode"))),
            Err(ChangeRefusal::NotRestrictive)
        );
        let remove = Change::Remove {
            table: Table::Grants,
            key: "g1".to_owned(),
        };
        assert_eq!(check_change(&viewer(Restrictive), &body(remove)), Err(ChangeRefusal::NotRestrictive));
        // The typed restrictions are fine, from anyone the roster lets pull the brake.
        for r in [
            Restriction::Lockdown,
            Restriction::StopBypass,
            Restriction::RevokeGrant {
                grant_id: "g1".to_owned(),
            },
        ] {
            assert_eq!(check_change(&viewer(Restrictive), &body(Change::Restrict(r.clone()))), Ok(()));
            // ...and only as restrictive ops.
            assert_eq!(
                check_change(&viewer(Proposal), &body(Change::Restrict(r.clone()))),
                Err(ChangeRefusal::NotRestrictive)
            );
            assert_eq!(
                check_change(&op("v", 5, Primary, 1), &body(Change::Restrict(r))),
                Err(ChangeRefusal::NotRestrictive)
            );
        }
        // Proposals: only the author's own, only in the proposals table.
        assert_eq!(check_change(&viewer(Proposal), &body(put(Table::Proposals, "w/p1"))), Ok(()));
        assert_eq!(
            check_change(&viewer(Proposal), &body(put(Table::Proposals, "v/p1"))),
            Err(ChangeRefusal::NotOwnProposal)
        );
        assert_eq!(
            check_change(&viewer(Proposal), &body(put(Table::Proposals, "w/"))),
            Err(ChangeRefusal::NotOwnProposal)
        );
        assert_eq!(
            check_change(&viewer(Proposal), &body(put(Table::Proposals, "wx/p1"))),
            Err(ChangeRefusal::NotOwnProposal)
        );
        assert_eq!(
            check_change(&viewer(Proposal), &body(put(Table::Grants, "w/p1"))),
            Err(ChangeRefusal::NotOwnProposal)
        );
        // The primary changes records and resolves proposals, but never writes one.
        assert_eq!(check_change(&op("v", 5, Primary, 1), &body(put(Table::Grants, "g1"))), Ok(()));
        assert_eq!(
            check_change(&op("v", 5, Primary, 1), &body(put(Table::Proposals, "v/p1"))),
            Err(ChangeRefusal::PrimaryProposal)
        );
        let resolve = Change::Remove {
            table: Table::Proposals,
            key: "w/p1".to_owned(),
        };
        assert_eq!(check_change(&op("v", 5, Primary, 1), &body(resolve)), Ok(()));
        assert_eq!(check_change(&op("v", 5, Primary, 1), &body(put(Table::Grants, ""))), Err(ChangeRefusal::BadKey));
        let v2 = OpBody {
            v: 2,
            change: Change::Restrict(Restriction::Lockdown),
        };
        assert_eq!(check_change(&viewer(Restrictive), &v2), Err(ChangeRefusal::Version));
    }

    #[test]
    fn a_changed_class_table_or_epoch_breaks_the_signature() {
        use crate::roster::tests::fake_verify;
        // The body as sealed (here its plaintext stands in for the ciphertext: the signature covers its hash).
        let sealed = |b: &OpBody| encode_payload(&serde_json::to_vec(b).unwrap());
        let mut signed = op("w", 5, OpClass::Restrictive, 1);
        signed.body = sealed(&body(Change::Restrict(Restriction::Lockdown)));
        signed.sig = encode_sig(&fake_sign(&[2; 32], &signed.signing_bytes().unwrap()));
        let verifies = |o: &Op| fake_verify(&[2; 32], &o.signing_bytes().unwrap(), &parse_sig(&o.sig).unwrap());
        assert!(verifies(&signed));
        let mut relabelled = signed.clone();
        relabelled.class = OpClass::Primary;
        assert!(!verifies(&relabelled));
        let mut other_table = signed.clone();
        other_table.body = sealed(&body(put(Table::Grants, "g1")));
        assert!(!verifies(&other_table));
        let mut other_epoch = signed.clone();
        other_epoch.epoch = 4;
        assert!(!verifies(&other_epoch));
        // The seal's associated data changes with every envelope field, so a body moved under another envelope does
        // not open.
        let aad = body_associated_data(&signed);
        for o in [relabelled, other_epoch] {
            assert_ne!(body_associated_data(&o), aad);
        }
        let mut renotified = signed;
        renotified.notify = true;
        assert_ne!(body_associated_data(&renotified), aad);
        // Wire format of a body.
        let v = serde_json::to_value(body(Change::Restrict(Restriction::RevokeGrant {
            grant_id: "g1".to_owned(),
        })))
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"v": 1, "change": {"kind": "restrict", "restriction": "revoke_grant", "grant_id": "g1"}})
        );
    }

    #[test]
    fn who_may_write_what() {
        use OpClass::{Primary, Proposal, Restrictive};
        let c = chain();
        let ok = |o: Op| authorize_op(&o, &c).map(|m| m.device_id.clone());
        // The primary, at any epoch of its tenure.
        assert_eq!(ok(op("v", 5, Primary, 1)), Ok("v".to_owned()));
        assert_eq!(ok(op("v", 4, Primary, 1)), Ok("v".to_owned()));
        // A viewer never writes primary ops.
        assert_eq!(ok(op("w", 5, Primary, 1)), Err(OpRefusal::NotPrimary));
        // The old primary, at its old epoch: primary refused, restrictive allowed (its late brake).
        assert_eq!(ok(op("p", 3, Primary, 1)), Err(OpRefusal::NotPrimary));
        assert_eq!(ok(op("p", 3, Restrictive, 1)), Ok("p".to_owned()));
        // Viewers propose and pull the brake.
        assert_eq!(ok(op("w", 5, Proposal, 1)), Ok("w".to_owned()));
        assert_eq!(ok(op("w", 5, Restrictive, 1)), Ok("w".to_owned()));
        assert_eq!(ok(op("y", 5, Restrictive, 1)), Ok("y".to_owned()));
        // y was not a member at epoch 4.
        assert_eq!(ok(op("y", 4, Proposal, 1)), Err(OpRefusal::NotAMember));
        assert_eq!(ok(op("z", 5, Proposal, 1)), Err(OpRefusal::NotAMember));
        assert_eq!(ok(op("v", 6, Primary, 1)), Err(OpRefusal::UnknownEpoch));
        let mut other = op("v", 5, Primary, 1);
        other.account = "other".to_owned();
        assert_eq!(ok(other), Err(OpRefusal::WrongAccount));
    }

    #[test]
    fn a_removed_viewer_and_a_recovered_primary_can_only_do_what_is_left() {
        use OpClass::{Primary, Proposal, Restrictive};
        let e1 = genesis();
        let e2 = next(&e1, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], 50);
        let e3 = next(&e2, RosterReason::Remove, member("p", 1), vec![member("v", 2)], 50);
        let e4 = next(&e3, RosterReason::Recovery, member("v", 2), vec![], 51);
        let c = vec![e1, e2, e3, e4];
        let ok = |o: Op| authorize_op(&o, &c).map(|m| m.device_id.clone());
        // w was removed: nothing, not even the brake.
        assert_eq!(ok(op("w", 2, Restrictive, 1)), Err(OpRefusal::NotAMember));
        assert_eq!(ok(op("w", 2, Proposal, 1)), Err(OpRefusal::NotAMember));
        // p lost the role in a recovery: its late restrictive ops still count, nothing else.
        assert_eq!(ok(op("p", 3, Restrictive, 1)), Ok("p".to_owned()));
        assert_eq!(ok(op("p", 3, Primary, 1)), Err(OpRefusal::NotAMember));
        assert_eq!(ok(op("p", 3, Proposal, 1)), Err(OpRefusal::NotAMember));
        // v, now primary: not with the key it was a viewer with at epoch 3? Same key here, so its primary ops count
        // only at epoch 4 (it was a viewer at 3).
        assert_eq!(ok(op("v", 4, Primary, 1)), Ok("v".to_owned()));
        assert_eq!(ok(op("v", 3, Primary, 1)), Err(OpRefusal::NotPrimary));
    }

    #[test]
    fn signing_bytes_cover_the_body_and_every_field() {
        let base = op("v", 5, OpClass::Primary, 7);
        let bytes = base.signing_bytes().unwrap();
        let edits: [fn(&mut Op); 7] = [
            |o| o.account.push('x'),
            |o| o.epoch += 1,
            |o| o.author.push('x'),
            |o| o.counter += 1,
            |o| o.class = OpClass::Restrictive,
            |o| o.notify = true,
            |o| o.body = encode_payload(b"sealeD"),
        ];
        for edit in edits {
            let mut o = base.clone();
            edit(&mut o);
            assert_ne!(o.signing_bytes().unwrap(), bytes);
        }
        let mut resigned = base;
        resigned.sig = encode_sig(&fake_sign(&[1; 32], &bytes));
        assert_eq!(resigned.signing_bytes().unwrap(), bytes, "the signature is not signed");
    }

    #[test]
    fn only_the_primary_writes_large_bodies() {
        for (class, max) in [
            (OpClass::Primary, MAX_OP_BODY_BYTES),
            (OpClass::Proposal, MAX_PROPOSAL_BODY_BYTES),
            (OpClass::Restrictive, MAX_RESTRICTIVE_BODY_BYTES),
        ] {
            let mut o = op("v", 1, class, 1);
            o.body = encode_payload(&vec![0; max]);
            o.validate().unwrap();
            o.body = encode_payload(&vec![0; max + 1]);
            assert!(o.validate().is_err(), "{class:?}");
        }
    }

    #[test]
    fn ops_are_bounded() {
        let mut o = op("v", 1, OpClass::Primary, 1);
        o.validate().unwrap();
        o.body = encode_payload(&vec![0; MAX_OP_BODY_BYTES + 1]);
        assert!(o.validate().is_err());
        o.body = String::new();
        assert!(o.validate().is_err());
        o.body = "not base64!".to_owned();
        assert!(o.validate().is_err());
        let mut o = op("v", 1, OpClass::Primary, 0);
        assert!(o.validate().is_err(), "counters start at 1");
        o.counter = 1;
        o.author = "bad id!".to_owned();
        assert!(o.validate().is_err());
    }

    #[test]
    fn the_applied_hash_depends_on_order_and_content() {
        let a = op("v", 1, OpClass::Primary, 1);
        let b = op("v", 1, OpClass::Primary, 2);
        let ab = applied_hash_step(&applied_hash_step(&GENESIS_APPLIED_HASH, 1, &a).unwrap(), 2, &b).unwrap();
        let ba = applied_hash_step(&applied_hash_step(&GENESIS_APPLIED_HASH, 1, &b).unwrap(), 2, &a).unwrap();
        assert_ne!(ab, ba);
        let shifted = applied_hash_step(&applied_hash_step(&GENESIS_APPLIED_HASH, 1, &a).unwrap(), 3, &b).unwrap();
        assert_ne!(ab, shifted);
    }

    #[test]
    fn checkpoints_and_heartbeats_sign_their_fields() {
        let cp = Checkpoint {
            v: 1,
            account: "acct".to_owned(),
            epoch: 2,
            author: "p".to_owned(),
            seq: 500,
            applied_hash: hex32(&[1; 32]),
            at: 9,
            body: encode_payload(b"snapshot"),
            sig: encode_sig(&[0; 64]),
        };
        cp.validate().unwrap();
        let mut cp2 = cp.clone();
        cp2.seq = 501;
        assert_ne!(cp.signing_bytes().unwrap(), cp2.signing_bytes().unwrap());
        let hb = Heartbeat {
            v: 1,
            account: "acct".to_owned(),
            epoch: 2,
            author: "p".to_owned(),
            head_seq: 500,
            applied_hash: hex32(&[1; 32]),
            t: 9,
            sig: encode_sig(&[0; 64]),
        };
        hb.validate().unwrap();
        // The same numbers under another context never verify as the other kind.
        assert_ne!(cp.signing_bytes().unwrap()[..30], hb.signing_bytes().unwrap()[..30]);
        let mut bad = hb;
        bad.applied_hash = "XYZ".to_owned();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn wire_format() {
        let o = op("v", 1, OpClass::Restrictive, 3);
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["class"], "restrictive");
        assert_eq!(v["notify"], false);
        let mut no_notify = v;
        no_notify.as_object_mut().unwrap().remove("notify");
        assert_eq!(serde_json::from_value::<Op>(no_notify).unwrap(), o, "notify defaults to false");
        let synced = SyncedOp {
            seq: 4,
            op: o,
        };
        let v = serde_json::to_value(&synced).unwrap();
        assert_eq!(v["seq"], 4);
        assert_eq!(v["op"]["counter"], 3);
        assert_eq!(
            serde_json::to_value(Ack {
                seq: 9
            })
            .unwrap(),
            serde_json::json!({"seq": 9})
        );
    }
}
