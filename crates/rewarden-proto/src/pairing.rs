use serde::{Deserialize, Serialize};

use crate::ids::PairingId;

/// Shown on the phone when an AI client asks to connect to the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequest {
    pub v: u32,
    pub id: PairingId,
    /// Client-declared name (e.g. "ChatGPT"). Untrusted: always shown with `client_host`.
    pub client_name: String,
    /// Host of the OAuth redirect URI (e.g. "chatgpt.com"). Server-verified.
    pub client_host: String,
    /// Three two-digit codes; exactly one equals the code shown in the browser.
    pub choices: [u8; 3],
    /// Unix seconds.
    pub created_at: i64,
    /// The Rewarden desktop app's public key (base64url, 32 bytes), when the client is that app. The phone shows its
    /// [`crate::desktop::key_fingerprint`] for the user to compare and pins it to the new connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResponse {
    pub v: u32,
    pub approved: bool,
    /// The code the user tapped; required when `approved`.
    pub chosen_code: Option<u8>,
    /// Label for the new connection; defaults to `client_name`.
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushKind {
    Req,
    Pair,
    /// The device's push registration was replaced; carries an empty id.
    Replaced,
    /// An upload arrived (the id is the blob's).
    Blob,
}

/// FCM data payload. Carries only an id; content is fetched over HTTPS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushMessage {
    pub t: PushKind,
    pub id: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn push_wire_format() {
        let p = PushMessage {
            t: PushKind::Req,
            id: "r1".into(),
        };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"t": "req", "id": "r1"}));
    }

    #[test]
    fn replaced_push_wire_format() {
        let p = PushMessage {
            t: PushKind::Replaced,
            id: String::new(),
        };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"t": "replaced", "id": ""}));
        assert_eq!(serde_json::from_value::<PushMessage>(json!({"t": "replaced", "id": ""})).unwrap(), p);
    }

    #[test]
    fn pairing_round_trips() {
        let req = PairingRequest {
            v: 1,
            id: "p1".into(),
            client_name: "ChatGPT".into(),
            client_host: "chatgpt.com".into(),
            choices: [12, 47, 83],
            created_at: 1,
            client_key: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["choices"], json!([12, 47, 83]));
        assert_eq!(serde_json::from_value::<PairingRequest>(v).unwrap(), req);
    }
}
