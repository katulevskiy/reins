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
//!
//! Then it runs exactly that request through the org's GitHub App or Google Workspace ([`connectors`]), or mints a
//! short-lived credential sealed to the requester ([`seal`], [`leases`]), and posts a receipt signed with its own key
//! ([`run`]). The server is reached through [`client::Client`] (the executor HTTP contract).

pub mod client;
pub mod config;
pub mod connectors;
pub mod leases;
pub mod run;
pub mod seal;
pub mod verify;

pub use client::Client;
pub use config::{ExecutorConfig, ExecutorMode, Pins};
pub use run::{Context, RunStats, run, run_once};
pub use seal::{Opener, seal};
pub use verify::{HeldStep, Refusal, Verified, verify_job};

#[cfg(test)]
pub(crate) mod testkit;

/// Unix seconds now.
#[must_use]
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
