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
//!   phone, wrong proofs sent to take its approval role;
//! - an approval device: concurrent `GET /pending` long-polls;
//! - an IP address: OAuth dynamic client registrations, device authorizations (QR codes to pair a computer).

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

/// Outbound requests (file sends and fetches, proxied MCP calls) made for one account (key: user id).
pub static OUTBOUND_REQUESTS: LazyLock<RateLimit<String>> =
    LazyLock::new(|| RateLimit::per_minute(CONFIG.reins_outbound_requests_per_minute()));

/// Outbound requests of one account running at once (key: user id).
pub static OUTBOUND_RUNNING: LazyLock<Concurrency<String>> =
    LazyLock::new(|| Concurrency::new(CONFIG.reins_outbound_max_concurrent()));

/// Housekeeping: drops rate-limit state of keys that are back at a full budget.
pub fn retain_recent() {
    CONNECTION_REQUESTS.retain_recent();
    ACCOUNT_CALLS.retain_recent();
    PAIRINGS.retain_recent();
    DEVICE_AUTHORIZATIONS.retain_recent();
    REGISTRATIONS.retain_recent();
    OUTBOUND_REQUESTS.retain_recent();
    DEVICE_PROOFS.retain_recent();
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
}
