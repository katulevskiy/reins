use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::connector::ConnectorCall;
use crate::validate::{ValidationError, invalid, normalize_address};

pub const MAX_SEARCH_RESULTS: u32 = 50;
pub const MAX_READ_IDS: usize = 20;
pub const MAX_QUERY_LEN: usize = 1024;
pub const MAX_RECIPIENTS: usize = 50;
pub const MAX_SUBJECT_LEN: usize = 998;
pub const MAX_BODY_LEN: usize = 1 << 20;
const MAX_MESSAGE_ID_LEN: usize = 64;
/// Largest arguments object of a call to an added MCP server (serialized).
pub const MAX_MCP_ARGUMENTS: usize = 512 * 1024;

/// A tool call made by an AI client, as relayed to the phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolCall {
    GmailSearch {
        query: String,
        max_results: u32,
    },
    GmailRead {
        message_ids: Vec<String>,
    },
    GmailSend {
        email: Box<OutgoingEmail>,
    },
    /// The AI asks for a standing permission in advance, as narrow as its task allows.
    RequestGrant {
        grant: Box<GrantRequest>,
    },
    /// A tool of another integration (Telegram, calendars, GitHub, ...): see [`crate::connector`].
    Connector(ConnectorCall),
    /// A tool of an MCP server the user added on the phone: see [`crate::remote_mcp`].
    Mcp(crate::remote_mcp::McpCall),
    /// `rewarden_upload`: the AI wants to pass on a file as a link (see [`crate::blob`]).
    RequestUpload {
        name: String,
        /// Expected size in bytes (the slot allows a little more).
        size: u64,
        #[serde(default)]
        content_type: Option<String>,
        reason: String,
    },
    /// Without `service`: which integrations the user has connected (answered without asking). With `service`: the
    /// accounts of that integration, which the user has to allow first.
    ListAccounts {
        #[serde(default)]
        service: Option<String>,
        /// Some accounts were left out last time: ask the user to share more.
        #[serde(default)]
        ask_for_more: bool,
    },
}

/// What a standing permission an AI asks for may cover. Kept small on purpose: the user reads this on
/// their phone and decides whether to allow it, so every field has hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantAction {
    Read,
    Send,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub action: GrantAction,
    /// How long the permission lasts, in seconds.
    pub duration_secs: u64,
    /// Optional cap on how many requests it may cover.
    #[serde(default)]
    pub max_uses: Option<u32>,
    /// Why the AI needs it, shown to the user.
    pub reason: String,
    /// Read only: every message, for at most [`MAX_ANY_GRANT_SECS`].
    #[serde(default)]
    pub any: bool,
    /// Read: senders (`a@b.com` or `@b.com`).
    #[serde(default)]
    pub from: Vec<String>,
    /// Read or send: the subject must contain this literal text.
    #[serde(default)]
    pub subject_contains: Option<String>,
    /// Send: recipients (`a@b.com` or `@b.com`).
    #[serde(default)]
    pub recipients: Vec<String>,
}

pub const MIN_GRANT_SECS: u64 = 60;
pub const MAX_GRANT_SECS: u64 = 30 * 86_400;
pub const MAX_ANY_GRANT_SECS: u64 = 7 * 86_400;
pub const MAX_REASON_LEN: usize = 300;
pub const MAX_GRANT_RULES: usize = 20;
pub const MAX_SUBJECT_TEXT_LEN: usize = 200;

/// An email an AI asks to send. Unknown fields (e.g. `bcc`) are rejected so a
/// recipient can never be silently dropped from the approval and policy checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutgoingEmail {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
}

/// Header-level view of a message, as released by `gmail_search`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: String,
    pub thread_id: String,
    /// Bare, lower-cased sender address.
    pub from: String,
    /// Sender display name. Untrusted: the sender chooses it, so never use it
    /// for policy decisions; show it only next to `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    /// Bare, lower-cased To addresses (Cc is in `cc`).
    pub to: Vec<String>,
    /// Bare, lower-cased Cc addresses.
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    /// Unix SECONDS: Gmail's `internalDate` (milliseconds) divided by 1000.
    pub date: i64,
    pub snippet: String,
}

/// A full message, as released by `gmail_read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFull {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub body_text: String,
}

/// Result of a successful `gmail_send`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentMessage {
    pub id: String,
    pub thread_id: String,
}

impl ToolCall {
    /// Validates limits and normalizes addresses and ids. Call on every call
    /// received from an AI before it is relayed, evaluated or executed.
    pub fn normalized(self) -> Result<Self, ValidationError> {
        match self {
            Self::GmailSearch {
                query,
                max_results,
            } => {
                let query = query.trim().to_owned();
                if query.is_empty() || query.len() > MAX_QUERY_LEN {
                    return Err(invalid("query", format!("length must be 1..={MAX_QUERY_LEN}")));
                }
                if !(1..=MAX_SEARCH_RESULTS).contains(&max_results) {
                    return Err(invalid("max_results", format!("must be 1..={MAX_SEARCH_RESULTS}")));
                }
                Ok(Self::GmailSearch {
                    query,
                    max_results,
                })
            }
            Self::GmailRead {
                message_ids,
            } => Ok(Self::GmailRead {
                message_ids: normalize_ids(message_ids)?,
            }),
            Self::GmailSend {
                email,
            } => Ok(Self::GmailSend {
                email: Box::new((*email).normalized()?),
            }),
            Self::RequestGrant {
                grant,
            } => Ok(Self::RequestGrant {
                grant: Box::new((*grant).normalized()?),
            }),
            Self::Connector(call) => {
                call.normalized().map(Self::Connector).map_err(|reason| invalid("connector", reason))
            }
            Self::Mcp(call) => {
                if !crate::remote_mcp::server_id_ok(&call.server) {
                    return Err(invalid("server", "not a server id"));
                }
                if call.tool.is_empty() || call.tool.len() > 128 || call.tool.chars().any(char::is_control) {
                    return Err(invalid("tool", "length must be 1..=128"));
                }
                let size = serde_json::to_vec(&call.arguments).map_or(usize::MAX, |v| v.len());
                if size > MAX_MCP_ARGUMENTS {
                    return Err(invalid("arguments", format!("at most {MAX_MCP_ARGUMENTS} bytes")));
                }
                Ok(Self::Mcp(call))
            }
            Self::RequestUpload {
                name,
                size,
                content_type,
                reason,
            } => {
                let name = name.trim().to_owned();
                if name.is_empty()
                    || name.chars().count() > 200
                    || name.contains(['/', '\\'])
                    || name.chars().any(char::is_control)
                {
                    return Err(invalid("name", "a file name of 1..=200 characters, without a path"));
                }
                if size == 0 || size > crate::blob::MAX_BLOB_BYTES {
                    return Err(invalid("size", format!("must be 1..={}", crate::blob::MAX_BLOB_BYTES)));
                }
                let reason = reason.trim().to_owned();
                if reason.is_empty() || reason.chars().count() > 300 || reason.chars().any(char::is_control) {
                    return Err(invalid("reason", "one line of 1..=300 characters"));
                }
                let content_type = content_type.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
                if content_type.as_ref().is_some_and(|c| c.len() > 100 || !c.is_ascii()) {
                    return Err(invalid("content_type", "a media type of at most 100 characters"));
                }
                Ok(Self::RequestUpload {
                    name,
                    size,
                    content_type,
                    reason,
                })
            }
            Self::ListAccounts {
                service,
                ask_for_more,
            } => Ok(Self::ListAccounts {
                service: service.map(|s| normalize_service(&s)).transpose()?,
                ask_for_more,
            }),
        }
    }
}

/// An integration name (`gmail`), lower-cased; anything else is refused.
pub fn normalize_service(raw: &str) -> Result<String, ValidationError> {
    let service = raw.trim().to_ascii_lowercase();
    let ok = !service.is_empty()
        && service.len() <= 32
        && service.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if ok {
        Ok(service)
    } else {
        Err(invalid("service", "must be an integration name such as gmail"))
    }
}

/// The account a call names (`work@gmail.com`), lower-cased; anything that is not an address is refused.
pub fn normalize_account(raw: &str) -> Result<String, ValidationError> {
    normalize_address(raw.trim())
        .map_err(|_| invalid("account", "must be one of the addresses from rewarden_list_accounts"))
}

/// `a@b.com` → itself (lower-cased); `@b.com` → the domain rule; anything else is refused.
fn normalize_rule(raw: &str) -> Result<String, ValidationError> {
    let trimmed = raw.trim();
    if let Some(domain) = trimmed.strip_prefix('@') {
        let probe = normalize_address(&format!("x@{domain}"))?;
        return Ok(format!("@{}", &probe[2..]));
    }
    normalize_address(trimmed)
}

impl GrantRequest {
    pub fn normalized(self) -> Result<Self, ValidationError> {
        if !(MIN_GRANT_SECS..=MAX_GRANT_SECS).contains(&self.duration_secs) {
            return Err(invalid("duration_secs", format!("must be {MIN_GRANT_SECS}..={MAX_GRANT_SECS} seconds")));
        }
        if self.max_uses == Some(0) {
            return Err(invalid("max_uses", "must be at least 1"));
        }
        let reason = self.reason.trim().to_owned();
        if reason.is_empty() || reason.len() > MAX_REASON_LEN || reason.chars().any(|c| c.is_control() && c != '\n') {
            return Err(invalid("reason", format!("say why in 1..={MAX_REASON_LEN} characters")));
        }
        let subject = self.subject_contains.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        if subject.as_ref().is_some_and(|s| s.len() > MAX_SUBJECT_TEXT_LEN || s.chars().any(char::is_control)) {
            return Err(invalid("subject_contains", format!("at most {MAX_SUBJECT_TEXT_LEN} characters")));
        }
        if self.from.len() > MAX_GRANT_RULES || self.recipients.len() > MAX_GRANT_RULES {
            return Err(invalid("from", format!("at most {MAX_GRANT_RULES} entries")));
        }
        let from = self.from.iter().map(|r| normalize_rule(r)).collect::<Result<Vec<_>, _>>()?;
        let recipients = self.recipients.iter().map(|r| normalize_rule(r)).collect::<Result<Vec<_>, _>>()?;
        match self.action {
            GrantAction::Read => {
                if !recipients.is_empty() {
                    return Err(invalid("recipients", "only for send permissions"));
                }
                if self.any {
                    if !from.is_empty() || subject.is_some() {
                        return Err(invalid("any", "cannot be combined with other limits"));
                    }
                    if self.duration_secs > MAX_ANY_GRANT_SECS {
                        return Err(invalid(
                            "duration_secs",
                            format!("access to all mail may last at most {MAX_ANY_GRANT_SECS} seconds"),
                        ));
                    }
                } else if from.is_empty() && subject.is_none() {
                    return Err(invalid("from", "name the senders or a subject text, or set any=true for all mail"));
                }
            }
            GrantAction::Send => {
                if self.any || !from.is_empty() {
                    return Err(invalid("any", "sending cannot be opened to everyone; list the recipients"));
                }
                if recipients.is_empty() {
                    return Err(invalid("recipients", "list at least one recipient"));
                }
            }
        }
        Ok(Self {
            reason,
            from,
            recipients,
            subject_contains: subject,
            ..self
        })
    }
}

impl OutgoingEmail {
    pub fn normalized(self) -> Result<Self, ValidationError> {
        let to = self.to.iter().map(String::as_str).map(normalize_address).collect::<Result<Vec<_>, _>>()?;
        let cc = self.cc.iter().map(String::as_str).map(normalize_address).collect::<Result<Vec<_>, _>>()?;
        if to.is_empty() {
            return Err(invalid("to", "at least one recipient is required"));
        }
        if to.len() + cc.len() > MAX_RECIPIENTS {
            return Err(invalid("to", format!("at most {MAX_RECIPIENTS} recipients")));
        }
        if self.subject.len() > MAX_SUBJECT_LEN || self.subject.chars().any(char::is_control) {
            return Err(invalid(
                "subject",
                format!("must be at most {MAX_SUBJECT_LEN} bytes without control characters"),
            ));
        }
        if self.body.len() > MAX_BODY_LEN {
            return Err(invalid("body", format!("must be at most {MAX_BODY_LEN} bytes")));
        }
        if let Some(id) = &self.reply_to_message_id {
            validate_message_id(id)?;
        }
        Ok(Self {
            to,
            cc,
            ..self
        })
    }

    /// All To and Cc addresses.
    pub fn recipients(&self) -> impl Iterator<Item = &str> {
        self.to.iter().chain(&self.cc).map(String::as_str)
    }
}

fn validate_message_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() || id.len() > MAX_MESSAGE_ID_LEN || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(invalid("message_id", format!("invalid id {id:?}")));
    }
    Ok(())
}

fn normalize_ids(ids: Vec<String>) -> Result<Vec<String>, ValidationError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        validate_message_id(&id)?;
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    if out.is_empty() || out.len() > MAX_READ_IDS {
        return Err(invalid("message_ids", format!("must contain 1..={MAX_READ_IDS} ids")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn email(to: &[&str], subject: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: subject.to_owned(),
            body: "hi".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn search_limits() {
        let ok = ToolCall::GmailSearch {
            query: " from:bank ".to_owned(),
            max_results: 50,
        };
        assert_eq!(
            ok.normalized().unwrap(),
            ToolCall::GmailSearch {
                query: "from:bank".to_owned(),
                max_results: 50
            }
        );
        for max_results in [0, 51] {
            let c = ToolCall::GmailSearch {
                query: "x".to_owned(),
                max_results,
            };
            assert!(c.normalized().is_err());
        }
        for query in [String::new(), "   ".to_owned(), "x".repeat(MAX_QUERY_LEN + 1)] {
            assert!(
                ToolCall::GmailSearch {
                    query,
                    max_results: 5
                }
                .normalized()
                .is_err()
            );
        }
    }

    #[test]
    fn read_ids_validated_and_deduplicated() {
        let c = ToolCall::GmailRead {
            message_ids: vec!["a1".into(), "b2".into(), "a1".into()],
        };
        assert_eq!(
            c.normalized().unwrap(),
            ToolCall::GmailRead {
                message_ids: vec!["a1".into(), "b2".into()]
            }
        );
        assert!(
            ToolCall::GmailRead {
                message_ids: vec![]
            }
            .normalized()
            .is_err()
        );
        assert!(
            ToolCall::GmailRead {
                message_ids: vec!["../x".into()]
            }
            .normalized()
            .is_err()
        );
        let many = (0..=MAX_READ_IDS).map(|i| format!("m{i}")).collect();
        assert!(
            ToolCall::GmailRead {
                message_ids: many
            }
            .normalized()
            .is_err()
        );
    }

    #[test]
    fn send_normalizes_and_rejects_injection() {
        let e = email(&["Alice@Bank.com"], "Hello").normalized().unwrap();
        assert_eq!(e.to, vec!["alice@bank.com"]);
        assert!(email(&[], "x").normalized().is_err());
        assert!(email(&["a@b.com"], "Hi\r\nBcc: eve@evil.com").normalized().is_err());
        assert!(email(&["a@b.com\r\nBcc: eve@evil.com"], "x").normalized().is_err());
        let mut cc_inj = email(&["a@b.com"], "x");
        cc_inj.cc = vec!["a@b.com\r\nBcc: x@evil.com".into()];
        assert!(cc_inj.normalized().is_err());
        let too_many: Vec<String> = (0..=MAX_RECIPIENTS).map(|i| format!("u{i}@b.com")).collect();
        let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert!(email(&refs, "x").normalized().is_err());
        let mut big = email(&["a@b.com"], "x");
        big.body = "x".repeat(MAX_BODY_LEN + 1);
        assert!(big.normalized().is_err());
        let mut reply = email(&["a@b.com"], "x");
        reply.reply_to_message_id = Some("bad id".into());
        assert!(reply.normalized().is_err());
    }

    #[test]
    fn send_via_toolcall_normalizes_boxed_email() {
        let c = ToolCall::GmailSend {
            email: Box::new(email(&["A@B.com"], "x")),
        };
        let ToolCall::GmailSend {
            email,
        } = c.normalized().unwrap()
        else {
            panic!("variant changed")
        };
        assert_eq!(email.to, vec!["a@b.com"]);
    }

    #[test]
    fn recipients_chains_to_and_cc() {
        let mut e = email(&["a@b.com"], "x");
        e.cc = vec!["c@d.com".into()];
        assert_eq!(e.recipients().collect::<Vec<_>>(), vec!["a@b.com", "c@d.com"]);
    }

    #[test]
    fn wire_format() {
        let c = ToolCall::GmailSearch {
            query: "x".into(),
            max_results: 5,
        };
        assert_eq!(serde_json::to_value(&c).unwrap(), json!({"tool": "gmail_search", "query": "x", "max_results": 5}));
        let parsed: ToolCall = serde_json::from_value(json!({
            "tool": "gmail_send",
            "email": {"to": ["a@b.com"], "subject": "s", "body": "b"}
        }))
        .unwrap();
        let ToolCall::GmailSend {
            email,
        } = parsed
        else {
            panic!("wrong variant")
        };
        assert!(email.cc.is_empty() && email.reply_to_message_id.is_none());
    }

    #[test]
    fn outgoing_email_rejects_unknown_fields() {
        let with_bcc = json!({"to": ["a@b.com"], "subject": "s", "body": "b", "bcc": ["x@y.com"]});
        assert!(serde_json::from_value::<OutgoingEmail>(with_bcc).is_err());
        let ok = json!({"to": ["a@b.com"], "subject": "s", "body": "b"});
        assert!(serde_json::from_value::<OutgoingEmail>(ok).is_ok());
    }

    #[test]
    fn summary_wire_format_and_defaults() {
        let s = MessageSummary {
            id: "m1".into(),
            thread_id: "t1".into(),
            from: "a@b.com".into(),
            from_name: None,
            to: vec!["me@x.com".into()],
            cc: vec![],
            subject: "s".into(),
            date: 5,
            snippet: "sn".into(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("from_name").is_none(), "None display name is omitted");
        assert_eq!(v["cc"], json!([]));
        // Older payloads without from_name/cc still parse.
        let legacy = json!({"id": "m1", "thread_id": "t1", "from": "a@b.com", "to": ["me@x.com"],
            "subject": "s", "date": 5, "snippet": "sn"});
        assert_eq!(serde_json::from_value::<MessageSummary>(legacy).unwrap(), s);
        let named = MessageSummary {
            from_name: Some("Alice".into()),
            cc: vec!["c@d.com".into()],
            ..s
        };
        let v = serde_json::to_value(&named).unwrap();
        assert_eq!(v["from_name"], json!("Alice"));
        assert_eq!(serde_json::from_value::<MessageSummary>(v).unwrap(), named);
    }

    fn read_request() -> GrantRequest {
        GrantRequest {
            action: GrantAction::Read,
            duration_secs: 3600,
            max_uses: None,
            reason: "Summarise this week's bank statements".to_owned(),
            any: false,
            from: vec!["Alerts@Bank.com".to_owned(), "@Statements.Bank.com".to_owned()],
            subject_contains: Some(" statement ".to_owned()),
            recipients: vec![],
        }
    }

    #[test]
    fn grant_requests_are_normalized_and_bounded() {
        let ok = ToolCall::RequestGrant {
            grant: Box::new(read_request()),
        }
        .normalized()
        .unwrap();
        let ToolCall::RequestGrant {
            grant,
        } = ok
        else {
            panic!("grant expected")
        };
        assert_eq!(grant.from, ["alerts@bank.com", "@statements.bank.com"]);
        assert_eq!(grant.subject_contains.as_deref(), Some("statement"));

        let bad = |f: &dyn Fn(&mut GrantRequest)| {
            let mut r = read_request();
            f(&mut r);
            r.normalized().is_err()
        };
        assert!(bad(&|r| r.duration_secs = 59), "too short");
        assert!(bad(&|r| r.duration_secs = MAX_GRANT_SECS + 1), "too long");
        assert!(bad(&|r| r.max_uses = Some(0)));
        assert!(bad(&|r| r.reason = "  ".to_owned()));
        assert!(bad(&|r| r.from = vec!["not an address".to_owned()]));
        assert!(bad(&|r| r.from = vec!["@nodot".to_owned()]));
        assert!(bad(&|r| r.recipients = vec!["a@b.com".to_owned()]), "recipients belong to send");
        assert!(
            bad(&|r| {
                r.from.clear();
                r.subject_contains = None;
            }),
            "a read grant must name something unless any=true"
        );
    }

    #[test]
    fn asking_for_all_mail_is_time_boxed_and_exclusive() {
        let mut r = read_request();
        r.from.clear();
        r.subject_contains = None;
        r.any = true;
        r.duration_secs = MAX_ANY_GRANT_SECS;
        assert!(r.clone().normalized().is_ok());
        r.duration_secs = MAX_ANY_GRANT_SECS + 1;
        assert!(r.clone().normalized().is_err());
        r.duration_secs = 3600;
        r.subject_contains = Some("x".to_owned());
        assert!(r.normalized().is_err(), "any excludes other limits");
    }

    #[test]
    fn send_grants_need_recipients_and_never_everyone() {
        let mut r = GrantRequest {
            action: GrantAction::Send,
            duration_secs: 600,
            max_uses: Some(1),
            reason: "Reply to Mira".to_owned(),
            any: false,
            from: vec![],
            subject_contains: None,
            recipients: vec!["Mira@Studio.example".to_owned()],
        };
        assert_eq!(r.clone().normalized().unwrap().recipients, ["mira@studio.example"]);
        r.any = true;
        assert!(r.clone().normalized().is_err());
        r.any = false;
        r.recipients.clear();
        assert!(r.normalized().is_err());
    }

    #[test]
    fn grant_requests_reject_unknown_fields() {
        let v = json!({"action": "read", "duration_secs": 60, "reason": "x", "from": ["a@b.com"], "bcc": "z"});
        assert!(serde_json::from_value::<GrantRequest>(v).is_err());
    }
}
