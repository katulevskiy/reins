//! The sync engine (phone roles, item 5): the account's state goes between phones as the signed op log, per record,
//! instead of the sealed snapshot (`account_sync.rs` stays for accounts without phone roles).
//!
//! **One cycle** ([`Engine::live_sync`]), under the account-sync lock:
//! 1. *What this phone owes the log.* The primary seals the view-key grants it owes (each under a key its viewer
//!    holds) and publishes what changed in its store: each synced row ([`crate::store::records`]) whose digest differs
//!    from the mirror becomes a put naming the version it replaces, each row gone a removal, each new Activity entry a
//!    put under its op's [`activity_key`]. Tokens are sealed to the credential key on top of the view key. Ops are
//!    numbered and signed into the outbox at once (so a handover counts them, see `roles::last_counter`).
//! 2. *Upload* the outbox, in counter order. A refusal (4xx) drops the refused ops and forgets their mirror, so the
//!    next publish compares again.
//! 3. *Pull and apply*, in seq order (`store::oplog::apply`). A checkpoint loads first when the log starts past this
//!    phone. A halt for an epoch this phone has not verified fetches the roster; for a view key it lacks, opens the
//!    grant the log brought for it.
//! 4. *Into the store:* every record that changed since the mirror is written to its table, unless an op of this phone
//!    still waits for it (the store is ahead there). A viewer never takes a token.
//! 5. *The roster, after applying* (so the ops signed before a change are judged as the roster stood), then the
//!    phone's standing: a viewer, or a replaced primary, forgets every token.
//! 6. *The heartbeat:* the current primary's at the current epoch must match what this phone applied (else the log
//!    forked and applying stops, for good); any other primary's that differs is only [`LiveSyncState::disagrees`],
//!    kept in memory, never a halt.
//! 7. *Ack*; on the primary, a heartbeat every 30 s and a checkpoint every 500 ops or day.

mod net;
mod proposals;

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use data_encoding::BASE64URL_NOPAD;
use reins_proto::canon::decode_payload;
use reins_proto::oplog::{
    Change, HEARTBEAT_EVERY_SECS, LIVE_SECS, MAX_CHECKPOINT_BYTES, OpBody, OpClass, Restriction, SyncResponse, Table,
    body_associated_data,
};
use serde_json::{Value as Json, json};

pub use proposals::{ProposalPreview, ProposalRowView, ProposalView};

use crate::CoreError;
use crate::engine::Engine;
use crate::phone_api::ApiFailure;
use crate::roles::keys::{SigningKey, ViewKeyring, seal_to};
use crate::roles::{Account, LocalRoster, PhoneRole, Standing};
use crate::session::{Session, api_call};
use crate::store::oplog::{self, Context, Halt, HeartbeatCheck, RecordPut, Version, activity_key, checkpoint, local};
use crate::store::records::{self, RESERVED_PREFIX};
use crate::store::unix_now;

/// A checkpoint every this many ops…
const CHECKPOINT_EVERY_OPS: u64 = 500;
/// …or this long (seconds), whichever comes first.
const CHECKPOINT_EVERY_SECS: i64 = 86_400;
/// The Activity entries a phone that starts the log publishes (the newest; older ones stay on it).
const FIRST_ACTIVITY: i64 = 500;
/// Activity entries published per cycle at most.
const ACTIVITY_PER_CYCLE: usize = 200;
/// Ops per upload (the server's batch limit).
const UPLOAD_BATCH: usize = reins_proto::oplog::MAX_OPS_PER_BATCH;
/// Marks (`oplog::local`).
const ACTIVITY_FLOOR: &str = "activity-floor";
const MATERIALIZED_SEQ: &str = "materialized-seq";
const CHECKPOINT_AT: &str = "checkpoint-at";
/// The roster epoch of the last checkpoint this phone stored.
const CHECKPOINT_EPOCH: &str = "checkpoint-epoch";
/// A `meta` record the primary writes when the roster moved to a new epoch, so that a checkpoint at that epoch (the
/// one a phone enrolled since can verify) has an op of its own to follow.
const EPOCH_KEY: &str = "~reins.epoch";
/// The record key prefix of a view-key grant (`~reins.view-grant/<device>/<kid>`, a `meta` record).
const GRANT_PREFIX: &str = "~reins.view-grant/";
/// Marks: rows kept here because no op could carry them; records that failed to write here, and how often; a start
/// this phone could not verify.
const UNPUBLISHED: &str = "unpublished\u{1f}";
const FAILED: &str = "failed\u{1f}";
const UNTRUSTED_START: &str = "untrusted-start";
/// How often a record that does not write is tried again before it waits for a newer version of it.
const MAX_WRITE_TRIES: u32 = 3;
/// How long a refused (or too large) checkpoint waits before the next try.
const CHECKPOINT_BACKOFF_SECS: i64 = 3_600;
/// The tables a viewer keeps only as the log has them (its own leftovers deleted): what decides or connects.
const LOG_ONLY: &[Table] = &[
    Table::Grants,
    Table::AutopilotSettings,
    Table::Profiles,
    Table::Targets,
    Table::Accounts,
    Table::DesktopKeys,
    Table::McpServers,
];

/// What the live loop knows besides the log (memory only: nothing of it is kept).
#[derive(Default)]
pub(crate) struct State {
    /// A primary other than the current one signed another history than this phone applied (a warning).
    disagrees: Mutex<bool>,
    /// The last heartbeat that verified: what it said of this phone, when it was signed, the head it signed.
    verified: Mutex<Option<(HeartbeatCheck, i64, u64)>>,
    /// The server's cursor from the last sync, passed back so that a long-poll waits for news.
    cursor: Mutex<Option<String>>,
    /// When this phone (the primary) last posted a heartbeat, and at which epoch (a new one posts at once).
    heartbeat_posted: AtomicI64,
    heartbeat_epoch: AtomicU64,
    /// No checkpoint is tried before then (one was refused, or would not fit).
    checkpoint_after: AtomicI64,
    /// Brakes and proposals of this phone the server refused for good: shown once, never dropped silently.
    refused: Mutex<Vec<String>>,
    /// The last state shown.
    last: Mutex<Option<LiveSyncState>>,
}

/// Where the live view stands on this phone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LiveSyncState {
    pub role: PhoneRole,
    pub epoch: u64,
    /// The last op this phone applied, and the log's head.
    pub applied_seq: u64,
    pub head_seq: u64,
    /// The primary's name.
    pub primary_name: String,
    /// When the primary's latest heartbeat was signed (unix seconds), if one came.
    pub primary_seen_at: Option<i64>,
    /// The head the current primary signed in its latest heartbeat that verified.
    pub primary_head_seq: Option<u64>,
    /// The current primary's latest heartbeat verified, is under two minutes old, and signs the very history this
    /// phone applied up to its head (not ahead of this phone).
    pub live: bool,
    /// Why this phone stopped applying (the log forked), if it did: show it, and recover or reinstall.
    pub forked: Option<String>,
    /// A replaced primary (or an older epoch) signed another history: "Your devices may disagree".
    pub disagrees: bool,
    /// The log is sealed with a view key this phone was not given yet.
    pub waiting_for_key: bool,
    /// This phone's changes not yet in the log.
    pub queued: u32,
    /// Authors whose ops never all came (a gap in their counters): something was withheld.
    pub counter_gaps: u32,
    /// Something to show, if the last cycle did not finish.
    pub notice: Option<String>,
}

/// What one cycle's pull left.
struct Pulled {
    response: Option<SyncResponse>,
    waiting_for_key: bool,
    notice: Option<String>,
}

/// The keys and roster a cycle signs and verifies with.
struct Keys {
    roster: LocalRoster,
    ring: ViewKeyring,
    signer: SigningKey,
    me: String,
    standing: Standing,
}

fn invalid(e: impl std::fmt::Display) -> CoreError {
    CoreError::invalid(e.to_string())
}

fn table_name(table: Table) -> String {
    serde_json::to_value(table).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Whether an op changing `table` alerts the viewers (a mode or Bypass change, a new connection or credential).
fn notifies(table: Table) -> bool {
    matches!(table, Table::AutopilotSettings | Table::Accounts | Table::McpServers | Table::DesktopKeys)
}

impl Engine {
    fn live(&self) -> &State {
        &self.live
    }

    fn sync_keys(&self, account: &Account) -> Result<Option<Keys>, CoreError> {
        let Some(roster) = self.local_roster(&account.slot)? else {
            return Ok(None);
        };
        let standing = self.standing_in(&roster)?;
        Ok(Some(Keys {
            ring: self.keyring(&account.slot)?,
            signer: self.sign_key()?,
            me: self.store.device_id()?,
            standing,
            roster,
        }))
    }

    /// One cycle of the live sync (see the module docs): `wait_secs` long-polls for news when there is nothing to do.
    /// On an account without phone roles, only says so (the legacy sync runs as before).
    pub async fn live_sync(&self, wait_secs: u32) -> Result<LiveSyncState, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        if self.local_roster(&account.slot)?.is_none() {
            return Ok(Self::legacy_state());
        }
        let _guard = self.account_sync.lock().await;
        let state = self.live_cycle(&session, &account, wait_secs).await?;
        *self.live().last.lock().unwrap_or_else(PoisonError::into_inner) = Some(state.clone());
        self.live().refused.lock().unwrap_or_else(PoisonError::into_inner).clear();
        Ok(state)
    }

    /// The live view's state as the last cycle left it (no network).
    pub fn live_sync_state(&self) -> Result<LiveSyncState, CoreError> {
        if !self.holds_roster_now()? {
            return Ok(Self::legacy_state());
        }
        Ok(self.live().last.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| LiveSyncState {
            role: self.role_now().unwrap_or(PhoneRole::Legacy),
            ..Self::legacy_state()
        }))
    }

    const fn legacy_state() -> LiveSyncState {
        LiveSyncState {
            role: PhoneRole::Legacy,
            epoch: 0,
            applied_seq: 0,
            head_seq: 0,
            primary_name: String::new(),
            primary_seen_at: None,
            primary_head_seq: None,
            live: false,
            forked: None,
            disagrees: false,
            waiting_for_key: false,
            queued: 0,
            counter_gaps: 0,
            notice: None,
        }
    }

    /// A cycle without waiting, for the places that used to upload the snapshot (after a change, at sign-in, on the
    /// primary's poll). Problems are logged: the next cycle tries again.
    pub(crate) async fn live_sync_now(&self) -> Result<(), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        if self.local_roster(&account.slot)?.is_none() {
            return Ok(());
        }
        let _guard = self.account_sync.lock().await;
        let state = self.live_cycle(&session, &account, 0).await?;
        *self.live().last.lock().unwrap_or_else(PoisonError::into_inner) = Some(state);
        Ok(())
    }

    async fn live_cycle(&self, session: &Session, account: &Account, wait: u32) -> Result<LiveSyncState, CoreError> {
        let mut notice = None;
        if let Some(keys) = self.sync_keys(account)?
            && keys.standing == Standing::Primary
        {
            // A primary that never brought its store up to the log (it just recovered the account, or took over) takes
            // the log first: what it publishes is then a change of the log's state, never its own empty store's.
            if self.store.oplog(|c| local::mark(c, MATERIALIZED_SEQ))??.is_none() {
                let first = self.pull_and_apply(session, account, 0).await?;
                if first.notice.is_some() || first.waiting_for_key {
                    notice = first.notice.or(Some("The account's log is not all here yet.".to_owned()));
                } else {
                    let failed = self.materialize(account)?;
                    self.tighten_untrusted_start(failed)?;
                }
            }
            self.deliver_grants(account, &keys)?;
            if self.store.oplog(|c| local::mark(c, MATERIALIZED_SEQ))??.is_some() {
                self.publish(account, &keys)?;
                self.mark_epoch(account, &keys)?;
            }
        }
        if let Err(e) = self.upload_outbox(session, account).await {
            notice = Some(format!("Changes wait to reach the server: {e}"));
        }
        let pulled = self.pull_and_apply(session, account, wait).await?;
        notice = notice.or(pulled.notice);
        let failed = self.materialize(account)?;
        self.tighten_untrusted_start(failed)?;
        if failed > 0 {
            notice = notice.or(Some(format!(
                "{failed} changes from the account's log could not be written on this phone; the next sync tries again."
            )));
        }
        // The roster after the log: ops signed before a roster change are judged as the roster stood.
        if let Some(response) = &pulled.response {
            let known = self.local_roster(&account.slot)?.map_or(0, |r| r.epoch());
            if response.epoch != known {
                // What was verified against the old roster says nothing about the new one.
                *self.live().verified.lock().unwrap_or_else(PoisonError::into_inner) = None;
                match self.refresh_roster(session, account).await {
                    Ok(_) => {}
                    Err(e) => notice = notice.or(Some(format!("The phones of this account could not be checked: {e}"))),
                }
            }
        }
        if let Some(keys) = self.sync_keys(account)? {
            if keys.standing != Standing::Primary {
                // A viewer opens no token; a demoted primary forgets them (and drops what it can no longer send).
                self.store.wipe_tokens()?;
                self.drop_primary_ops(account)?;
            }
            if let Some(response) = &pulled.response {
                self.check_live_heartbeat(account, &keys, response)?;
                let applied = self.store.oplog(|c| oplog::state(c))??.seq;
                if applied > 0
                    && let Err(e) = api_call!(session, |api| api.ack(applied))
                {
                    log::debug!("could not acknowledge the log: {}", e.into_core());
                }
                if keys.standing == Standing::Primary {
                    self.post_heartbeat(session, account, &keys).await;
                    self.maybe_checkpoint(session, account, &keys, response).await;
                }
            }
        }
        self.synced_writes.store(self.store.write_count(), Ordering::Release);
        self.live_state(account, pulled.response.as_ref(), pulled.waiting_for_key, notice)
    }

    fn live_state(
        &self,
        account: &Account,
        response: Option<&SyncResponse>,
        waiting_for_key: bool,
        notice: Option<String>,
    ) -> Result<LiveSyncState, CoreError> {
        let roster = self.local_roster(&account.slot)?;
        let role = match &roster {
            Some(r) => self.standing_in(r)?.into(),
            None => PhoneRole::Legacy,
        };
        let s = self.store.oplog(|c| oplog::state(c))??;
        let queued = self.store.oplog(|c| local::outbox(c))??.len();
        let gaps = self.store.oplog(|c| oplog::counter_gaps(c))??.len();
        let disagrees = *self.live().disagrees.lock().unwrap_or_else(PoisonError::into_inner);
        let current = roster.as_ref().map(LocalRoster::current);
        // Only a heartbeat that verified as the current primary's, at the current epoch: Matches is live, Behind (it
        // signed a head this phone has not applied) is not.
        let verified = *self.live().verified.lock().unwrap_or_else(PoisonError::into_inner);
        let of_primary = verified.filter(|(check, _, _)| {
            matches!(check, HeartbeatCheck::Matches | HeartbeatCheck::Behind | HeartbeatCheck::Unknown)
        });
        let fresh = of_primary.is_some_and(|(check, t, _)| is_live(check, t, unix_now()));
        // Shown until the user's own sync showed them (a background cycle never takes them).
        let refused = self.live().refused.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let notice = if refused.is_empty() {
            notice
        } else {
            Some(refused.join(" "))
        };
        Ok(LiveSyncState {
            role,
            epoch: roster.as_ref().map_or(0, LocalRoster::epoch),
            applied_seq: s.seq,
            head_seq: response.map_or(s.seq, |r| r.head_seq),
            primary_name: current.map(|c| c.primary.name.clone()).unwrap_or_default(),
            primary_seen_at: of_primary.map(|(_, t, _)| t),
            primary_head_seq: of_primary.map(|(_, _, head)| head),
            live: role == PhoneRole::Primary || (fresh && s.forked.is_none() && !disagrees),
            forked: s.forked,
            disagrees,
            waiting_for_key,
            queued: u32::try_from(queued).unwrap_or(u32::MAX),
            counter_gaps: u32::try_from(gaps).unwrap_or(u32::MAX),
            notice,
        })
    }

    // ---- 1. what this phone owes the log ------------------------------------------------------------------------

    /// Signs and queues one op of this phone's (numbered after everything it took or queued), covering `(table, key)`.
    fn enqueue(
        &self,
        account: &Account,
        keys: &Keys,
        ring: &ViewKeyring,
        class: OpClass,
        change: Change,
        covers: (&str, &str),
    ) -> Result<Version, CoreError> {
        let key = covers.1.to_owned();
        self.enqueue_with(account, keys, ring, class, covers.0, |_| (change, key))
    }

    /// [`Self::enqueue`] for a change that depends on its own version (an Activity entry is keyed by it): `build`
    /// gets the version the op will have and returns the change and the key it covers.
    fn enqueue_with(
        &self,
        account: &Account,
        keys: &Keys,
        ring: &ViewKeyring,
        class: OpClass,
        table: &str,
        build: impl FnOnce(&Version) -> (Change, String),
    ) -> Result<Version, CoreError> {
        let sign_pub = keys.signer.public_b64();
        self.store.oplog(|c| -> Result<Version, CoreError> {
            let counter =
                oplog::author_last_counter(c, &keys.me, &sign_pub)?.max(local::outbox_head(c)?).saturating_add(1);
            let version = Version {
                epoch: keys.roster.epoch(),
                author: keys.me.clone(),
                counter,
            };
            let (change, covers) = build(&version);
            let notify = match &change {
                Change::Put {
                    table,
                    ..
                } => notifies(*table),
                Change::Restrict(Restriction::Lockdown | Restriction::StopBypass) => true,
                _ => false,
            };
            let body = OpBody {
                v: reins_proto::PROTOCOL_VERSION,
                change,
            };
            let op = oplog::seal_op(
                &account.id,
                ring,
                &keys.signer,
                &keys.me,
                keys.roster.epoch(),
                counter,
                class,
                notify,
                &body,
            )?;
            op.validate().map_err(invalid)?;
            local::queue(c, &op, table, &covers)?;
            Ok(Version::of(&op))
        })?
    }

    /// The view-key grants this phone owes (after a removal or a recovery), each as a `meta` record sealed under the
    /// newest key its viewer already holds, queued before anything sealed with the new key.
    fn deliver_grants(&self, account: &Account, keys: &Keys) -> Result<(), CoreError> {
        let owed = self.outgoing_grants(&account.slot)?;
        if owed.is_empty() {
            return Ok(());
        }
        for grant in &owed {
            let older = older_ring(&keys.ring, grant.kid);
            if older.is_empty() {
                log::warn!("a view-key grant has no older key to travel under; the member is enrolled again instead");
                continue;
            }
            let key = format!("{GRANT_PREFIX}{}/{}", grant.device_id, grant.kid);
            let change = Change::Put {
                table: Table::Meta,
                key: key.clone(),
                value: serde_json::to_value(RecordPut {
                    prev: None,
                    row: json!({"sealed": grant.sealed}),
                })
                .map_err(invalid)?,
            };
            self.enqueue(account, keys, &older, OpClass::Primary, change, (&table_name(Table::Meta), &key))?;
        }
        self.clear_outgoing_grants(&account.slot)
    }

    /// Queues what changed in this phone's store since the mirror (the primary only; see the module docs).
    fn publish(&self, account: &Account, keys: &Keys) -> Result<(), CoreError> {
        let rows = self.store.sync_rows()?;
        let mirror = self.store.oplog(|c| local::mirror(c))??;
        let credential = keys.roster.current().credential_pub.clone();
        let mut present = HashSet::new();
        for row in rows {
            let tbl = table_name(row.table);
            present.insert((tbl.clone(), row.key.clone()));
            let digest = row.digest();
            let seen = mirror.get(&(tbl.clone(), row.key.clone()));
            if seen.is_some_and(|m| m.digest == digest) {
                continue;
            }
            let prev = seen.and_then(|m| m.version.clone());
            let value = if row.table == Table::Secrets {
                // Sealed to the credential key too: a viewer holds the view key, never a token.
                let plain = serde_json::to_vec(&row.value).map_err(invalid)?;
                json!({"sealed": BASE64URL_NOPAD.encode(&seal_to(&credential, &plain)?)})
            } else {
                row.value.clone()
            };
            let change = Change::Put {
                table: row.table,
                key: row.key.clone(),
                value: serde_json::to_value(RecordPut {
                    prev: prev.clone(),
                    row: value,
                })
                .map_err(invalid)?,
            };
            let unpublished = unpublished_mark(&tbl, &row.key);
            let version = match self.enqueue(account, keys, &keys.ring, OpClass::Primary, change, (&tbl, &row.key)) {
                Ok(v) => {
                    self.store.oplog(|c| local::clear_mark(c, &unpublished))??;
                    Some(v)
                }
                Err(e) => {
                    // Too large for an op (a big icon), or the like: kept here (noted, so that a handover waits for it
                    // and a demotion never deletes it), not retried every cycle.
                    log::warn!("a {tbl} record is not synced: {e}");
                    self.store.oplog(|c| local::set_mark(c, &unpublished, &digest))??;
                    prev
                }
            };
            self.set_mirror(&tbl, &row.key, version, digest, Some(row.local))?;
        }
        // A row kept aside that is gone now (removed, or trimmed below the limit and sent) is no longer noted.
        for (mark, _) in self.store.oplog(|c| local::marks(c, UNPUBLISHED))?? {
            let gone = mark
                .strip_prefix(UNPUBLISHED)
                .and_then(|rest| rest.split_once('\u{1f}'))
                .is_none_or(|(tbl, key)| !present.contains(&(tbl.to_owned(), key.to_owned())));
            if gone {
                self.store.oplog(|c| local::clear_mark(c, &mark))??;
            }
        }
        for ((tbl, key), m) in &mirror {
            let synced =
                records::TABLES.iter().any(|(t, _, _)| table_name(*t) == *tbl) || *tbl == table_name(Table::Meta);
            if !synced
                || m.digest.is_empty()
                || key.starts_with(RESERVED_PREFIX)
                || present.contains(&(tbl.clone(), key.clone()))
            {
                continue;
            }
            let table = records::TABLES
                .iter()
                .map(|(t, _, _)| *t)
                .chain([Table::Meta])
                .find(|t| table_name(*t) == *tbl)
                .ok_or_else(|| CoreError::storage("unknown table"))?;
            let change = Change::Remove {
                table,
                key: key.clone(),
            };
            let version = self.enqueue(account, keys, &keys.ring, OpClass::Primary, change, (tbl, key))?;
            self.set_mirror(tbl, key, Some(version), String::new(), None)?;
        }
        self.publish_activity(account, keys, &mirror)
    }

    /// New Activity entries, each under its op's key. A phone that starts the log sends only its newest ones; one that
    /// became primary sends none of what the log brought it.
    fn publish_activity(
        &self,
        account: &Account,
        keys: &Keys,
        mirror: &std::collections::HashMap<(String, String), local::Mirrored>,
    ) -> Result<(), CoreError> {
        let audit = table_name(Table::Audit);
        let marked = self.store.oplog(|c| local::mark(c, ACTIVITY_FLOOR))??;
        let floor = if let Some(f) = marked {
            f.parse::<i64>().unwrap_or(0)
        } else {
            let head = self.store.audit_head()?;
            let logged = mirror.keys().any(|(t, _)| *t == audit);
            if logged {
                head
            } else {
                (head - FIRST_ACTIVITY).max(0)
            }
        };
        let brought: HashSet<i64> = mirror
            .iter()
            .filter(|((t, _), _)| *t == audit)
            .filter_map(|(_, m)| m.local.as_deref().and_then(|l| serde_json::from_str::<Vec<i64>>(l).ok()))
            .filter_map(|v| v.first().copied())
            .collect();
        let mut last = floor;
        for (seq, value) in self.store.audit_rows_after(floor, ACTIVITY_PER_CYCLE)? {
            last = seq;
            if brought.contains(&seq) {
                continue;
            }
            let digest = records::digest(&value);
            let put = serde_json::to_value(RecordPut {
                prev: None,
                row: value,
            })
            .map_err(invalid)?;
            match self.enqueue_activity(account, keys, &keys.ring, OpClass::Primary, put) {
                Ok((version, key)) => {
                    self.set_mirror(&audit, &key, Some(version), digest, Some(json!([seq]).to_string()))?;
                }
                Err(e) => log::warn!("an Activity entry is not synced: {e}"),
            }
        }
        self.store.oplog(|c| local::set_mark(c, ACTIVITY_FLOOR, &last.to_string()))?
    }

    /// After a roster change: an op at the new epoch, so the next checkpoint is at it (see [`EPOCH_KEY`]).
    fn mark_epoch(&self, account: &Account, keys: &Keys) -> Result<(), CoreError> {
        let epoch = keys.roster.epoch();
        let (stored, record, pending) = self.store.oplog(|c| -> Result<_, CoreError> {
            Ok((
                local::mark(c, CHECKPOINT_EPOCH)?.and_then(|v| v.parse::<u64>().ok()).unwrap_or(0),
                oplog::record(c, Table::Meta, EPOCH_KEY)?,
                local::pending_keys(c)?.contains(&(table_name(Table::Meta), EPOCH_KEY.to_owned())),
            ))
        })??;
        let marked = record.as_ref().and_then(|r| r.value.as_ref()).and_then(|v| v.get("epoch")).and_then(Json::as_u64);
        if stored >= epoch || marked == Some(epoch) || pending {
            return Ok(());
        }
        let change = Change::Put {
            table: Table::Meta,
            key: EPOCH_KEY.to_owned(),
            value: serde_json::to_value(RecordPut {
                prev: record.map(|r| r.version),
                row: json!({"epoch": epoch}),
            })
            .map_err(invalid)?,
        };
        self.enqueue(account, keys, &keys.ring, OpClass::Primary, change, (&table_name(Table::Meta), EPOCH_KEY))?;
        Ok(())
    }

    /// An Activity entry (`put`, a [`RecordPut`]) under its own op's key.
    fn enqueue_activity(
        &self,
        account: &Account,
        keys: &Keys,
        ring: &ViewKeyring,
        class: OpClass,
        put: Json,
    ) -> Result<(Version, String), CoreError> {
        let mut keyed = String::new();
        let version = self.enqueue_with(account, keys, ring, class, &table_name(Table::Audit), |v| {
            keyed = activity_key(v);
            (
                Change::Put {
                    table: Table::Audit,
                    key: keyed.clone(),
                    value: put,
                },
                keyed.clone(),
            )
        })?;
        Ok((version, keyed))
    }

    /// The rows kept on this phone only because no op could carry them (too large): a handover waits for them.
    pub(crate) fn unpublished_rows(&self) -> Result<Vec<String>, CoreError> {
        Ok(self
            .store
            .oplog(|c| local::marks(c, UNPUBLISHED))??
            .into_iter()
            .map(|(k, _)| k.trim_start_matches(UNPUBLISHED).replace('\u{1f}', " "))
            .collect())
    }

    fn set_mirror(
        &self,
        table: &str,
        key: &str,
        version: Option<Version>,
        digest: String,
        local_key: Option<String>,
    ) -> Result<(), CoreError> {
        let m = local::Mirrored {
            version,
            digest,
            local: local_key,
        };
        self.store.oplog(|c| local::set_mirror(c, table, key, &m))?
    }

    /// The brake pulled on a viewer, as a restrictive op for the primary (and every phone): queued and sent at once
    /// when the server can be reached. On the primary and on a legacy account there is nothing to send.
    pub(crate) async fn queue_restriction(&self, restriction: Restriction) -> Result<(), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let Some(keys) = self.sync_keys(&account)? else {
            return Ok(());
        };
        if keys.standing == Standing::Primary {
            return Ok(());
        }
        let covers = match &restriction {
            Restriction::RevokeGrant {
                grant_id,
            } => (table_name(Table::Grants), grant_id.clone()),
            Restriction::Lockdown => (table_name(Table::AutopilotSettings), "*".to_owned()),
            Restriction::StopBypass => (table_name(Table::AutopilotSettings), local::WHOLE_TABLE.to_owned()),
        };
        let _guard = self.account_sync.lock().await;
        self.enqueue(
            &account,
            &keys,
            &keys.ring,
            OpClass::Restrictive,
            Change::Restrict(restriction),
            (&covers.0, &covers.1),
        )?;
        if let Err(e) = self.upload_outbox(&session, &account).await {
            log::warn!("the brake waits to reach the server: {e}");
        }
        Ok(())
    }

    /// After the brake was pulled on this phone: a viewer sends it to the others as a restrictive op (the primary's
    /// own change goes out with its publish). The brake holds here whatever happens to the op.
    pub(crate) async fn brake_out(&self, restriction: Restriction) -> Result<(), CoreError> {
        if self.decides() {
            return Ok(());
        }
        if let Err(e) = self.queue_restriction(restriction).await {
            log::warn!("the brake was pulled here but is not queued for the other phones: {e}");
        }
        Ok(())
    }

    /// A phone that is no longer primary sends no primary op: they would be refused (and must not go out late). What
    /// it queued above them (its brake as a viewer) is renumbered, so no counter gap holds it back on the others.
    fn drop_primary_ops(&self, account: &Account) -> Result<(), CoreError> {
        let primary: Vec<local::Queued> = self
            .store
            .oplog(|c| local::outbox(c))??
            .into_iter()
            .filter(|q| q.op.class == OpClass::Primary && !q.sent)
            .collect();
        if primary.is_empty() {
            return Ok(());
        }
        self.drop_and_renumber(account, &primary, None)
    }

    /// Takes `dropped` out of the outbox (their records compared afresh by the next publish; a brake or proposal among
    /// them is told to the user, `why`), and renumbers every unsent op queued above them.
    fn drop_and_renumber(
        &self,
        account: &Account,
        dropped: &[local::Queued],
        why: Option<&str>,
    ) -> Result<(), CoreError> {
        let Some(keys) = self.sync_keys(account)? else {
            return Ok(());
        };
        let lowest = dropped.iter().map(|q| q.counter).min().unwrap_or(u64::MAX);
        if let Some(why) = why {
            let mut refused = self.live().refused.lock().unwrap_or_else(PoisonError::into_inner);
            for q in dropped.iter().filter(|q| q.op.class != OpClass::Primary) {
                let what = if q.op.class == OpClass::Restrictive {
                    "A brake"
                } else {
                    "A proposal"
                };
                refused.push(format!("{what} from this phone was refused by the server: {why}"));
            }
        }
        self.store.oplog(|c| local::drop_queued(c, dropped))??;
        let above: Vec<local::Queued> =
            self.store.oplog(|c| local::outbox(c))??.into_iter().filter(|q| !q.sent && q.counter > lowest).collect();
        self.renumber(account, &keys, above)
    }

    /// Queues `ops` again under new counters (after everything this phone took or queued), each with its change as it
    /// was: what a primary op can no longer be (this phone is not primary) is dropped instead.
    fn renumber(&self, account: &Account, keys: &Keys, ops: Vec<local::Queued>) -> Result<(), CoreError> {
        if ops.is_empty() {
            return Ok(());
        }
        self.store.oplog(|c| local::forget_queued(c, &ops))??;
        let mirror = self.store.oplog(|c| local::mirror(c))??;
        let audit = table_name(Table::Audit);
        for q in ops {
            let body = (q.op.class != OpClass::Primary || keys.standing == Standing::Primary)
                .then(|| open_body(&keys.ring, &q.op))
                .flatten();
            // Sealed again under the very key it was sealed with: a view-key grant stays under the key its viewer
            // holds, and nothing queued before a grant moves to the key the grant brings (its viewer would wait at it).
            let ring = sealed_kid_of(&q.op).map_or_else(|| keys.ring.clone(), |kid| older_ring(&keys.ring, kid + 1));
            let requeued = match body.map(|b| b.change) {
                // An Activity entry is keyed by its op: under its new number, its key is new too.
                Some(Change::Put {
                    table: Table::Audit,
                    value,
                    ..
                }) => {
                    self.enqueue_activity(account, keys, &ring, q.op.class, value.clone()).and_then(|(version, key)| {
                        let seen = mirror.get(&(audit.clone(), q.key.clone()));
                        let digest = value.get("row").map(records::digest).unwrap_or_default();
                        self.store.oplog(|c| local::forget_mirror(c, &audit, &q.key))??;
                        self.set_mirror(&audit, &key, Some(version), digest, seen.and_then(|m| m.local.clone()))
                    })
                }
                Some(change) => self.enqueue(account, keys, &ring, q.op.class, change, (&q.table, &q.key)).map(drop),
                None => Err(CoreError::invalid("this phone can no longer send it")),
            };
            if let Err(e) = requeued {
                // Not sent, and not taken for sent either: its record is compared afresh by the next publish (a brake
                // or proposal is told).
                log::warn!("a queued change of this phone could not be renumbered: {e}");
                self.store.oplog(|c| local::forget_mirror(c, &q.table, &q.key))??;
                if q.op.class != OpClass::Primary {
                    self.live()
                        .refused
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(format!("A change from this phone could not be sent again: {e}"));
                }
            }
        }
        Ok(())
    }

    /// After a start from a checkpoint this phone could not verify as its own (a fresh phone that recovered): every grant
    /// revoked, every mode at the tightest, no bypass, before anything is published from it. The user approves again.
    /// The mark stays until every record of the log wrote (`failed` is 0): what writes later is tightened too.
    fn tighten_untrusted_start(&self, failed: usize) -> Result<(), CoreError> {
        if !self.untrusted_start()? {
            return Ok(());
        }
        self.store.tighten_untrusted()?;
        if failed == 0 {
            self.store.oplog(|c| local::clear_mark(c, UNTRUSTED_START))??;
        }
        Ok(())
    }

    fn untrusted_start(&self) -> Result<bool, CoreError> {
        Ok(self.store.oplog(|c| local::mark(c, UNTRUSTED_START))??.is_some())
    }

    /// Writes a record into this phone's store as a sign-in or a setting would (tests: a row too large for an op).
    #[doc(hidden)]
    pub fn keep_sync_row(&self, table: &str, value: &str) -> Result<(), CoreError> {
        let table = serde_json::from_value::<Table>(Json::String(table.to_owned())).map_err(invalid)?;
        let value: Json = serde_json::from_str(value).map_err(invalid)?;
        let key = records::key_of_row(table, &value).unwrap_or_default();
        self.store.write_sync_row(table, &key, None, Some(&value)).map(drop)
    }

    /// Whether this phone's store has the record `key` of `table` (tests).
    #[doc(hidden)]
    pub fn has_sync_row(&self, table: &str, key: &str) -> Result<bool, CoreError> {
        let table = serde_json::from_value::<Table>(Json::String(table.to_owned())).map_err(invalid)?;
        Ok(self.store.sync_rows()?.iter().any(|r| r.table == table && r.key == key))
    }

    /// Whether this phone keeps a token for `service`/`account` (tests: a viewer never does).
    #[doc(hidden)]
    pub fn holds_token(&self, service: &str, account: &str) -> Result<bool, CoreError> {
        Ok(self.store.secret_get(service, account)?.is_some())
    }

    /// Keeps a token the way an integration's sign-in does (tests: a phone that held one before it became a viewer).
    #[doc(hidden)]
    pub fn keep_token(&self, service: &str, account: &str, value: &[u8]) -> Result<(), CoreError> {
        self.store.secret_put(service, account, value)
    }

    // ---- 2. upload ----------------------------------------------------------------------------------------------

    /// Sends the outbox's unsent ops, in counter order. Ok when nothing waits to be sent.
    pub(crate) async fn upload_outbox(&self, session: &Session, account: &Account) -> Result<(), CoreError> {
        loop {
            let batch: Vec<local::Queued> =
                self.store.oplog(|c| local::outbox(c))??.into_iter().filter(|q| !q.sent).take(UPLOAD_BATCH).collect();
            if batch.is_empty() {
                return Ok(());
            }
            let ops = batch.iter().map(|q| q.op.clone()).collect::<Vec<_>>();
            match api_call!(session, |api| api.post_ops(ops.clone())) {
                Ok(_) => {
                    let counters = batch.iter().map(|q| q.counter).collect::<Vec<_>>();
                    self.store.oplog(|c| local::mark_sent(c, &counters))??;
                }
                Err(ApiFailure::Status {
                    status: 409 | 429,
                    code,
                    message,
                }) => {
                    // A counter the server holds already (renumbered once this phone's own ops come back applied), a
                    // full log (the primary writes a checkpoint), or too many at once: kept, and tried again.
                    return Err(CoreError::invalid(format!(
                        "the server holds this phone's changes back ({code}): {message}"
                    )));
                }
                Err(ApiFailure::Status {
                    status: status @ 400..=499,
                    code,
                    message,
                }) => {
                    // Refused for good: the op the server names (else the batch) is dropped, and every one queued above
                    // it renumbered.
                    log::warn!("the server refused ops ({status} {code}): {message}");
                    let refused = match refused_index(&message) {
                        Some(i) if i < batch.len() => vec![batch[i].clone()],
                        _ => batch,
                    };
                    self.drop_and_renumber(account, &refused, Some(&message))?;
                    return Err(CoreError::invalid(format!("the server refused this phone's changes: {message}")));
                }
                Err(e) => return Err(e.into_core()),
            }
        }
    }

    /// The highest counter this phone used, taken or still queued: what a handover names as its last op.
    pub(crate) fn own_last_counter(&self) -> Result<u64, CoreError> {
        let me = self.store.device_id()?;
        let sign_pub = self.sign_key()?.public_b64();
        self.store.oplog(|c| -> Result<u64, CoreError> {
            Ok(oplog::author_last_counter(c, &me, &sign_pub)?.max(local::outbox_head(c)?))
        })?
    }

    /// Whether ops of this phone wait to be sent.
    pub(crate) fn has_unsent_ops(&self) -> Result<bool, CoreError> {
        Ok(self.store.oplog(|c| local::outbox(c))??.iter().any(|q| !q.sent))
    }

    // ---- 3. pull and apply --------------------------------------------------------------------------------------

    fn with_context<T>(
        &self,
        account: &Account,
        work: impl FnOnce(&mut rusqlite::Connection, &Context<'_>) -> T,
    ) -> Result<Option<T>, CoreError> {
        let Some(roster) = self.local_roster(&account.slot)? else {
            return Ok(None);
        };
        let ring = self.keyring(&account.slot)?;
        let ctx = Context {
            account: &account.id,
            roster: &roster,
            keyring: &ring,
            now: unix_now(),
        };
        Ok(Some(self.store.oplog(|c| work(c, &ctx))?))
    }

    async fn pull_and_apply(&self, session: &Session, account: &Account, wait: u32) -> Result<Pulled, CoreError> {
        let mut wait = wait;
        let (mut refreshed, mut key_tries, mut rounds) = (false, 0, 0);
        let (mut want_checkpoint, mut tried_checkpoint) = (false, false);
        let mut last = None;
        loop {
            rounds += 1;
            if rounds > 50 {
                return Ok(Pulled {
                    response: last,
                    waiting_for_key: false,
                    notice: Some("The log is long; the rest comes with the next sync.".to_owned()),
                });
            }
            let s = self.store.oplog(|c| oplog::state(c))??;
            if let Some(why) = s.forked {
                return Ok(Pulled {
                    response: last,
                    waiting_for_key: false,
                    notice: Some(format!("This phone stopped applying the account's history: {why}")),
                });
            }
            let seen = if wait > 0 {
                self.live().cursor.lock().unwrap_or_else(PoisonError::into_inner).clone()
            } else {
                None
            };
            let response =
                api_call!(session, |api| api.sync(s.seq, seen.as_deref(), wait)).map_err(ApiFailure::into_core)?;
            wait = 0;
            *self.live().cursor.lock().unwrap_or_else(PoisonError::into_inner) = Some(response.cursor.clone());
            let starts_past = response.ops.first().is_some_and(|o| o.seq > s.seq + 1)
                || (s.seq < response.floor_seq && response.checkpoint_seq.is_some());
            if (starts_past || want_checkpoint) && !tried_checkpoint {
                // The log starts past this phone, or before what it can verify (it joined later): the checkpoint first.
                (want_checkpoint, tried_checkpoint) = (false, true);
                let cp = api_call!(session, |api| api.checkpoint()).map_err(ApiFailure::into_core)?;
                match self.with_context(account, |c, ctx| checkpoint::load(c, ctx, &cp))? {
                    Some(Ok(_)) => continue,
                    Some(Err(Halt::UnknownEpoch(_))) if !refreshed => {
                        refreshed = true;
                        self.refresh_roster(session, account).await?;
                        continue;
                    }
                    Some(Err(Halt::MissingKey(kid))) => {
                        if key_tries < 2 && self.take_key(session, account, kid, &mut refreshed).await? {
                            key_tries += 1;
                            continue;
                        }
                        return Ok(Pulled {
                            response: Some(response),
                            waiting_for_key: true,
                            notice: None,
                        });
                    }
                    Some(Err(e)) => {
                        // A primary with no state at all (it recovered the account on a fresh phone) cannot verify
                        // a checkpoint of its own epoch, which it writes itself: it starts from the last one there is.
                        let primary = self
                            .local_roster(&account.slot)?
                            .map(|r| self.standing_in(&r))
                            .transpose()?
                            .is_some_and(|st| st == Standing::Primary);
                        if primary
                            && s.seq == 0
                            && let Some(Ok(_)) =
                                self.with_context(account, |c, ctx| checkpoint::load_bootstrap(c, ctx, &cp))?
                        {
                            // Untrusted: once in the store, everything it loosens is tightened (see `tighten`).
                            self.store.oplog(|c| local::set_mark(c, UNTRUSTED_START, "1"))??;
                            continue;
                        }
                        return Ok(Pulled {
                            response: Some(response),
                            waiting_for_key: false,
                            notice: Some(format!("The account's checkpoint was not loaded: {e}")),
                        });
                    }
                    None => return Err(CoreError::invalid("This account has no phone roles on this phone.")),
                }
            }
            let (_, halt) = self
                .with_context(account, |c, ctx| oplog::apply_all(c, ctx, &response.ops))?
                .ok_or_else(|| CoreError::invalid("This account has no phone roles on this phone."))?;
            let more = response.more;
            last = Some(response);
            match halt {
                None if more => {}
                None => {
                    return Ok(Pulled {
                        response: last,
                        waiting_for_key: false,
                        notice: None,
                    });
                }
                Some(Halt::UnknownEpoch(_)) if !refreshed => {
                    refreshed = true;
                    self.refresh_roster(session, account).await?;
                }
                Some(Halt::MissingKey(kid)) => {
                    if !(key_tries < 2 && self.take_key(session, account, kid, &mut refreshed).await?) {
                        return Ok(Pulled {
                            response: last,
                            waiting_for_key: true,
                            notice: None,
                        });
                    }
                    key_tries += 1;
                }
                Some(Halt::Gap {
                    ..
                }) if rounds < 5 => {}
                Some(Halt::Unverifiable(_))
                    if !tried_checkpoint && last.as_ref().is_some_and(|r| r.checkpoint_seq.is_some()) =>
                {
                    want_checkpoint = true;
                }
                Some(halt) => {
                    return Ok(Pulled {
                        response: last,
                        waiting_for_key: false,
                        notice: Some(format!("Applying the account's history stopped: {halt}")),
                    });
                }
            }
        }
    }

    /// Before a recovery is signed: applies the account's log under `roster` (the roster before the recovery) with
    /// `ring` (every view key of the outgoing owner), and returns the last op of `old_primary` it verified, the one the
    /// entry may name. A phone that is no member yet reads the log with its recovery proof (`proof`).
    ///
    /// The outgoing primary's checkpoint loads only when it verifies (the current primary's, at the current epoch or,
    /// on a phone with nothing, an earlier epoch of its tenure), then the signed ops after it. On a phone that started
    /// with nothing, whatever a checkpoint brought is untrusted (a thief may have put it just before): it is tightened
    /// before anything is published. Refused, so that nothing is signed: a history that does not verify, an op of the
    /// outgoing primary held for one before it (the entry would vouch for a gap), and a log that ends before the head
    /// the outgoing primary signed in its last heartbeat, or differs from it there.
    pub(crate) async fn recovery_catch_up(
        &self,
        session: &Session,
        account: &Account,
        roster: &LocalRoster,
        ring: &ViewKeyring,
        proof: &str,
        old_primary: &reins_proto::roster::Member,
    ) -> Result<u64, CoreError> {
        // One applier at a time: the live loop waits.
        let _guard = self.account_sync.lock().await;
        let started_empty = self.store.oplog(|c| -> Result<bool, CoreError> {
            let s = oplog::state(c)?;
            Ok(s.seq == 0 && oplog::all_records(c)?.is_empty())
        })??;
        let mut tried_checkpoint = false;
        let mut want_checkpoint = false;
        let mut heartbeat = None;
        let mut done = false;
        for _ in 0..400 {
            let s = self.store.oplog(|c| oplog::state(c))??;
            if let Some(why) = s.forked {
                return Err(CoreError::invalid(format!("This phone's copy of the account's history forked: {why}")));
            }
            let request = reins_proto::oplog::RecoveryLogRequest {
                master_password_hash: proof.to_owned(),
                after: s.seq,
                with_checkpoint: want_checkpoint,
            };
            let log = api_call!(session, |api| api.recovery_log(&request)).map_err(ApiFailure::into_core)?;
            heartbeat = log.heartbeat.clone().or(heartbeat);
            let ctx = Context {
                account: &account.id,
                roster,
                keyring: ring,
                now: unix_now(),
            };
            let starts_past = s.seq < log.floor_seq || log.ops.first().is_some_and(|o| o.seq > s.seq + 1);
            if starts_past {
                let Some(cp) = log.checkpoint.as_ref().filter(|cp| cp.seq > s.seq) else {
                    if want_checkpoint {
                        return Err(CoreError::invalid(
                            "The server holds no checkpoint for the history it dropped; nothing was signed.",
                        ));
                    }
                    want_checkpoint = true;
                    continue;
                };
                if tried_checkpoint {
                    return Err(CoreError::invalid("The account's history does not continue its checkpoint."));
                }
                tried_checkpoint = true;
                want_checkpoint = false;
                let loaded = match self.store.oplog(|c| checkpoint::load(c, &ctx, cp))? {
                    Ok(check) => Ok(check),
                    Err(_) if started_empty => self.store.oplog(|c| checkpoint::load_tenure(c, &ctx, cp))?,
                    Err(e) => Err(e),
                };
                loaded.map_err(|e| CoreError::invalid(format!("The account's checkpoint does not verify: {e}")))?;
                if started_empty {
                    self.store.oplog(|c| local::set_mark(c, UNTRUSTED_START, "1"))??;
                }
                continue;
            }
            if log.ops.is_empty() {
                done = true;
                break;
            }
            let (_, halt) = self.store.oplog(|c| oplog::apply_all(c, &ctx, &log.ops))?;
            match halt {
                None if log.more => {}
                None => {
                    done = true;
                    break;
                }
                Some(halt) => {
                    return Err(CoreError::invalid(format!(
                        "The account's history could not be verified ({halt}); nothing was signed."
                    )));
                }
            }
        }
        if !done {
            return Err(CoreError::invalid("The account's history is too long to verify now; nothing was signed."));
        }
        if let Some(hb) = &heartbeat {
            self.reaches_signed_head(roster, hb)?;
        }
        let sign_pub = old_primary.sign_pub.clone();
        let (applied, held) = self.store.oplog(|c| -> Result<(u64, u64), CoreError> {
            Ok((
                oplog::author_last_counter(c, &old_primary.device_id, &sign_pub)?,
                oplog::seen_counter(c, old_primary)?,
            ))
        })??;
        if held > applied {
            return Err(CoreError::invalid(
                "Some of the lost primary's changes came without the ones before them; nothing was signed. Try again \
                 later.",
            ));
        }
        Ok(applied)
    }

    /// Whether this phone's log reaches the head `hb` signs, with the same history there: else the server withheld or
    /// changed part of the outgoing primary's history (refused). A heartbeat that does not verify against `roster` is
    /// no evidence either way.
    fn reaches_signed_head(&self, roster: &LocalRoster, hb: &reins_proto::oplog::Heartbeat) -> Result<(), CoreError> {
        let Some(entry) = roster.at(hb.epoch) else {
            return Ok(());
        };
        let verifies = entry.primary.device_id == hb.author
            && entry
                .primary
                .sign_key()
                .ok()
                .zip(reins_proto::canon::parse_sig(&hb.sig))
                .zip(hb.signing_bytes().ok())
                .is_some_and(|((key, sig), message)| crate::roles::keys::ed25519_verify(&key, &message, &sig));
        if !verifies {
            return Ok(());
        }
        let withheld = || {
            CoreError::invalid(
                "The server shows less of the account's history than the lost primary signed it had; nothing was \
                 signed.",
            )
        };
        let s = self.store.oplog(|c| oplog::state(c))??;
        if hb.head_seq > s.seq {
            return Err(withheld());
        }
        let theirs = reins_proto::canon::parse_hex32(&hb.applied_hash);
        match self.store.oplog(|c| oplog::applied_at(c, hb.head_seq))?? {
            Some(mine) if Some(mine) != theirs => Err(withheld()),
            _ => Ok(()),
        }
    }

    /// Applies what the log holds, up to the first op this phone cannot take yet (no roster fetch, no store writes;
    /// the next cycle writes the store): so that a roster entry naming ops of the outgoing primary this phone had not
    /// applied can come in.
    pub(crate) async fn catch_up_log(&self, session: &Session, account: &Account) -> Result<(), CoreError> {
        for _ in 0..20 {
            let s = self.store.oplog(|c| oplog::state(c))??;
            if s.forked.is_some() {
                return Ok(());
            }
            let response = api_call!(session, |api| api.sync(s.seq, None, 0)).map_err(ApiFailure::into_core)?;
            if response.ops.is_empty() {
                return Ok(());
            }
            let Some((_, halt)) = self.with_context(account, |c, ctx| oplog::apply_all(c, ctx, &response.ops))? else {
                return Ok(());
            };
            match halt {
                // A key rotated by a removal: the grant the log brought (the roster is not fetched again here).
                Some(Halt::MissingKey(kid)) => {
                    let mut refreshed = true;
                    if !self.take_key(session, account, kid, &mut refreshed).await? {
                        return Ok(());
                    }
                }
                Some(_) => return Ok(()),
                None if !response.more => return Ok(()),
                None => {}
            }
        }
        Ok(())
    }

    /// Brings in view key `kid`: from the grant the log brought this phone, else (the primary derives its own) from a
    /// fresh roster. True when the keyring has it now.
    async fn take_key(
        &self,
        session: &Session,
        account: &Account,
        kid: u64,
        refreshed: &mut bool,
    ) -> Result<bool, CoreError> {
        let me = self.store.device_id()?;
        let key = format!("{GRANT_PREFIX}{me}/{kid}");
        let record = self.store.oplog(|c| oplog::record(c, Table::Meta, &key))??;
        if let Some(sealed) =
            record.and_then(|r| r.value).and_then(|v| v.get("sealed").and_then(Json::as_str).map(str::to_owned))
        {
            if !*refreshed {
                *refreshed = true;
                self.refresh_roster(session, account).await?;
            }
            let roster = self.local_roster(&account.slot)?.ok_or(CoreError::NotFound)?;
            match crate::roles::sealed::open_view_grant(&self.box_key()?, &sealed, &account.id, &me, |k| {
                roster.view_key_signer(k)
            }) {
                Ok(view) => {
                    let mut ring = self.keyring(&account.slot)?;
                    ring.insert(view);
                    self.save_keyring(&account.slot, &ring)?;
                    return Ok(ring.get(kid).is_some());
                }
                Err(e) => log::warn!("a view-key grant for this phone does not open: {e}"),
            }
        }
        if !*refreshed {
            *refreshed = true;
            self.refresh_roster(session, account).await?;
            return Ok(self.keyring(&account.slot)?.get(kid).is_some());
        }
        Ok(false)
    }

    // ---- 4. into the store --------------------------------------------------------------------------------------

    /// Writes into the store every record that changed since the mirror (see the module docs).
    fn materialize(&self, account: &Account) -> Result<usize, CoreError> {
        let Some(keys) = self.sync_keys(account)? else {
            return Ok(0);
        };
        let primary = keys.standing == Standing::Primary;
        let credential = if primary {
            self.owner_seed(&account.slot)?.map(|s| s.credential_key())
        } else {
            None
        };
        // This phone's ops the log took are no longer pending (their records are the log's now, joined or not).
        self.settle_outbox(account, &keys)?;
        let (all, mirror, pending, since, applied) = self.store.oplog(|c| -> Result<_, CoreError> {
            Ok((
                oplog::all_records(c)?,
                local::mirror(c)?,
                local::pending_keys(c)?,
                local::mark(c, MATERIALIZED_SEQ)?.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0),
                oplog::state(c)?.seq,
            ))
        })??;
        let audit = table_name(Table::Audit);
        let untrusted = self.untrusted_start()?;
        let mut failed = 0;
        let live_keys: HashSet<(Table, String)> =
            all.iter().filter(|r| r.value.is_some()).map(|r| (r.table, r.key.clone())).collect();
        for record in all {
            if record.key.starts_with(RESERVED_PREFIX) || records::sql_table(record.table).is_none() {
                continue;
            }
            let tbl = table_name(record.table);
            if pending.contains(&(tbl.clone(), record.key.clone()))
                || pending.contains(&(tbl.clone(), local::WHOLE_TABLE.to_owned()))
            {
                continue;
            }
            // A viewer takes no token; nor does anyone from a start it could not verify (seen, never written).
            if record.table == Table::Secrets && (!primary || untrusted) {
                if untrusted && primary {
                    self.set_mirror(&tbl, &record.key, Some(record.version.clone()), String::new(), None)?;
                }
                continue;
            }
            let seen = mirror.get(&(tbl.clone(), record.key.clone()));
            let moved = seen.and_then(|m| m.version.as_ref()) != Some(&record.version);
            if !moved && record.seq <= since {
                continue;
            }
            if tbl == audit {
                // Grow-only: an entry the log brought is added once.
                if seen.is_none()
                    && let Some(value) = &record.value
                {
                    let local_key = match self.store.insert_sync_audit(value) {
                        Ok(seq) => Some(json!([seq]).to_string()),
                        Err(e) => {
                            log::warn!("an Activity entry from the log is not shown: {e}");
                            None
                        }
                    };
                    self.set_mirror(
                        &tbl,
                        &record.key,
                        Some(record.version.clone()),
                        records::digest(value),
                        local_key,
                    )?;
                }
                continue;
            }
            let plain = match (&record.value, record.table) {
                (None, _) => None,
                (Some(v), Table::Secrets) => {
                    let opened = credential.as_ref().and_then(|key| {
                        let raw = BASE64URL_NOPAD.decode(v.get("sealed")?.as_str()?.as_bytes()).ok()?;
                        serde_json::from_slice::<Json>(&key.open(&raw).ok()?).ok()
                    });
                    if opened.is_none() {
                        log::warn!("a token from the log does not open with this phone's credential key");
                        self.set_mirror(
                            &tbl,
                            &record.key,
                            Some(record.version.clone()),
                            String::new(),
                            seen.and_then(|m| m.local.clone()),
                        )?;
                        continue;
                    }
                    opened
                }
                (Some(v), _) => Some(v.clone()),
            };
            let plain = match plain {
                Some(v) => Some(self.store.normalized(record.table, &v)?),
                None => None,
            };
            let digest = plain.as_ref().map(records::digest).unwrap_or_default();
            // A row this phone had before it took the log (a leftover from before phone roles) is found by its key.
            let local_key = seen.and_then(|m| m.local.clone()).or_else(|| records::local_of(record.table, &record.key));
            if seen.is_some_and(|m| m.digest == digest) {
                self.set_mirror(&tbl, &record.key, Some(record.version.clone()), digest, local_key)?;
                continue;
            }
            let failing = failed_mark(&tbl, &record.key);
            let this_version = format!("{}.{}.{}", record.version.epoch, record.version.author, record.version.counter);
            let (due, tries) = retry_due(
                record.table,
                self.store.oplog(|c| local::mark(c, &failing))??.as_deref(),
                &this_version,
                unix_now(),
            );
            if !due {
                // Tried enough for now: waits for the record to change, or (a brake's table) for the next try after a
                // longer wait each time; still counted, so it is shown.
                failed += 1;
                continue;
            }
            match self.store.write_sync_row(record.table, &record.key, local_key.as_deref(), plain.as_ref()) {
                Ok(found) => {
                    self.store.oplog(|c| local::clear_mark(c, &failing))??;
                    self.set_mirror(&tbl, &record.key, Some(record.version.clone()), digest, found.or(local_key))?;
                }
                Err(e) => {
                    // Not marked seen: the next syncs try again, a few times (a brake that does not land is an error,
                    // not a note).
                    log::warn!("a {tbl} record from the log is not written here: {e}");
                    let mark = format!("{this_version}|{}|{}", tries + 1, unix_now());
                    self.store.oplog(|c| local::set_mark(c, &failing, &mark))??;
                    failed += 1;
                }
            }
        }
        if !primary && applied > 0 {
            failed += self.drop_leftovers(&live_keys, &pending)?;
        }
        if failed == 0 {
            self.store.oplog(|c| local::set_mark(c, MATERIALIZED_SEQ, &applied.to_string()))??;
        }
        Ok(failed)
    }

    /// On a viewer: rows of the security tables the log does not have (left from before phone roles, or removed since)
    /// are deleted, so that none can become live state if this phone is primary one day.
    fn drop_leftovers(
        &self,
        live: &HashSet<(Table, String)>,
        pending: &HashSet<(String, String)>,
    ) -> Result<usize, CoreError> {
        let mut failed = 0;
        let kept: HashSet<String> =
            self.store.oplog(|c| local::marks(c, UNPUBLISHED))??.into_iter().map(|(k, _)| k).collect();
        for row in self.store.sync_rows()? {
            if !LOG_ONLY.contains(&row.table) || live.contains(&(row.table, row.key.clone())) {
                continue;
            }
            // A row this phone kept because no op could carry it is its own, never a leftover.
            if kept.contains(&unpublished_mark(&table_name(row.table), &row.key)) {
                continue;
            }
            let tbl = table_name(row.table);
            if pending.contains(&(tbl.clone(), row.key.clone()))
                || pending.contains(&(tbl, local::WHOLE_TABLE.to_owned()))
            {
                continue;
            }
            if let Err(e) = self.store.write_sync_row(row.table, &row.key, Some(&row.local), None) {
                log::warn!("a leftover row is not removed here: {e}");
                failed += 1;
            }
        }
        Ok(failed)
    }

    /// Forgets the ops of this phone the log brought back applied. One that was never marked sent, at or below this
    /// phone's counter now, either landed (its record has its version) or lost its number to another: renumbered.
    fn settle_outbox(&self, account: &Account, keys: &Keys) -> Result<(), CoreError> {
        let own = self.own_applied_counter()?;
        let stale: Vec<local::Queued> = self.store.oplog(|c| -> Result<Vec<local::Queued>, CoreError> {
            local::prune_sent(c, own)?;
            Ok(local::outbox(c)?.into_iter().filter(|q| !q.sent && q.counter <= own).collect())
        })??;
        let mut landed = Vec::new();
        let mut lost = Vec::new();
        for q in stale {
            // Landed only if the log holds this very op (a brake on a whole table, or a record another op wrote
            // since, says nothing of it).
            let applied =
                q.op.hash().ok().map(|h| self.store.oplog(|c| oplog::holds_op(c, &h))).transpose()?.transpose()?;
            // Beyond what this phone keeps of the log (older than its window, or before the checkpoint it loaded), the
            // record its op wrote is the evidence left: renumbering it would send it twice.
            let beyond = self.store.oplog(|c| oplog::first_kept_seq(c))??.is_some_and(|first| first > 1);
            let wrote = || -> Result<bool, CoreError> {
                let Ok(table) = serde_json::from_value::<Table>(Json::String(q.table.clone())) else {
                    return Ok(false);
                };
                let record = self.store.oplog(|c| oplog::record(c, table, &q.key))??;
                Ok(record.is_some_and(|r| r.version == Version::of(&q.op)))
            };
            if applied == Some(true) || (applied == Some(false) && beyond && wrote()?) {
                landed.push(q);
            } else {
                lost.push(q);
            }
        }
        self.store.oplog(|c| local::forget_queued(c, &landed))??;
        self.renumber(account, keys, lost)
    }

    fn own_applied_counter(&self) -> Result<u64, CoreError> {
        let me = self.store.device_id()?;
        let sign_pub = self.sign_key()?.public_b64();
        self.store.oplog(|c| oplog::author_last_counter(c, &me, &sign_pub))?
    }

    // ---- 6 and 7. heartbeats, acks, checkpoints -----------------------------------------------------------------

    fn check_live_heartbeat(&self, account: &Account, keys: &Keys, response: &SyncResponse) -> Result<(), CoreError> {
        let Some(hb) = &response.heartbeat else {
            *self.live().verified.lock().unwrap_or_else(PoisonError::into_inner) = None;
            return Ok(());
        };
        if keys.standing == Standing::Primary && hb.author == keys.me {
            return Ok(());
        }
        let checked = self.with_context(account, |c, ctx| oplog::check_heartbeat(c, ctx, hb))?;
        let mut verified = self.live().verified.lock().unwrap_or_else(PoisonError::into_inner);
        match checked {
            Some(Ok(check)) => {
                // A heartbeat dated ahead of this phone's clock does not stretch "Live".
                *verified = Some((check, seen_at(hb.t, unix_now()), hb.head_seq));
                let mut disagrees = self.live().disagrees.lock().unwrap_or_else(PoisonError::into_inner);
                match check {
                    HeartbeatCheck::Disagrees => *disagrees = true,
                    HeartbeatCheck::Matches => *disagrees = false,
                    _ => {}
                }
            }
            // Not verified: as if there were none (the warning stays as it was).
            Some(Err(e)) => {
                *verified = None;
                match e {
                    Halt::Forked(why) => log::warn!("the primary's heartbeat shows another history: {why}"),
                    other => log::debug!("a heartbeat was not checked: {other}"),
                }
            }
            None => *verified = None,
        }
        Ok(())
    }

    async fn post_heartbeat(&self, session: &Session, account: &Account, keys: &Keys) {
        let now = unix_now();
        let epoch = keys.roster.epoch();
        if now - self.live().heartbeat_posted.load(Ordering::Acquire) < HEARTBEAT_EVERY_SECS
            && self.live().heartbeat_epoch.load(Ordering::Acquire) == epoch
        {
            return;
        }
        let made = self
            .with_context(account, |c, ctx| checkpoint::heartbeat(c, ctx, &keys.signer, &keys.me, keys.roster.epoch()));
        let Ok(Some(Ok(hb))) = made else {
            return;
        };
        match api_call!(session, |api| api.heartbeat(&hb)) {
            Ok(()) => {
                self.live().heartbeat_posted.store(now, Ordering::Release);
                self.live().heartbeat_epoch.store(epoch, Ordering::Release);
            }
            Err(e) => log::debug!("the heartbeat was not posted: {}", e.into_core()),
        }
    }

    async fn maybe_checkpoint(&self, session: &Session, account: &Account, keys: &Keys, response: &SyncResponse) {
        let Ok(Ok(s)) = self.store.oplog(|c| oplog::state(c)) else {
            return;
        };
        let last = response.checkpoint_seq.unwrap_or(0);
        let at = self
            .store
            .oplog(|c| local::mark(c, CHECKPOINT_AT))
            .ok()
            .and_then(Result::ok)
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let now = unix_now();
        let epoch = self
            .store
            .oplog(|c| local::mark(c, CHECKPOINT_EPOCH))
            .ok()
            .and_then(Result::ok)
            .flatten()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let due = s.seq > last
            && (s.seq - last >= CHECKPOINT_EVERY_OPS
                || response.checkpoint_seq.is_none()
                || now - at >= CHECKPOINT_EVERY_SECS
                || epoch < keys.roster.epoch());
        if !due || s.forked.is_some() || now < self.live().checkpoint_after.load(Ordering::Acquire) {
            return;
        }
        let made = self
            .with_context(account, |c, ctx| checkpoint::produce(c, ctx, &keys.signer, &keys.me, keys.roster.epoch()));
        let Ok(Some(Ok(cp))) = made else {
            return;
        };
        let size = decode_payload(&cp.body).map_or(usize::MAX, |b| b.len());
        if size > MAX_CHECKPOINT_BYTES {
            log::warn!("the checkpoint would be {size} bytes, over the server's limit; not sent, tried again later");
            self.live().checkpoint_after.store(now + CHECKPOINT_BACKOFF_SECS, Ordering::Release);
            return;
        }
        match api_call!(session, |api| api.put_checkpoint(&cp)) {
            Ok(()) => {
                let marked = self.store.oplog(|c| -> Result<(), CoreError> {
                    local::set_mark(c, CHECKPOINT_AT, &now.to_string())?;
                    local::set_mark(c, CHECKPOINT_EPOCH, &cp.epoch.to_string())
                });
                if let Err(e) = marked {
                    log::debug!("{e}");
                }
            }
            // Another checkpoint got there first: nothing to wait for.
            Err(ApiFailure::Status {
                status: 409,
                ..
            }) => {}
            Err(e) => {
                // Refused, or the server could not be reached: not rebuilt every cycle.
                let wait = if matches!(e, ApiFailure::Status { .. }) {
                    CHECKPOINT_BACKOFF_SECS
                } else {
                    60
                };
                self.live().checkpoint_after.store(now + wait, Ordering::Release);
                log::debug!("the checkpoint was not stored: {}", e.into_core());
            }
        }
    }
}

/// When a heartbeat signed at `t` counts as seen, checked at `now`: never later than now (a heartbeat dated ahead of
/// this phone's clock does not stretch "Live").
const fn seen_at(t: i64, now: i64) -> i64 {
    if t < now {
        t
    } else {
        now
    }
}

/// Whether a heartbeat that verified as `check`, seen at `t`, makes the primary live at `now`.
fn is_live(check: HeartbeatCheck, t: i64, now: i64) -> bool {
    check == HeartbeatCheck::Matches && now - t < LIVE_SECS
}

/// How often a record of version `version` failed to write here, and when last (the mark is
/// `<version>|<tries>|<at>`; another version starts again at 0).
fn write_tries(mark: Option<&str>, version: &str) -> (u32, i64) {
    let Some(rest) = mark.and_then(|m| m.strip_prefix(version)?.strip_prefix('|')) else {
        return (0, 0);
    };
    let mut parts = rest.split('|');
    let tries = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
    let at = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
    (tries, at)
}

/// Whether a record of `table` that failed to write is tried again now, and how often it failed. A record of a table
/// the brake changes (grants, Autopilot settings) is never given up on: after [`MAX_WRITE_TRIES`] it waits longer each
/// time (at most an hour); any other waits for a newer version.
fn retry_due(table: Table, mark: Option<&str>, version: &str, now: i64) -> (bool, u32) {
    let (tries, at) = write_tries(mark, version);
    if tries < MAX_WRITE_TRIES {
        return (true, tries);
    }
    if !matches!(table, Table::Grants | Table::AutopilotSettings) {
        return (false, tries);
    }
    let wait = 60_i64.saturating_mul(1 << (tries - MAX_WRITE_TRIES).min(6)).min(3_600);
    (now - at >= wait, tries)
}

/// The ring a view-key grant of `kid` travels under: the keys older than it (the ones its viewer holds).
fn older_ring(ring: &ViewKeyring, kid: u64) -> ViewKeyring {
    let mut older = ViewKeyring::default();
    for k in ring.kids().into_iter().filter(|k| *k < kid) {
        if let Some(key) = ring.get(k) {
            older.insert(key.clone());
        }
    }
    older
}

/// The key id an op's body is sealed under.
fn sealed_kid_of(op: &reins_proto::oplog::Op) -> Option<u64> {
    crate::roles::keys::sealed_kid(&decode_payload(&op.body)?)
}

fn unpublished_mark(table: &str, key: &str) -> String {
    format!("{UNPUBLISHED}{table}\u{1f}{key}")
}

fn failed_mark(table: &str, key: &str) -> String {
    format!("{FAILED}{table}\u{1f}{key}")
}

/// The op index a refusal of `POST /ops` names ("op 3: …").
fn refused_index(message: &str) -> Option<usize> {
    message.strip_prefix("op ")?.split(':').next()?.trim().parse().ok()
}

/// An op's body, opened with this phone's keys.
fn open_body(ring: &ViewKeyring, op: &reins_proto::oplog::Op) -> Option<OpBody> {
    let sealed = decode_payload(&op.body)?;
    let plain = ring.open(&body_associated_data(op), &sealed).ok()?;
    serde_json::from_slice(&plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heartbeat_dated_ahead_never_stretches_live() {
        let now = 1_000_000;
        // Dated an hour ahead: seen as now, and live only for the usual two minutes from now.
        let t = seen_at(now + 3_600, now);
        assert_eq!(t, now);
        assert!(is_live(HeartbeatCheck::Matches, t, now + LIVE_SECS - 1));
        assert!(!is_live(HeartbeatCheck::Matches, t, now + LIVE_SECS));
        // Only a heartbeat that matched counts.
        for other in [HeartbeatCheck::Behind, HeartbeatCheck::Unknown, HeartbeatCheck::Stale, HeartbeatCheck::Disagrees]
        {
            assert!(!is_live(other, now, now));
        }
    }

    #[test]
    fn a_record_that_does_not_write_is_tried_a_few_times_and_a_brake_never_given_up() {
        let now = 1_000_000;
        assert_eq!(retry_due(Table::Meta, None, "3.p.7", now), (true, 0));
        assert_eq!(retry_due(Table::Meta, Some("3.p.7|2|5"), "3.p.7", now), (true, 2));
        // Enough for a setting: it waits for a newer version, which starts again.
        assert_eq!(retry_due(Table::Meta, Some("3.p.7|3|5"), "3.p.7", now), (false, 3));
        assert_eq!(retry_due(Table::Meta, Some("3.p.7|3|5"), "3.p.8", now), (true, 0));
        // A brake's table: tried again after a wait that grows, never given up.
        assert!(!retry_due(Table::AutopilotSettings, Some(&format!("3.v.1|3|{now}")), "3.v.1", now + 30).0);
        assert!(retry_due(Table::AutopilotSettings, Some(&format!("3.v.1|3|{now}")), "3.v.1", now + 60).0);
        assert!(!retry_due(Table::Grants, Some(&format!("3.v.1|9|{now}")), "3.v.1", now + 1_000).0);
        assert!(retry_due(Table::Grants, Some(&format!("3.v.1|40|{now}")), "3.v.1", now + 3_600).0);
    }

    #[test]
    fn a_grant_and_a_renumbered_op_travel_under_the_key_they_were_sealed_with() {
        let owner = crate::roles::keys::OwnerSeed::generate().unwrap();
        let mut ring = ViewKeyring::default();
        for kid in [1, 4, 6] {
            ring.insert(owner.view_key("acct", kid));
        }
        assert_eq!(older_ring(&ring, 6).kids(), [1, 4]);
        // An op renumbered goes under the key it was sealed with, not the newest.
        let body = OpBody {
            v: reins_proto::PROTOCOL_VERSION,
            change: Change::Restrict(Restriction::Lockdown),
        };
        let signer = SigningKey::from_seed(&[1; 32]).unwrap();
        let op = oplog::seal_op("acct", &older_ring(&ring, 5), &signer, "p", 4, 1, OpClass::Restrictive, false, &body)
            .unwrap();
        assert_eq!(sealed_kid_of(&op), Some(4));
        assert_eq!(older_ring(&ring, sealed_kid_of(&op).unwrap() + 1).current().map(|k| k.kid), Some(4));
    }
}
