//! Rewarden desktop app. A small daemon that brings the user's connections to any AI harness on this computer. Its
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
pub mod git;
pub mod guard;
pub mod harden;
pub mod harness;
pub mod hooks;
pub mod http;
pub mod identity;
pub mod mcp_bridge;
pub mod notice;
pub mod phone;
pub mod proxy;
pub mod run;
pub mod secrets;
pub mod server;
pub mod service;
pub mod setup;
pub mod ssh_agent;
pub mod update;
pub mod win;

/// Unix seconds.
#[must_use]
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
