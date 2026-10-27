//! Reins grant model and policy engine.
//!
//! Decides which messages an AI connection may receive and which emails it may
//! send, from grants the user created on their phone. Pure and IO-free.
//! Every check fails closed: malformed or unknown data never grants access.

pub mod authorization;
pub mod context;
pub mod effects;
mod evaluate;
mod grant;
pub mod guard;
pub mod matrix;
mod pattern;
mod scope;
pub mod shell;
pub mod strict;

use reins_proto::ValidationError;
use thiserror::Error;

pub use evaluate::{
    AccountCoverage, ReadDecision, SendDecision, account_coverage, evaluate_read, evaluate_send, headlines_allows,
    needs_body, secrets_allows, service_allows,
};
pub use grant::{Grant, MAX_ANY_MAIL_SECS, record_uses};
pub use pattern::{AddrRule, MAX_PATTERN_LEN, Pattern};
pub use scope::{
    AccountsScope, HeadlinesScope, MAX_ANY_RESOURCE_SECS, MessageFacts, ReadScope, Scope, SecretsFacts, SecretsScope,
    SendScope, ServiceScope, resource_covers,
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
