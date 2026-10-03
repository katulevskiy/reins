//! The Rewarden server: logging in (OAuth with the phone's approval, the app's key pinned on the phone) and the desktop
//! API git access requests go through.

pub mod client;
pub mod device;
pub mod oauth;

use crypto_box::aead::OsRng;
use crypto_box::aead::rand_core::RngCore as _;
use data_encoding::BASE64URL_NOPAD;

/// Why talking to the Rewarden server did not work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// No session, or the server ended it (the refresh token was refused): `rewarden login` again.
    LoggedOut(String),
    /// The server does not know the request (unknown, expired, or another connection's).
    NotFound,
    /// The server could not be reached or answered something unexpected; may work later.
    Failed(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoggedOut(m) | Self::Failed(m) => f.write_str(m),
            Self::NotFound => f.write_str("the Rewarden server does not know this request"),
        }
    }
}

/// The base URL of a Rewarden server as the user typed it: https, or http only on this computer (localhost,
/// 127.0.0.1, [::1]); no credentials, query or fragment. A bare host name means https. No trailing slash.
pub fn server_base(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let url = match url::Url::parse(raw) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            url::Url::parse(&format!("https://{raw}")).map_err(|e| format!("`{raw}` is not a server address: {e}"))?
        }
        Err(e) => return Err(format!("`{raw}` is not a server address: {e}")),
    };
    check_url(&url).map_err(|e| format!("`{raw}`: {e}"))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("`{raw}`: a server address has no query or fragment"));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// https, or plain http on a loopback name only (tests, a server on this computer); never credentials in the URL.
pub(crate) fn check_url(url: &url::Url) -> Result<(), String> {
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("must be https (http only for localhost)".to_owned());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("has no host".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("must not contain a user name or password".to_owned());
    }
    Ok(())
}

/// `bytes` random bytes, base64url without padding (nonces, PKCE verifiers, OAuth state).
pub(crate) fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    BASE64URL_NOPAD.encode(&buf)
}

/// Reads a response body, refusing more than `limit` bytes (the server is not trusted with our memory).
pub(crate) async fn read_limited(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err("the server's answer is too large".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.without_url().to_string())? {
        if body.len() + chunk.len() > limit {
            return Err("the server's answer is too large".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `{"error": …, "error_description"/"message": …}` from an error answer, for messages.
pub(crate) fn error_text(status: reqwest::StatusCode, body: &[u8]) -> String {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let code = v.get("error").and_then(serde_json::Value::as_str);
    let detail = v.get("error_description").or_else(|| v.get("message")).and_then(serde_json::Value::as_str);
    match (code, detail) {
        (Some(c), Some(d)) => format!("{c}: {d}"),
        (Some(c), None) => c.to_owned(),
        (None, Some(d)) => d.to_owned(),
        (None, None) => format!("HTTP {}", status.as_u16()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_https_or_local_http() {
        assert_eq!(server_base("https://rw.example.com/").unwrap(), "https://rw.example.com");
        assert_eq!(server_base("rw.example.com").unwrap(), "https://rw.example.com");
        assert_eq!(server_base("https://example.com/rewarden/").unwrap(), "https://example.com/rewarden");
        assert_eq!(server_base("http://127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(server_base("http://localhost:8080/").unwrap(), "http://localhost:8080");
        assert_eq!(server_base("http://[::1]:8080").unwrap(), "http://[::1]:8080");
        for bad in [
            "http://rw.example.com",
            "http://127.0.0.2",
            "ftp://rw.example.com",
            "https://me:pw@rw.example.com",
            "https://rw.example.com/?x=1",
            "https://rw.example.com/#x",
            "",
        ] {
            assert!(server_base(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn random_tokens_are_base64url_and_differ() {
        let a = random_token(16);
        assert_eq!(a.len(), 22);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(a, random_token(16));
        assert_eq!(random_token(32).len(), 43);
    }

    #[test]
    fn error_answers_become_short_messages() {
        let s = reqwest::StatusCode::BAD_REQUEST;
        assert_eq!(error_text(s, br#"{"error":"invalid_grant","error_description":"gone"}"#), "invalid_grant: gone");
        assert_eq!(
            error_text(s, br#"{"error":"invalid_arguments","message":"bad repo"}"#),
            "invalid_arguments: bad repo"
        );
        assert_eq!(error_text(s, b"<html>"), "HTTP 400");
    }
}
