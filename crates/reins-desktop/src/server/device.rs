//! `reins login` without a browser: OAuth 2.0 device authorization (RFC 8628). This computer shows a QR code; the
//! phone scans it, the user taps the number shown here among three, compares this app's key and approves. The phone
//! pins the key to the new connection exactly as after the browser login, and the session is saved the same way
//! (`session.json`), so everything after the login is the same.
//!
//! The desktop app's window uses the same API: [`DevicePairing::start`], draw [`DevicePairing::qr_url`] (for example
//! with [`qr_modules`]), then [`DevicePairing::wait`] (or [`DevicePairing::poll`] on its own timer).

use std::time::Duration;

use serde::Deserialize;
use zeroize::Zeroizing;

use super::oauth::{DEVICE_CODE_GRANT, MAX_ANSWER_BYTES, Metadata, Session, discover, register, save_session};
use super::{check_url, error_text, read_limited, server_base};
use crate::config::Paths;
use crate::identity::Identity;

/// The interval when the server names none (RFC 8628 §3.2), and what each `slow_down` adds (§3.5).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// Polls are never closer than this, whatever the server says.
const MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Codes are never waited on longer than this, whatever the server says.
const MAX_EXPIRES_SECS: i64 = 30 * 60;

/// Why a QR code could not be had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The server does not offer the device flow (an older server): log in with the browser instead.
    Unsupported,
    Failed(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("this server cannot pair by QR code; use `reins login --browser`"),
            Self::Failed(m) => f.write_str(m),
        }
    }
}

/// Where a pairing stands after a poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    /// Nobody has approved yet: poll again after [`DevicePairing::interval`].
    Waiting,
    /// Approved on the phone; the session is saved.
    LoggedIn {
        server: String,
    },
}

/// A login waiting for the phone: what to show, and how to finish it.
pub struct DevicePairing {
    /// The server's base URL.
    pub server: String,
    /// What the QR code holds: the server's pairing page with the code (`https://app.reins2fa.com/pair?code=BCDF-GHJK`).
    /// The phone's camera opens it in the Reins app; the app's own scanner reads the code from it.
    pub qr_url: String,
    /// The code itself (`BCDF-GHJK`), for typing it into the app when scanning is not possible.
    pub user_code: String,
    /// The server's pairing page without the code.
    pub verification_uri: String,
    /// The number the phone offers among three: the user taps this one. `None` from a server that does not send it.
    pub confirm_code: Option<u8>,
    /// This app's key fingerprint ("4821 9930"), which the phone shows too.
    pub key_fingerprint: String,
    /// Unix seconds after which the code no longer works.
    pub expires_at: i64,
    client_id: String,
    token_endpoint: String,
    device_code: Zeroizing<String>,
    interval: Duration,
    http: reqwest::Client,
}

impl std::fmt::Debug for DevicePairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DevicePairing")
            .field("server", &self.server)
            .field("user_code", &self.user_code)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Authorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    reins_confirm_code: Option<u8>,
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// `https` (or loopback `http`) without credentials, else `None`: what the server says to show must be a plain link.
fn plain_link(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    check_url(&url).ok()?;
    Some(url.into())
}

impl DevicePairing {
    /// Registers this app with the server at `server_url` (as the browser login does) and asks for a code to show.
    pub async fn start(identity: &Identity, server_url: &str) -> Result<Self, StartError> {
        let server = server_base(server_url).map_err(StartError::Failed)?;
        let http = crate::http::client(Some(Duration::from_secs(30))).map_err(StartError::Failed)?;
        let meta: Metadata = discover(&http, &server).await.map_err(StartError::Failed)?;
        let endpoint = meta.device_authorization_endpoint.clone().ok_or(StartError::Unsupported)?;
        let client_id = register(&http, &meta).await.map_err(StartError::Failed)?;
        let resource = format!("{server}/mcp");
        let key = identity.public_key();
        let form = [
            ("client_id", client_id.as_str()),
            ("scope", "mcp"),
            ("resource", resource.as_str()),
            ("reins_client_key", key.as_str()),
        ];
        let resp = http
            .post(&endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| StartError::Failed(format!("cannot reach the server: {}", e.without_url())))?;
        let status = resp.status();
        let body = read_limited(resp, MAX_ANSWER_BYTES).await.map_err(StartError::Failed)?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(StartError::Unsupported);
        }
        if !status.is_success() {
            return Err(StartError::Failed(format!("the server did not give a code: {}", error_text(status, &body))));
        }
        let a: Authorization = serde_json::from_slice(&body)
            .map_err(|e| StartError::Failed(format!("unexpected answer from the server: {e}")))?;
        let page = url::Url::parse(&a.verification_uri)
            .ok()
            .filter(|u| check_url(u).is_ok())
            .ok_or_else(|| StartError::Failed("the server's pairing page is not an https link".to_owned()))?;
        let verification_uri = page.to_string();
        let qr_url = a.verification_uri_complete.as_deref().and_then(plain_link).unwrap_or_else(|| {
            let mut url = page;
            url.query_pairs_mut().append_pair("code", &a.user_code);
            url.into()
        });
        if a.device_code.is_empty() || a.user_code.is_empty() || a.user_code.chars().any(char::is_control) {
            return Err(StartError::Failed("the server gave an empty or malformed code".to_owned()));
        }
        Ok(Self {
            server,
            qr_url,
            user_code: a.user_code,
            verification_uri,
            confirm_code: a.reins_confirm_code,
            key_fingerprint: identity.fingerprint(),
            expires_at: crate::now_unix().saturating_add(a.expires_in.clamp(1, MAX_EXPIRES_SECS)),
            client_id,
            token_endpoint: meta.token_endpoint,
            device_code: Zeroizing::new(a.device_code),
            interval: a.interval.map_or(DEFAULT_INTERVAL, Duration::from_secs).max(MIN_INTERVAL),
            http,
        })
    }

    /// How long to wait before the next [`DevicePairing::poll`] (it grows when the server says to slow down).
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Asks the server once whether the phone approved; saves the session (under `paths`) when it did. Network
    /// trouble counts as still waiting. An error ends the pairing: denied on the phone, or the code expired.
    pub async fn poll(&mut self, paths: &Paths) -> Result<DeviceStatus, String> {
        let form = [
            ("grant_type", DEVICE_CODE_GRANT),
            ("device_code", self.device_code.as_str()),
            ("client_id", self.client_id.as_str()),
        ];
        let resp = match self.http.post(&self.token_endpoint).form(&form).send().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("cannot reach the server while waiting for the phone: {}", e.without_url());
                return Ok(DeviceStatus::Waiting);
            }
        };
        let status = resp.status();
        let body = Zeroizing::new(read_limited(resp, MAX_ANSWER_BYTES).await.unwrap_or_default());
        if status.is_success() {
            return self.finish(paths, &body).map(|server| DeviceStatus::LoggedIn {
                server,
            });
        }
        let error = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(serde_json::Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        match error.as_str() {
            "authorization_pending" => Ok(DeviceStatus::Waiting),
            "slow_down" => {
                self.interval += DEFAULT_INTERVAL;
                Ok(DeviceStatus::Waiting)
            }
            "access_denied" => Err("the pairing was denied on your phone; not logged in".to_owned()),
            "expired_token" => {
                Err("the code expired before your phone approved it; run `reins login` again".to_owned())
            }
            _ if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() => {
                log::warn!("the server answered {status} while waiting for the phone");
                Ok(DeviceStatus::Waiting)
            }
            _ => Err(format!("the server did not log this app in: {}", error_text(status, &body))),
        }
    }

    /// Polls until the phone approves (returns the server) or the pairing ends (denied, expired).
    pub async fn wait(&mut self, paths: &Paths) -> Result<String, String> {
        loop {
            tokio::time::sleep(self.interval).await;
            if crate::now_unix() >= self.expires_at {
                return Err("the code expired before your phone approved it; run `reins login` again".to_owned());
            }
            if let DeviceStatus::LoggedIn {
                server,
            } = self.poll(paths).await?
            {
                return Ok(server);
            }
        }
    }

    fn finish(&self, paths: &Paths, body: &[u8]) -> Result<String, String> {
        let a: TokenAnswer =
            serde_json::from_slice(body).map_err(|_| "the server issued an unexpected token answer".to_owned())?;
        if a.access_token.is_empty() || a.token_type.as_deref().is_some_and(|t| !t.eq_ignore_ascii_case("bearer")) {
            return Err("the server issued no bearer token".to_owned());
        }
        let session = Session {
            server: self.server.clone(),
            client_id: self.client_id.clone(),
            token_endpoint: self.token_endpoint.clone(),
            access_expires_at: crate::now_unix().saturating_add(a.expires_in.unwrap_or(3600).clamp(0, 365 * 86_400)),
            access_token: a.access_token,
            refresh_token: a.refresh_token.filter(|r| !r.is_empty()),
        };
        save_session(paths, &session)?;
        log::info!("logged in to {} by QR code", self.server);
        Ok(self.server.clone())
    }
}

/// The modules of a QR code for `text`, row by row (`true` is dark), without the quiet zone around it.
pub fn qr_modules(text: &str) -> Result<Vec<Vec<bool>>, String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("cannot make a QR code: {e}"))?;
    let width = code.width();
    let colors = code.to_colors();
    Ok(colors.chunks(width).map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect()).collect())
}

/// Quiet zone around a QR code in a terminal, in modules (the standard asks for 4; scanners read 2 well).
const QUIET: usize = 2;

/// A QR code for `text` drawn with half blocks (two rows of modules per line). With `ansi`, it is drawn black on white
/// with escape codes, so it reads the same on a dark terminal; without, dark modules are the text color.
pub fn terminal_qr(text: &str, ansi: bool) -> Result<String, String> {
    let modules = qr_modules(text)?;
    let size = modules.len() + 2 * QUIET;
    let dark = |row: usize, col: usize| {
        row.checked_sub(QUIET)
            .zip(col.checked_sub(QUIET))
            .and_then(|(r, c)| modules.get(r).and_then(|line| line.get(c)))
            .copied()
            .unwrap_or(false)
    };
    let mut out = String::new();
    for row in (0..size).step_by(2) {
        if ansi {
            out.push_str("\x1b[30;107m");
        }
        for col in 0..size {
            out.push(match (dark(row, col), dark(row + 1, col)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        if ansi {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_codes_hold_the_pairing_link() {
        let link = "https://app.reins2fa.com/pair?code=BCDF-GHJK";
        let modules = qr_modules(link).unwrap();
        let width = modules.len();
        assert!((25..=41).contains(&width), "a small code: {width} modules");
        assert!(modules.iter().all(|row| row.len() == width));
        // The finder pattern's corner modules are dark, the separator next to it light.
        assert!(modules[0][0] && modules[0][6] && modules[6][0] && !modules[7][7]);
        let plain = terminal_qr(link, false).unwrap();
        let lines: Vec<&str> = plain.lines().collect();
        assert_eq!(lines.len(), (width + 2 * QUIET).div_ceil(2));
        assert!(lines.iter().all(|l| l.chars().count() == width + 2 * QUIET));
        assert!(lines[0].chars().all(|c| c == ' '), "quiet zone on top");
        let ansi = terminal_qr(link, true).unwrap();
        assert!(ansi.starts_with("\x1b[30;107m") && ansi.contains("\x1b[0m\n"));
    }

    #[test]
    fn links_to_show_must_be_plain_https() {
        assert_eq!(plain_link("https://app.reins2fa.com/pair").as_deref(), Some("https://app.reins2fa.com/pair"));
        assert_eq!(
            plain_link("http://127.0.0.1:8000/pair?code=X").as_deref(),
            Some("http://127.0.0.1:8000/pair?code=X")
        );
        for bad in ["http://evil.example/pair", "javascript:alert(1)", "https://u:p@a.example/pair", "pair"] {
            assert_eq!(plain_link(bad), None, "{bad}");
        }
    }
}
