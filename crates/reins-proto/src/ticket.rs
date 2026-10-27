//! Tickets: requests whose review the agent does not wait for (Decisions v2, 03 §1-§4, 08 R9).
//!
//! A request starts Instant: the agent's call (or hook) waits for the phone, as the relay always did. When nobody
//! answers within the Instant window, when the agent asks for it, or when the reviewer taps Later, it becomes a
//! **ticket**: the agent gets a ticket id at once and carries on, and learns the outcome later.
//!
//! The server keeps a ticket's metadata in clear (id, state, deadlines, connection, the request digest) and its body
//! sealed to the approval device. Each transition is an event; the phone's are signed with its device key
//! ([`event_message`]), so an agent (or a webhook receiver) can tell an approval the phone made from anything the
//! server could have written.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{
    connector::{self, Effect},
    gmail::ToolCall,
    ids::{ConnectionId, RequestId},
    relay::{RelayRequest, ToolResult},
};

/// What a ticket id starts with, so an agent (and a person reading a log) can tell it from other ids.
pub const TICKET_PREFIX: &str = "rt_";
/// Instant window: how long a request waits for the reviewer before it becomes a ticket (03 §1).
pub const INSTANT_DEFAULT_SECS: u64 = 120;
pub const INSTANT_MAX_SECS: u64 = 600;
/// Deferred window: how long a ticket waits for its review.
pub const DEFERRED_DEFAULT_SECS: u64 = 4 * 3600;
pub const DEFERRED_MAX_SECS: u64 = 7 * 86_400;
/// Shortest window of either class.
pub const MIN_WINDOW_SECS: u64 = 60;
/// Open tickets one connection may have (03 §8).
pub const MAX_OPEN_TICKETS: usize = 50;
/// How long an approval of a command the agent runs itself stays usable (03 §6).
pub const AGENT_RUN_VALIDITY_SECS: i64 = 3600;
/// How long an approval of an action an executor runs (the phone, the desktop app) stays usable.
pub const EXECUTOR_RUN_VALIDITY_SECS: i64 = 86_400;
/// Longest reviewer note, "Why?" question or agent reply, in characters.
pub const MAX_NOTE_CHARS: usize = 500;
/// Longest agent reply to a "Why?" (`reins_reply`), in characters.
pub const MAX_REPLY_CHARS: usize = 2000;
/// Longest `reins_wait`.
pub const MAX_WAIT_SECS: u64 = 600;
/// Longest ticket or request id.
const MAX_ID_LEN: usize = 64;

/// The ticket id an agent sees for request `id`.
#[must_use]
pub fn ticket_of(id: &RequestId) -> String {
    format!("{TICKET_PREFIX}{id}")
}

/// The request id of a ticket (`rt_<id>`) or of a bare request id; `None` for anything that cannot be one.
#[must_use]
pub fn request_of(ticket: &str) -> Option<RequestId> {
    let id = ticket.strip_prefix(TICKET_PREFIX).unwrap_or(ticket);
    let ok = !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    ok.then(|| RequestId(id.to_owned()))
}

/// Instant or Deferred (03 §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketClass {
    Instant,
    Deferred,
}

/// A window of `class`, in seconds: the default when none is given, never shorter than [`MIN_WINDOW_SECS`] or longer
/// than the class allows.
#[must_use]
pub fn clamp_window(class: TicketClass, secs: Option<u64>) -> u64 {
    let (default, max) = match class {
        TicketClass::Instant => (INSTANT_DEFAULT_SECS, INSTANT_MAX_SECS),
        TicketClass::Deferred => (DEFERRED_DEFAULT_SECS, DEFERRED_MAX_SECS),
    };
    secs.unwrap_or(default).clamp(MIN_WINDOW_SECS, max)
}

/// What happens when a ticket's window ends (02 §2 `on_expiry`, 03 §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExpiry {
    /// Expired: not approved.
    #[default]
    Deny,
    /// Asked again once, with the window restarted.
    Escalate,
}

impl OnExpiry {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Escalate => "escalate",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "deny" => Some(Self::Deny),
            "escalate" => Some(Self::Escalate),
            _ => None,
        }
    }
}

/// Where a request stands (03 §2), plus `needs_info` (a reviewer asked "Why?", 03 §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Received,
    Reviewing,
    Deferred,
    NeedsInfo,
    Escalated,
    Approved,
    Revalidating,
    Executing,
    Stale,
    Done,
    Failed,
    Unknown,
    Denied,
    Expired,
    Cancelled,
}

impl TicketState {
    pub const ALL: [Self; 15] = [
        Self::Received,
        Self::Reviewing,
        Self::Deferred,
        Self::NeedsInfo,
        Self::Escalated,
        Self::Approved,
        Self::Revalidating,
        Self::Executing,
        Self::Stale,
        Self::Done,
        Self::Failed,
        Self::Unknown,
        Self::Denied,
        Self::Expired,
        Self::Cancelled,
    ];

    /// Whether a ticket may move from `self` to `to`. A ticket can be decided straight from the inbox (Deferred to
    /// Approved), without an "opened" event in between.
    #[must_use]
    pub fn can_go(self, to: Self) -> bool {
        use TicketState::{
            Approved, Cancelled, Deferred, Denied, Done, Escalated, Executing, Expired, Failed, NeedsInfo, Received,
            Revalidating, Reviewing, Stale, Unknown,
        };
        match self {
            Received => matches!(to, Reviewing | Deferred | Approved | Denied | Cancelled),
            Reviewing => matches!(to, Approved | Denied | Deferred | NeedsInfo | Cancelled),
            Deferred => matches!(to, Reviewing | Approved | Denied | NeedsInfo | Expired | Escalated | Cancelled),
            NeedsInfo => matches!(to, Deferred | Approved | Denied | Expired | Cancelled),
            Escalated => matches!(to, Reviewing | Approved | Denied | Expired | Cancelled),
            Approved => matches!(to, Revalidating | Executing | Done | Failed | Stale | Expired),
            Revalidating => matches!(to, Executing | Stale),
            Executing => matches!(to, Done | Failed | Unknown),
            Stale => matches!(to, Reviewing | Approved | Denied | Expired | Cancelled),
            Done | Failed | Unknown | Denied | Expired | Cancelled => false,
        }
    }

    /// Nothing more happens to it.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Unknown | Self::Denied | Self::Expired | Self::Cancelled)
    }

    /// It waits for a reviewer (what counts against [`MAX_OPEN_TICKETS`] and shows under "Waiting for you").
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(
            self,
            Self::Received | Self::Reviewing | Self::Deferred | Self::NeedsInfo | Self::Escalated | Self::Stale
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Reviewing => "reviewing",
            Self::Deferred => "deferred",
            Self::NeedsInfo => "needs_info",
            Self::Escalated => "escalated",
            Self::Approved => "approved",
            Self::Revalidating => "revalidating",
            Self::Executing => "executing",
            Self::Stale => "stale",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == s)
    }

    /// The webhook event type for a transition into this state, when there is one (03 §4.4).
    #[must_use]
    pub const fn webhook_type(self) -> Option<&'static str> {
        match self {
            Self::Approved => Some("ticket.approved"),
            Self::Denied => Some("ticket.denied"),
            Self::Expired => Some("ticket.expired"),
            Self::Stale => Some("ticket.stale"),
            Self::Done => Some("ticket.done"),
            _ => None,
        }
    }
}

/// When an approved ticket runs (08 R9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    /// Writes: the phone runs it when approved (the agent may never come back); a minimal result is kept.
    OnApprove,
    /// Reads and desktop answers: approval keeps only the decision; the phone runs it when the agent collects, and
    /// the result goes straight to the agent.
    OnCollect,
}

impl Execution {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnApprove => "on_approve",
            Self::OnCollect => "on_collect",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "on_approve" => Some(Self::OnApprove),
            "on_collect" => Some(Self::OnCollect),
            _ => None,
        }
    }
}

/// When `call` runs once approved. Tools of added MCP servers count as writes: their read hints come from a third
/// party.
#[must_use]
pub fn execution(call: &ToolCall) -> Execution {
    match call {
        ToolCall::GmailSearch {
            ..
        }
        | ToolCall::GmailRead {
            ..
        }
        | ToolCall::ListAccounts {
            ..
        }
        | ToolCall::RequestUpload {
            ..
        } => Execution::OnCollect,
        ToolCall::Connector(c) => match c.spec() {
            Some(spec) if spec.desktop_only || spec.effect != Effect::Write => Execution::OnCollect,
            _ => Execution::OnApprove,
        },
        ToolCall::GmailSend {
            ..
        }
        | ToolCall::RequestGrant {
            ..
        }
        | ToolCall::Mcp(_) => Execution::OnApprove,
    }
}

/// How long an approval of `call` stays usable (03 §6): an hour for a command the agent runs itself (`desktop_ask`),
/// a day for what an executor runs.
#[must_use]
pub fn approval_validity(call: &ToolCall) -> i64 {
    match call {
        ToolCall::Connector(c) if c.service == "desktop" && c.op == "ask" => AGENT_RUN_VALIDITY_SECS,
        _ => EXECUTOR_RUN_VALIDITY_SECS,
    }
}

#[derive(Serialize)]
struct DigestInput<'a> {
    connection_id: &'a ConnectionId,
    account: Option<&'a str>,
    call: &'a ToolCall,
}

/// The request digest (03 §6): SHA-256 (hex) over the connection, the account and the call. The same call again from
/// the same connection is the same request; anything else is a new one, and approvals never transfer.
#[must_use]
pub fn request_digest(connection_id: &ConnectionId, account: Option<&str>, call: &ToolCall) -> String {
    let input = DigestInput {
        connection_id,
        account,
        call,
    };
    let mut hasher = Sha256::new();
    hasher.update(b"reins-ticket/1\n");
    // Serializing a value this crate defines cannot fail.
    hasher.update(serde_json::to_vec(&input).unwrap_or_default());
    hex(&hasher.finalize())
}

/// [`request_digest`] of a relayed request.
#[must_use]
pub fn digest_of(request: &RelayRequest) -> String {
    request_digest(&request.connection_id, request.account.as_deref(), &request.call)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Top-level fields of an integration's answer that identify what a write made, and nothing of its content.
const MINIMAL_KEYS: [&str; 13] = [
    "id",
    "node_id",
    "number",
    "sha",
    "html_url",
    "url",
    "status",
    "state",
    "created_at",
    "updated_at",
    "message_id",
    "event_id",
    "ok",
];

/// The part of a write's result the server may keep for the ticket's lifetime (08 R9): ids, a status, a link.
#[must_use]
pub fn minimal_result(result: &ToolResult) -> Value {
    let mut out = Map::new();
    match result {
        ToolResult::Sent(sent) => {
            out.insert("id".to_owned(), Value::String(sent.id.clone()));
            out.insert("thread_id".to_owned(), Value::String(sent.thread_id.clone()));
        }
        ToolResult::Granted {
            expires_at,
            max_uses,
            ..
        } => {
            out.insert("granted".to_owned(), Value::Bool(true));
            out.insert("expires_at".to_owned(), Value::from(*expires_at));
            out.insert("max_uses".to_owned(), max_uses.map_or(Value::Null, Value::from));
        }
        ToolResult::Connector {
            data: Value::Object(data),
        } => {
            for key in MINIMAL_KEYS {
                if let Some(v) = data.get(key).filter(|v| !v.is_object() && !v.is_array()) {
                    out.insert(key.to_owned(), v.clone());
                }
            }
        }
        ToolResult::Mcp {
            result,
        } => {
            let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
            out.insert("isError".to_owned(), Value::Bool(is_error));
        }
        _ => {}
    }
    Value::Object(out)
}

/// Every key [`minimal_result`] may produce.
const MINIMAL_ANY: [&str; 17] = [
    "id",
    "node_id",
    "number",
    "sha",
    "html_url",
    "url",
    "status",
    "state",
    "created_at",
    "updated_at",
    "message_id",
    "event_id",
    "ok",
    "thread_id",
    "granted",
    "expires_at",
    "isError",
];

/// `value` cut down to what a minimal result may hold (top-level scalars under [`minimal_result`]'s keys): what a
/// server keeps of a result it was handed as JSON, whoever made it.
#[must_use]
pub fn minimal_value(value: &Value) -> Value {
    let mut out = Map::new();
    if let Value::Object(map) = value {
        for key in MINIMAL_ANY.into_iter().chain(["max_uses"]) {
            if let Some(v) = map.get(key).filter(|v| !v.is_object() && !v.is_array()) {
                let keep = match v {
                    Value::String(s) => s.chars().count() <= 300,
                    _ => true,
                };
                if keep {
                    out.insert(key.to_owned(), v.clone());
                }
            }
        }
    }
    Value::Object(out)
}

/// One transition of a ticket. The phone signs the ones it makes ([`SignedTicketEvent`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketEvent {
    pub v: u32,
    /// `rt_<request id>`.
    pub ticket: String,
    /// Where the ticket stood when this was signed: an event applies only from there, so a signed transition cannot be
    /// replayed at another point of the ticket's history.
    pub from: TicketState,
    pub state: TicketState,
    /// Unix seconds, by the signer's clock.
    pub at: i64,
    /// [`request_digest`] of the request the transition is about.
    pub digest: String,
    pub connection_id: ConnectionId,
    /// The new deadline (a deferral, an escalation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<i64>,
    /// The reviewer's note to the agent (a denial) or question ("Why?").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// A write's minimal result ([`minimal_result`]), on `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// What changed since the approval, on `stale` ("main moved from 1a2b3c4 to 5d6e7f8").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
}

/// The bytes a ticket event's signature covers: a domain-separation line, then the event's JSON.
#[must_use]
pub fn event_message(event: &TicketEvent) -> Vec<u8> {
    let mut out = b"reins/ticket-event/v1\n".to_vec();
    out.extend(serde_json::to_vec(event).unwrap_or_default());
    out
}

/// Twelve digits, in three groups, that name an approval device's signing key ("4821 9930 1274"): shown on the phone
/// (a connection's More), so an agent or the desktop app that pinned the key can compare it, and a key a server
/// swapped in shows as different digits. Spaces, never dashes: not the phone key's twelve digits for `reins vault add`.
#[must_use]
pub fn signing_key_fingerprint(key: &str) -> Option<String> {
    let raw: [u8; 32] = data_encoding::BASE64URL_NOPAD.decode(key.as_bytes()).ok()?.try_into().ok()?;
    let mut h = Sha256::new();
    h.update(b"reins-signing-key/1");
    h.update(raw);
    let d = h.finalize();
    let n = u64::from_be_bytes([0, 0, d[0], d[1], d[2], d[3], d[4], d[5]]) % 1_000_000_000_000;
    Some(format!("{:04} {:04} {:04}", n / 100_000_000, n / 10_000 % 10_000, n % 10_000))
}

/// A ticket event and the approval device's Ed25519 signature over [`event_message`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedTicketEvent {
    pub event: TicketEvent,
    /// The signer's public key, base64url without padding.
    pub key: String,
    /// base64url without padding.
    pub sig: String,
}

/// What the server seals to the approval device when a request becomes a ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedTicketBody {
    pub v: u32,
    pub request: RelayRequest,
}

/// A ticket as the approval device gets it (`GET /reins/api/pending`, `GET /reins/api/tickets`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketItem {
    pub id: RequestId,
    pub connection_id: ConnectionId,
    pub connection_label: String,
    pub created_at: i64,
    /// When it became a ticket (the window's start).
    pub deferred_at: i64,
    pub deadline: i64,
    pub state: TicketState,
    pub digest: String,
    /// [`SealedTicketBody`], sealed to this device's box key, base64url.
    pub sealed: String,
    /// The reviewer's open "Why?".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// The agent's answer to it, sealed to this device's box key, base64url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_reply: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
    /// An agent is collecting this approved ticket now: run it ([`Execution::OnCollect`]).
    #[serde(default)]
    pub collect: bool,
}

/// `POST /reins/api/tickets/<id>/events`: the phone's signed transitions, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketEvents {
    pub v: u32,
    pub events: Vec<SignedTicketEvent>,
    /// Decide later: the Deferred window asked for (clamped by the server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_expiry: Option<OnExpiry>,
}

/// The server's answer to [`TicketEvents`]: where the ticket stands now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketStatus {
    pub ticket: String,
    pub state: TicketState,
    pub deadline: i64,
}

/// A ticket as the desktop app sees it (`GET /reins/desktop/tickets`): never its body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopTicket {
    pub ticket: String,
    pub state: TicketState,
    pub deadline: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
}

/// `GET /reins/desktop/tickets`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopTickets {
    pub tickets: Vec<DesktopTicket>,
    /// The approval device's signing key (base64url), as the server says: the desktop app pins the first one it sees
    /// and refuses a changed one until the user compares [`signing_key_fingerprint`] with the phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
}

/// One line of a reviewer note, question or agent reply: control characters and surrounding space gone, at most
/// `max` characters; `None` when nothing is left.
#[must_use]
pub fn clean_text(raw: &str, max: usize) -> Option<String> {
    let text: String = raw
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect();
    (!text.is_empty()).then_some(text)
}

/// [`Effect`] of a connector call, for callers that only have the tool name.
#[must_use]
pub fn tool_effect(tool: &str) -> Option<Effect> {
    connector::spec_for_tool(tool).map(|s| s.effect)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        PROTOCOL_VERSION,
        connector::{ConnectorCall, specs},
        gmail::{MessageFull, MessageSummary, OutgoingEmail, SentMessage},
        remote_mcp::McpCall,
    };

    fn connector(tool: &str, args: &Value) -> ToolCall {
        let spec = connector::spec_for_tool(tool).unwrap();
        ToolCall::Connector(ConnectorCall {
            service: spec.service.to_owned(),
            op: spec.op.to_owned(),
            args: args.as_object().cloned().unwrap_or_default(),
        })
    }

    fn send() -> ToolCall {
        ToolCall::GmailSend {
            email: Box::new(OutgoingEmail {
                to: vec!["ann@acme.com".to_owned()],
                cc: vec![],
                subject: "Hi".to_owned(),
                body: "Hello".to_owned(),
                reply_to_message_id: None,
            }),
        }
    }

    #[test]
    fn ticket_ids_round_trip_and_reject_junk() {
        let id = RequestId("abc-1".to_owned());
        assert_eq!(ticket_of(&id), "rt_abc-1");
        assert_eq!(request_of("rt_abc-1"), Some(id.clone()));
        assert_eq!(request_of("abc-1"), Some(id), "a bare request id still works");
        for bad in ["", "rt_", "a/b", "rt_a b", "rt_\u{e9}"] {
            assert_eq!(request_of(bad), None, "{bad}");
        }
        assert_eq!(request_of(&"x".repeat(65)), None);
        assert!(request_of(&"x".repeat(64)).is_some());
    }

    #[test]
    fn windows_clamp_to_the_spec() {
        assert_eq!(clamp_window(TicketClass::Instant, None), 120);
        assert_eq!(clamp_window(TicketClass::Instant, Some(5_000)), 600);
        assert_eq!(clamp_window(TicketClass::Deferred, None), 14_400);
        assert_eq!(clamp_window(TicketClass::Deferred, Some(10_000_000)), 604_800);
        assert_eq!(clamp_window(TicketClass::Deferred, Some(0)), 60);
    }

    #[test]
    fn state_machine_follows_the_lifecycle() {
        use TicketState::*;
        let allowed: &[(TicketState, &[TicketState])] = &[
            (Received, &[Reviewing, Deferred, Approved, Denied, Cancelled]),
            (Reviewing, &[Approved, Denied, Deferred, NeedsInfo, Cancelled]),
            (Deferred, &[Reviewing, Approved, Denied, NeedsInfo, Expired, Escalated, Cancelled]),
            (NeedsInfo, &[Deferred, Approved, Denied, Expired, Cancelled]),
            (Escalated, &[Reviewing, Approved, Denied, Expired, Cancelled]),
            (Approved, &[Revalidating, Executing, Done, Failed, Stale, Expired]),
            (Revalidating, &[Executing, Stale]),
            (Executing, &[Done, Failed, Unknown]),
            (Stale, &[Reviewing, Approved, Denied, Expired, Cancelled]),
        ];
        for from in TicketState::ALL {
            let to: &[TicketState] = allowed.iter().find(|(f, _)| *f == from).map_or(&[], |(_, t)| t);
            for target in TicketState::ALL {
                assert_eq!(from.can_go(target), to.contains(&target), "{from:?} -> {target:?}");
            }
        }
        for (from, to) in [(Done, Approved), (Denied, Approved), (Expired, Deferred), (Deferred, Done)] {
            assert!(!from.can_go(to), "{from:?} -> {to:?}");
        }
        let terminal: Vec<_> = TicketState::ALL.into_iter().filter(|s| s.is_terminal()).collect();
        assert_eq!(terminal, [Done, Failed, Unknown, Denied, Expired, Cancelled]);
        let open: Vec<_> = TicketState::ALL.into_iter().filter(|s| s.is_open()).collect();
        assert_eq!(open, [Received, Reviewing, Deferred, NeedsInfo, Escalated, Stale]);
        for s in TicketState::ALL {
            assert_eq!(TicketState::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        assert_eq!(Approved.webhook_type(), Some("ticket.approved"));
        assert_eq!(Deferred.webhook_type(), None);
    }

    #[test]
    fn execution_is_on_collect_for_reads_and_on_approve_for_writes() {
        let search = ToolCall::GmailSearch {
            query: "x".to_owned(),
            max_results: 1,
        };
        assert_eq!(execution(&search), Execution::OnCollect);
        assert_eq!(execution(&send()), Execution::OnApprove);
        let write = specs().iter().find(|s| s.effect == Effect::Write && !s.desktop_only).unwrap();
        assert_eq!(execution(&connector(write.tool, &json!({}))), Execution::OnApprove, "{}", write.tool);
        let read = specs().iter().find(|s| s.effect == Effect::Read && !s.desktop_only).unwrap();
        assert_eq!(execution(&connector(read.tool, &json!({}))), Execution::OnCollect, "{}", read.tool);
        let mcp = ToolCall::Mcp(McpCall {
            server: "linear".to_owned(),
            tool: "search".to_owned(),
            arguments: Map::new(),
        });
        assert_eq!(execution(&mcp), Execution::OnApprove, "a third party's read hint is not trusted");
        let ask = connector("desktop_ask", &json!({}));
        assert_eq!(execution(&ask), Execution::OnCollect);
        assert_eq!(approval_validity(&ask), 3600);
        assert_eq!(approval_validity(&send()), 86_400);
    }

    #[test]
    fn digest_is_stable_and_binds_connection_account_and_call() {
        let conn = ConnectionId("c1".to_owned());
        let call = connector("github_create_issue", &json!({"repo": "o/r", "title": "Bug"}));
        let d = request_digest(&conn, Some("me"), &call);
        assert_eq!(d.len(), 64);
        let again: ToolCall = serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        assert_eq!(request_digest(&conn, Some("me"), &again), d, "survives the relay's JSON");
        assert_ne!(request_digest(&ConnectionId("c2".to_owned()), Some("me"), &call), d);
        assert_ne!(request_digest(&conn, None, &call), d);
        let other = connector("github_create_issue", &json!({"repo": "o/r", "title": "Bug!"}));
        assert_ne!(request_digest(&conn, Some("me"), &other), d);
    }

    #[test]
    fn minimal_results_drop_content() {
        let data = json!({"id": 7, "number": 12, "html_url": "https://x/12", "title": "Secret", "body": "text",
            "user": {"login": "a"}});
        assert_eq!(
            minimal_result(&ToolResult::Connector {
                data
            }),
            json!({"id": 7, "number": 12, "html_url": "https://x/12"})
        );
        let summary = MessageSummary {
            id: "m".to_owned(),
            thread_id: "t".to_owned(),
            from: "a@b.c".to_owned(),
            from_name: None,
            to: vec![],
            cc: vec![],
            subject: "Private".to_owned(),
            date: 0,
            snippet: "secret".to_owned(),
        };
        let read = ToolResult::Read {
            messages: vec![MessageFull {
                summary,
                body_text: "secret".to_owned(),
            }],
        };
        assert_eq!(minimal_result(&read), json!({}));
        let sent = ToolResult::Sent(SentMessage {
            id: "m1".to_owned(),
            thread_id: "t1".to_owned(),
        });
        assert_eq!(minimal_result(&sent), json!({"id": "m1", "thread_id": "t1"}));
        let mcp = ToolResult::Mcp {
            result: json!({"content": [{"type": "text", "text": "secret"}], "isError": false}),
        };
        assert_eq!(minimal_result(&mcp), json!({"isError": false}));
        assert_eq!(
            minimal_value(
                &json!({"id": 1, "body": "secret", "html_url": "u", "nested": {"id": 2}, "url": "x".repeat(400)})
            ),
            json!({"id": 1, "html_url": "u"}),
            "content, nesting and long strings are dropped"
        );
        assert_eq!(minimal_value(&json!("text")), json!({}));
    }

    fn event(note: Option<&str>) -> TicketEvent {
        TicketEvent {
            v: PROTOCOL_VERSION,
            ticket: "rt_r1".to_owned(),
            from: TicketState::Deferred,
            state: TicketState::Denied,
            at: 1,
            digest: "d".repeat(64),
            connection_id: ConnectionId("c1".to_owned()),
            deadline: None,
            note: note.map(str::to_owned),
            result: None,
            stale: None,
        }
    }

    #[test]
    fn event_message_is_domain_separated() {
        let a = event_message(&event(None));
        assert!(a.starts_with(b"reins/ticket-event/v1\n"));
        assert_ne!(a, event_message(&event(Some("use staging"))));
        let wire = serde_json::to_value(event(None)).unwrap();
        assert!(wire.get("note").is_none(), "absent fields stay absent, so a signature survives a round trip");
        let back: TicketEvent = serde_json::from_value(wire).unwrap();
        assert_eq!(event_message(&back), a);
    }

    #[test]
    fn old_wire_forms_still_parse() {
        let pending: crate::device::Pending = serde_json::from_str(r#"{"requests":[],"pairings":[]}"#).unwrap();
        assert!(pending.tickets.is_empty());
        let response: crate::relay::RelayResponse =
            serde_json::from_str(r#"{"v":1,"outcome":"denied","reason":null}"#).unwrap();
        assert!(response.events.is_empty());
        let ask: crate::desktop::AskAnswer = serde_json::from_str(r#"{"v":1,"nonce":"n","approved":true}"#).unwrap();
        assert_eq!((ask.digest, ask.expires_at), (None, None));
        let reg: crate::device::DeviceRegistration = serde_json::from_str(r#"{"fcm_token":null}"#).unwrap();
        assert_eq!((reg.sign_pub, reg.box_pub), (None, None));
    }

    #[test]
    fn signing_keys_have_a_stable_twelve_digit_name() {
        let key = data_encoding::BASE64URL_NOPAD.encode(&[7u8; 32]);
        let fp = signing_key_fingerprint(&key).unwrap();
        assert_eq!(fp.len(), 14);
        assert_eq!(fp.split(' ').count(), 3);
        assert_eq!(signing_key_fingerprint(&key), Some(fp.clone()));
        assert_ne!(signing_key_fingerprint(&data_encoding::BASE64URL_NOPAD.encode(&[8u8; 32])), Some(fp));
        assert_eq!(signing_key_fingerprint("short"), None);
    }

    #[test]
    fn the_from_state_is_signed() {
        let mut other = event(None);
        other.from = TicketState::Stale;
        assert_ne!(event_message(&event(None)), event_message(&other));
    }

    #[test]
    fn notes_are_one_clean_bounded_text() {
        assert_eq!(clean_text("  use\u{7}staging \n", 500).as_deref(), Some("use staging"));
        assert_eq!(clean_text(" \t ", 500), None);
        assert_eq!(clean_text(&"x".repeat(600), 500).map(|s| s.chars().count()), Some(500));
    }
}
