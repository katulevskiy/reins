//! In-memory relay between MCP tool calls and the approval device (spec §4.3).
//!
//! Nothing here touches disk: tool arguments and results live only in these maps.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use reins_proto::{
    PROTOCOL_VERSION,
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

/// Upper bound on relay requests held in memory at once.
pub const MAX_REQUESTS: usize = 10_000;
/// Default for the unanswered requests one account may have queued for its phone (`REINS_ACCOUNT_MAX_QUEUED`).
pub const DEFAULT_MAX_QUEUED: usize = 100;

/// Why a request was not queued.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueFull {
    /// The relay holds [`MAX_REQUESTS`] already.
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RequestState {
    /// The phone fetched the request (A2 or A3).
    delivered: bool,
    /// The phone's answer (A4), kept until the entry expires.
    outcome: Option<RelayOutcome>,
    /// Nothing can wake the phone (no push, the app not open) or the push failed: the first wait need not wait for
    /// the offline threshold.
    unreachable: bool,
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
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// Unknown, expired, or belongs to another user.
    NotFound,
    AlreadyAnswered,
}

pub struct RelayHub {
    timing: Timing,
    signal: Arc<ItemSignal>,
    /// Unanswered requests per account; `0`: no limit but the relay's own.
    max_queued: usize,
    requests: Mutex<TtlMap<RequestId, RequestEntry>>,
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

    fn lock(&self) -> MutexGuard<'_, TtlMap<RequestId, RequestEntry>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues a normalized tool call for `user`'s approval device and wakes long-polls. Refused when the relay is full,
    /// or when `user` already has as many unanswered requests as an account may (a phone that is away should not let
    /// one account fill the relay for everyone). The same call from the same connection while an earlier one is still
    /// unanswered (a client retrying after a timeout) is that earlier request: the phone shows it once, and both
    /// callers get its answer.
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
        let entry = RequestEntry {
            user: user.to_owned(),
            request: request.clone(),
            state: watch::Sender::new(RequestState::default()),
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
            if entry.user == user && entry.state.send_if_modified(mark_delivered) {
                out.push(entry.request.clone());
            }
        }
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// One request of `user` (answered or not); marks it delivered.
    pub fn fetch(&self, user: &str, id: &RequestId) -> Option<RelayRequest> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user)?;
        entry.state.send_if_modified(mark_delivered);
        Some(entry.request.clone())
    }

    /// Stores the phone's answer; only the first answer counts.
    pub fn answer(&self, user: &str, id: &RequestId, outcome: RelayOutcome) -> Result<(), AnswerError> {
        let map = self.lock();
        let entry = map.get(id).filter(|e| e.user == user).ok_or(AnswerError::NotFound)?;
        let stored = entry.state.send_if_modified(move |s| {
            if s.outcome.is_some() {
                return false;
            }
            s.delivered = true;
            s.outcome = Some(outcome);
            true
        });
        if stored {
            Ok(())
        } else {
            Err(AnswerError::AlreadyAnswered)
        }
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
        self.wait_inner(id, connection_id, true).await
    }

    /// Waits for the phone per spec §4.3, measured from now: `Offline` if the request is still
    /// undelivered after `timing.offline`, `Pending` after `timing.relay_wait`, else the answer.
    /// Only the connection that created the request may wait on it.
    pub async fn wait(&self, id: &RequestId, connection_id: &ConnectionId) -> WaitResult {
        self.wait_inner(id, connection_id, false).await
    }

    async fn wait_inner(&self, id: &RequestId, connection_id: &ConnectionId, quick_offline: bool) -> WaitResult {
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
        loop {
            let (delivered, outcome, unreachable) = {
                let state = rx.borrow_and_update();
                (state.delivered, state.outcome.clone(), state.unreachable)
            };
            if let Some(outcome) = outcome {
                return WaitResult::Answered(outcome);
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

    /// Drops `user`'s requests (a deleted account); calls still waiting on one see it gone.
    pub fn forget_user(&self, user: &str) {
        self.lock().retain(|e| e.user != user);
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
}
