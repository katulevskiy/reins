//! The Reins organization executor (`docs/design/decisions-v2/06-trust-and-compliance.md` §3,
//! `08-review-resolutions.md` R5): a separate program and trust domain that holds an org's managed credentials and acts
//! only on decisions it has checked itself.
//!
//! Before it acts on a job it verifies ([`verify::verify_job`]):
//! - the decision service's countersignature, with a pinned key;
//! - the issuer-signed policy snapshot and roster, with a pinned key;
//! - that the request it would run is exactly the countersigned digest;
//! - the reviewers' device signatures;
//! - its own re-evaluation of the request with the same engine ([`reins_policy::org::evaluate_managed`]).

pub mod config;
pub mod verify;

#[cfg(test)]
pub(crate) mod testkit;

/// Unix seconds now.
#[must_use]
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
