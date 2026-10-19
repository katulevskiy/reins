//! Rate and concurrency limits for a public, multi-tenant deployment.
//!
//! Every limit is a setting (`REINS_*` in `.env.template`); `0` turns a rate or concurrency limit off. Rates use the
//! same keyed GCRA limiters as Vaultwarden's own (`src/ratelimit.rs`, governor); concurrency limits are counters
//! released when the guard drops. Quotas on stored files live with the blob store (`blob.rs`).
//!
//! Who is limited by what:
//! - an AI connection: requests to `/mcp` and the desktop API per minute, calls waiting for the phone at once;
//! - an account: calls relayed to its phone per minute (each may wake it with a push), outbound requests the server
//!   makes for its phone (per minute and at once), connection requests (pairings) naming its email or scanned by its
//!   phone (at most [`super::pairing::MAX_PER_PARTY`] open per email, shared with decoys for that email, and the same
//!   number per client network), wrong proofs sent to take its approval role (a device key that proved itself
//!   within a day keeps its own tries);
//! - a device: concurrent `GET /pending` and `GET /sync` long-polls;
//! - a phone of an account with phone roles (key: account and device): op uploads per minute, proposals per hour;
//! - an account with phone roles: roster changes per hour, ops and bytes its log holds (`REINS_ACCOUNT_LOG_*`);
//! - an IP address: OAuth dynamic client registrations, device authorizations (QR codes to pair a computer, at most
//!   [`super::device_flow::MAX_PER_ADDRESS`] open), "add another phone" join requests (with at most
//!   [`super::join::MAX_PER_USER`] open per account). Pairing decoys share the email's cap and the per-network cap
//!   ([`super::pairing::MAX_PER_PARTY`]). The address is [`client_network`].

use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    net::IpAddr,
    num::NonZeroU32,
    sync::{Arc, LazyLock, Mutex, PoisonError},
    time::{Duration, Instant},
};

use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock},
    state::keyed::DashMapStateStore,
};

use crate::CONFIG;

type Keyed<K> = RateLimiter<K, DashMapStateStore<K>, DefaultClock>;

/// A keyed rate limit; disabled when built from a zero setting.
pub struct RateLimit<K: Hash + Eq + Clone> {
    limiter: Option<Keyed<K>>,
}

impl<K: Hash + Eq + Clone> RateLimit<K> {
    fn from_quota(quota: Option<Quota>) -> Self {
        Self {
            limiter: quota.map(RateLimiter::keyed),
        }
    }

    /// `per_minute` requests a minute, all of which may come at once. `0` disables the limit.
    pub fn per_minute(per_minute: u32) -> Self {
        Self::from_quota(NonZeroU32::new(per_minute).map(Quota::per_minute))
    }

    /// One request every `seconds` on average, bursts of up to `burst`. `0` for either disables the limit.
    pub fn with_period(seconds: u64, burst: u32) -> Self {
        let quota = NonZeroU32::new(burst)
            .zip(Quota::with_period(Duration::from_secs(seconds)))
            .map(|(burst, quota)| quota.allow_burst(burst));
        Self::from_quota(quota)
    }

    /// Counts one request for `key`: `Err` with the time until the next one is allowed when over the limit.
    pub fn check(&self, key: &K) -> Result<(), Duration> {
        let Some(limiter) = &self.limiter else {
            return Ok(());
        };
        limiter.check_key(key).map_err(|not_until| not_until.wait_time_from(limiter.clock().now()))
    }

    /// Forgets keys whose budget is full again, so the state does not grow with every key ever seen.
    pub fn retain_recent(&self) {
        if let Some(limiter) = &self.limiter {
            limiter.retain_recent();
        }
    }
}

type Counts<K> = Arc<Mutex<HashMap<K, usize>>>;

/// At most `max` holders per key at once. `0` disables the limit.
pub struct Concurrency<K: Hash + Eq + Clone> {
    max: usize,
    active: Counts<K>,
}

/// One admitted holder; leaving (dropping the guard) frees the place.
#[must_use = "the place is freed when the guard drops"]
pub struct Admitted<K: Hash + Eq + Clone> {
    key: K,
    active: Counts<K>,
}

impl<K: Hash + Eq + Clone> Drop for Admitted<K> {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = active.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                active.remove(&self.key);
            }
        }
    }
}

impl<K: Hash + Eq + Clone> Concurrency<K> {
    pub fn new(max: u32) -> Self {
        Self {
            max: usize::try_from(max).unwrap_or(usize::MAX),
            active: Arc::default(),
        }
    }

    /// A place for `key`, or `None` when `max` holders already have one.
    pub fn try_enter(&self, key: &K) -> Option<Admitted<K>> {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let n = active.entry(key.clone()).or_insert(0);
        if self.max != 0 && *n >= self.max {
            return None;
        }
        *n += 1;
        Some(Admitted {
            key: key.clone(),
            active: Arc::clone(&self.active),
        })
    }

    /// Holders of `key` right now.
    #[cfg(test)]
    pub fn active(&self, key: &K) -> usize {
        self.active.lock().unwrap_or_else(PoisonError::into_inner).get(key).copied().unwrap_or(0)
    }
}

/// At most `max` failures per key within `window`; then the key is refused until its oldest failure ages out. Unlike
/// [`RateLimit`], only failures count, and asking whether a key is refused costs nothing. `0` disables the limit.
pub struct FailureLimit<K: Hash + Eq + Clone> {
    max: usize,
    window: Duration,
    failures: Mutex<HashMap<K, VecDeque<Instant>>>,
}

impl<K: Hash + Eq + Clone> FailureLimit<K> {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max: usize::try_from(max).unwrap_or(usize::MAX),
            window,
            failures: Mutex::default(),
        }
    }

    /// `Err` with the time until `key` may try again when it has `max` failures within the window.
    pub fn check(&self, key: &K) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    /// Counts a failure of `key`.
    pub fn fail(&self, key: &K) {
        self.fail_at(key, Instant::now());
    }

    fn check_at(&self, key: &K, now: Instant) -> Result<(), Duration> {
        if self.max == 0 {
            return Ok(());
        }
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(times) = failures.get_mut(key) else {
            return Ok(());
        };
        while times.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            times.pop_front();
        }
        match times.front() {
            None => {
                failures.remove(key);
                Ok(())
            }
            Some(oldest) if times.len() >= self.max => Err(self.window.saturating_sub(now.duration_since(*oldest))),
            Some(_) => Ok(()),
        }
    }

    fn fail_at(&self, key: &K, now: Instant) {
        if self.max == 0 {
            return;
        }
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let times = failures.entry(key.clone()).or_default();
        times.push_back(now);
        while times.len() > self.max {
            times.pop_front();
        }
    }

    /// Forgets keys whose failures all aged out.
    pub fn retain_recent(&self) {
        let now = Instant::now();
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, times| times.back().is_some_and(|t| now.duration_since(*t) < self.window));
    }
}

/// The address a limit sees: the IPv4 address, or the /64 of an IPv6 address (one host usually holds a whole /64, so
/// single addresses in it would each get their own quota). IPv4-mapped IPv6 is the IPv4 address.
pub fn client_network(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(std::net::Ipv6Addr::from(octets))
        }
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// Whole seconds to wait, rounded up, at least 1 (what error messages and `Retry-After` show).
pub fn retry_secs(wait: Duration) -> u64 {
    (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1)
}

/// The plain message every rate-limited caller gets: what was limited and when to try again.
pub fn rate_limited_text(what: &str, wait: Duration) -> String {
    format!("Rate limited: {what}; try again in {} s.", retry_secs(wait))
}

/// MCP and desktop requests per AI connection (key: connection id).
pub static CONNECTION_REQUESTS: LazyLock<RateLimit<String>> =
    LazyLock::new(|| RateLimit::per_minute(CONFIG.reins_connection_requests_per_minute()));

/// Tool calls of one AI connection waiting for the phone at once (key: connection id).
pub static CONNECTION_WAITING: LazyLock<Concurrency<String>> =
    LazyLock::new(|| Concurrency::new(CONFIG.reins_connection_max_waiting()));

/// Calls relayed to one account's phone (key: user id).
pub static ACCOUNT_CALLS: LazyLock<RateLimit<String>> =
    LazyLock::new(|| RateLimit::per_minute(CONFIG.reins_account_calls_per_minute()));

/// Connection requests (pairings) for one account email, known or not, so the limit says nothing about whether the
/// account exists (key: the lowercased email).
pub static PAIRINGS: LazyLock<RateLimit<String>> = LazyLock::new(|| {
    RateLimit::with_period(CONFIG.reins_pairing_ratelimit_seconds(), CONFIG.reins_pairing_ratelimit_max_burst())
});

/// Device authorizations (a computer asking for a QR code to pair, RFC 8628) per IP address, with the pairing
/// settings: each may become a pairing.
pub static DEVICE_AUTHORIZATIONS: LazyLock<RateLimit<IpAddr>> = LazyLock::new(|| {
    RateLimit::with_period(CONFIG.reins_pairing_ratelimit_seconds(), CONFIG.reins_pairing_ratelimit_max_burst())
});

/// Client documents (OAuth client metadata documents and key sets) fetched because of requests from one IP address:
/// a burst of 20, then one every 3 s. Every fetch a request makes the server do is charged to the request's address;
/// refreshes of documents the server already holds (known clients) are not. A request over budget is refused alone.
pub static CLIENT_DOCUMENT_FETCHES: LazyLock<RateLimit<IpAddr>> = LazyLock::new(|| RateLimit::with_period(3, 20));

/// Background refreshes of client documents the server holds that requests from one IP address may start: a burst
/// of 10, then one every 6 s. Over it, the request is served from what the server holds, without a refresh.
pub static CLIENT_DOCUMENT_REFRESHES: LazyLock<RateLimit<IpAddr>> = LazyLock::new(|| RateLimit::with_period(6, 10));

/// OAuth dynamic client registrations per IP address.
pub static REGISTRATIONS: LazyLock<RateLimit<IpAddr>> = LazyLock::new(|| {
    RateLimit::with_period(CONFIG.reins_register_ratelimit_seconds(), CONFIG.reins_register_ratelimit_max_burst())
});

/// `GET /pending` long-polls of one approval device at once (key: device id).
pub static DEVICE_POLLS: LazyLock<Concurrency<String>> =
    LazyLock::new(|| Concurrency::new(CONFIG.reins_device_max_polls()));

/// Wrong proofs sent to take the approval role (`PUT /reins/api/device` with a master password hash) for one
/// account (key: user id).
pub static DEVICE_PROOFS: LazyLock<FailureLimit<String>> = LazyLock::new(|| {
    FailureLimit::new(
        CONFIG.reins_device_proof_max_failures(),
        Duration::from_secs(CONFIG.reins_device_proof_window_seconds()),
    )
});

/// Wrong recovery proofs (the recovery wrap, a recovery) from one device (key: `<user id>/<device id>`). Each also counts
/// against [`DEVICE_PROOFS`], the account's limit.
pub static DEVICE_PROOFS_PER_DEVICE: LazyLock<FailureLimit<String>> = LazyLock::new(|| {
    FailureLimit::new(
        CONFIG.reins_device_proof_max_failures_per_device(),
        Duration::from_secs(CONFIG.reins_device_proof_window_seconds()),
    )
});

/// How long a right recovery proof exempts that device key from the account-wide limit.
const PROVEN_FOR: Duration = Duration::from_hours(24);

/// Most device keys remembered for one account. A new proof past this drops the oldest key of that same account.
const PROVEN_PER_USER: usize = 8;

/// Most device keys remembered at once, across every account. Past this a new key is refused. Nothing already
/// remembered is dropped to make room for another account.
const PROVEN_CAP: usize = 10_000;

struct ProvenEntry {
    at: Instant,
    /// Vaultwarden device id that proved, so signing that id out can drop the exemption when the key is not stored
    /// on a member row. Not part of the exemption: a different key on this id is not exempt.
    device_uuid: String,
}

/// Device keys that sent a right recovery proof, keyed by `<user id>/<device key hash>`: the same hash
/// [`require_approval_device`](super::device_api::require_approval_device) and the roster's member check compare.
#[derive(Default)]
pub struct ProvenDevices {
    entries: Mutex<HashMap<String, ProvenEntry>>,
}

impl ProvenDevices {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ProvenEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn map_key(user: &str, device_key: &str) -> String {
        format!("{user}/{device_key}")
    }

    /// Whether `device_key` proved itself for `user` within [`PROVEN_FOR`]. An empty key never is.
    pub fn is_proven(&self, user: &str, device_key: &str) -> bool {
        self.is_proven_at(user, device_key, Instant::now())
    }

    fn is_proven_at(&self, user: &str, device_key: &str, now: Instant) -> bool {
        if device_key.is_empty() {
            return false;
        }
        let mut entries = self.lock();
        let key = Self::map_key(user, device_key);
        let Some(entry) = entries.get(&key) else {
            return false;
        };
        if now.saturating_duration_since(entry.at) < PROVEN_FOR {
            return true;
        }
        entries.remove(&key);
        false
    }

    /// Remembers a right proof from `device_key` (the device key hash) on `device_uuid`. No key, nothing remembered.
    pub fn remember(&self, user: &str, device_key: &str, device_uuid: &str) {
        self.remember_at(user, device_key, device_uuid, Instant::now());
    }

    fn remember_at(&self, user: &str, device_key: &str, device_uuid: &str, now: Instant) {
        if device_key.is_empty() {
            return;
        }
        let mut entries = self.lock();
        entries.retain(|_, entry| now.saturating_duration_since(entry.at) < PROVEN_FOR);
        let key = Self::map_key(user, device_key);
        if let Some(entry) = entries.get_mut(&key) {
            entry.at = now;
            device_uuid.clone_into(&mut entry.device_uuid);
            return;
        }
        let prefix = format!("{user}/");
        if entries.keys().filter(|k| k.starts_with(&prefix)).count() >= PROVEN_PER_USER
            && let Some(oldest) = entries
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .min_by_key(|(_, entry)| entry.at)
                .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
        if entries.len() >= PROVEN_CAP {
            return;
        }
        entries.insert(
            key,
            ProvenEntry {
                at: now,
                device_uuid: device_uuid.to_owned(),
            },
        );
    }

    fn forget(&self, user: &str, device_key: &str) {
        if device_key.is_empty() {
            return;
        }
        self.lock().remove(&Self::map_key(user, device_key));
    }

    /// Drops every exemption this Vaultwarden device id proved, for this account.
    pub fn forget_device(&self, user: &str, device_uuid: &str) {
        let prefix = format!("{user}/");
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|key, entry| !(key.starts_with(&prefix) && entry.device_uuid == device_uuid));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// Device keys (`<user id>/<device key hash>`) that sent a right recovery proof: for [`PROVEN_FOR`] they keep their
/// own tries when other devices used up the account's limit, so a flood from new device ids never locks the owner
/// out. The key is the device key hash, the same identity the approval device and a roster member are checked by. A
/// caller that sent no device key is never exempt. One account holds at most [`PROVEN_PER_USER`] keys; a further proof
/// drops only that account's oldest. Past [`PROVEN_CAP`] a new key is refused, so one account cannot evict another's.
/// The entry is dropped when that device is signed out, deleted, or leaves the roster ([`forget_proven`]).
pub static PROVEN_DEVICES: LazyLock<ProvenDevices> = LazyLock::new(ProvenDevices::default);

/// Drops the proof exemption for this device key, if it has one.
pub fn forget_proven(user: &str, device_key: &str) {
    PROVEN_DEVICES.forget(user, device_key);
}

/// Outbound requests (file sends and fetches, proxied MCP calls) made for one account (key: user id).
pub static OUTBOUND_REQUESTS: LazyLock<RateLimit<String>> =
    LazyLock::new(|| RateLimit::per_minute(CONFIG.reins_outbound_requests_per_minute()));

/// Outbound requests of one account running at once (key: user id).
pub static OUTBOUND_RUNNING: LazyLock<Concurrency<String>> =
    LazyLock::new(|| Concurrency::new(CONFIG.reins_outbound_max_concurrent()));

/// "Add another phone" join requests (`POST /reins/api/joins`, also enrollments) per IP address.
pub static JOIN_CREATIONS: LazyLock<RateLimit<IpAddr>> = LazyLock::new(|| {
    RateLimit::with_period(CONFIG.reins_join_ratelimit_seconds(), CONFIG.reins_join_ratelimit_max_burst())
});

/// `POST /reins/api/ops` requests of one phone (key: user id and device id).
pub static MEMBER_OP_REQUESTS: LazyLock<RateLimit<(String, String)>> =
    LazyLock::new(|| RateLimit::per_minute(CONFIG.reins_member_op_requests_per_minute()));

/// Proposal ops of one phone (key: user id and device id): the hourly number, all of which may come at once.
pub static MEMBER_PROPOSALS: LazyLock<RateLimit<(String, String)>> = LazyLock::new(|| {
    let per_hour = CONFIG.reins_member_proposals_per_hour();
    RateLimit::with_period(
        if per_hour == 0 {
            0
        } else {
            3600 / u64::from(per_hour).max(1)
        },
        per_hour,
    )
});

/// Handover requests of one phone (key: user id and device id): each wakes the primary.
pub static HANDOVER_REQUESTS: LazyLock<RateLimit<(String, String)>> = LazyLock::new(|| {
    let per_hour = CONFIG.reins_handover_requests_per_hour();
    RateLimit::with_period(
        if per_hour == 0 {
            0
        } else {
            3600 / u64::from(per_hour).max(1)
        },
        per_hour,
    )
});

/// Alert pushes to an account's primary for viewers' proposals and brakes (key: user id): one per half minute; the
/// rest reach it by its poll (`log_head`), unannounced.
pub static PRIMARY_ALERTS: LazyLock<RateLimit<String>> = LazyLock::new(|| RateLimit::with_period(30, 1));

/// Roster appends of one account (key: user id).
pub static ROSTER_CHANGES: LazyLock<RateLimit<String>> = LazyLock::new(|| {
    let per_hour = CONFIG.reins_roster_changes_per_hour();
    RateLimit::with_period(
        if per_hour == 0 {
            0
        } else {
            3600 / u64::from(per_hour).max(1)
        },
        per_hour,
    )
});

/// Housekeeping: drops rate-limit state of keys that are back at a full budget.
pub fn retain_recent() {
    CONNECTION_REQUESTS.retain_recent();
    ACCOUNT_CALLS.retain_recent();
    PAIRINGS.retain_recent();
    DEVICE_AUTHORIZATIONS.retain_recent();
    REGISTRATIONS.retain_recent();
    CLIENT_DOCUMENT_FETCHES.retain_recent();
    CLIENT_DOCUMENT_REFRESHES.retain_recent();
    OUTBOUND_REQUESTS.retain_recent();
    DEVICE_PROOFS.retain_recent();
    MEMBER_OP_REQUESTS.retain_recent();
    MEMBER_PROPOSALS.retain_recent();
    ROSTER_CHANGES.retain_recent();
    HANDOVER_REQUESTS.retain_recent();
    PRIMARY_ALERTS.retain_recent();
    JOIN_CREATIONS.retain_recent();
}

/// Cross-field checks of the limit settings; only called when Reins is enabled.
pub fn validate_settings(max_blob_bytes: u64, account_files: u32, max_downloads: u32) -> Result<(), String> {
    use reins_proto::blob::MAX_BLOB_BYTES;
    if !(1..=MAX_BLOB_BYTES).contains(&max_blob_bytes) {
        return Err(format!("`REINS_BLOB_MAX_BYTES` must be between 1 and {MAX_BLOB_BYTES} (1 GiB)"));
    }
    if account_files == 0 {
        return Err("`REINS_BLOB_ACCOUNT_FILES` must be at least 1".to_owned());
    }
    if max_downloads == 0 {
        return Err("`REINS_BLOB_MAX_DOWNLOADS` must be at least 1".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_allows_its_burst_then_says_when_to_retry() {
        let limit = RateLimit::<String>::with_period(60, 3);
        let key = "conn-1".to_owned();
        for _ in 0..3 {
            assert_eq!(limit.check(&key), Ok(()));
        }
        let wait = limit.check(&key).unwrap_err();
        assert!(wait > Duration::from_secs(50) && wait <= Duration::from_secs(60), "{wait:?}");
        assert_eq!(limit.check(&"conn-2".to_owned()), Ok(()), "keys are limited separately");
    }

    #[test]
    fn per_minute_allows_a_minute_at_once() {
        let limit = RateLimit::<String>::per_minute(5);
        let key = "u".to_owned();
        assert!((0..5).all(|_| limit.check(&key).is_ok()));
        let wait = limit.check(&key).unwrap_err();
        assert!(wait <= Duration::from_secs(12), "one more every 12 s: {wait:?}");
    }

    #[test]
    fn zero_disables_a_rate() {
        for limit in [RateLimit::<u8>::per_minute(0), RateLimit::with_period(0, 5), RateLimit::with_period(60, 0)] {
            assert!((0..1000).all(|_| limit.check(&1).is_ok()));
        }
    }

    #[test]
    fn concurrency_counts_holders_per_key_until_they_leave() {
        let limit = Concurrency::<String>::new(2);
        let key = "device".to_owned();
        let a = limit.try_enter(&key).unwrap();
        let b = limit.try_enter(&key).unwrap();
        assert!(limit.try_enter(&key).is_none());
        assert!(limit.try_enter(&"other".to_owned()).is_some(), "per key");
        drop(a);
        let c = limit.try_enter(&key).expect("a place was freed");
        assert_eq!(limit.active(&key), 2);
        drop((b, c));
        assert_eq!(limit.active(&key), 0);
        assert!(limit.active.lock().unwrap().is_empty(), "idle keys are forgotten");
        let unlimited = Concurrency::<u8>::new(0);
        let mut held = Vec::new();
        for _ in 0..100 {
            held.push(unlimited.try_enter(&1).expect("no limit"));
        }
        assert_eq!(unlimited.active(&1), 100);
    }

    #[test]
    fn failures_refuse_a_key_until_the_oldest_ages_out() {
        let limit = FailureLimit::<String>::new(3, Duration::from_secs(60));
        let (key, t0) = ("u1".to_owned(), Instant::now());
        assert_eq!(limit.check_at(&key, t0), Ok(()));
        limit.fail_at(&key, t0);
        limit.fail_at(&key, t0 + Duration::from_secs(10));
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(11)), Ok(()), "asking does not count");
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(11)), Ok(()));
        limit.fail_at(&key, t0 + Duration::from_secs(20));
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(30)), Err(Duration::from_secs(30)));
        assert_eq!(limit.check_at(&"u2".to_owned(), t0 + Duration::from_secs(30)), Ok(()), "per key");
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(60)), Ok(()), "the first failure aged out");
        limit.fail_at(&key, t0 + Duration::from_secs(61));
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(62)), Err(Duration::from_secs(8)));
        assert_eq!(limit.check_at(&key, t0 + Duration::from_secs(200)), Ok(()));
        assert!(limit.failures.lock().unwrap().is_empty(), "an idle key is forgotten");
        let off = FailureLimit::<u8>::new(0, Duration::from_secs(60));
        (0..100).for_each(|_| off.fail(&1));
        assert_eq!(off.check(&1), Ok(()));
    }

    #[test]
    fn retry_times_round_up_to_whole_seconds() {
        assert_eq!(retry_secs(Duration::ZERO), 1);
        assert_eq!(retry_secs(Duration::from_millis(1)), 1);
        assert_eq!(retry_secs(Duration::from_millis(1500)), 2);
        assert_eq!(retry_secs(Duration::from_secs(12)), 12);
        assert_eq!(
            rate_limited_text("too many requests on this connection", Duration::from_millis(2500)),
            "Rate limited: too many requests on this connection; try again in 3 s."
        );
    }

    #[test]
    fn settings_are_bounded() {
        assert_eq!(validate_settings(1 << 30, 20, 20), Ok(()));
        assert_eq!(validate_settings(1, 1, 1), Ok(()));
        assert!(validate_settings(0, 20, 20).unwrap_err().contains("REINS_BLOB_MAX_BYTES"));
        assert!(validate_settings((1 << 30) + 1, 20, 20).unwrap_err().contains("REINS_BLOB_MAX_BYTES"));
        assert!(validate_settings(1 << 30, 0, 20).unwrap_err().contains("REINS_BLOB_ACCOUNT_FILES"));
        assert!(validate_settings(1 << 30, 20, 0).unwrap_err().contains("REINS_BLOB_MAX_DOWNLOADS"));
    }

    #[test]
    fn a_proven_exemption_is_that_device_key_until_it_expires_or_is_forgotten() {
        let set = ProvenDevices::default();
        let t0 = Instant::now();
        set.remember_at("user", "key-a", "device-1", t0);
        assert!(set.is_proven_at("user", "key-a", t0), "the key that proved is exempt");
        assert!(!set.is_proven_at("user", "key-b", t0), "another key of the same device id is not");
        assert!(!set.is_proven_at("other", "key-a", t0), "another account is not");
        set.remember_at("user", "", "device-1", t0);
        assert_eq!(set.len(), 1, "a device with no key is not remembered");

        let just_before = (t0 + PROVEN_FOR).checked_sub(Duration::from_nanos(1)).unwrap();
        assert!(set.is_proven_at("user", "key-a", just_before));
        assert!(!set.is_proven_at("user", "key-a", t0 + PROVEN_FOR), "a day later it is gone");
        assert_eq!(set.len(), 0, "an expired entry is dropped on use");

        set.remember_at("user", "stale", "device-1", t0);
        set.remember_at("user", "fresh", "device-1", t0 + PROVEN_FOR);
        assert!(!set.is_proven_at("user", "stale", t0 + PROVEN_FOR), "insert prunes what has expired");
        assert!(set.is_proven_at("user", "fresh", t0 + PROVEN_FOR));

        set.remember_at("user", "other", "device-2", t0 + PROVEN_FOR);
        set.forget("user", "fresh");
        assert!(!set.is_proven_at("user", "fresh", t0 + PROVEN_FOR));
        assert!(set.is_proven_at("user", "other", t0 + PROVEN_FOR), "forgetting one key leaves the rest");
        set.forget_device("user", "device-2");
        assert!(!set.is_proven_at("user", "other", t0 + PROVEN_FOR), "removing the device drops what it proved");
        set.remember_at("user-b", "kept", "device-2", t0 + PROVEN_FOR);
        set.forget_device("user", "device-2");
        assert!(set.is_proven_at("user-b", "kept", t0 + PROVEN_FOR), "another account's device is left alone");
    }

    #[test]
    fn a_proof_past_eight_drops_only_that_accounts_oldest_key() {
        let set = ProvenDevices::default();
        let t0 = Instant::now();
        set.remember_at("victim", "kept", "phone", t0);
        for n in 0..PROVEN_PER_USER {
            set.remember_at(
                "attacker",
                &format!("k{n}"),
                "phone",
                t0 + Duration::from_secs(u64::try_from(n).unwrap() + 1),
            );
        }
        assert_eq!(set.len(), PROVEN_PER_USER + 1);
        let later = t0 + Duration::from_secs(100);
        set.remember_at("attacker", "k1", "phone", later);
        assert_eq!(set.len(), PROVEN_PER_USER + 1, "refreshing a key does not grow the set or drop another");
        assert!(set.is_proven_at("attacker", "k0", later));
        assert!(set.is_proven_at("victim", "kept", later));
        set.remember_at("attacker", "newest", "phone", later + Duration::from_secs(1));
        assert_eq!(set.len(), PROVEN_PER_USER + 1);
        assert!(set.is_proven_at("attacker", "newest", later + Duration::from_secs(1)));
        assert!(
            !set.is_proven_at("attacker", "k0", later + Duration::from_secs(1)),
            "the attacker's oldest key is the one dropped"
        );
        assert!(set.is_proven_at("attacker", "k1", later + Duration::from_secs(1)));
        assert!(set.is_proven_at("victim", "kept", later + Duration::from_secs(1)), "another account is left alone");
    }

    #[test]
    fn a_full_set_refuses_a_new_account_instead_of_evicting_one() {
        let set = ProvenDevices::default();
        let t0 = Instant::now();
        assert_eq!(PROVEN_CAP % PROVEN_PER_USER, 0, "the safety cap is a whole number of accounts");
        let users = PROVEN_CAP / PROVEN_PER_USER;
        for user in 0..users {
            for key in 0..PROVEN_PER_USER {
                let n = user * PROVEN_PER_USER + key;
                set.remember_at(
                    &format!("u{user}"),
                    &format!("k{key}"),
                    "phone",
                    t0 + Duration::from_secs(u64::try_from(n).unwrap()),
                );
            }
        }
        assert_eq!(set.len(), PROVEN_CAP);
        let later = t0 + Duration::from_secs(u64::try_from(PROVEN_CAP).unwrap() + 5);
        set.remember_at("newcomer", "k0", "phone", later);
        assert_eq!(set.len(), PROVEN_CAP);
        assert!(!set.is_proven_at("newcomer", "k0", later), "a full set refuses the new key");
        assert!(set.is_proven_at("u0", "k0", later), "the oldest account is not evicted to make room");
        set.remember_at("u0", "k0", "phone", later);
        assert!(set.is_proven_at("u0", "k0", later), "refreshing a key that is already remembered still works");
        assert_eq!(set.len(), PROVEN_CAP);
        // an account already at its own cap replaces its oldest key, which frees a place in the global set
        set.remember_at("u1", "extra", "phone", later + Duration::from_secs(1));
        assert!(set.is_proven_at("u1", "extra", later + Duration::from_secs(1)));
        assert!(!set.is_proven_at("u1", "k0", later + Duration::from_secs(1)), "only that account's oldest key");
        assert!(set.is_proven_at("u0", "k0", later + Duration::from_secs(1)));
        assert!(!set.is_proven_at("newcomer", "k0", later + Duration::from_secs(1)));
    }
}
