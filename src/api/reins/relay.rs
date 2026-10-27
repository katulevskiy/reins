//! In-memory relay between MCP tool calls and the approval device (spec §4.3).
//!
//! Nothing here touches disk: tool arguments and results live only in these maps.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use reins_proto::{
    PROTOCOL_VERSION,
    autopilot::{Decided, Suggestion},
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
    relay::{RelayOutcome, RelayRequest},
};
use tokio::{sync::watch, time::Instant};

use super::{
    ITEM_TTL, Timing,
    ttl::{Full, TtlMap},
};
use crate::util::get_uuid;

/// Upper bound on relay requests held in memory at once. When it is reached, the oldest request of another account
/// that holds more than the caller (an already-answered one first) is dropped, so one account cannot lock the
/// others out. An account that already holds as many as everyone else is refused instead of dropping its own call.
pub const MAX_REQUESTS: usize = 10_000;
/// Default for the unanswered requests one account may have queued for its phone (`REINS_ACCOUNT_MAX_QUEUED`).
pub const DEFAULT_MAX_QUEUED: usize = 100;

/// Why a request was not queued.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueFull {
    /// This account already holds as many as anyone else, so nothing of another account's can be dropped to make
    /// room. An account over its own unanswered cap is [`QueueFull::Account`] instead.
    Server,
    /// This account has as many unanswered requests as it may.
    Account,
}

impl From<Full> for QueueFull {
    fn from(_: Full) -> Self {
        Self::Server
    }
}

/// Bumped whenever a request or pairing is queued; phone long-polls (A2) wait on it.
pub struct ItemSignal {
    tx: watch::Sender<u64>,
}

impl ItemSignal {
    pub fn new() -> Self {
        Self {
            tx: watch::Sender::new(0),
        }
    }

    pub fn notify(&self) {
        self.tx.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }
}

impl Default for ItemSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// What a waiting tool call can observe about its request.
#[derive(Clone, Debug, Default, PartialEq)]
struct RequestState {
    /// The phone fetched the request (A2 or A3).
    delivered: bool,
    /// The phone's answer (A4), kept until the entry expires.
    outcome: Option<RelayOutcome>,
    /// Nothing can wake the phone (no push, the app not open) or the push failed: the first wait need not wait for
    /// the offline threshold.
    unreachable: bool,
    /// Who decided it when the user did not (the phone's answer says; shown on the desktop app only).
    decided: Option<Decided>,
    /// Where Autopilot leans while it waits for the user (shown on the desktop app only).
    suggestion: Option<Suggestion>,
    /// Unix seconds when, still unanswered, it becomes a ticket ([`RelayHub::due_for_deferral`]).
    instant_until: i64,
    /// It became this ticket (`rt_…`): the waits end with [`WaitResult::Deferred`], and the entry is gone.
    deferred: Option<String>,
    /// Being turned into a ticket ([`RelayHub::hold_for_deferral`]): never handed to the phone meanwhile.
    holding: bool,
}

/// What the phone said about a request beside its answer: for the desktop app that asked, never for an AI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Told {
    pub decided: Option<Decided>,
    pub suggestion: Option<Suggestion>,
}

struct RequestEntry {
    user: String,
    request: RelayRequest,
    state: watch::Sender<RequestState>,
}

/// Outcome of waiting on a relay request (spec §4.3 table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitResult {
    /// The phone answered (result, denied or error).
    Answered(RelayOutcome),
    /// The phone did not fetch the request within the offline threshold.
    Offline,
    /// Fetched, but no decision within the relay wait.
    Pending,
    /// Unknown, expired, or created by another connection.
    NotFound,
    /// Nobody answered within the Instant window (or the reviewer chose Later, or the agent asked): it is this
    /// ticket now, and its outcome comes later.
    Deferred(String),
}

/// What `reins_cancel` did to an Instant request.
#[derive(Debug, PartialEq, Eq)]
pub enum Cancel {
    Cancelled,
    /// The phone has it (it may be running it): not withdrawn.
    PhoneHasIt,
    Answered,
    NotFound,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// Unknown, expired, or belongs to another user.
    NotFound,
    AlreadyAnswered,
}

pub struct RelayHub {
    timing: Timing,
    /// How long an unanswered request stays Instant before it becomes a ticket (03 §1).
    instant_window: std::time::Duration,
    signal: Arc<ItemSignal>,
    /// Unanswered requests per account; `0`: no limit but the relay's own.
    max_queued: usize,
    requests: Mutex<TtlMap<RequestId, RequestEntry>>,
}

/// Drops one request of another account that holds more than `user` (answered first). `false` when no other
/// account holds more, so `user` cannot push someone else's call out once they hold the same number.
fn evict_other(map: &mut TtlMap<RequestId, RequestEntry>, user: &str) -> bool {
    let mine = map.values().filter(|e| e.user == user).count();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in map.values() {
        if entry.user != user {
            *counts.entry(entry.user.clone()).or_default() += 1;
        }
    }
    let Some(hog) = counts.into_iter().filter(|(_, n)| *n > mine).max_by_key(|(_, n)| *n).map(|(owner, _)| owner)
    else {
        return false;
    };
    map.evict_where(|e| e.user == hog && e.state.borrow().outcome.is_some()) || map.evict_where(|e| e.user == hog)
}

fn mark_delivered(state: &mut RequestState) -> bool {
    if state.delivered {
        false
    } else {
        state.delivered = true;
        true
    }
}

impl RelayHub {
    pub fn new(timing: Timing, signal: Arc<ItemSignal>) -> Self {
        Self::with_capacity(timing, signal, MAX_REQUESTS)
    }

    pub fn with_capacity(timing: Timing, signal: Arc<ItemSignal>, capacity: usize) -> Self {
        Self {
            timing,
            instant_window: std::time::Duration::from_secs(reins_proto::ticket::INSTANT_DEFAULT_SECS),
            signal,
            max_queued: DEFAULT_MAX_QUEUED,
            requests: Mutex::new(TtlMap::new(ITEM_TTL, capacity)),
        }
    }

    /// The same relay, with at most `max_queued` unanswered requests per account (`0`: no per-account limit).
    #[must_use]
    pub fn with_max_queued(self, max_queued: usize) -> Self {
        Self {
            max_queued,
            ..self
        }
    }

    /// The same relay, with unanswered requests becoming tickets after `window` (never past the request's lifetime).
    #[must_use]
    pub fn with_instant_window(self, window: std::time::Duration) -> Self {
        Self {
            instant_window: window.min(ITEM_TTL),
            ..self
        }
    }

    fn lock(&self) -> MutexGuard<'_, TtlMap<RequestId, RequestEntry>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues a normalized tool call for `user`'s approval device and wakes long-polls. Refused when `user` already
    /// has as many unanswered requests as an account may (a phone that is away should not let one account fill the
    /// relay for everyone), or when the relay is full of this account's own requests. Another account is not refused
    /// because the relay is full: the account holding the most loses its oldest request (an answered one first, so a
    /// call the phone has not decided yet stays when a finished one can go). The same call from the same connection
    /// while an earlier one is still unanswered (a client retrying after a timeout) is that earlier request: the
    /// phone shows it once, and both callers get its answer.
    #[cfg(test)]
    pub fn submit(
        &self,
        user: &str,
        connection_id: &ConnectionId,
        connection_label: &str,
        call: ToolCall,
        account: Option<String>,
        now_unix: i64,
    ) -> Result<RelayRequest, QueueFull> {
        self.submit_waiting(user, connection_id, connection_label, call, account, now_unix, None)
    }

    /// Like [`RelayHub::submit`] for a caller that keeps asking for up to `waits` (the desktop app waits longer than
    /// one relay wait, polling again): the phone counts down that instead, never past the request's lifetime.
    #[allow(clippy::too_many_arguments, reason = "a request is described by this many independent facts")]
    pub fn submit_waiting(
        &self,
        user: &str,
        connection_id: &ConnectionId,
        connection_label: &str,
        call: ToolCall,
        account: Option<String>,
        now_unix: i64,
        waits: Option<std::time::Duration>,
    ) -> Result<RelayRequest, QueueFull> {
        let wait = waits.map_or(self.timing.relay_wait, |w| w.clamp(self.timing.relay_wait, ITEM_TTL));
        let request = RelayRequest {
            v: PROTOCOL_VERSION,
            id: RequestId(get_uuid()),
            connection_id: connection_id.clone(),
            connection_label: connection_label.to_owned(),
            created_at: now_unix,
            wait_until: Some(now_unix.saturating_add(i64::try_from(wait.as_secs()).unwrap_or(0))),
            account,
            call,
        };
        let instant = i64::try_from(self.instant_window.max(self.timing.relay_wait).as_secs()).unwrap_or(0);
        let entry = RequestEntry {
            user: user.to_owned(),
            request: request.clone(),
            state: watch::Sender::new(RequestState {
                instant_until: now_unix.saturating_add(instant),
                ..RequestState::default()
            }),
        };
        {
            let mut map = self.lock();
            let same = map.values().find(|e| {
                e.user == user
                    && e.request.connection_id == request.connection_id
                    && e.request.account == request.account
                    && e.request.call == request.call
                    && e.state.borrow().outcome.is_none()
            });
            if let Some(earlier) = same {
                let earlier = earlier.request.clone();
                drop(map);
                self.signal.notify();
                return Ok(earlier);
            }
            if self.max_queued != 0 {
                let queued = map.values().filter(|e| e.user == user && e.state.borrow().outcome.is_none()).count();
                if queued >= self.max_queued {
                    return Err(QueueFull::Account);
                }
            }
            map.purge();
            if !map.has_room() && !evict_other(&mut map, user) {
                return Err(QueueFull::Server);
            }
            map.insert(request.id.clone(), entry)?;
        }
        self.signal.notify();
        Ok(request)
    }

    /// Requests of `user` never handed to the phone before, oldest first; marks them delivered.
    pub fn take_undelivered(&self, user: &str) -> Vec<RelayRequest> {
        let map = self.lock();
        let mut out = Vec::new();
        for entry in map.values() {
            if entry.user == user && !entry.state.borrow().holding && entry.state.send_if_modified(mark_delivered) {
                out.push(entry.request.clone());
            }
        }
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// One request of `user` (answered or not); marks it delivered.
    pub fn fetch(&self, user: &str, id: &RequestId) -> Option<RelayRequest> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user && !e.state.borrow().holding)?;
        entry.state.send_if_modified(mark_delivered);
        Some(entry.request.clone())
    }

    /// Stores the phone's answer; only the first answer counts.
    #[cfg(test)]
    pub fn answer(&self, user: &str, id: &RequestId, outcome: RelayOutcome) -> Result<(), AnswerError> {
        self.answer_decided(user, id, outcome, None)
    }

    /// [`RelayHub::answer`], with who decided it when the user did not (made safe to show).
    pub fn answer_decided(
        &self,
        user: &str,
        id: &RequestId,
        outcome: RelayOutcome,
        decided: Option<Decided>,
    ) -> Result<(), AnswerError> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user).ok_or(AnswerError::NotFound)?;
        let stored = entry.state.send_if_modified(move |s| {
            if s.outcome.is_some() {
                return false;
            }
            s.delivered = true;
            s.outcome = Some(outcome);
            s.decided = decided.map(Decided::cleaned);
            true
        });
        if stored {
            Ok(())
        } else {
            Err(AnswerError::AlreadyAnswered)
        }
    }

    /// Where Autopilot leans on `user`'s request `id` while it waits; the latest counts, none once it is answered.
    pub fn suggest(&self, user: &str, id: &RequestId, suggestion: Suggestion) -> Result<(), AnswerError> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user).ok_or(AnswerError::NotFound)?;
        let mut answered = false;
        // Wakes the waits of the desktop app, which shows it ([`RelayHub::wait_leaning`]); the others wait on.
        entry.state.send_if_modified(|s| {
            answered = s.outcome.is_some();
            let new = !answered && s.suggestion.as_ref() != Some(&suggestion);
            if new {
                s.suggestion = Some(suggestion);
            }
            new
        });
        if answered {
            Err(AnswerError::AlreadyAnswered)
        } else {
            Ok(())
        }
    }

    /// Where `id` stands now, for the connection that created it, without waiting: answered, with the phone
    /// (pending), not picked up yet (offline), or unknown.
    pub fn peek(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult {
        let map = self.lock();
        let Some(entry) = map.get(id).filter(|e| e.request.connection_id == *connection_id) else {
            return WaitResult::NotFound;
        };
        let state = entry.state.borrow();
        if let Some(ticket) = &state.deferred {
            return WaitResult::Deferred(ticket.clone());
        }
        match (&state.outcome, state.delivered) {
            (Some(outcome), _) => WaitResult::Answered(outcome.clone()),
            (None, true) => WaitResult::Pending,
            (None, false) => WaitResult::Offline,
        }
    }

    /// What the phone said beside its answer to `id`, for the connection that created it only.
    pub fn told(&self, id: &RequestId, connection_id: &ConnectionId) -> Told {
        let map = self.lock();
        map.get(id).filter(|e| e.request.connection_id == *connection_id).map_or_else(Told::default, |e| {
            let s = e.state.borrow();
            Told {
                decided: s.decided.clone(),
                suggestion: s.suggestion.clone().filter(|_| s.outcome.is_none()),
            }
        })
    }

    /// Nothing can reach the phone for `id` right now (see [`RequestState::unreachable`]): a first wait on it says
    /// `Offline` at once.
    pub fn mark_unreachable(&self, id: &RequestId) {
        if let Some(entry) = self.lock().get(id) {
            entry.state.send_if_modified(|s| {
                let changed = !s.unreachable && !s.delivered;
                s.unreachable = true;
                changed
            });
        }
    }

    /// The first wait after submitting: like [`RelayHub::wait`], and `Offline` at once when the phone cannot be
    /// reached ([`RelayHub::mark_unreachable`]), even while waiting.
    pub async fn wait_first(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult {
        self.wait_inner(id, connection_id, true, false).await
    }

    /// [`RelayHub::wait_first`] or [`RelayHub::wait`] for the desktop app, which shows where Autopilot leans: a new
    /// leaning ends the wait at once as `Pending` (the app asks again).
    pub async fn wait_leaning(&self, id: &RequestId, connection_id: &ConnectionId, first: bool) -> WaitResult {
        self.wait_inner(id, connection_id, first, true).await
    }

    /// Waits for the phone per spec §4.3, measured from now: `Offline` if the request is still
    /// undelivered after `timing.offline`, `Pending` after `timing.relay_wait`, else the answer.
    /// Only the connection that created the request may wait on it.
    pub async fn wait(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult {
        self.wait_inner(id, connection_id, false, false).await
    }

    async fn wait_inner(
        &self,
        id: &RequestId,
        connection_id: &ConnectionId,
        quick_offline: bool,
        leaning: bool,
    ) -> WaitResult {
        let start = Instant::now();
        let receiver = {
            let map = self.lock();
            map.get(id).filter(|e| e.request.connection_id == *connection_id).map(|e| e.state.subscribe())
        };
        let Some(mut rx) = receiver else {
            return WaitResult::NotFound;
        };
        let offline_at = start + self.timing.offline;
        let pending_at = start + self.timing.relay_wait;
        let leaned = rx.borrow().suggestion.clone();
        loop {
            let (delivered, outcome, unreachable, suggestion, deferred) = {
                let state = rx.borrow_and_update();
                (
                    state.delivered,
                    state.outcome.clone(),
                    state.unreachable,
                    state.suggestion.clone(),
                    state.deferred.clone(),
                )
            };
            if let Some(ticket) = deferred {
                return WaitResult::Deferred(ticket);
            }
            if let Some(outcome) = outcome {
                return WaitResult::Answered(outcome);
            }
            if leaning && suggestion != leaned {
                return WaitResult::Pending;
            }
            let now = Instant::now();
            if !delivered && (now >= offline_at || (quick_offline && unreachable)) {
                return WaitResult::Offline;
            }
            if now >= pending_at {
                return WaitResult::Pending;
            }
            let deadline = if delivered {
                pending_at
            } else {
                offline_at.min(pending_at)
            };
            if matches!(tokio::time::timeout_at(deadline, rx.changed()).await, Ok(Err(_))) {
                // The entry was dropped (expired and purged) while we waited.
                return WaitResult::NotFound;
            }
        }
    }

    pub fn purge(&self) {
        self.lock().purge();
    }

    /// Unanswered requests whose Instant window ended at `now_unix` or before, oldest first, with their account:
    /// what becomes a ticket ([`RelayHub::mark_deferred`]) or stays Instant ([`RelayHub::keep_instant`]).
    pub fn due_for_deferral(&self, now_unix: i64) -> Vec<(String, RelayRequest)> {
        let mut out: Vec<(String, RelayRequest)> = self
            .lock()
            .values()
            .filter(|e| {
                let s = e.state.borrow();
                // Only what the phone never got: a request the phone holds becomes a ticket by the phone's own word
                // (Later), never behind its back, so its answer always lands where it expects.
                s.outcome.is_none() && s.deferred.is_none() && !s.delivered && !s.holding && s.instant_until <= now_unix
            })
            .map(|e| (e.user.clone(), e.request.clone()))
            .collect();
        out.sort_by(|a, b| (a.1.created_at, &a.1.id).cmp(&(b.1.created_at, &b.1.id)));
        out
    }

    /// `user`'s unanswered request `id`, without marking it delivered (the reviewer chose Later on it).
    pub fn unanswered(&self, user: &str, id: &RequestId) -> Option<RelayRequest> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user)?;
        let s = entry.state.borrow();
        (s.outcome.is_none() && s.deferred.is_none()).then(|| entry.request.clone())
    }

    /// `id` became `ticket`: the calls waiting on it end with [`WaitResult::Deferred`], and the relay forgets it (its
    /// body is no longer kept in clear). `false` when it was answered (or gone) first.
    pub fn mark_deferred(&self, id: &RequestId, ticket: &str) -> bool {
        let mut map = self.lock();
        let Some(entry) = map.get(id) else {
            return false;
        };
        let deferred = entry.state.send_if_modified(|s| {
            let open = s.outcome.is_none() && s.deferred.is_none();
            if open {
                s.deferred = Some(ticket.to_owned());
            }
            open
        });
        if deferred {
            map.remove(id);
        }
        drop(map);
        if deferred {
            self.signal.notify();
        }
        deferred
    }

    /// Reserves unanswered request `id` for becoming a ticket, atomically with the phone's fetches: refused when it was
    /// answered, or when the phone already has it and the reviewer did not choose Later (`later`). While held, the
    /// phone is not handed it; [`RelayHub::mark_deferred`] completes it, [`RelayHub::release_hold`] gives it back.
    pub fn hold_for_deferral(&self, id: &RequestId, later: bool) -> bool {
        let map = self.lock();
        map.get(id).is_some_and(|entry| {
            let mut held = false;
            entry.state.send_if_modified(|s| {
                held = s.outcome.is_none() && s.deferred.is_none() && !s.holding && (later || !s.delivered);
                if held {
                    s.holding = true;
                }
                false
            });
            held
        })
    }

    /// Ends a hold that did not become a ticket.
    pub fn release_hold(&self, id: &RequestId) {
        if let Some(entry) = self.lock().get(id) {
            entry.state.send_if_modified(|s| {
                s.holding = false;
                false
            });
        }
    }

    /// `id` cannot become a ticket (no key to seal it to, too many open): it stays Instant until it expires.
    pub fn keep_instant(&self, id: &RequestId) {
        if let Some(entry) = self.lock().get(id) {
            entry.state.send_if_modified(|s| {
                s.instant_until = i64::MAX;
                false
            });
        }
    }

    /// The agent withdraws `id` (`reins_cancel`). Only while the phone has not got it: once it has, it may be running
    /// it, and only its answer says what happened.
    pub fn cancel(&self, id: &RequestId, connection_id: &ConnectionId) -> Cancel {
        let mut map = self.lock();
        let Some(entry) = map.get(id).filter(|e| e.request.connection_id == *connection_id) else {
            return Cancel::NotFound;
        };
        let state = entry.state.borrow().clone();
        if state.outcome.is_some() {
            return Cancel::Answered;
        }
        if state.delivered || state.holding {
            return Cancel::PhoneHasIt;
        }
        map.remove(id);
        Cancel::Cancelled
    }

    /// Drops `user`'s requests (a deleted account); calls still waiting on one see it gone.
    pub fn forget_user(&self, user: &str) {
        self.lock().retain(|e| e.user != user);
    }

    /// Unanswered requests of `user`, oldest first, without marking them delivered (what a viewer shows).
    pub fn peek_waiting(&self, user: &str) -> Vec<RelayRequest> {
        let mut out: Vec<RelayRequest> = self
            .lock()
            .values()
            .filter(|e| e.user == user && e.state.borrow().outcome.is_none())
            .map(|e| e.request.clone())
            .collect();
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// Ids of `user`'s answered requests still held.
    pub fn answered_ids(&self, user: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .values()
            .filter(|e| e.user == user && e.state.borrow().outcome.is_some())
            .map(|e| e.request.id.0.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Hands `user`'s unanswered requests to the next `GET /pending` again (a new primary took the role). A call
    /// still waiting on one counts it as undelivered from then on.
    pub fn requeue(&self, user: &str) {
        for e in self.lock().values().filter(|e| e.user == user) {
            e.state.send_if_modified(|s| {
                let again = s.outcome.is_none() && s.delivered;
                if again {
                    s.delivered = false;
                }
                again
            });
        }
        self.signal.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reins_proto::relay::ToolResult;
    use tokio::time::{Instant, sleep};

    use super::*;

    const USER: &str = "user-1";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn hub() -> RelayHub {
        RelayHub::new(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::new(ItemSignal::new()),
        )
    }

    fn conn() -> ConnectionId {
        "conn-1".into()
    }

    /// A search no other test call repeats (identical unanswered calls are one request).
    fn search() -> ToolCall {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        ToolCall::GmailSearch {
            query: format!("from:bank {}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            max_results: 10,
        }
    }

    fn found() -> RelayOutcome {
        RelayOutcome::Result {
            result: ToolResult::Search {
                messages: vec![],
            },
        }
    }

    #[tokio::test(start_paused = true)]
    async fn undelivered_request_is_offline_after_the_threshold_not_the_full_wait() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn delivered_but_undecided_is_pending_after_the_wait() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let start = Instant::now();
        let cid = conn();
        let (res, ()) = tokio::join!(h.wait(&req.id, &cid), async {
            sleep(secs(3)).await;
            assert!(h.fetch(USER, &req.id).is_some());
        });
        assert_eq!(res, WaitResult::Pending);
        assert_eq!(start.elapsed(), secs(45));
    }

    #[tokio::test(start_paused = true)]
    async fn answer_within_the_wait_is_returned_immediately() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let start = Instant::now();
        let cid = conn();
        let (res, ()) = tokio::join!(h.wait(&req.id, &cid), async {
            sleep(secs(2)).await;
            assert_eq!(h.take_undelivered(USER).len(), 1);
            sleep(secs(5)).await;
            h.answer(USER, &req.id, found()).unwrap();
        });
        assert_eq!(res, WaitResult::Answered(found()));
        assert_eq!(start.elapsed(), secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn late_result_is_readable_by_the_same_connection_only() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let cid = conn();
        let (first, ()) = tokio::join!(h.wait(&req.id, &cid), async {
            sleep(secs(1)).await;
            h.fetch(USER, &req.id).unwrap();
        });
        assert_eq!(first, WaitResult::Pending);
        sleep(secs(5)).await;
        h.answer(USER, &req.id, found()).unwrap();
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Answered(found()));
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Answered(found()), "idempotent until expiry");
        assert_eq!(h.wait(&req.id, &"conn-2".into()).await, WaitResult::NotFound);
    }

    #[tokio::test(start_paused = true)]
    async fn get_result_on_a_still_undelivered_request_is_offline_again() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        let start = Instant::now();
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn take_undelivered_is_per_user_ordered_and_one_shot() {
        let h = hub();
        let a = h.submit(USER, &conn(), "ChatGPT", search(), None, 5).unwrap();
        let b = h.submit(USER, &conn(), "ChatGPT", search(), None, 1).unwrap();
        h.submit("user-2", &conn(), "ChatGPT", search(), None, 0).unwrap();
        let got: Vec<RequestId> = h.take_undelivered(USER).into_iter().map(|r| r.id).collect();
        assert_eq!(got, vec![b.id, a.id]);
        assert!(h.take_undelivered(USER).is_empty());
        assert_eq!(h.take_undelivered("user-2").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn answers_are_checked_for_owner_and_accepted_once() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert!(h.fetch("user-2", &req.id).is_none());
        assert_eq!(h.answer("user-2", &req.id, found()), Err(AnswerError::NotFound));
        assert_eq!(h.answer(USER, &"nope".into(), found()), Err(AnswerError::NotFound));
        h.answer(USER, &req.id, found()).unwrap();
        let denied = RelayOutcome::Denied {
            reason: None,
        };
        assert_eq!(h.answer(USER, &req.id, denied), Err(AnswerError::AlreadyAnswered));
    }

    #[tokio::test(start_paused = true)]
    async fn requests_expire_after_ten_minutes() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        tokio::time::advance(secs(600)).await;
        assert!(h.fetch(USER, &req.id).is_none());
        assert!(h.take_undelivered(USER).is_empty());
        assert_eq!(h.answer(USER, &req.id, found()), Err(AnswerError::NotFound));
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::NotFound);
    }

    #[tokio::test(start_paused = true)]
    async fn submit_wakes_long_polls_and_respects_capacity() {
        let signal = Arc::new(ItemSignal::new());
        let h = RelayHub::with_capacity(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::clone(&signal),
            1,
        );
        let mut rx = signal.subscribe();
        assert!(!rx.has_changed().unwrap());
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        assert_eq!(req.v, PROTOCOL_VERSION);
        assert_eq!(req.connection_label, "ChatGPT");
        assert_eq!(h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap_err(), QueueFull::Server);
        assert!(!rx.has_changed().unwrap(), "a rejected submit does not wake anyone");
    }

    #[tokio::test(start_paused = true)]
    async fn an_account_queues_a_bounded_number_of_unanswered_requests() {
        let h = hub().with_max_queued(2);
        let first = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert_eq!(h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap_err(), QueueFull::Account);
        h.submit("user-2", &conn(), "ChatGPT", search(), None, 0).expect("other accounts are not affected");
        h.answer(USER, &first.id, found()).unwrap();
        h.submit(USER, &conn(), "ChatGPT", search(), None, 0).expect("answered requests do not count");
        tokio::time::advance(secs(600)).await;
        let unlimited = hub().with_max_queued(0);
        for _ in 0..10 {
            unlimited.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_retried_call_is_the_same_request_until_it_is_answered() {
        let h = hub();
        let call = search();
        let first = h.submit(USER, &conn(), "ChatGPT", call.clone(), None, 0).unwrap();
        let again = h.submit(USER, &conn(), "ChatGPT", call.clone(), None, 5).unwrap();
        assert_eq!(again.id, first.id, "the phone shows it once");
        assert_eq!(h.take_undelivered(USER).len(), 1);
        let other_ai = h.submit(USER, &"conn-2".into(), "Claude", call.clone(), None, 5).unwrap();
        assert_ne!(other_ai.id, first.id, "another connection asks for itself");
        let other_account = h.submit(USER, &conn(), "ChatGPT", call.clone(), Some("b@x.com".to_owned()), 5).unwrap();
        assert_ne!(other_account.id, first.id);
        h.answer(USER, &first.id, found()).unwrap();
        let after = h.submit(USER, &conn(), "ChatGPT", call, None, 9).unwrap();
        assert_ne!(after.id, first.id, "once answered, the same call is a new request");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_phone_is_reported_at_once_on_the_first_wait_only() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.mark_unreachable(&req.id);
        let start = Instant::now();
        assert_eq!(h.wait_first(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert_eq!(h.wait(&req.id, &conn()).await, WaitResult::Offline);
        assert_eq!(start.elapsed(), secs(10), "a later wait gives the phone the usual time");
        // A push failing while the first wait runs ends it too.
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let start = Instant::now();
        let cid = conn();
        let (res, ()) = tokio::join!(h.wait_first(&req.id, &cid), async {
            sleep(secs(2)).await;
            h.mark_unreachable(&req.id);
        });
        assert_eq!((res, start.elapsed()), (WaitResult::Offline, secs(2)));
        // Once the phone has it, nothing about reachability matters.
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.fetch(USER, &req.id).unwrap();
        h.mark_unreachable(&req.id);
        let start = Instant::now();
        assert_eq!(h.wait_first(&req.id, &conn()).await, WaitResult::Pending);
        assert_eq!(start.elapsed(), secs(45));
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_that_waits_longer_shows_its_own_deadline_never_past_the_lifetime() {
        let h = hub();
        let wait = |w: Option<u64>| {
            h.submit_waiting(USER, &conn(), "Desktop", search(), None, 1_000, w.map(secs)).unwrap().wait_until
        };
        assert_eq!(wait(None), Some(1_045));
        assert_eq!(wait(Some(120)), Some(1_120));
        assert_eq!(wait(Some(5)), Some(1_045), "never shorter than one relay wait");
        assert_eq!(wait(Some(3_600)), Some(1_600), "never past the request's lifetime");
    }

    #[tokio::test(start_paused = true)]
    async fn viewers_peek_without_delivering_and_a_new_primary_gets_requests_again() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert_eq!(h.peek_waiting(USER).len(), 1);
        assert_eq!(h.take_undelivered(USER).len(), 1, "peeking did not deliver it");
        assert!(h.take_undelivered(USER).is_empty());
        h.requeue(USER);
        assert_eq!(h.take_undelivered(USER).len(), 1, "handed out again");
        assert!(h.answered_ids(USER).is_empty());
        h.answer(USER, &req.id, found()).unwrap();
        assert!(h.peek_waiting(USER).is_empty());
        assert_eq!(h.answered_ids(USER), [req.id.0.clone()]);
        h.requeue(USER);
        assert!(h.take_undelivered(USER).is_empty(), "an answered request stays answered");
        assert!(h.peek_waiting("someone-else").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn who_decided_and_the_leaning_reach_only_the_connection_that_asked() {
        use reins_proto::autopilot::{DecidedBy, Leaning};
        let h = hub();
        let req = h.submit(USER, &conn(), "Desktop", search(), None, 0).unwrap();
        let leaning = Suggestion {
            v: 1,
            leaning: Leaning::Approve,
            confidence: 0.9,
        };
        assert_eq!(h.suggest("user-2", &req.id, leaning.clone()), Err(AnswerError::NotFound), "another account's");
        h.suggest(USER, &req.id, leaning.clone()).unwrap();
        assert_eq!(h.told(&req.id, &conn()).suggestion, Some(leaning.clone()));
        assert_eq!(h.told(&req.id, &"conn-2".into()), Told::default(), "another connection learns nothing");
        let decided = Decided {
            by: DecidedBy::Autopilot,
            confidence: Some(2.0),
            reasons: vec![reins_proto::autopilot::Reason::Routine {
                approve: 140,
            }],
            correctable: true,
        };
        h.answer_decided(USER, &req.id, found(), Some(decided)).unwrap();
        let told = h.told(&req.id, &conn());
        assert_eq!(told.suggestion, None, "answered: nothing leans any more");
        let decided = told.decided.unwrap();
        assert_eq!(
            (decided.confidence, decided.reasons[0]),
            (
                None,
                reins_proto::autopilot::Reason::Routine {
                    approve: 100
                }
            ),
            "made safe to show"
        );
        assert_eq!(h.suggest(USER, &req.id, leaning), Err(AnswerError::AlreadyAnswered));
        // An older phone's answer says nothing of it.
        let older = h.submit(USER, &conn(), "Desktop", search(), None, 0).unwrap();
        h.answer(USER, &older.id, found()).unwrap();
        assert_eq!(h.told(&older.id, &conn()), Told::default());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_leaning_ends_the_desktop_apps_wait_only() {
        use reins_proto::autopilot::Leaning;
        let h = hub();
        let req = h.submit(USER, &conn(), "Desktop", search(), None, 0).unwrap();
        h.fetch(USER, &req.id).unwrap();
        let lean = |c: f32| Suggestion {
            v: 1,
            leaning: Leaning::Ask,
            confidence: c,
        };
        let start = Instant::now();
        let cid = conn();
        let (desk, ai, ()) = tokio::join!(h.wait_leaning(&req.id, &cid, true), h.wait(&req.id, &cid), async {
            sleep(secs(3)).await;
            h.suggest(USER, &req.id, lean(0.5)).unwrap();
        });
        assert_eq!((desk, ai), (WaitResult::Pending, WaitResult::Pending));
        assert_eq!(start.elapsed(), secs(45), "the AI's wait ran its course");
        // Already seen: the next wait waits; the same leaning again wakes nobody, a new one does.
        let start = Instant::now();
        let (desk, ()) = tokio::join!(h.wait_leaning(&req.id, &cid, false), async {
            sleep(secs(2)).await;
            h.suggest(USER, &req.id, lean(0.5)).unwrap();
            sleep(secs(2)).await;
            h.suggest(USER, &req.id, lean(0.9)).unwrap();
        });
        assert_eq!((desk, start.elapsed()), (WaitResult::Pending, secs(4)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_peek_says_where_a_request_stands_without_waiting() {
        let h = hub();
        let req = h.submit(USER, &conn(), "Desktop", search(), None, 0).unwrap();
        assert_eq!(h.peek(&req.id, &conn()), WaitResult::Offline);
        h.fetch(USER, &req.id).unwrap();
        assert_eq!(h.peek(&req.id, &conn()), WaitResult::Pending);
        h.answer(USER, &req.id, found()).unwrap();
        assert_eq!(h.peek(&req.id, &conn()), WaitResult::Answered(found()));
        assert_eq!(h.peek(&req.id, &"conn-2".into()), WaitResult::NotFound, "another connection's");
    }

    fn small() -> RelayHub {
        RelayHub::with_capacity(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::new(ItemSignal::new()),
            2,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn one_account_filling_the_relay_does_not_block_another() {
        let h = small().with_max_queued(10);
        let oldest = h.submit("attacker", &conn(), "ChatGPT", search(), None, 1).unwrap();
        let newer = h.submit("attacker", &conn(), "ChatGPT", search(), None, 2).unwrap();
        assert_eq!(
            h.submit("attacker", &conn(), "ChatGPT", search(), None, 3).unwrap_err(),
            QueueFull::Server,
            "it does not drop its own call"
        );
        assert!(h.fetch("attacker", &oldest.id).is_some());
        let victim = h.submit("victim", &conn(), "Claude", search(), None, 4).unwrap();
        assert!(h.fetch("victim", &victim.id).is_some());
        assert!(h.fetch("attacker", &oldest.id).is_none(), "the account that held the most loses its oldest");
        assert!(h.fetch("attacker", &newer.id).is_some());
        // Tied: the attacker cannot push the victim back out.
        assert_eq!(h.submit("attacker", &conn(), "ChatGPT", search(), None, 5).unwrap_err(), QueueFull::Server);
        assert!(h.fetch("victim", &victim.id).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_relay_drops_an_answered_request_before_an_open_one() {
        let h = small().with_max_queued(10);
        let open = h.submit("attacker", &conn(), "ChatGPT", search(), None, 1).unwrap();
        let done = h.submit("attacker", &conn(), "ChatGPT", search(), None, 2).unwrap();
        h.answer("attacker", &done.id, found()).unwrap();
        let victim = h.submit("victim", &conn(), "Claude", search(), None, 3).unwrap();
        assert!(h.fetch("attacker", &open.id).is_some(), "the call the phone has not decided stays");
        assert!(h.fetch("attacker", &done.id).is_none());
        assert!(h.fetch("victim", &victim.id).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn an_account_with_no_queue_cap_still_cannot_lock_the_relay() {
        let h = RelayHub::with_capacity(
            Timing {
                relay_wait: secs(45),
                offline: secs(10),
            },
            Arc::new(ItemSignal::new()),
            1,
        )
        .with_max_queued(0);
        let hog = h.submit("attacker", &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert_eq!(h.submit("attacker", &conn(), "ChatGPT", search(), None, 1).unwrap_err(), QueueFull::Server);
        let victim = h.submit("victim", &conn(), "Claude", search(), None, 2).unwrap();
        assert!(h.fetch("attacker", &hog.id).is_none());
        assert!(h.fetch("victim", &victim.id).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn unanswered_requests_are_due_for_deferral_after_the_instant_window() {
        let h = hub().with_instant_window(secs(120));
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 1_000).unwrap();
        let answered = h.submit(USER, &conn(), "ChatGPT", search(), None, 1_000).unwrap();
        h.answer(USER, &answered.id, found()).unwrap();
        assert!(h.due_for_deferral(1_119).is_empty(), "still Instant");
        let due = h.due_for_deferral(1_120);
        assert_eq!(due.len(), 1, "an answered request never becomes a ticket");
        assert_eq!((due[0].0.as_str(), &due[0].1.id), (USER, &req.id));
        h.keep_instant(&req.id);
        assert!(h.due_for_deferral(9_999).is_empty(), "kept Instant: it expires as before");
        // The window is never shorter than one relay wait.
        let short = hub().with_instant_window(secs(5));
        short.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert!(short.due_for_deferral(44).is_empty());
        assert_eq!(short.due_for_deferral(45).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_deferred_request_ends_its_waits_with_the_ticket_and_is_forgotten() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.fetch(USER, &req.id).unwrap();
        let cid = conn();
        let start = Instant::now();
        let (res, ()) = tokio::join!(h.wait(&req.id, &cid), async {
            sleep(secs(3)).await;
            assert!(h.mark_deferred(&req.id, "rt_x"));
        });
        assert_eq!((res, start.elapsed()), (WaitResult::Deferred("rt_x".to_owned()), secs(3)));
        assert!(h.fetch(USER, &req.id).is_none(), "the body is no longer held");
        assert_eq!(h.peek(&req.id, &conn()), WaitResult::NotFound);
        assert!(!h.mark_deferred(&req.id, "rt_x"), "once");
        let answered = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.answer(USER, &answered.id, found()).unwrap();
        assert!(!h.mark_deferred(&answered.id, "rt_y"), "an answer came first");
        assert!(h.unanswered(USER, &answered.id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_the_phone_has_is_parked_only_on_its_word() {
        let h = hub().with_instant_window(secs(60));
        let fetched = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        let offline = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.fetch(USER, &fetched.id).unwrap();
        let due: Vec<RequestId> = h.due_for_deferral(60).into_iter().map(|(_, r)| r.id).collect();
        assert_eq!(due, [offline.id.clone()], "the phone has the other one: it may be answering it right now");
        assert!(!h.hold_for_deferral(&fetched.id, false), "never behind the phone's back");
        assert!(h.hold_for_deferral(&fetched.id, true), "the reviewer chose Later");
        // While held, the phone is not handed it; an answer that still comes wins over the deferral.
        assert!(h.hold_for_deferral(&offline.id, false));
        assert!(h.take_undelivered(USER).is_empty());
        assert!(h.fetch(USER, &offline.id).is_none());
        h.answer(USER, &fetched.id, found()).unwrap();
        assert!(!h.mark_deferred(&fetched.id, "rt_x"), "answered first: no ticket");
        h.release_hold(&offline.id);
        assert_eq!(h.take_undelivered(USER).len(), 1, "released: the phone gets it");
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_connection_that_asked_can_cancel_an_unanswered_request() {
        let h = hub();
        let req = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        assert_eq!(h.cancel(&req.id, &"conn-2".into()), Cancel::NotFound);
        assert!(h.unanswered(USER, &req.id).is_some());
        assert_eq!(h.cancel(&req.id, &conn()), Cancel::Cancelled);
        assert!(h.fetch(USER, &req.id).is_none());
        // Once the phone has it, it may be running it: never withdrawn behind its back.
        let fetched = h.submit(USER, &conn(), "ChatGPT", search(), None, 0).unwrap();
        h.fetch(USER, &fetched.id).unwrap();
        assert_eq!(h.cancel(&fetched.id, &conn()), Cancel::PhoneHasIt);
        assert!(h.unanswered(USER, &fetched.id).is_some());
    }
}
