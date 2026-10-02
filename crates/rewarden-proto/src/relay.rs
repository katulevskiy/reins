use serde::{Deserialize, Serialize};

use crate::gmail::{MessageFull, MessageSummary, SentMessage, ToolCall};
use crate::ids::{ConnectionId, RequestId};

/// A tool call forwarded by the server to the approval device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRequest {
    pub v: u32,
    pub id: RequestId,
    pub connection_id: ConnectionId,
    /// User-chosen label of the AI connection (e.g. "ChatGPT").
    pub connection_label: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds until which the AI is still waiting for an answer. After that the user can still approve;
    /// the answer is kept for `rewarden_get_result` and, for reads, a one-time retry pass is created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<i64>,
    /// The connected account the call is about (e.g. a Gmail address); absent when the AI did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub call: ToolCall,
}

/// One integration the user has connected, as told to the AI (never its accounts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationInfo {
    /// "gmail"
    pub service: String,
    /// "Gmail"
    pub name: String,
}

/// One connected account, as told to the AI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    /// "gmail"
    pub service: String,
    pub account: String,
}

/// What the phone released or did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResult {
    Search {
        messages: Vec<MessageSummary>,
    },
    Read {
        messages: Vec<MessageFull>,
    },
    Sent(SentMessage),
    /// What an integration returned or did: any JSON (items, an id, a confirmation) built on the phone.
    Connector {
        data: serde_json::Value,
    },
    /// The integrations the user has connected.
    Integrations {
        integrations: Vec<IntegrationInfo>,
    },
    /// The accounts one integration has, as the user allowed.
    Accounts {
        accounts: Vec<AccountInfo>,
        /// How many more accounts the integration has that the user did not share.
        #[serde(default)]
        withheld: u32,
    },
    /// The result of a tool of an MCP server the user added, passed through as that server gave it (`content`,
    /// `structuredContent`, `isError`), with large content replaced by download links.
    Mcp {
        result: serde_json::Value,
    },
    /// A permission the AI asked for was granted.
    Granted {
        summary: String,
        expires_at: i64,
        max_uses: Option<u32>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RelayOutcome {
    Result {
        result: ToolResult,
    },
    Denied {
        reason: Option<String>,
    },
    Error {
        message: String,
    },
}

/// The phone's answer to a `RelayRequest`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayResponse {
    pub v: u32,
    #[serde(flatten)]
    pub outcome: RelayOutcome,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::PROTOCOL_VERSION;

    #[test]
    fn request_round_trips() {
        let req = RelayRequest {
            v: PROTOCOL_VERSION,
            id: "r1".into(),
            connection_id: "c1".into(),
            connection_label: "ChatGPT".into(),
            created_at: 1_700_000_000,
            wait_until: None,
            account: Some("me@gmail.com".into()),
            call: ToolCall::GmailRead {
                message_ids: vec!["m1".into()],
            },
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["call"], json!({"tool": "gmail_read", "message_ids": ["m1"]}));
        assert_eq!(v["id"], json!("r1"));
        assert_eq!(serde_json::from_value::<RelayRequest>(v).unwrap(), req);
    }

    #[test]
    fn response_wire_format() {
        let denied = RelayResponse {
            v: 1,
            outcome: RelayOutcome::Denied {
                reason: None,
            },
        };
        assert_eq!(serde_json::to_value(&denied).unwrap(), json!({"v": 1, "outcome": "denied", "reason": null}));
        let sent = RelayResponse {
            v: 1,
            outcome: RelayOutcome::Result {
                result: ToolResult::Sent(SentMessage {
                    id: "m9".into(),
                    thread_id: "t9".into(),
                }),
            },
        };
        let v = serde_json::to_value(&sent).unwrap();
        assert_eq!(v, json!({"v": 1, "outcome": "result", "result": {"kind": "sent", "id": "m9", "thread_id": "t9"}}));
        assert_eq!(serde_json::from_value::<RelayResponse>(v).unwrap(), sent);
    }

    #[test]
    fn full_message_flattens_summary() {
        let m = MessageFull {
            summary: MessageSummary {
                id: "m1".into(),
                thread_id: "t1".into(),
                from: "a@b.com".into(),
                from_name: Some("Alice".into()),
                to: vec!["me@x.com".into()],
                cc: vec![],
                subject: "s".into(),
                date: 5,
                snippet: "sn".into(),
            },
            body_text: "body".into(),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["id"], json!("m1"));
        assert_eq!(v["body_text"], json!("body"));
        assert_eq!(serde_json::from_value::<MessageFull>(v).unwrap(), m);
    }
}
