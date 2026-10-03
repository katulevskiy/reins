//! The daemon's control API under `/_reins/`, used by the CLI: `GET status`, `GET pending`,
//! `POST pending/<id>/approve`, `POST pending/<id>/deny`, and `POST shutdown` (how the Windows background service is
//! stopped: there is no service manager there to send it a signal). Every call needs `X-Reins-Token` with the secret the
//! daemon wrote to `control.token` (0600), so only the user (and what runs as the user) can approve.

use std::sync::Arc;
use std::time::Duration;

use hyper::{HeaderMap, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::prompt::{Pending, PendingItem};
use crate::config::Paths;
use crate::proxy::{Body, full, text};

pub const TOKEN_HEADER: &str = "x-reins-token";
pub const PREFIX: &str = "/_reins/";

/// `GET /_reins/status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub mode: String,
    /// Who decides ("the local policy on this computer", "your phone, through …").
    pub decides: String,
    pub listen: String,
    /// What `reins git setup` points git at.
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
    stop: Arc<tokio::sync::Notify>,
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
            stop: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Notified when `POST shutdown` asks the daemon to stop.
    #[must_use]
    pub fn stop_handle(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.stop)
    }

    #[must_use]
    pub fn handle(&self, method: &Method, path: &str, headers: &HeaderMap) -> Response<Body> {
        let given = headers.get(TOKEN_HEADER).map_or(&b""[..], |v| v.as_bytes());
        if !same(given, self.token.as_bytes()) {
            return text(StatusCode::UNAUTHORIZED, "The control API needs the X-Reins-Token header.");
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
            (&Method::POST, ["shutdown"]) => {
                log::info!("asked to stop from the command line");
                self.stop.notify_one();
                json(&serde_json::json!({ "ok": true }))
            }
            _ => text(StatusCode::NOT_FOUND, "Unknown control path."),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Reins daemon is not running (start it with `reins daemon` or `reins service install`)")]
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

    /// Asks the daemon to stop and waits up to `wait` until it no longer answers. `Ok(false)`: it was not running.
    pub async fn shutdown(&self, wait: Duration) -> Result<bool, ClientError> {
        match self.call::<serde_json::Value>(Method::POST, "shutdown").await {
            Err(ClientError::NotRunning) => return Ok(false),
            Err(ClientError::Other(e)) if e.contains("Unknown control path") => {
                return Err(ClientError::Other("the running daemon is too old to be stopped this way".to_owned()));
            }
            // Anything else may be the daemon going away while it answers.
            Ok(_) | Err(ClientError::Other(_)) => {}
        }
        let deadline = std::time::Instant::now() + wait;
        while self.status().await.is_ok() {
            if std::time::Instant::now() > deadline {
                return Err(ClientError::Other("the daemon did not stop".to_owned()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(true)
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

    #[tokio::test]
    async fn shutdown_needs_the_token_and_tells_the_daemon() {
        let control = Control::new(
            Zeroizing::new("t".to_owned()),
            Arc::new(Pending::default()),
            Box::new(|| unreachable!("the status is not asked for")),
        );
        let stop = control.stop_handle();
        let mut headers = HeaderMap::new();
        assert_eq!(control.handle(&Method::POST, "/_reins/shutdown", &headers).status(), StatusCode::UNAUTHORIZED);
        headers.insert(TOKEN_HEADER, "t".parse().unwrap());
        assert_eq!(control.handle(&Method::GET, "/_reins/shutdown", &headers).status(), StatusCode::NOT_FOUND);
        assert_eq!(control.handle(&Method::POST, "/_reins/shutdown", &headers).status(), StatusCode::OK);
        tokio::time::timeout(Duration::from_secs(1), stop.notified()).await.expect("the daemon is told to stop");
    }
}
