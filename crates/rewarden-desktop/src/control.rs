//! The daemon's control API under `/_rewarden/`, used by the CLI: `GET status`, `GET pending`,
//! `POST pending/<id>/approve`, `POST pending/<id>/deny`. Every call needs `X-Rewarden-Token` with the secret the
//! daemon wrote to `control.token` (0600), so only the user (and what runs as the user) can approve.

use std::sync::Arc;
use std::time::Duration;

use hyper::{HeaderMap, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::prompt::{Pending, PendingItem};
use crate::config::Paths;
use crate::proxy::{Body, full, text};

pub const TOKEN_HEADER: &str = "x-rewarden-token";
pub const PREFIX: &str = "/_rewarden/";

/// `GET /_rewarden/status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub mode: String,
    /// Who decides ("the local policy on this computer", "your phone, through …").
    pub decides: String,
    pub listen: String,
    /// What `rewarden git setup` points git at.
    pub proxy_base: String,
    pub server: Option<String>,
    pub fingerprint: String,
    pub pending: usize,
}

/// The daemon side.
pub struct Control {
    token: Zeroizing<String>,
    pending: Arc<Pending>,
    status: Box<dyn Fn() -> Status + Send + Sync>,
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn json<T: Serialize>(value: &T) -> Response<Body> {
    match serde_json::to_vec(value) {
        Ok(bytes) => full(StatusCode::OK, "application/json", bytes),
        Err(e) => text(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// A new random control token (64 hex digits).
#[must_use]
pub fn new_token() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    crypto_box::aead::rand_core::RngCore::fill_bytes(&mut crypto_box::aead::OsRng, bytes.as_mut());
    Zeroizing::new(data_encoding::HEXLOWER.encode(bytes.as_ref()))
}

impl Control {
    pub fn new(token: Zeroizing<String>, pending: Arc<Pending>, status: Box<dyn Fn() -> Status + Send + Sync>) -> Self {
        Self {
            token,
            pending,
            status,
        }
    }

    #[must_use]
    pub fn handle(&self, method: &Method, path: &str, headers: &HeaderMap) -> Response<Body> {
        let given = headers.get(TOKEN_HEADER).map_or(&b""[..], |v| v.as_bytes());
        if !same(given, self.token.as_bytes()) {
            return text(StatusCode::UNAUTHORIZED, "The control API needs the X-Rewarden-Token header.");
        }
        let rest = path.strip_prefix(PREFIX).unwrap_or("");
        let parts: Vec<&str> = rest.split('/').collect();
        match (method, parts.as_slice()) {
            (&Method::GET, ["status"]) => json(&(self.status)()),
            (&Method::GET, ["pending"]) => json(&self.pending.list()),
            (&Method::POST, ["pending", id, action @ ("approve" | "deny")]) => {
                if self.pending.answer(id, *action == "approve") {
                    log::info!("pending {id}: {action} from the command line");
                    json(&serde_json::json!({ "ok": true }))
                } else {
                    text(StatusCode::NOT_FOUND, &format!("No pending approval {id}."))
                }
            }
            _ => text(StatusCode::NOT_FOUND, "Unknown control path."),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Rewarden daemon is not running (start it with `rewarden daemon` or `rewarden service install`)")]
    NotRunning,
    #[error("{0}")]
    Other(String),
}

/// The CLI side.
pub struct Client {
    base: String,
    token: Zeroizing<String>,
    http: reqwest::Client,
}

impl Client {
    /// For the daemon listening on `listen`, with the token it wrote under `paths`.
    pub fn new(paths: &Paths, listen: std::net::SocketAddr) -> Result<Self, ClientError> {
        let token = match std::fs::read_to_string(paths.control_token_file()) {
            Ok(t) => Zeroizing::new(t.trim().to_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ClientError::NotRunning),
            Err(e) => return Err(ClientError::Other(format!("{}: {e}", paths.control_token_file().display()))),
        };
        Ok(Self {
            base: format!("http://{listen}{PREFIX}"),
            token,
            http: crate::http::client(Some(Duration::from_secs(10))).map_err(ClientError::Other)?,
        })
    }

    async fn call<T: for<'de> Deserialize<'de>>(&self, method: Method, path: &str) -> Result<T, ClientError> {
        let resp = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header(TOKEN_HEADER, self.token.as_str())
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::NotRunning
                } else {
                    ClientError::Other(e.without_url().to_string())
                }
            })?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| ClientError::Other(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(ClientError::Other(String::from_utf8_lossy(&bytes).trim().to_owned()));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| ClientError::Other(format!("unexpected answer from the daemon: {e}")))
    }

    pub async fn status(&self) -> Result<Status, ClientError> {
        self.call(Method::GET, "status").await
    }

    pub async fn pending(&self) -> Result<Vec<PendingItem>, ClientError> {
        self.call(Method::GET, "pending").await
    }

    pub async fn answer(&self, id: &str, approve: bool) -> Result<(), ClientError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ClientError::Other(format!("`{id}` is not a pending approval id")));
        }
        let action = if approve {
            "approve"
        } else {
            "deny"
        };
        self.call::<serde_json::Value>(Method::POST, &format!("pending/{id}/{action}")).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
        assert_eq!(new_token().len(), 64);
        assert_ne!(*new_token(), *new_token());
    }
}
