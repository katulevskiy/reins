//! Shared HTTP plumbing: the reqwest client (rustls + ring + webpki roots),
//! server-URL validation and error text that never carries URLs or secrets.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::CoreError;

/// Hosts that may be reached over plain HTTP (local development and the
/// Android emulator's alias for the host machine).
const PLAIN_HTTP_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "10.0.2.2", "[::1]"];
const MAX_ERROR_TEXT: usize = 200;

/// A validated Reins server base URL, without a trailing slash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerUrl(String);

impl ServerUrl {
    /// Accepts `https://host[:port][/path]`, or `http://` for local hosts only.
    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        let bad = |why: &str| CoreError::invalid(format!("server URL {why}"));
        let url = Url::parse(raw.trim()).map_err(|_| bad("is not a valid URL"))?;
        let host = url.host_str().ok_or_else(|| bad("has no host"))?.to_owned();
        match url.scheme() {
            "https" => {}
            "http" if PLAIN_HTTP_HOSTS.contains(&host.as_str()) => {}
            "http" => return Err(bad("must use https://")),
            _ => return Err(bad("must start with https://")),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(bad("must not contain credentials"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(bad("must not contain ? or #"));
        }
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        let path = url.path().trim_end_matches('/');
        Ok(Self(format!("{}://{host}{port}{path}", url.scheme())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `path` must start with `/`.
    pub fn join(&self, path: &str) -> String {
        format!("{}{path}", self.0)
    }
}

/// The client used for every request. Redirects are never followed, so a
/// bearer token can never be forwarded to another location.
pub fn client() -> Result<reqwest::Client, CoreError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| CoreError::storage(format!("TLS setup: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("reins-core/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| CoreError::storage(format!("HTTP client setup: {}", e.without_url())))
}

/// Transport errors become `Network`, described without the URL (queries may
/// hold user data) and without any header or body content.
impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        network_error(&e)
    }
}

fn network_error(e: &reqwest::Error) -> CoreError {
    let kind = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "could not connect"
    } else if e.is_decode() || e.is_body() {
        "invalid response"
    } else {
        "request failed"
    };
    CoreError::Network {
        reason: kind.to_owned(),
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// `(error code, human message)` from a Vaultwarden or Reins error body.
pub fn error_text(status: reqwest::StatusCode, body: &str) -> (String, String) {
    let parsed: Option<ErrorBody> = serde_json::from_str(body).ok();
    let code = parsed.as_ref().and_then(|b| b.error.clone()).unwrap_or_default();
    let message = parsed
        .and_then(|b| b.message.filter(|m| !m.is_empty()).or(b.error_description.filter(|m| !m.is_empty())))
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
    (code, truncate(&message, MAX_ERROR_TEXT))
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_rules() {
        for (raw, want) in [
            ("https://rw.example.com", "https://rw.example.com"),
            (" https://RW.Example.com/ ", "https://rw.example.com"),
            ("https://example.com/vault/", "https://example.com/vault"),
            ("https://example.com:8443", "https://example.com:8443"),
            ("http://localhost:8000", "http://localhost:8000"),
            ("http://127.0.0.1:8000/", "http://127.0.0.1:8000"),
            ("http://10.0.2.2:8000", "http://10.0.2.2:8000"),
            ("http://[::1]:8000", "http://[::1]:8000"),
        ] {
            assert_eq!(ServerUrl::parse(raw).unwrap().as_str(), want, "{raw}");
        }
        for bad in [
            "http://rw.example.com",
            "http://192.168.1.2:8000",
            "http://localhost.evil.com",
            "ftp://example.com",
            "https://user:pw@example.com",
            "https://example.com/?x=1",
            "https://example.com/#f",
            "example.com",
            "",
        ] {
            assert!(ServerUrl::parse(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(ServerUrl::parse("https://e.com/v").unwrap().join("/identity/x"), "https://e.com/v/identity/x");
    }

    #[test]
    fn error_text_prefers_message() {
        let s = reqwest::StatusCode::BAD_REQUEST;
        assert_eq!(error_text(s, r#"{"error":"","message":"Nope"}"#), (String::new(), "Nope".to_owned()));
        assert_eq!(
            error_text(s, r#"{"error":"wrong_code","message":"Wrong code"}"#),
            ("wrong_code".to_owned(), "Wrong code".to_owned())
        );
        assert_eq!(error_text(s, r#"{"error":"invalid_grant","error_description":"x"}"#).1, "x");
        assert_eq!(error_text(s, "<html>").1, "Bad Request");
        assert_eq!(error_text(s, &format!(r#"{{"message":"{}"}}"#, "a".repeat(500))).1.chars().count(), 201);
    }

    #[test]
    fn client_builds() {
        client().unwrap();
    }
}
