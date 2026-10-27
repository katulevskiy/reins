//! Who is on call now (Task 5).

use super::Resolution;
use crate::db::DbPool;

/// Who is on call for `alias` at `now`.
pub async fn current(_pool: &DbPool, _org: &str, alias: &str, _now: i64) -> Resolution {
    Resolution::nobody("oncall", &format!("on-call: {alias} unavailable"))
}

/// An on-call alias: 1-64 of `a-z`, `0-9`, `.`, `_`, `-`.
#[must_use]
pub fn alias_ok(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 64
        && a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A provider's schedule id: 1-128 of letters, digits, `-` and `_`.
#[must_use]
pub fn schedule_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
