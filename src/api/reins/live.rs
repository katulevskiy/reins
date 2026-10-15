//! Live sync between an account's phones (`reins_proto::oplog`): the op log, the sync long-poll, acknowledgements,
//! checkpoints and the primary's heartbeat. Only for accounts with a roster ([`super::roles`]); the server stores what
//! the phones signed and sealed and checks who may write it.
//!
//! - `POST /reins/api/ops` (a member, or a removed one: `authorize_op` decides): signed ops, all stored or none.
//! - `GET /reins/api/sync?after=<seq>&seen=<cursor>&wait=<s>` (a current member): ops after `after`, the epoch, the
//!   heartbeat, what waits for the primary; long-polls like `GET /pending`.
//! - `POST /reins/api/ack` (a current member): what it applied, for tombstone GC.
//! - `PUT /reins/api/checkpoint` (the primary), `GET /reins/api/checkpoint` (a current member).
//! - `POST /reins/api/heartbeat` (the primary).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use reins_proto::{
    canon::parse_sig,
    device::codes,
    oplog::{
        Ack, Checkpoint, HEARTBEAT_SKEW_SECS, Heartbeat, MAX_CHECKPOINT_BYTES, MAX_OPS_PER_BATCH, MAX_SYNC_OPS,
        MIN_HEARTBEAT_SECS, Op, OpBatch, OpClass, OpsAppended, SyncResponse, SyncedOp, Waiting, authorize_op,
    },
    pairing::PushMessage,
    roster::RosterEntry,
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};
use tokio::sync::watch;

use super::{
    HUB,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, clamp_wait, internal, rate_limited, read_body_limited,
        require_approval_device, too_many_running, user_key,
    },
    now_unix, push,
    roles::{chain, ed25519_verify, primary_at, require_any_member, require_member},
};
use crate::{
    CONFIG,
    auth::Headers,
    db::{
        DbConn, DbPool,
        models::{
            LogLimits, ROLE_PRIMARY, ROLE_VIEWER, ReinsCheckpointRow, ReinsDevice, ReinsMember, ReinsOps, ReinsRoster,
            WriteError,
        },
    },
};

pub fn routes() -> Vec<Route> {
    routes![post_ops, get_sync, post_ack, put_checkpoint, get_checkpoint, post_heartbeat]
}

/// Per account: a version bumped on every change members sync (an op, the roster, a heartbeat, a waiting item answered)
/// and the log head, so long-polls wake without asking the database. Entries nobody listens to are dropped by the
/// purge; there is no global cap (a missing entry is only a cache miss).
#[derive(Default)]
pub struct LiveHub {
    accounts: Mutex<HashMap<String, Arc<Live>>>,
}

struct Live {
    version: watch::Sender<u64>,
    /// The log head as last stored; -1 unknown.
    head: AtomicI64,
}

impl LiveHub {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<Live>>> {
        self.accounts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn entry(&self, user: &str) -> Arc<Live> {
        Arc::clone(self.lock().entry(user.to_owned()).or_insert_with(|| {
            Arc::new(Live {
                version: watch::Sender::new(0),
                head: AtomicI64::new(-1),
            })
        }))
    }

    /// Something members sync changed: wake their long-polls.
    pub fn bump(&self, user: &str) {
        self.entry(user).version.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub fn subscribe(&self, user: &str) -> watch::Receiver<u64> {
        self.entry(user).version.subscribe()
    }

    /// Raises the cached log head.
    pub fn set_head(&self, user: &str, head: i64) {
        self.entry(user).head.fetch_max(head, Ordering::SeqCst);
    }

    pub fn head(&self, user: &str) -> Option<u64> {
        let head = self.lock().get(user)?.head.load(Ordering::SeqCst);
        u64::try_from(head).ok()
    }

    /// Drops accounts nobody waits on.
    pub fn purge(&self) {
        self.lock().retain(|_, live| live.version.receiver_count() > 0);
    }

    pub fn forget(&self, user: &str) {
        self.lock().remove(user);
    }
}

/// The account's log head (and caches it).
pub async fn log_head(user: &str, conn: &DbConn) -> PhoneResult<u64> {
    let state = ReinsOps::state(user, conn).await.map_err(|e| internal(&e))?;
    HUB.live.set_head(user, state.head);
    Ok(u64::try_from(state.head).unwrap_or(0))
}

fn refused(
    code: &str,
    i: usize,
    message: impl std::fmt::Display,
) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    let status = match code {
        codes::COUNTER_REPLAY | codes::LOG_FULL => Status::Conflict,
        codes::BAD_REQUEST | codes::BAD_VERSION => Status::BadRequest,
        _ => Status::Forbidden,
    };
    api_err(status, code, format!("op {i}: {message}"))
}

fn log_limits() -> LogLimits {
    LogLimits {
        max_ops: i64::from(CONFIG.reins_account_log_max_ops()),
        max_bytes: i64::try_from(CONFIG.reins_account_log_max_bytes()).unwrap_or(i64::MAX),
    }
}

/// Largest `POST /ops` body: a full batch of the largest ops, in base64 and JSON.
const MAX_OPS_BODY: u64 = 10 * 1024 * 1024;

/// Stores the caller's signed ops. Each must be the caller's own, valid, allowed by the roster for its class
/// (`authorize_op`), signed by the key the roster lists for its author at its epoch, and numbered above the author's
/// last stored counter (the same op again, byte for byte, is answered with its `seq`: a retry). Proposals count
/// against the author's hourly budget. The batch is stored whole or not at all. The other members are woken: viewers
/// with a `sync` push (an alert when an op asks for one), the primary when a viewer proposed or pulled the brake.
#[post("/reins/api/ops", data = "<data>")]
async fn post_ops(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<OpsAppended>> {
    let member = require_any_member(&headers, &key, &conn).await?;
    let (user, author) = (member.user_uuid.clone(), member.device_uuid.clone());
    if let Err(wait) = super::limits::MEMBER_OP_REQUESTS.check(&(user.clone(), author.clone())) {
        return Err(rate_limited("too many op uploads from this phone", wait));
    }
    let batch: OpBatch = serde_json::from_slice(&read_body_limited(data, MAX_OPS_BODY).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    if batch.ops.is_empty() || batch.ops.len() > MAX_OPS_PER_BATCH {
        return Err(bad_request(format!("1 to {MAX_OPS_PER_BATCH} ops per request")));
    }
    let entries = chain(&user, &conn).await?;
    let mut previous_counter = 0u64;
    let mut retried = Vec::new();
    let mut fresh = Vec::new();
    for (i, op) in batch.ops.iter().enumerate() {
        op.validate().map_err(|e| refused(codes::BAD_REQUEST, i, e))?;
        if op.author != author {
            return Err(refused(codes::OP_REFUSED, i, "a phone posts only its own ops"));
        }
        if op.counter <= previous_counter {
            return Err(refused(codes::BAD_REQUEST, i, "counters rise within a batch"));
        }
        previous_counter = op.counter;
        let signer = authorize_op(op, &entries).map_err(|e| refused(codes::OP_REFUSED, i, e))?;
        let verified = signer
            .sign_key()
            .ok()
            .zip(parse_sig(&op.sig))
            .zip(op.signing_bytes().ok())
            .is_some_and(|((k, s), m)| ed25519_verify(&k, &m, &s));
        if !verified {
            return Err(refused(codes::BAD_SIGNATURE, i, "the signature does not verify against the author's key"));
        }
        let counter = i64::try_from(op.counter).map_err(|_| refused(codes::BAD_REQUEST, i, "counter out of range"))?;
        let json = serde_json::to_string(op).map_err(|e| refused(codes::BAD_REQUEST, i, e))?;
        if counter <= member.last_counter {
            if !fresh.is_empty() {
                return Err(refused(codes::COUNTER_REPLAY, i, "an old counter after new ones"));
            }
            // A retry of an op stored before (its answer was lost) is answered with its seq; anything else is a replay.
            let stored = ReinsOps::find(&user, &author, counter, &conn).await.map_err(|e| internal(&e))?;
            match stored.filter(|row| serde_json::from_str::<Op>(&row.op).is_ok_and(|o| o == *op)) {
                Some(row) => retried.push(u64::try_from(row.seq).unwrap_or(0)),
                None => {
                    return Err(refused(
                        codes::COUNTER_REPLAY,
                        i,
                        format!("counter {} is not above the last stored, {}", op.counter, member.last_counter),
                    ));
                }
            }
        } else {
            fresh.push((counter, json, op));
        }
    }
    let proposals = fresh.iter().filter(|(_, _, op)| op.class == OpClass::Proposal).count();
    for _ in 0..proposals {
        if let Err(wait) = super::limits::MEMBER_PROPOSALS.check(&(user.clone(), author.clone())) {
            return Err(rate_limited("too many proposals from this phone", wait));
        }
    }
    let alert = fresh.iter().any(|(_, _, op)| op.notify);
    let brake_or_proposal = fresh.iter().any(|(_, _, op)| op.class != OpClass::Primary);
    let rows: Vec<(i64, String)> = fresh.iter().map(|(c, j, _)| (*c, j.clone())).collect();
    let seqs = if rows.is_empty() {
        Vec::new()
    } else {
        let mut attempts = 0;
        loop {
            attempts += 1;
            match ReinsOps::append(&user, &author, member.last_counter, rows.clone(), log_limits(), now_unix(), &conn)
                .await
            {
                Ok(seqs) => break seqs,
                Err(WriteError::Retry) if attempts < 5 => {}
                Err(WriteError::Conflict | WriteError::Retry) => {
                    return Err(api_err(
                        Status::Conflict,
                        codes::COUNTER_REPLAY,
                        "Another upload from this phone stored ops meanwhile; fetch the last counter and retry",
                    ));
                }
                Err(WriteError::Full) => {
                    return Err(api_err(
                        Status::Conflict,
                        codes::LOG_FULL,
                        "The account's log is full; the primary writes a checkpoint first",
                    ));
                }
                Err(WriteError::Db(e)) => return Err(internal(&e)),
            }
        }
    };
    let head = log_head(&user, &conn).await?;
    let mut all: Vec<u64> = retried;
    all.extend(seqs.iter().map(|s| u64::try_from(*s).unwrap_or(0)));
    if !seqs.is_empty() {
        HUB.live.bump(&user);
        wake_members(&headers, &author, head, alert, brake_or_proposal, &conn, pool.inner()).await?;
    }
    Ok(Json(OpsAppended {
        seqs: all,
        head_seq: head,
    }))
}

/// Pushes `sync` to the other members: viewers (an alert when an op asked), and the primary when the author is not it
/// and wrote a proposal or a restriction (an alert: it may have to decide).
async fn wake_members(
    headers: &Headers,
    author: &str,
    head: u64,
    alert: bool,
    brake_or_proposal: bool,
    conn: &DbConn,
    pool: &DbPool,
) -> PhoneResult<()> {
    let user = user_key(headers);
    let members = ReinsMember::all(&user, conn).await.map_err(|e| internal(&e))?;
    for m in members.iter().filter(|m| m.role == ROLE_VIEWER && m.device_uuid != author) {
        push::spawn_member_push(
            pool.clone(),
            headers.user.uuid.clone(),
            m.fcm_token.clone(),
            PushMessage::sync(head, alert),
        );
    }
    let author_is_primary = members.iter().any(|m| m.role == ROLE_PRIMARY && m.device_uuid == author);
    if brake_or_proposal
        && !author_is_primary
        && let Some(approval) = ReinsDevice::find_by_user(&headers.user.uuid, conn).await
    {
        push::spawn_push(pool.clone(), headers.user.uuid.clone(), approval.fcm_token, PushMessage::sync(head, true));
    }
    Ok(())
}

/// A short digest of what a viewer shows besides the ops; it changes when any of it does.
fn cursor(
    epoch: u64,
    checkpoint: Option<i64>,
    heartbeat_at: Option<i64>,
    waiting: &Waiting,
    answered: &[String],
) -> String {
    let mut c = reins_proto::canon::Canon::new("reins/sync-cursor/v1");
    c.u64(epoch).i64(checkpoint.unwrap_or(-1)).i64(heartbeat_at.unwrap_or(-1));
    let ids = waiting
        .requests
        .iter()
        .map(|r| r.id.0.as_str())
        .chain(waiting.pairings.iter().map(|p| p.id.0.as_str()))
        .chain(waiting.joins.iter().map(|j| j.id.as_str()))
        .chain(waiting.handovers.iter().map(|h| h.id.as_str()));
    for id in ids {
        c.str(id);
    }
    // The joins' enrollment steps change what a viewer shows without changing the ids.
    for j in &waiting.joins {
        c.bool(j.reveal.is_some());
    }
    c.count(answered.len());
    for id in answered {
        c.str(id);
    }
    reins_proto::canon::hex32(&reins_proto::canon::sha256(&c.finish()))[..32].to_owned()
}

/// The sync answer, and the time the primary's heartbeat was taken (part of the cursor).
async fn snapshot(user: &str, after: u64, pool: &DbPool) -> PhoneResult<(SyncResponse, Option<i64>)> {
    let conn = pool.get().await.map_err(|e| internal(&e))?;
    let latest = ReinsRoster::latest(user, &conn)
        .await
        .map_err(|e| internal(&e))?
        .ok_or_else(|| api_err(Status::Conflict, codes::NO_ROSTER, "This account has no roster"))?;
    let state = ReinsOps::state(user, &conn).await.map_err(|e| internal(&e))?;
    HUB.live.set_head(user, state.head);
    let from = i64::try_from(after).unwrap_or(i64::MAX).max(state.floor);
    let limit = i64::try_from(MAX_SYNC_OPS).unwrap_or(500);
    let rows = ReinsOps::after(user, from, limit + 1, &conn).await.map_err(|e| internal(&e))?;
    let more = rows.len() > MAX_SYNC_OPS;
    let ops = rows
        .iter()
        .take(MAX_SYNC_OPS)
        .filter_map(|row| {
            Some(SyncedOp {
                seq: u64::try_from(row.seq).ok()?,
                op: serde_json::from_str(&row.op).ok()?,
            })
        })
        .collect();
    let members = ReinsMember::all(user, &conn).await.map_err(|e| internal(&e))?;
    drop(conn);
    let primary = members.iter().find(|m| m.role == ROLE_PRIMARY);
    let heartbeat =
        primary.and_then(|m| m.heartbeat.as_deref()).and_then(|h| serde_json::from_str::<Heartbeat>(h).ok());
    let heartbeat_at = primary.and_then(|m| m.heartbeat_at);
    let min_acked = members.iter().filter(|m| m.is_current()).map(|m| m.acked_seq).min().unwrap_or(0);
    let mut response = SyncResponse {
        cursor: String::new(),
        epoch: u64::try_from(latest.epoch).unwrap_or(0),
        head_seq: u64::try_from(state.head).unwrap_or(0),
        floor_seq: u64::try_from(state.floor).unwrap_or(0),
        ops,
        more,
        checkpoint_seq: state.checkpoint.and_then(|s| u64::try_from(s).ok()),
        heartbeat,
        min_acked_seq: u64::try_from(min_acked).unwrap_or(0),
        waiting: Waiting::default(),
        answered: Vec::new(),
    };
    refresh_items(user, &mut response, heartbeat_at);
    Ok((response, heartbeat_at))
}

/// Fills in what waits for the primary and what was answered (memory only), and the cursor over all of it.
fn refresh_items(user: &str, response: &mut SyncResponse, heartbeat_at: Option<i64>) {
    let now = now_unix();
    response.waiting = Waiting {
        requests: HUB.relay.peek_waiting(user),
        pairings: HUB.pairings.peek_waiting(user),
        joins: HUB.joins.peek_waiting(user, now),
        handovers: HUB.handovers.peek_waiting(user, now),
    };
    let mut answered = HUB.relay.answered_ids(user);
    answered.extend(HUB.pairings.answered_ids(user));
    answered.extend(HUB.joins.answered_ids(user));
    answered.extend(HUB.handovers.answered_ids(user));
    response.answered = answered;
    let checkpoint = response.checkpoint_seq.and_then(|s| i64::try_from(s).ok());
    response.cursor = cursor(response.epoch, checkpoint, heartbeat_at, &response.waiting, &response.answered);
}

/// The members' live view (contracts as `GET /pending`: the same long-poll limit per device, at most 25 s). Answers at
/// once when there are ops after `after` or the cursor differs from `seen`; otherwise when either changes or the wait
/// ends. Looking never marks anything delivered.
#[get("/reins/api/sync?<after>&<seen>&<wait>")]
async fn get_sync(
    after: Option<u64>,
    seen: Option<String>,
    wait: Option<String>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Json<SyncResponse>> {
    require_member(&headers, &key, &conn).await?;
    // Never hold a pooled DB connection during a long-poll.
    drop(conn);
    let _poll = super::limits::DEVICE_POLLS
        .try_enter(&headers.device.uuid.to_string())
        .ok_or_else(|| too_many_running("too many long-polls of this device are open"))?;
    let user = user_key(&headers);
    let after = after.unwrap_or(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(clamp_wait(wait.as_deref())));
    let mut live = HUB.live.subscribe(&user);
    let mut items = HUB.signal.subscribe();
    live.borrow_and_update();
    items.borrow_and_update();
    let (mut response, mut heartbeat_at) = snapshot(&user, after, pool.inner()).await?;
    loop {
        if !response.ops.is_empty()
            || seen.as_deref() != Some(response.cursor.as_str())
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(Json(response));
        }
        // This account's own changes reload from the database; new items (a signal every account shares) only
        // refresh what is in memory.
        tokio::select! {
            changed = live.changed() => {
                if changed.is_err() {
                    tokio::time::sleep_until(deadline).await;
                }
                live.borrow_and_update();
                (response, heartbeat_at) = snapshot(&user, after, pool.inner()).await?;
            }
            changed = items.changed() => {
                if changed.is_err() {
                    tokio::time::sleep_until(deadline).await;
                }
                items.borrow_and_update();
                refresh_items(&user, &mut response, heartbeat_at);
            }
            () = tokio::time::sleep_until(deadline) => {}
        }
    }
}

/// The caller applied the log up to `seq` (kept as its floor for tombstone GC; never lowered, never above the head).
#[post("/reins/api/ack", data = "<data>")]
async fn post_ack(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    let member = require_member(&headers, &key, &conn).await?;
    let ack: Ack = serde_json::from_slice(&read_body_limited(data, 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let head = log_head(&member.user_uuid, &conn).await?;
    let seq = i64::try_from(ack.seq.min(head)).unwrap_or(0);
    ReinsMember::ack(&member.user_uuid, &member.device_uuid, seq, &conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// Checks that a primary-only message (checkpoint, heartbeat) of `author` at `epoch` comes from the caller as the
/// current primary, and that `sig` over `message` verifies against its key.
fn check_primary_signed(
    entries: &[RosterEntry],
    caller: &str,
    author: &str,
    epoch: u64,
    message: &[u8],
    sig: &str,
) -> PhoneResult<()> {
    let signer = primary_at(entries, epoch)
        .filter(|m| m.device_id == author && author == caller)
        .ok_or_else(|| api_err(Status::Forbidden, codes::NOT_APPROVAL_DEVICE, "Only the primary signs this"))?;
    let ok = signer.sign_key().ok().zip(parse_sig(sig)).is_some_and(|(k, s)| ed25519_verify(&k, message, &s));
    if ok {
        Ok(())
    } else {
        Err(api_err(Status::Forbidden, codes::BAD_SIGNATURE, "The signature does not verify against the primary's key"))
    }
}

/// The primary's sealed snapshot at `seq` (at most the head, above the latest checkpoint). The server keeps the
/// latest two and drops the ops up to the older one.
#[put("/reins/api/checkpoint", data = "<data>")]
async fn put_checkpoint(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let entries = chain(&user, &conn).await?;
    if entries.is_empty() {
        return Err(api_err(Status::Conflict, codes::NO_ROSTER, "This account has no roster"));
    }
    let limit = u64::try_from(MAX_CHECKPOINT_BYTES).unwrap_or(u64::MAX) * 4 / 3 + 64 * 1024;
    let checkpoint: Checkpoint = serde_json::from_slice(&read_body_limited(data, limit).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    checkpoint.validate().map_err(|e| bad_request(e.to_string()))?;
    if checkpoint.account != user {
        return Err(bad_request("The checkpoint names another account"));
    }
    let message = checkpoint.signing_bytes().map_err(|e| bad_request(e.to_string()))?;
    check_primary_signed(
        &entries,
        &headers.device.uuid.to_string(),
        &checkpoint.author,
        checkpoint.epoch,
        &message,
        &checkpoint.sig,
    )?;
    let head = log_head(&user, &conn).await?;
    if checkpoint.seq > head {
        return Err(bad_request(format!("The log's head is {head}; a checkpoint cannot be ahead of it")));
    }
    let row = ReinsCheckpointRow {
        user_uuid: user.clone(),
        seq: i64::try_from(checkpoint.seq).map_err(|_| bad_request("seq out of range"))?,
        checkpoint: serde_json::to_string(&checkpoint).map_err(|e| bad_request(e.to_string()))?,
        created_at: now_unix(),
    };
    match ReinsOps::put_checkpoint(row, &conn).await {
        Ok(()) => {}
        Err(WriteError::Conflict | WriteError::Retry) => {
            return Err(api_err(Status::Conflict, codes::EPOCH_CONFLICT, "A checkpoint at or after this seq exists"));
        }
        Err(WriteError::Full) => return Err(bad_request("checkpoint refused")),
        Err(WriteError::Db(e)) => return Err(internal(&e)),
    }
    HUB.live.bump(&user);
    Ok(Status::NoContent)
}

/// The latest checkpoint (a member bootstrapping, or one whose floor fell behind).
#[get("/reins/api/checkpoint")]
async fn get_checkpoint(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Checkpoint>> {
    let member = require_member(&headers, &key, &conn).await?;
    let row = ReinsOps::latest_checkpoint(&member.user_uuid, &conn)
        .await
        .map_err(|e| internal(&e))?
        .ok_or_else(super::device_api::not_found)?;
    serde_json::from_str(&row.checkpoint)
        .map(Json)
        .map_err(|e| internal(&crate::Error::new("A stored Reins checkpoint does not parse", e.to_string())))
}

/// The primary's signed heartbeat: what it applied, and when (its clock within [`HEARTBEAT_SKEW_SECS`] of the
/// server's). At most one per [`MIN_HEARTBEAT_SECS`].
#[post("/reins/api/heartbeat", data = "<data>")]
async fn post_heartbeat(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let caller = headers.device.uuid.to_string();
    let entries = chain(&user, &conn).await?;
    if entries.is_empty() {
        return Err(api_err(Status::Conflict, codes::NO_ROSTER, "This account has no roster"));
    }
    let heartbeat: Heartbeat = serde_json::from_slice(&read_body_limited(data, 4096).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    heartbeat.validate().map_err(|e| bad_request(e.to_string()))?;
    if heartbeat.account != user {
        return Err(bad_request("The heartbeat names another account"));
    }
    let now = now_unix();
    if (heartbeat.t - now).abs() > HEARTBEAT_SKEW_SECS {
        return Err(bad_request("The heartbeat's time is too far from the server's"));
    }
    let message = heartbeat.signing_bytes().map_err(|e| bad_request(e.to_string()))?;
    check_primary_signed(&entries, &caller, &heartbeat.author, heartbeat.epoch, &message, &heartbeat.sig)?;
    if heartbeat.head_seq > log_head(&user, &conn).await? {
        return Err(bad_request("The heartbeat is ahead of the log"));
    }
    let member = ReinsMember::find(&user, &caller, &conn).await.map_err(|e| internal(&e))?;
    if let Some(at) = member.and_then(|m| m.heartbeat_at)
        && now - at < MIN_HEARTBEAT_SECS
    {
        return Err(rate_limited(
            "a heartbeat was posted moments ago",
            Duration::from_secs(u64::try_from(MIN_HEARTBEAT_SECS - (now - at)).unwrap_or(1)),
        ));
    }
    let json = serde_json::to_string(&heartbeat).map_err(|e| bad_request(e.to_string()))?;
    ReinsMember::set_heartbeat(&user, &caller, &json, now, &conn).await.map_err(|e| internal(&e))?;
    HUB.live.bump(&user);
    Ok(Status::NoContent)
}
