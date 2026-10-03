//! Pairing this computer with the phone.
//!
//! The main way is a device flow: the app asks the server for a short code and a link, shows the link as a QR code,
//! and the phone (its camera, or the Reins app's scanner) opens it; the user approves on the phone and the app's
//! polling sees it. [`DeviceFlow`] is the app's side of it. [`ServerFlow`] goes to the server through the
//! `reins_desktop` library (`server::device`, behind the `device-flow` feature until that module is on this
//! branch), which keeps the session exactly like `reins login` does; [`DemoFlow`] pretends, for `--demo`.
//!
//! The other way is the browser sign-in `reins login` uses (`Backend::sign_in_with_browser`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::backend::Backend;

/// A started pairing: what the app shows while the phone has not answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCode {
    /// What the QR code holds (the server's `verification_uri_complete`).
    pub qr_url: String,
    /// The short code, for typing it into the phone when scanning is not possible ("WDJB-MJHT").
    pub user_code: String,
    /// The number the phone offers among three: the user taps this one.
    pub confirm_code: Option<u8>,
    pub expires_at: Instant,
    /// How often to ask whether the phone answered.
    pub interval: Duration,
}

/// The phone approved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approved {
    pub server: String,
    /// The phone's name ("Daniel's iPhone"), when the flow says.
    pub phone: Option<String>,
    /// The account, when the flow says.
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Waiting,
    Approved(Approved),
    /// The pairing is over without a session (denied on the phone, the code ran out): why.
    #[cfg_attr(not(feature = "device-flow"), allow(dead_code))]
    Ended(String),
}

#[async_trait]
pub trait DeviceFlow: Send + Sync {
    /// Asks the server for a code.
    async fn start(&self) -> Result<DeviceCode, String>;
    /// Whether the phone answered. Saves the session when it approved.
    async fn poll(&self) -> Result<Poll, String>;
}

/// Shown when the server (or this build) cannot pair by QR code; the onboarding screen then leads with the browser
/// sign-in, which works with every server.
pub const UNAVAILABLE: &str = "This server cannot pair by QR code. Sign in with the browser instead.";

/// The real pairing: `reins_desktop::server::device`, the same device flow `reins login` uses.
#[cfg_attr(not(feature = "device-flow"), allow(dead_code))]
pub struct ServerFlow {
    pub backend: Arc<Backend>,
    pub server: String,
    #[cfg(feature = "device-flow")]
    pairing: tokio::sync::Mutex<Option<reins_desktop::server::device::DevicePairing>>,
}

impl ServerFlow {
    pub fn new(backend: Arc<Backend>, server: String) -> Self {
        Self {
            backend,
            server,
            #[cfg(feature = "device-flow")]
            pairing: tokio::sync::Mutex::new(None),
        }
    }
}

#[cfg(feature = "device-flow")]
#[async_trait]
impl DeviceFlow for ServerFlow {
    async fn start(&self) -> Result<DeviceCode, String> {
        use reins_desktop::server::device::{DevicePairing, StartError};
        let identity = self.backend.identity()?;
        let pairing = DevicePairing::start(&identity, &self.server).await.map_err(|e| match e {
            StartError::Unsupported => UNAVAILABLE.to_owned(),
            StartError::Failed(m) => m,
        })?;
        let left = u64::try_from(pairing.expires_at - reins_desktop::now_unix()).unwrap_or(0);
        let code = DeviceCode {
            qr_url: pairing.qr_url.clone(),
            user_code: pairing.user_code.clone(),
            confirm_code: pairing.confirm_code,
            expires_at: Instant::now() + Duration::from_secs(left),
            interval: pairing.interval(),
        };
        *self.pairing.lock().await = Some(pairing);
        Ok(code)
    }

    async fn poll(&self) -> Result<Poll, String> {
        use reins_desktop::server::device::DeviceStatus;
        let mut guard = self.pairing.lock().await;
        let pairing = guard.as_mut().ok_or("no pairing was started")?;
        match pairing.poll(self.backend.paths()).await {
            Ok(DeviceStatus::Waiting) => Ok(Poll::Waiting),
            Ok(DeviceStatus::LoggedIn {
                server,
            }) => {
                *guard = None;
                Ok(Poll::Approved(Approved {
                    server,
                    phone: None,
                    account: None,
                }))
            }
            Err(e) => {
                *guard = None;
                Ok(Poll::Ended(if e.contains("denied") {
                    "Not approved on your phone.".to_owned()
                } else {
                    e
                }))
            }
        }
    }
}

/// Without the library's device flow (`--features device-flow`), the app leads with the browser sign-in.
#[cfg(not(feature = "device-flow"))]
#[async_trait]
impl DeviceFlow for ServerFlow {
    async fn start(&self) -> Result<DeviceCode, String> {
        Err(UNAVAILABLE.to_owned())
    }

    async fn poll(&self) -> Result<Poll, String> {
        Err(UNAVAILABLE.to_owned())
    }
}

/// `--demo`: a made-up code that "the phone" approves after a few seconds. Nothing leaves the computer.
pub struct DemoFlow {
    pub approve_after: Duration,
    pub started: std::sync::Mutex<Option<Instant>>,
}

#[async_trait]
impl DeviceFlow for DemoFlow {
    async fn start(&self) -> Result<DeviceCode, String> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Ok(mut started) = self.started.lock() {
            *started = Some(Instant::now());
        }
        Ok(DeviceCode {
            qr_url: format!("{}/pair?code=WDJB-MJHT", crate::links::SERVER),
            user_code: "WDJB-MJHT".to_owned(),
            confirm_code: Some(37),
            expires_at: Instant::now() + Duration::from_mins(10),
            interval: Duration::from_secs(1),
        })
    }

    async fn poll(&self) -> Result<Poll, String> {
        let started = self.started.lock().ok().and_then(|s| *s).unwrap_or_else(Instant::now);
        if started.elapsed() < self.approve_after {
            return Ok(Poll::Waiting);
        }
        Ok(Poll::Approved(Approved {
            server: crate::links::SERVER.to_owned(),
            phone: Some("Daniel's iPhone".to_owned()),
            account: Some("daniel@example.com".to_owned()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_demo_flow_approves_after_its_delay() {
        let flow = DemoFlow {
            approve_after: Duration::from_millis(50),
            started: std::sync::Mutex::new(None),
        };
        let code = flow.start().await.unwrap();
        assert_eq!(code.user_code, "WDJB-MJHT");
        assert!(code.qr_url.starts_with("https://"));
        assert_eq!(flow.poll().await.unwrap(), Poll::Waiting);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let Poll::Approved(approved) = flow.poll().await.unwrap() else {
            panic!("not approved")
        };
        assert_eq!(approved.phone.as_deref(), Some("Daniel's iPhone"));
    }
}
