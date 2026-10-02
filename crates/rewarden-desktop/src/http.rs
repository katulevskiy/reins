//! The HTTP client for GitHub and the Rewarden server: rustls with ring and the webpki roots, no redirects (a token is
//! never sent to a place it was not meant for; callers follow same-host redirects themselves when they need to).

use std::sync::Arc;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("rewarden-desktop/", env!("CARGO_PKG_VERSION"));

/// `timeout`: whole-request limit; `None` for streams of unknown length (git packs).
pub fn client(timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut builder = reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .user_agent(USER_AGENT);
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    builder.build().map_err(|e| format!("HTTP client setup: {}", e.without_url()))
}
