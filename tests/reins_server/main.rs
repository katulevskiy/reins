//! HTTP tests against the real `vaultwarden` binary with Rewarden enabled
//! (plan Decision 27). Each test starts its own server on a free port with a
//! temporary SQLite database.

mod blobs;
mod desktop;
mod harness;
mod limits;
mod mcp;
mod mock;
mod oauth;
mod phone_api;
mod remote_mcp;
mod takeover;
