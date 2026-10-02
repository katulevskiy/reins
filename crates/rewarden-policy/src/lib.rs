//! Rewarden grant model and policy engine.
//!
//! Decides which messages an AI connection may receive and which emails it may
//! send, from grants the user created on their phone. Pure and IO-free.
//! Every check fails closed: malformed or unknown data never grants access.

mod evaluate;
mod grant;
mod pattern;
mod scope;

use rewarden_proto::ValidationError;
use thiserror::Error;

pub use evaluate::{
    AccountCoverage, ReadDecision, SendDecision, account_coverage, evaluate_read, evaluate_send, needs_body,
    service_allows,
};
pub use grant::{Grant, MAX_ANY_MAIL_SECS, record_uses};
pub use pattern::{AddrRule, MAX_PATTERN_LEN, Pattern};
pub use scope::{
    AccountsScope, MAX_ANY_RESOURCE_SECS, MessageFacts, ReadScope, Scope, SendScope, ServiceScope, resource_covers,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("invalid pattern {pattern:?}: {reason}")]
    InvalidPattern {
        pattern: String,
        reason: String,
    },
    #[error("invalid address: {0}")]
    InvalidAddress(#[from] ValidationError),
    #[error("invalid scope: {0}")]
    InvalidScope(String),
    #[error("invalid grant: {0}")]
    InvalidGrant(String),
}
