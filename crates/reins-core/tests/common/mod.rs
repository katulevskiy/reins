//! Fakes for the Kotlin-side traits, shared by the integration tests.
#![allow(dead_code, reason = "each test binary uses a different subset")]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use rewarden_core::{
    AutoDecisionView, AutopilotEvent, ForeignError, GoogleTokenProvider, KeyWrapper, Notifier, PendingItem,
};

pub mod desktop;
pub mod mcp_mock;

/// Stand-in for the Android Keystore.
#[derive(Default)]
pub struct FakeKeys;

impl KeyWrapper for FakeKeys {
    fn wrap(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
        let mut out = b"KW1".to_vec();
        out.extend(plaintext.iter().map(|b| b ^ 0x5a));
        Ok(out)
    }

    fn unwrap(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, ForeignError> {
        wrapped.strip_prefix(b"KW1").map(|rest| rest.iter().map(|b| b ^ 0x5a).collect()).ok_or(ForeignError::Failed {
            reason: "bad blob".to_owned(),
        })
    }
}

pub const TOKEN_OK: u8 = 0;
pub const TOKEN_NEEDS_CONSENT: u8 = 1;
pub const TOKEN_FAILS: u8 = 2;

/// Hands out `google-token-<n>`; the counter shows how often a token was requested.
pub struct FakeGoogle {
    pub mode: AtomicU8,
    pub calls: AtomicU32,
    /// The account of every token request, in order (empty = the phone's default account).
    pub accounts: Mutex<Vec<String>>,
}

impl FakeGoogle {
    pub fn new() -> Self {
        Self {
            mode: AtomicU8::new(TOKEN_OK),
            calls: AtomicU32::new(0),
            accounts: Mutex::new(Vec::new()),
        }
    }

    pub fn set_mode(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl GoogleTokenProvider for FakeGoogle {
    async fn access_token(&self, account: String, _service: String) -> Result<String, ForeignError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.accounts.lock().unwrap().push(account.clone());
        match self.mode.load(Ordering::SeqCst) {
            TOKEN_NEEDS_CONSENT => Err(ForeignError::NeedsUserInteraction),
            TOKEN_FAILS => Err(ForeignError::Failed {
                reason: "play services unavailable".to_owned(),
            }),
            _ if account.is_empty() => Ok(format!("google-token-{n}")),
            _ => Ok(format!("google-token-{account}-{n}")),
        }
    }
}

/// Records notifications.
#[derive(Default)]
pub struct RecordingNotifier {
    pub pending: Mutex<Vec<PendingItem>>,
    pub resolved: Mutex<Vec<String>>,
    pub decided: Mutex<Vec<AutoDecisionView>>,
    pub events: Mutex<Vec<AutopilotEvent>>,
}

impl Notifier for RecordingNotifier {
    fn item_pending(&self, item: PendingItem) {
        self.pending.lock().unwrap().push(item);
    }

    fn item_resolved(&self, id: String) {
        self.resolved.lock().unwrap().push(id);
    }

    fn auto_decided(&self, decision: AutoDecisionView) {
        self.decided.lock().unwrap().push(decision);
    }

    fn autopilot_changed(&self, event: AutopilotEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// A Gmail message as the API returns it (`format=metadata` or `full`).
pub fn gmail_message(id: &str, from: &str, subject: &str, body: Option<&str>) -> serde_json::Value {
    use data_encoding::BASE64URL_NOPAD;
    let mut payload = serde_json::json!({
        "mimeType": "text/plain",
        "headers": [
            {"name": "From", "value": from},
            {"name": "To", "value": "me@example.com"},
            {"name": "Subject", "value": subject},
        ],
    });
    if let Some(body) = body {
        payload["body"] = serde_json::json!({"data": BASE64URL_NOPAD.encode(body.as_bytes())});
    }
    serde_json::json!({
        "id": id, "threadId": format!("t-{id}"), "labelIds": ["INBOX"], "snippet": format!("snippet of {id}"),
        "internalDate": "1700000000000", "payload": payload
    })
}
