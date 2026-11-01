//! Deferred decisions (Decisions v2, 03): requests the agent does not wait for.
//!
//! A relay request stays Instant (in memory, as always) until it is answered, or until its Instant window ends, the
//! agent asks to defer it, or the reviewer taps Later. Then it becomes a **ticket**: a row in `reins_tickets` with its
//! metadata in clear and its body sealed to the approval device's box key, and the server forgets the clear body.
//!
//! Transitions the phone makes come signed with its device key and are checked here ([`apply_signed`]); the ones the
//! server makes (deferral, expiry, cancellation, an agent's reply) are recorded as the server's own. Only signed
//! transitions reach webhooks.
//!
//! Who runs an approved ticket (08 R9): writes run on the phone when approved, and the server keeps their minimal
//! result; reads and desktop answers run when the agent collects them, and their result passes through memory only.

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use data_encoding::BASE64URL_NOPAD;
use reins_proto::{
    PROTOCOL_VERSION,
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
    pairing::{PushKind, PushMessage},
    relay::{RelayOutcome, RelayRequest},
    ticket::{
        self, Execution, OnExpiry, SealedTicketBody, SignedTicketEvent, TicketClass, TicketItem, TicketState,
        event_message, ticket_of,
    },
};
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};

use super::{HUB, now_unix, push, roles::ed25519_verify};
use crate::{
    CONFIG,
    db::{
        DbConn, DbPool,
        models::{ReinsDevice, ReinsDeviceKeys, ReinsTicket, ReinsTicketEvent, UserId},
    },
};

/// How long a result waits in memory for the agent that asked for it (a collect's result is kept this long whether or
/// not an agent waits at that moment; the ticket is done only once an agent received it).
const TRANSIT_SECS: i64 = 300;
/// Closed tickets are kept this long for `reins_tickets` and the audit trail, then deleted.
pub const CLOSED_KEEP_SECS: i64 = 7 * 86_400;
/// How often the sweeper runs.
const SWEEP_EVERY: Duration = Duration::from_secs(5);

/// The Instant window configured for this server (`REINS_INSTANT_WINDOW_SECS`), within the spec's bounds.
pub fn instant_window() -> Duration {
    Duration::from_secs(ticket::clamp_window(TicketClass::Instant, Some(CONFIG.reins_instant_window_secs())))
}

/// A Deferred window: `asked` (the reviewer's or the agent's), else this server's default
/// (`REINS_DEFERRED_WINDOW_SECS`), within the spec's bounds.
pub fn deferred_window(asked: Option<u64>) -> u64 {
    ticket::clamp_window(TicketClass::Deferred, asked.or(Some(CONFIG.reins_deferred_window_secs())))
}

/// In-memory side of tickets: what wakes waits, the phone and event streams, and results passing through.
pub struct TicketHub {
    /// Per ticket: bumped on every change; waits (`reins_wait`, `reins_get_result`) watch it.
    changes: Mutex<HashMap<String, watch::Sender<u64>>>,
    /// Accounts whose approval device has ticket news to fetch.
    news: Mutex<HashSet<String>>,
    /// Approved tickets an agent is collecting now, with their account and since when.
    collecting: Mutex<HashMap<String, (String, i64)>>,
    /// Results on their way to the agent that asked (collected reads, a write's full result): memory only, briefly.
    transit: Mutex<HashMap<String, (RelayOutcome, i64)>>,
    /// (connection, ticket id) of every change, for MCP event streams.
    updates: broadcast::Sender<(String, String)>,
    /// Per connection: the tickets it subscribed to (`resources/subscribe`).
    subscriptions: Mutex<HashMap<String, HashSet<String>>>,
    /// Per connection: its open event streams.
    streams: std::sync::Arc<Mutex<HashMap<String, usize>>>,
}

/// Most event streams one connection may hold open.
pub const MAX_STREAMS: usize = 2;
/// Most tickets one connection may subscribe to.
const MAX_SUBSCRIPTIONS: usize = 200;

/// An open event stream; dropping it frees the place.
pub struct StreamSlot {
    connection: String,
    streams: std::sync::Arc<Mutex<HashMap<String, usize>>>,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut streams = lock(&self.streams);
        if let Some(n) = streams.get_mut(&self.connection) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                streams.remove(&self.connection);
            }
        }
    }
}

impl Default for TicketHub {
    fn default() -> Self {
        Self {
            changes: Mutex::default(),
            news: Mutex::default(),
            collecting: Mutex::default(),
            transit: Mutex::default(),
            updates: broadcast::channel(256).0,
            subscriptions: Mutex::default(),
            streams: std::sync::Arc::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TicketHub {
    /// What changes when ticket `id` changes.
    pub fn watch(&self, id: &str) -> watch::Receiver<u64> {
        lock(&self.changes).entry(id.to_owned()).or_insert_with(|| watch::Sender::new(0)).subscribe()
    }

    /// Ticket `id` of `connection` changed: wakes its waits and event streams.
    pub fn changed(&self, connection: &str, id: &str) {
        let mut changes = lock(&self.changes);
        if let Some(tx) = changes.get(id) {
            tx.send_modify(|n| *n = n.wrapping_add(1));
            if tx.receiver_count() == 0 {
                changes.remove(id);
            }
        }
        drop(changes);
        self.updates.send((connection.to_owned(), id.to_owned())).ok();
    }

    pub fn updates(&self) -> broadcast::Receiver<(String, String)> {
        self.updates.subscribe()
    }

    /// `connection` (un)subscribes to ticket `id`'s changes.
    pub fn subscribe(&self, connection: &str, id: &str, on: bool) {
        let mut subs = lock(&self.subscriptions);
        let mine = subs.entry(connection.to_owned()).or_default();
        if on {
            if mine.len() < MAX_SUBSCRIPTIONS {
                mine.insert(id.to_owned());
            }
        } else {
            mine.remove(id);
        }
    }

    pub fn subscribed(&self, connection: &str, id: &str) -> bool {
        lock(&self.subscriptions).get(connection).is_some_and(|s| s.contains(id))
    }

    /// A place for one more event stream of `connection`, if it has fewer than [`MAX_STREAMS`].
    pub fn open_stream(&self, connection: &str) -> Option<StreamSlot> {
        let mut streams = lock(&self.streams);
        let n = streams.entry(connection.to_owned()).or_default();
        if *n >= MAX_STREAMS {
            return None;
        }
        *n += 1;
        Some(StreamSlot {
            connection: connection.to_owned(),
            streams: std::sync::Arc::clone(&self.streams),
        })
    }

    /// `user`'s approval device has ticket news; wakes its long-poll.
    pub fn flag_news(&self, user: &str) {
        lock(&self.news).insert(user.to_owned());
        HUB.signal.notify();
    }

    /// Whether `user` had ticket news (and clears it).
    pub fn take_news(&self, user: &str) -> bool {
        lock(&self.news).remove(user)
    }

    pub fn start_collect(&self, user: &str, id: &str, now: i64) {
        lock(&self.collecting).entry(id.to_owned()).or_insert_with(|| (user.to_owned(), now));
        self.flag_news(user);
    }

    pub fn is_collecting(&self, id: &str) -> bool {
        lock(&self.collecting).contains_key(id)
    }

    pub fn end_collect(&self, id: &str) {
        lock(&self.collecting).remove(id);
    }

    /// Tickets of `user` being collected (dropped after a minute nobody answered).
    pub fn collects_of(&self, user: &str, now: i64) -> Vec<String> {
        let mut collecting = lock(&self.collecting);
        collecting.retain(|_, (_, since)| now - *since < TRANSIT_SECS);
        collecting.iter().filter(|(_, (u, _))| u == user).map(|(id, _)| id.clone()).collect()
    }

    /// Forgets ticket `id`'s watch when nobody waits on it any more.
    pub fn release(&self, id: &str) {
        let mut changes = lock(&self.changes);
        if changes.get(id).is_some_and(|tx| tx.receiver_count() == 0) {
            changes.remove(id);
        }
    }

    /// How many tickets are watched (tests).
    #[cfg(test)]
    pub fn watched(&self) -> usize {
        lock(&self.changes).len()
    }

    /// Whether some agent is waiting on ticket `id` right now.
    pub fn has_waiters(&self, id: &str) -> bool {
        lock(&self.changes).get(id).is_some_and(|tx| tx.receiver_count() > 0)
    }

    pub fn put_transit(&self, id: &str, outcome: RelayOutcome, now: i64) {
        let mut transit = lock(&self.transit);
        transit.retain(|_, (_, at)| now - *at < TRANSIT_SECS);
        transit.insert(id.to_owned(), (outcome, now));
    }

    /// The result passing through for `id`, if it is still fresh. Every wait for it gets it until it expires.
    pub fn transit(&self, id: &str, now: i64) -> Option<RelayOutcome> {
        lock(&self.transit).get(id).filter(|(_, at)| now - *at < TRANSIT_SECS).map(|(o, _)| o.clone())
    }

    /// Drops what is held for a deleted account.
    pub fn forget_user(&self, user: &str) {
        lock(&self.news).remove(user);
        lock(&self.collecting).retain(|_, (u, _)| u != user);
    }
}

/// Seals what the approval device needs to show `request` to its box key (`box_pub`, base64url X25519): an anonymous
/// sealed box, base64url.
pub fn seal_body(box_pub: &str, request: &RelayRequest) -> Result<String, String> {
    let body = SealedTicketBody {
        v: PROTOCOL_VERSION,
        request: RelayRequest {
            wait_until: None,
            ..request.clone()
        },
    };
    let plain = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    seal_bytes(box_pub, &plain)
}

/// Seals `plain` to `box_pub` (base64url X25519), base64url.
pub fn seal_bytes(box_pub: &str, plain: &[u8]) -> Result<String, String> {
    let key: [u8; 32] = BASE64URL_NOPAD
        .decode(box_pub.as_bytes())
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| "not an X25519 public key".to_owned())?;
    let sealed = crypto_box::PublicKey::from(key)
        .seal(&mut crypto_box::aead::OsRng, plain)
        .map_err(|_| "could not seal".to_owned())?;
    Ok(BASE64URL_NOPAD.encode(&sealed))
}

/// Why a request did not become a ticket.
#[derive(Debug, PartialEq, Eq)]
pub enum ParkError {
    /// The approval device published no box key (an older app): nothing can be sealed to it.
    NoBoxKey,
    /// The connection has [`ticket::MAX_OPEN_TICKETS`] open already.
    TooMany,
    /// The phone already has the request (and may be answering it), or answered it: it stays Instant.
    PhoneHasIt,
    Db,
}

/// How an approved ticket of `call` ends: a parked push when the desktop app reports it pushed, another desktop
/// answer or read when an agent received it, a write when the phone ran it.
pub fn finish_of(call: &ToolCall) -> &'static str {
    match call {
        ToolCall::Connector(c) if c.spec().is_some_and(|s| s.desktop_only) && c.op.ends_with("push") => "report",
        _ if ticket::execution(call) == Execution::OnCollect => "collect",
        _ => "run",
    }
}

/// The approval device's keys. With phone roles, the roster's primary's keys (owner-signed), whatever a device
/// published; otherwise the keys the approval device published, while it is still the approval device.
pub async fn device_keys(user: &str, conn: &DbConn) -> Option<ReinsDeviceKeys> {
    if let Ok(entries) = super::roles::chain(user, conn).await
        && let Some(current) = entries.last()
    {
        return Some(ReinsDeviceKeys {
            user_uuid: user.to_owned(),
            device_uuid: current.primary.device_id.clone(),
            sign_pub: current.primary.sign_pub.clone(),
            box_pub: current.primary.box_pub.clone(),
            updated_at: 0,
        });
    }
    let keys = ReinsDeviceKeys::find(user, conn).await?;
    let device = ReinsDevice::find_by_user(&UserId::from(user.to_owned()), conn).await?;
    (device.device_uuid.to_string() == keys.device_uuid).then_some(keys)
}

/// The server's record of a transition it made (unsigned).
fn server_event(state: TicketState, by: &str, now: i64, extra: &Value) -> String {
    let mut event = json!({"state": state.as_str(), "by": by, "at": now});
    if let (Some(obj), Some(more)) = (event.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            obj.insert(k.clone(), v.clone());
        }
    }
    event.to_string()
}

/// Turns `request` (of `user`) into a ticket, or finds the open ticket of the same request. `initial`: `Deferred`
/// (the window ran out, the agent asked) or `Reviewing` (the reviewer's signed Later follows). The relay forgets the
/// request (its waits end with the ticket), and the approval device is told.
pub async fn park(
    user: &str,
    request: &RelayRequest,
    initial: TicketState,
    window: Option<u64>,
    conn: &DbConn,
    pool: &DbPool,
) -> Result<ReinsTicket, ParkError> {
    let keys = device_keys(user, conn).await.ok_or(ParkError::NoBoxKey)?;
    // Atomic with the phone's fetches: what the phone already has becomes a ticket only by its word (Later).
    if !HUB.relay.hold_for_deferral(&request.id, initial == TicketState::Reviewing) {
        return Err(ParkError::PhoneHasIt);
    }
    match park_held(user, request, initial, window, &keys, conn, pool).await {
        Ok(row) => Ok(row),
        Err(e) => {
            HUB.relay.release_hold(&request.id);
            Err(e)
        }
    }
}

async fn park_held(
    user: &str,
    request: &RelayRequest,
    initial: TicketState,
    window: Option<u64>,
    keys: &ReinsDeviceKeys,
    conn: &DbConn,
    pool: &DbPool,
) -> Result<ReinsTicket, ParkError> {
    let digest = ticket::digest_of(request);
    let connection = request.connection_id.0.clone();
    if let Some(open) = ReinsTicket::open_by_digest(&connection, &digest, conn).await {
        if !HUB.relay.mark_deferred(&request.id, &ticket_of(&RequestId(open.id.clone()))) {
            return Err(ParkError::PhoneHasIt);
        }
        return Ok(open);
    }
    let limit = i64::try_from(ticket::MAX_OPEN_TICKETS).unwrap_or(i64::MAX);
    if ReinsTicket::count_open(&connection, conn).await >= limit {
        return Err(ParkError::TooMany);
    }
    let sealed = seal_body(&keys.box_pub, request).map_err(|_| ParkError::NoBoxKey)?;
    let now = now_unix();
    let window = deferred_window(window);
    let window_i = i64::try_from(window).unwrap_or(i64::MAX);
    let row = ReinsTicket {
        id: request.id.0.clone(),
        user_uuid: user.to_owned(),
        connection_uuid: connection.clone(),
        connection_label: request.connection_label.clone(),
        state: initial.as_str().to_owned(),
        digest,
        execution: ticket::execution(&request.call).as_str().to_owned(),
        on_expiry: OnExpiry::Deny.as_str().to_owned(),
        finish: finish_of(&request.call).to_owned(),
        created_at: request.created_at,
        deferred_at: now,
        deadline: now.saturating_add(window_i),
        window_secs: window_i,
        escalated: false,
        approved_at: None,
        valid_secs: ticket::approval_validity(&request.call),
        sealed_body: sealed,
        question: None,
        sealed_reply: None,
        note: None,
        result: None,
        stale: None,
        delivered: false,
        updated_at: now,
    };
    if row.insert(conn).await.is_err() {
        // The same request parked twice at once: the first one won.
        return ReinsTicket::find_for_connection(&row.id, &connection, conn).await.ok_or(ParkError::Db);
    }
    let event = server_event(initial, "server", now, &json!({"deadline": row.deadline}));
    if let Err(e) = ReinsTicketEvent::append(&row.id, initial.as_str(), false, &event, now, conn).await {
        warn!("Reins tickets: {e:?}");
    }
    if !HUB.relay.mark_deferred(&request.id, &ticket_of(&request.id)) {
        // Answered while it was being parked: the answer stands, the ticket never existed.
        if let Err(e) = ReinsTicket::delete(&row.id, conn).await {
            warn!("Reins tickets: {e:?}");
        }
        return Err(ParkError::PhoneHasIt);
    }
    wake_phone(user, &row.id, conn, pool).await;
    HUB.tickets.changed(&connection, &row.id);
    Ok(row)
}

/// The approval device changed (another phone, new keys): what was sealed to the old one can never be opened, so
/// every ticket still waiting or approved closes, saying why, instead of sitting in the connection's cap.
pub async fn close_for_new_device(user: &str, conn: &DbConn) {
    let why = json!({"reason": "the approval phone changed"});
    for mut row in ReinsTicket::list_for_user(user, conn).await {
        let to = match TicketState::parse(&row.state) {
            Some(s) if s.can_go(TicketState::Expired) => TicketState::Expired,
            _ => TicketState::Cancelled,
        };
        transition(&mut row, to, "server", &why, |_| {}, conn).await;
    }
}

/// The open ticket `connection` already has for exactly this call, if any: the same request again shares it instead
/// of asking twice.
pub async fn open_for(connection: &str, account: Option<&str>, call: &ToolCall, conn: &DbConn) -> Option<ReinsTicket> {
    let digest = ticket::request_digest(&ConnectionId(connection.to_owned()), account, call);
    ReinsTicket::open_by_digest(connection, &digest, conn).await
}

/// Pushes `user`'s approval device about ticket `id` and wakes its long-poll.
pub async fn wake_phone(user: &str, id: &str, conn: &DbConn, pool: &DbPool) {
    HUB.tickets.flag_news(user);
    let user_uuid = UserId::from(user.to_owned());
    if let Some(device) = ReinsDevice::find_by_user(&user_uuid, conn).await {
        push::spawn_push(
            pool.clone(),
            user_uuid,
            device.fcm_token,
            PushMessage {
                t: PushKind::Req,
                id: id.to_owned(),
                p: None,
            },
        );
    }
}

/// Moves `ticket` to `to` as the server (or `by`: "agent", "desktop"), after `change` adjusts the row. `false` when the
/// state machine does not allow it or someone moved the ticket first.
pub async fn transition(
    ticket: &mut ReinsTicket,
    to: TicketState,
    by: &str,
    extra: &Value,
    change: impl FnOnce(&mut ReinsTicket),
    conn: &DbConn,
) -> bool {
    let Some(from) = TicketState::parse(&ticket.state) else {
        return false;
    };
    if !from.can_go(to) {
        return false;
    }
    let now = now_unix();
    let mut next = ticket.clone();
    change(&mut next);
    to.as_str().clone_into(&mut next.state);
    next.updated_at = now;
    next.delivered = false;
    if !matches!(next.update(from.as_str(), conn).await, Ok(true)) {
        return false;
    }
    let event = server_event(to, by, now, extra);
    if let Err(e) = ReinsTicketEvent::append(&next.id, to.as_str(), false, &event, now, conn).await {
        warn!("Reins tickets: {e:?}");
    }
    *ticket = next;
    HUB.tickets.flag_news(&ticket.user_uuid);
    HUB.tickets.changed(&ticket.connection_uuid, &ticket.id);
    true
}

/// Why a phone's ticket events were refused.
#[derive(Debug, PartialEq, Eq)]
pub enum TicketError {
    /// No such ticket of this account.
    NotFound,
    /// Not signed by the approval device's published key, or not about this ticket.
    BadSignature,
    /// The state machine does not allow it from where the ticket is.
    BadTransition,
    Db,
}

fn verify(signed: &SignedTicketEvent, sign_pub: &str) -> bool {
    if signed.key != sign_pub {
        return false;
    }
    let key: Option<[u8; 32]> = BASE64URL_NOPAD.decode(signed.key.as_bytes()).ok().and_then(|k| k.try_into().ok());
    let sig = reins_proto::canon::parse_sig(&signed.sig);
    match (key, sig) {
        (Some(key), Some(sig)) => ed25519_verify(&key, &event_message(&signed.event), &sig),
        _ => false,
    }
}

/// Applies the approval device's signed transitions to `ticket`, in order. Each must verify against the device's
/// published signing key, name this ticket, its digest and its connection, and be a move the state machine allows
/// from where the ticket is. `window`: the Deferred window a Later asked for.
pub async fn apply_signed(
    ticket: &mut ReinsTicket,
    events: &[SignedTicketEvent],
    window: Option<u64>,
    on_expiry: Option<OnExpiry>,
    conn: &DbConn,
) -> Result<(), TicketError> {
    let keys = device_keys(&ticket.user_uuid, conn).await.ok_or(TicketError::BadSignature)?;
    let name = ticket_of(&RequestId(ticket.id.clone()));
    // What the device signed for this ticket so far: no signature twice, and time only moves forward.
    let stored: Vec<SignedTicketEvent> = ReinsTicketEvent::list(&ticket.id, conn)
        .await
        .into_iter()
        .filter(|e| e.signed)
        .filter_map(|e| serde_json::from_str(&e.event).ok())
        .collect();
    let mut last_at = stored.iter().map(|e| e.event.at).max();
    let mut seen: HashSet<String> = stored.into_iter().map(|e| e.sig).collect();
    for signed in events {
        let e = &signed.event;
        if !verify(signed, &keys.sign_pub)
            || e.ticket != name
            || e.digest != ticket.digest
            || e.connection_id != ConnectionId(ticket.connection_uuid.clone())
        {
            return Err(TicketError::BadSignature);
        }
        let from = TicketState::parse(&ticket.state).ok_or(TicketError::BadTransition)?;
        if !seen.insert(signed.sig.clone()) || last_at.is_some_and(|last| e.at < last) {
            // A replay, or an older signature brought back after later transitions.
            return Err(TicketError::BadTransition);
        }
        last_at = Some(e.at);
        if from == TicketState::Expired && e.state == TicketState::Expired {
            // The device confirms an expiry the server recorded by its clock: kept, and sent to the webhook, once.
            let confirmed =
                ReinsTicketEvent::list(&ticket.id, conn).await.iter().any(|ev| ev.signed && ev.state == "expired");
            if !confirmed {
                let text = serde_json::to_string(signed).map_err(|_| TicketError::Db)?;
                ReinsTicketEvent::append(&ticket.id, "expired", true, &text, now_unix(), conn)
                    .await
                    .map_err(|_| TicketError::Db)?;
                super::webhooks::fire(&ticket.connection_uuid, signed, conn).await;
            }
            continue;
        }
        if e.from != from || !from.can_go(e.state) {
            return Err(TicketError::BadTransition);
        }
        let now = now_unix();
        let mut next = ticket.clone();
        e.state.as_str().clone_into(&mut next.state);
        next.updated_at = now;
        next.delivered = false;
        match e.state {
            TicketState::Approved => {
                next.approved_at = Some(now);
                next.question = None;
                next.note = None;
            }
            TicketState::Deferred => {
                let secs = deferred_window(window);
                next.window_secs = i64::try_from(secs).unwrap_or(i64::MAX);
                next.deferred_at = now;
                next.deadline = now.saturating_add(next.window_secs);
                // The phone's own words about the park ("Parked until morning": Overnight), signed with the event.
                next.note = e.note.as_deref().and_then(|n| ticket::clean_text(n, ticket::MAX_NOTE_CHARS));
                if let Some(on_expiry) = on_expiry {
                    on_expiry.as_str().clone_into(&mut next.on_expiry);
                }
            }
            TicketState::NeedsInfo => {
                next.question = e.note.as_deref().and_then(|n| ticket::clean_text(n, ticket::MAX_NOTE_CHARS));
                next.sealed_reply = None;
            }
            TicketState::Denied => {
                next.note = e.note.as_deref().and_then(|n| ticket::clean_text(n, ticket::MAX_NOTE_CHARS));
            }
            TicketState::Done => next.result = e.result.as_ref().map(|r| ticket::minimal_value(r).to_string()),
            TicketState::Stale => {
                next.stale = e.stale.as_deref().and_then(|s| ticket::clean_text(s, ticket::MAX_NOTE_CHARS));
            }
            _ => {}
        }
        match next.update(from.as_str(), conn).await {
            Ok(true) => {}
            Ok(false) => return Err(TicketError::BadTransition),
            Err(_) => return Err(TicketError::Db),
        }
        let text = serde_json::to_string(signed).map_err(|_| TicketError::Db)?;
        ReinsTicketEvent::append(&next.id, e.state.as_str(), true, &text, now, conn)
            .await
            .map_err(|_| TicketError::Db)?;
        *ticket = next;
        HUB.tickets.changed(&ticket.connection_uuid, &ticket.id);
        if e.state.webhook_type().is_some() {
            super::webhooks::fire(&ticket.connection_uuid, signed, conn).await;
        }
    }
    Ok(())
}

/// What the approval device gets for `row` (`collect`: an agent is collecting it now).
pub fn item(row: &ReinsTicket, collect: bool) -> Option<TicketItem> {
    Some(TicketItem {
        id: RequestId(row.id.clone()),
        connection_id: ConnectionId(row.connection_uuid.clone()),
        connection_label: row.connection_label.clone(),
        created_at: row.created_at,
        deferred_at: row.deferred_at,
        deadline: row.deadline,
        state: TicketState::parse(&row.state)?,
        digest: row.digest.clone(),
        sealed: row.sealed_body.clone(),
        question: row.question.clone(),
        sealed_reply: row.sealed_reply.clone(),
        stale: row.stale.clone(),
        collect,
    })
}

/// The tickets `user`'s approval device has not seen in their current version, and those being collected; marks them
/// delivered.
pub async fn for_phone(user: &str, conn: &DbConn) -> Vec<TicketItem> {
    let now = now_unix();
    let collecting = HUB.tickets.collects_of(user, now);
    let mut rows = ReinsTicket::undelivered(user, conn).await;
    for id in &collecting {
        if !rows.iter().any(|r| &r.id == id)
            && let Some(row) = ReinsTicket::find_for_user(id, user, conn).await
        {
            rows.push(row);
        }
    }
    let delivered: Vec<String> = rows.iter().filter(|r| !r.delivered).map(|r| r.id.clone()).collect();
    if !delivered.is_empty()
        && let Err(e) = ReinsTicket::mark_delivered(delivered, conn).await
    {
        warn!("Reins tickets: {e:?}");
    }
    rows.iter()
        .filter_map(|r| item(r, collecting.contains(&r.id) && r.state == TicketState::Approved.as_str()))
        .collect()
}

/// Turns what is due into tickets and closes or escalates what ran out. Runs every few seconds.
pub async fn sweep(pool: &DbPool, now: i64, purge: bool) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    for (user, request) in HUB.relay.due_for_deferral(now) {
        if park(&user, &request, TicketState::Deferred, None, &conn, pool).await.is_err() {
            HUB.relay.keep_instant(&request.id);
        }
    }
    for mut row in ReinsTicket::due(now, &conn).await {
        let escalate = row.on_expiry == OnExpiry::Escalate.as_str() && !row.escalated && row.is_open();
        let moved = if escalate {
            let window = row.window_secs;
            transition(
                &mut row,
                TicketState::Escalated,
                "server",
                &json!({}),
                |t| {
                    t.escalated = true;
                    t.deferred_at = now;
                    t.deadline = now.saturating_add(window);
                },
                &conn,
            )
            .await
        } else {
            transition(&mut row, TicketState::Expired, "server", &json!({}), |_| {}, &conn).await
        };
        if moved {
            wake_phone(&row.user_uuid, &row.id, &conn, pool).await;
        }
    }
    if purge && let Err(e) = ReinsTicket::purge(now - CLOSED_KEEP_SECS, &conn).await {
        warn!("Reins tickets: {e:?}");
    }
}

/// Starts the sweeper.
pub fn spawn_sweeper(pool: DbPool) {
    tokio::spawn(async move {
        let mut n: u32 = 0;
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            n = n.wrapping_add(1);
            sweep(&pool, now_unix(), n.is_multiple_of(720)).await;
        }
    });
}

fn rfc3339(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map_or_else(String::new, |d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// One line saying what `call` is, for the answer that hands out its ticket (never stored).
pub fn summary_of(call: &ToolCall) -> String {
    let text = match call {
        ToolCall::GmailSend {
            email,
        } => format!("Send email to {}", email.to.join(", ")),
        ToolCall::GmailSearch {
            query,
            ..
        } => format!("Search mail: {query}"),
        ToolCall::GmailRead {
            message_ids,
        } => format!("Read {} email(s)", message_ids.len()),
        ToolCall::RequestGrant {
            ..
        } => "A standing permission".to_owned(),
        ToolCall::Connector(c) => match c.spec() {
            Some(spec) => match c.resource() {
                Some(resource) => format!("{} ({resource})", spec.title),
                None => spec.title.to_owned(),
            },
            None => format!("{}.{}", c.service, c.op),
        },
        ToolCall::Mcp(m) => format!("{}: {}", m.server, m.tool),
        ToolCall::RequestUpload {
            name,
            ..
        } => format!("Upload {name}"),
        ToolCall::ListAccounts {
            ..
        } => "List accounts".to_owned(),
    };
    let one_line: String = text
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out: String = one_line.chars().take(100).collect();
    if one_line.chars().count() > 100 {
        out.push('\u{2026}');
    }
    out
}

/// What an agent is told when its call became a ticket (03 §4.1): not an error.
pub fn pending_review(row: &ReinsTicket, summary: Option<&str>) -> Value {
    let ticket = ticket_of(&RequestId(row.id.clone()));
    let mut out = json!({
        "status": "pending_review",
        "ticket": ticket,
        "expected_by": rfc3339(row.deadline),
        "next": "Continue with other work. Call reins_get_result with this ticket later, or reins_wait to block up to \
    10 minutes. Do not call the tool again: that would ask a second time."
    });
    // Parked by the phone with a note (Overnight: "Parked until morning"): the agent is told that.
    let parked = row.note.as_deref().filter(|_| row.state == TicketState::Deferred.as_str());
    match (summary, parked) {
        (Some(summary), Some(note)) => out["summary"] = json!(format!("{summary} \u{2014} {}", note.to_lowercase())),
        (Some(summary), None) => out["summary"] = json!(format!("{summary} \u{2014} waiting for review")),
        (None, Some(note)) => out["summary"] = json!(note),
        (None, None) => {}
    }
    out
}

/// Where a ticket stands, as `reins_tickets` and `resources/read` show it.
pub fn status_value(row: &ReinsTicket) -> Value {
    let mut out = json!({
        "ticket": ticket_of(&RequestId(row.id.clone())),
        "state": row.state,
        "created_at": rfc3339(row.created_at),
        "expected_by": rfc3339(row.deadline),
    });
    if let Some(q) = &row.question {
        out["question"] = json!(q);
    }
    if let Some(n) = &row.note {
        out["note"] = json!(n);
    }
    if let Some(s) = &row.stale {
        out["stale"] = json!(s);
    }
    out
}

/// `Execution` of a stored ticket.
pub fn execution_of(row: &ReinsTicket) -> Execution {
    Execution::parse(&row.execution).unwrap_or(Execution::OnApprove)
}

/// A wait on one ticket: its watch entry goes when the last wait on it does, whichever way the wait ends.
struct Watching {
    id: String,
    rx: Option<watch::Receiver<u64>>,
}

impl Watching {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            rx: Some(HUB.tickets.watch(id)),
        }
    }

    fn rx(&mut self) -> &mut watch::Receiver<u64> {
        self.rx.get_or_insert_with(|| HUB.tickets.watch(&self.id))
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        drop(self.rx.take());
        HUB.tickets.release(&self.id);
    }
}

/// Where a ticket stands for the agent that made it, after waiting (or collecting) a while.
#[derive(Debug, PartialEq)]
pub enum AgentView {
    /// Still waiting for review (deferred, needs_info, escalated).
    Open(ReinsTicket),
    /// Approved; the phone has not produced the result yet (it is offline, or a write is still running).
    Approved(ReinsTicket),
    /// The result, or the denial, on its way through (never stored).
    Outcome(RelayOutcome),
    /// A write that ran: its minimal result.
    Done(ReinsTicket),
    /// Expired, cancelled, failed, stale or unknown.
    Closed(ReinsTicket),
}

/// Waits up to `max` for `id` (of `connection`) to move, and collects it when it is an approved read (`collect`):
/// the phone runs it now and the result passes through. `None`: no such ticket of this connection.
pub async fn agent_wait(id: &str, connection: &str, max: Duration, collect: bool, pool: &DbPool) -> Option<AgentView> {
    let deadline = tokio::time::Instant::now() + max;
    // Only a ticket of this connection is watched: an id an agent made up leaves nothing behind.
    {
        let conn = pool.get().await.ok()?;
        ReinsTicket::find_for_connection(id, connection, &conn).await?;
    }
    let mut watching = Watching::new(id);
    let mut asked = false;
    loop {
        watching.rx().borrow_and_update();
        let conn = pool.get().await.ok()?;
        let row = ReinsTicket::find_for_connection(id, connection, &conn).await?;
        // What already came through goes first: a result kept for this agent ends the wait (and spends a collect).
        if let Some(outcome) = HUB.tickets.transit(id, now_unix()) {
            if matches!(outcome, RelayOutcome::Result { .. })
                && row.state == TicketState::Approved.as_str()
                && row.finish == "collect"
            {
                let mut done = row.clone();
                transition(&mut done, TicketState::Done, "server", &json!({"reason": "collected"}), |_| {}, &conn)
                    .await;
            }
            return Some(AgentView::Outcome(outcome));
        }
        if collect
            && !asked
            && row.state == TicketState::Approved.as_str()
            && execution_of(&row) == Execution::OnCollect
        {
            asked = true;
            HUB.tickets.start_collect(&row.user_uuid, &row.id, now_unix());
            wake_phone(&row.user_uuid, &row.id, &conn, pool).await;
        }
        drop(conn);
        let state = TicketState::parse(&row.state).unwrap_or(TicketState::Unknown);
        let settled = match state {
            TicketState::Denied => {
                return Some(AgentView::Outcome(RelayOutcome::Denied {
                    reason: row.note.clone(),
                }));
            }
            TicketState::Done => return Some(AgentView::Done(row)),
            TicketState::Stale => return Some(AgentView::Closed(row)),
            s if s.is_terminal() => return Some(AgentView::Closed(row)),
            TicketState::Approved | TicketState::Revalidating | TicketState::Executing => AgentView::Approved(row),
            _ => AgentView::Open(row),
        };
        if tokio::time::timeout_at(deadline, watching.rx().changed()).await.is_err() {
            return Some(settled);
        }
    }
}

/// The approval device's answer for ticket `id` (A4 for a request that became a ticket): its signed transitions,
/// then the outcome. A write's full result goes to whoever waits for it now; only its minimal result is kept. A
/// collected read's result passes through and is not kept at all.
pub async fn answer(
    user: &str,
    id: &str,
    response: reins_proto::relay::RelayResponse,
    conn: &DbConn,
) -> Result<(), TicketError> {
    let mut row = ReinsTicket::find_for_user(id, user, conn).await.ok_or(TicketError::NotFound)?;
    if response.events.is_empty() && row.is_open() {
        // Every move out of review is the device's signed event. (Nothing the phone holds becomes a ticket behind its
        // back, so an unsigned answer to an open ticket is never legitimate; the phone shows the refusal.)
        return Err(TicketError::BadTransition);
    }
    apply_signed(&mut row, &response.events, None, None, conn).await?;
    let now = now_unix();
    if matches!(response.outcome, RelayOutcome::Denied { .. }) && row.state != TicketState::Denied.as_str() {
        // A denial counts only as the device's signed transition, never as a bare outcome.
        return Err(TicketError::BadTransition);
    }
    if let RelayOutcome::Result {
        result,
    } = &response.outcome
    {
        match execution_of(&row) {
            Execution::OnApprove => {
                if row.state != TicketState::Done.as_str() {
                    return Err(TicketError::BadTransition);
                }
                let mut next = row.clone();
                next.result = Some(ticket::minimal_result(result).to_string());
                if !matches!(next.update(&row.state, conn).await, Ok(true)) {
                    return Err(TicketError::Db);
                }
            }
            Execution::OnCollect => {
                if row.state != TicketState::Approved.as_str() {
                    return Err(TicketError::BadTransition);
                }
            }
        }
    }
    let collecting = HUB.tickets.is_collecting(id);
    HUB.tickets.end_collect(id);
    // A collect's result waits for the agent that asked a while (the ticket is done once it got it); any other result
    // (an approval's own run, a write) goes only to an agent waiting right now, and is never kept for later.
    if collecting || HUB.tickets.has_waiters(id) {
        HUB.tickets.put_transit(id, response.outcome, now);
    }
    HUB.tickets.changed(&row.connection_uuid, id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::OutgoingEmail;

    use super::*;

    fn email() -> RelayRequest {
        RelayRequest {
            v: 1,
            id: RequestId("r1".to_owned()),
            connection_id: ConnectionId("c1".to_owned()),
            connection_label: "Claude".to_owned(),
            created_at: 5,
            wait_until: Some(50),
            account: None,
            call: ToolCall::GmailSend {
                email: Box::new(OutgoingEmail {
                    to: vec!["ann@acme.com".to_owned()],
                    cc: vec![],
                    subject: "Quarterly numbers".to_owned(),
                    body: "Attached".to_owned(),
                    reply_to_message_id: None,
                }),
            },
            batch: None,
        }
    }

    #[test]
    fn the_body_is_stored_sealed_never_in_clear() {
        let secret = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let public = BASE64URL_NOPAD.encode(secret.public_key().as_bytes());
        let sealed = seal_body(&public, &email()).unwrap();
        for clear in ["ann@acme.com", "Quarterly", "Attached", "acme"] {
            assert!(!sealed.contains(clear), "{clear}");
        }
        let bytes = BASE64URL_NOPAD.decode(sealed.as_bytes()).unwrap();
        let body: SealedTicketBody = serde_json::from_slice(&secret.unseal(&bytes).unwrap()).unwrap();
        assert_eq!(body.request.id, email().id);
        assert_eq!(body.request.wait_until, None, "an Instant deadline means nothing once parked");
        assert_eq!(ticket::digest_of(&body.request), ticket::digest_of(&email()));
        assert!(seal_body("not-a-key", &email()).is_err());
    }

    #[test]
    fn signatures_are_checked_against_the_published_key() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key = BASE64URL_NOPAD.encode(pair.public_key().as_ref());
        let event = ticket::TicketEvent {
            v: 1,
            ticket: "rt_r1".to_owned(),
            from: TicketState::Deferred,
            state: TicketState::Approved,
            at: 1,
            digest: "d".to_owned(),
            connection_id: ConnectionId("c1".to_owned()),
            deadline: None,
            note: None,
            result: None,
            stale: None,
        };
        let sig = BASE64URL_NOPAD.encode(pair.sign(&event_message(&event)).as_ref());
        let signed = SignedTicketEvent {
            event,
            key: key.clone(),
            sig,
        };
        assert!(verify(&signed, &key));
        assert!(!verify(&signed, "another-key"), "only the published key counts");
        let mut forged = signed.clone();
        forged.event.state = TicketState::Denied;
        assert!(!verify(&forged, &key), "the signature covers the state");
        forged = signed;
        forged.sig = "x".to_owned();
        assert!(!verify(&forged, &key));
    }

    #[cfg(sqlite)]
    #[test]
    fn a_phone_note_names_the_parking() {
        let mut row = crate::db::models::test_ticket("r1", "c1", "deferred", 4_000_000_000);
        row.note = Some("Parked until morning".to_owned());
        let v = pending_review(&row, Some("Send email to ann@acme.io"));
        assert_eq!(v["summary"], "Send email to ann@acme.io \u{2014} parked until morning");
        assert_eq!(pending_review(&row, None)["summary"], "Parked until morning");
        assert_eq!(status_value(&row)["note"], "Parked until morning");
        // Without a note, or once it isn't parked any more, the usual words.
        row.note = None;
        assert_eq!(pending_review(&row, Some("Send")).get("summary").unwrap(), "Send \u{2014} waiting for review");
        row.note = Some("Parked until morning".to_owned());
        "denied".clone_into(&mut row.state);
        assert_eq!(pending_review(&row, Some("Send"))["summary"], "Send \u{2014} waiting for review");
    }

    #[test]
    fn summaries_are_one_short_line() {
        assert_eq!(summary_of(&email().call), "Send email to ann@acme.com");
        let long = ToolCall::GmailSearch {
            query: format!("a\nb{}", "x".repeat(200)),
            max_results: 1,
        };
        let s = summary_of(&long);
        assert!(!s.contains('\n'));
        assert_eq!(s.chars().count(), 101);
    }

    #[test]
    fn watches_go_with_their_last_wait() {
        let hub = TicketHub::default();
        let a = hub.watch("t1");
        let b = hub.watch("t1");
        hub.release("t1");
        assert_eq!(hub.watched(), 1, "still waited on");
        drop((a, b));
        hub.release("t1");
        assert_eq!(hub.watched(), 0);
        for i in 0..100 {
            drop(hub.watch(&format!("random-{i}")));
            hub.release(&format!("random-{i}"));
        }
        assert_eq!(hub.watched(), 0, "made-up ids leave nothing behind");
    }

    #[test]
    fn transit_results_expire_and_collects_are_per_account() {
        let hub = TicketHub::default();
        let outcome = RelayOutcome::Denied {
            reason: None,
        };
        hub.put_transit("t1", outcome.clone(), 100);
        assert_eq!(hub.transit("t1", 399), Some(outcome), "every wait gets it while fresh");
        assert_eq!(hub.transit("t1", 400), None);
        hub.collecting.lock().unwrap().insert("t2".to_owned(), ("u1".to_owned(), 100));
        hub.collecting.lock().unwrap().insert("t3".to_owned(), ("u2".to_owned(), 100));
        assert_eq!(hub.collects_of("u1", 120), ["t2"]);
        assert!(hub.collects_of("u1", 400).is_empty(), "nobody answered for five minutes");
    }
}
