//! End-to-end harness: the real `vaultwarden` binary (Rewarden enabled), the real
//! phone core, and a simulated AI client. Only Gmail is faked (wiremock).

#![allow(
    clippy::must_use_candidate,
    clippy::assigning_clones,
    clippy::map_unwrap_or,
    reason = "test-support crate: these pedantic lints add noise, not safety"
)]

pub mod ai;
pub mod phone;
pub mod server;
pub mod workos;

pub use ai::AiClient;
pub use phone::Phone;
pub use server::Server;

/// Master password used by every test account.
pub const PASSWORD: &str = "correct horse battery staple";

/// reqwest is built without a bundled TLS provider workspace-wide; install one once.
pub fn init_tls() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Another test thread may win the race; either way a provider is installed.
        rustls::crypto::ring::default_provider().install_default().ok();
    }
}
