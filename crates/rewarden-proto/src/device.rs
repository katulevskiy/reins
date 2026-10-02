//! Phone API bodies (`{domain}/rewarden/api/*`, contracts §A), shared by the
//! server and the phone core.

use serde::{Deserialize, Serialize};

use crate::ids::ConnectionId;
use crate::pairing::PairingRequest;
use crate::relay::RelayRequest;

/// Largest `wait` (seconds) honoured by `GET /pending`; larger values are clamped.
pub const MAX_PENDING_WAIT_SECS: u32 = 25;

/// Values of [`ApiError::error`].
pub mod codes {
    /// 403: the caller is not the user's registered approval device.
    pub const NOT_APPROVAL_DEVICE: &str = "not_approval_device";
    /// 404: unknown, expired, or belongs to another user.
    pub const NOT_FOUND: &str = "not_found";
    /// 409: the request or pairing was already answered.
    pub const ALREADY_ANSWERED: &str = "already_answered";
    /// 409: the chosen pairing code is not the one shown in the browser; the pairing is cancelled.
    pub const WRONG_CODE: &str = "wrong_code";
    /// 400: message `v` is not the supported protocol version.
    pub const BAD_VERSION: &str = "bad_version";
    /// 400: malformed body or invalid field.
    pub const BAD_REQUEST: &str = "bad_request";
    /// 429: a rate or concurrency limit; the message says when to try again.
    pub const RATE_LIMITED: &str = "rate_limited";
    /// 401: missing or invalid Vaultwarden access token.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// 500: server-side failure (e.g. database); retry later.
    pub const INTERNAL: &str = "internal_error";
}

/// A1 `PUT /device` body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistration {
    #[serde(default)]
    pub fcm_token: Option<String>,
}

/// `PUT /services` body: which integrations have an account on the phone, so that the server lists only their tools
/// to the AI. Only the integration ids (`gmail`, `github`, ...) travel, never an account name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServicesReport {
    pub services: Vec<String>,
    /// MCP servers the user added on the phone, with their tools (see [`crate::remote_mcp`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<crate::remote_mcp::McpServerReport>,
}

/// A1 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistered {
    /// True when another device was the approval device before this call.
    pub replaced_previous: bool,
}

/// A2 `GET /pending` response. Items returned here are marked delivered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub requests: Vec<RelayRequest>,
    #[serde(default)]
    pub pairings: Vec<PairingRequest>,
    /// Uploads that arrived and wait for the user ([`crate::blob::BlobPurpose::Upload`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<crate::blob::BlobInfo>,
    /// Phones of the account asking for its secret ([`crate::join`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joins: Vec<crate::join::JoinRequest>,
}

impl Pending {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.pairings.is_empty() && self.blobs.is_empty() && self.joins.is_empty()
    }
}

/// A6 response; `connection_id` is `None` when the user denied the pairing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResult {
    pub connection_id: Option<ConnectionId>,
}

/// One authorized AI client, as listed by A7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub label: String,
    /// Client-declared name (sanitized by the server, still untrusted).
    pub client_name: String,
    /// Host of the redirect URI used when the connection was authorized.
    pub client_host: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds of the last MCP call, if any.
    pub last_used_at: Option<i64>,
}

/// A7 response.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connections {
    pub connections: Vec<ConnectionInfo>,
}

/// Error body of every phone API error response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// One of [`codes`].
    pub error: String,
    pub message: String,
}

impl ApiError {
    #[must_use]
    pub fn new(error: &str, message: impl Into<String>) -> Self {
        Self {
            error: error.to_owned(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pairing::PairingRequest;

    #[test]
    fn registration_token_is_optional() {
        let r: DeviceRegistration = serde_json::from_value(json!({})).unwrap();
        assert_eq!(r.fcm_token, None);
        let r: DeviceRegistration = serde_json::from_value(json!({"fcm_token": "tok"})).unwrap();
        assert_eq!(r.fcm_token.as_deref(), Some("tok"));
        assert_eq!(
            serde_json::to_value(DeviceRegistered {
                replaced_previous: true
            })
            .unwrap(),
            json!({"replaced_previous": true})
        );
    }

    #[test]
    fn pending_wire_format() {
        let empty = Pending::default();
        assert!(empty.is_empty());
        assert_eq!(serde_json::to_value(&empty).unwrap(), json!({"requests": [], "pairings": []}));
        let p = Pending {
            requests: vec![],
            pairings: vec![PairingRequest {
                v: 1,
                id: "p1".into(),
                client_name: "ChatGPT".into(),
                client_host: "chatgpt.com".into(),
                choices: [12, 47, 83],
                created_at: 5,
                client_key: None,
            }],
            blobs: vec![],
            joins: vec![],
        };
        assert!(!p.is_empty());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["pairings"][0]["choices"], json!([12, 47, 83]));
        assert_eq!(serde_json::from_value::<Pending>(v).unwrap(), p);
    }

    #[test]
    fn pairing_result_denied_is_null() {
        let denied = PairingResult {
            connection_id: None,
        };
        assert_eq!(serde_json::to_value(&denied).unwrap(), json!({"connection_id": null}));
        let ok = PairingResult {
            connection_id: Some("c1".into()),
        };
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"connection_id": "c1"}));
    }

    #[test]
    fn connections_wire_format() {
        let c = Connections {
            connections: vec![ConnectionInfo {
                id: "c1".into(),
                label: "Work ChatGPT".into(),
                client_name: "ChatGPT".into(),
                client_host: "chatgpt.com".into(),
                created_at: 10,
                last_used_at: None,
            }],
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v,
            json!({"connections": [{"id": "c1", "label": "Work ChatGPT", "client_name": "ChatGPT",
                "client_host": "chatgpt.com", "created_at": 10, "last_used_at": null}]})
        );
        assert_eq!(serde_json::from_value::<Connections>(v).unwrap(), c);
    }

    #[test]
    fn api_error_wire_format() {
        let e = ApiError::new(codes::NOT_APPROVAL_DEVICE, "This device is not the approval device");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"error": "not_approval_device", "message": "This device is not the approval device"})
        );
        assert_eq!(codes::WRONG_CODE, "wrong_code");
        assert_eq!(MAX_PENDING_WAIT_SECS, 25);
    }
}
