//! Reins desktop app. A small daemon that brings the user's connections to any AI harness on this computer. Its
//! first part is a git proxy: git talks plain HTTP to the daemon on loopback, and the daemon forwards to GitHub with a
//! credential the agent never sees, after the phone (or, without a phone, the local policy) allowed exactly that read
//! or that push.

pub mod agents_cli;
pub mod api_proxy;
pub mod ask;
pub mod auth;
pub mod config;
pub mod control;
pub mod daemon;
pub mod doctor;
pub mod git;
pub mod guard;
pub mod harden;
pub mod harness;
pub mod hooks;
pub mod http;
pub mod identity;
pub mod journal;
pub mod mcp_bridge;
pub mod mcp_payments;
pub mod notice;
pub mod notify;
pub mod phone;
pub mod proxy;
pub mod run;
pub mod secrets;
pub mod server;
pub mod service;
pub mod settings;
pub mod setup;
pub mod ssh_agent;
pub mod stats;
pub mod update;
pub mod vault_cli;
pub mod win;
pub mod work_session;

/// Unix seconds.
#[must_use]
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A duration in seconds, short and rounded down: `12 s`, `5 min`, `3 h`, `2 d`.
#[must_use]
pub fn ago(secs: i64) -> String {
    match secs.max(0) {
        s @ 0..60 => format!("{s} s"),
        s @ 60..3_600 => format!("{} min", s / 60),
        s @ 3_600..86_400 => format!("{} h", s / 3_600),
        s => format!("{} d", s / 86_400),
    }
}
