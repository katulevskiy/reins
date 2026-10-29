//! The MCP tools Reins exposes (spec §4.2): schemas, strict argument parsing and result
//! rendering (Decisions 9, 10, 14). Unknown properties are rejected, never dropped, so an
//! approved action is exactly what the AI asked for.

use chrono::{DateTime, SecondsFormat};
use reins_proto::{
    batch::MAX_BATCH_CALLS,
    blob::MAX_BLOB_BYTES,
    connector,
    gmail::{
        GrantAction, GrantRequest, MAX_ANY_GRANT_SECS, MAX_GRANT_RULES, MAX_GRANT_SECS, MAX_READ_IDS, MAX_REASON_LEN,
        MAX_RECIPIENTS, MAX_SEARCH_RESULTS, MIN_GRANT_SECS, MessageSummary, OutgoingEmail, ToolCall, normalize_account,
    },
    ids::RequestId,
    permission::{self, MAX_RESOURCE_LEN},
    relay::{RelayOutcome, ToolResult},
    remote_mcp::McpServerReport,
};
use serde_json::{Map, Value, json};

use reins_proto::ticket::{MAX_NOTE_CHARS, MAX_REPLY_CHARS, MAX_WAIT_SECS, clean_text, request_of, ticket_of};

use super::{
    mcp_tools,
    presence::PhoneSeen,
    relay::WaitResult,
    tickets::{self, AgentView},
};

/// Applied by the server when the AI omits `max_results` (contracts C).
pub const DEFAULT_MAX_RESULTS: u32 = 10;
/// Always listed: hands the AI an upload link for a file it wants to pass on (files spec, S3).
pub const UPLOAD_TOOL: &str = "reins_upload";

pub const DENIED_TEXT: &str = "Denied by the user on their Reins device.";

/// The phone did not pick the request up: when it was last heard from, and how long the request waits for it.
pub fn offline_text(id: &RequestId, seen: PhoneSeen) -> String {
    let when = seen.phrase().map(|p| format!(" (last seen {p})")).unwrap_or_default();
    let minutes = super::ITEM_TTL.as_secs() / 60;
    format!(
        "Reins: your approval device is offline{when}. Ask the user to open the Reins app; the request waits there \
for up to {minutes} minutes. Then call reins_get_result with request_id={id}."
    )
}

pub fn pending_text(id: &RequestId) -> String {
    format!(
        "Waiting for the user to approve on their phone; nothing has happened yet. Tell the user to approve it in \
the Reins app. Do not call the tool again: that would ask a second time. After the user responds (they can still \
approve after this message), call reins_get_result with request_id={id}."
    )
}

/// Listed in no `tools/list` but still callable: `github_request_*` duplicates `github_api_*` (same GitHub REST API,
/// read and write), and every listed tool counts against the clients that cap how many a connector may have.
pub const UNLISTED_TOOLS: [&str; 2] = ["github_request_read", "github_request_write"];

fn account_property() -> Value {
    json!({
        "type": "string",
        "description": "Which connected account to use: an address from reins_list_accounts. Optional when only \
        one account is connected."
    })
}

/// The annotations of a tool that only reads the user's own data, all four set (the directories check them).
fn read_hints() -> Value {
    json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false})
}

/// The annotations of a tool that changes something or sends something out (never idempotent here).
fn write_hints(destructive: bool, open_world: bool) -> Value {
    json!({"readOnlyHint": false, "destructiveHint": destructive, "idempotentHint": false, "openWorldHint": open_world})
}

/// Every tool in a fixed order (`tools/list` must be deterministic), whatever the user has connected: an AI client
/// reads the list once per chat, so a tool left out stays missing even after the user connects its integration.
/// A call to an integration that is not connected goes to the phone like any other, which answers at once with how
/// to connect it: the phone knows the moment an account is added, the server only at the phone's next report.
/// Desktop-only tools are never listed.
pub fn tool_definitions() -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "gmail_read",
            "title": "Read Gmail messages",
            "description": "Read the full text of specific Gmail messages by id (ids come from gmail_search). \
        Attached files are not included: list them with gmail_list_attachments. The user may have to approve on their \
        phone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_property(),
                    "message_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                        "maxItems": MAX_READ_IDS,
                        "description": "Gmail message ids to read."
                    }
                },
                "required": ["message_ids"],
                "additionalProperties": false
            },
            "annotations": read_hints()
        }),
        json!({
            "name": "gmail_search",
            "title": "Search Gmail",
            "description": "Search the user's Gmail with Gmail search syntax (for example `from:bank newer_than:7d`) and \
        returns sender, recipients, subject, date and a snippet for each match. The user may have to approve on their phone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_property(),
                    "query": {"type": "string", "minLength": 1, "description": "Gmail search query."},
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_SEARCH_RESULTS,
                        "default": DEFAULT_MAX_RESULTS,
                        "description": "Maximum number of messages to return."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_hints()
        }),
        json!({
            "name": "gmail_send",
            "title": "Send an email",
            "description": "Send an email from the user's Gmail account. Call it directly for each email: the user \
        sees the recipients and the full text on their phone and approves it there (unless a standing permission \
        already covers it), so no permission is needed first. Bcc is not supported.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_property(),
                    "to": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": MAX_RECIPIENTS,
                        "description": "Recipient email addresses (plain addresses, no display names)."},
                    "cc": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_RECIPIENTS,
                        "description": "Cc email addresses."},
                    "subject": {"type": "string", "description": "Subject line."},
                    "body": {"type": "string", "description": "Plain-text body."},
                    "reply_to_message_id": {"type": "string",
                        "description": "Gmail message id to reply to, to keep the conversation thread."}
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": false
            },
            "annotations": write_hints(true, true)
        }),
        json!({
            "name": "reins_request_access",
            "title": "Ask for a standing permission",
            "description": "Ask the user, on their phone, for a time-limited standing permission so that many similar \
        calls over a while need no approval each (for example reading every email from one sender this week, or \
        commenting on issues of one repository this month). Do NOT use it for a single action, and never as a step \
        before one: call the tool directly and the user approves that call. No tool needs a permission first. When \
        you do need one, ask for the NARROWEST permission and the SHORTEST duration. `read` covers gmail_search and \
        gmail_read (name the exact senders `a@b.com`, domains `@b.com` or a subject text; `any: true` is all mail for \
        at most 7 days). `send` covers gmail_send to the listed recipients. `write` covers ONE other tool that \
        changes something (its `_meta` says `standing_grant: write`), only on the one `resource` you name (for \
        example the repository `owner/name`), for at most 30 days; a tool of an MCP server the user added is named \
        as listed and needs no `resource`. Tools asked for every time (deleting, purchases) are never covered. The \
        user sees and can revoke every permission on their phone, and each expires on its own. Always give a short, \
        honest `reason`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_property(),
                    "action": {"type": "string", "enum": ["read", "send", "write"],
                        "description": "`read` covers gmail_search and gmail_read, `send` covers gmail_send, `write` \
                        covers the one `tool` on the one `resource`."},
                    "duration_seconds": {"type": "integer", "minimum": MIN_GRANT_SECS, "maximum": MAX_GRANT_SECS,
                        "description": "How long the permission lasts (at most 30 days). Ask for as little as you need."},
                    "tool": {"type": "string",
                        "description": "write: the tool it covers, exactly as listed (e.g. `github_issue_comment_edit`)."},
                    "resource": {"type": "string", "maxLength": MAX_RESOURCE_LEN,
                        "description": "write: the one thing it covers, the value of the tool's `resource_param` \
                        (e.g. the repository `owner/name`). Not for MCP tools."},
                    "reason": {"type": "string", "maxLength": MAX_REASON_LEN,
                        "description": "Why you need it; shown to the user."},
                    "from": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_GRANT_RULES,
                        "description": "read: sender addresses (`a@b.com`) or whole domains (`@b.com`)."},
                    "subject_contains": {"type": "string",
                        "description": "read or send: the subject must contain this text."},
                    "recipients": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_GRANT_RULES,
                        "description": "send: recipient addresses (`a@b.com`) or domains (`@b.com`)."},
                    "max_uses": {"type": "integer", "minimum": 1,
                        "description": "Optional cap on how many calls it may cover (1 = a single call)."},
                    "any": {"type": "boolean",
                        "description": format!("read only: all of the user's mail, at most {MAX_ANY_GRANT_SECS} seconds. \
                        Cannot be combined with from/subject_contains.")}
                },
                "required": ["action", "duration_seconds", "reason"],
                "additionalProperties": false
            },
            "annotations": write_hints(false, false)
        }),
        json!({
            "name": BATCH_TOOL,
            "title": "Make several calls at once",
            "description": format!("Make up to {MAX_BATCH_CALLS} tool calls at once, for example labelling five \
        issues. Each call is exactly what it would be alone: the user sees them together on their phone as one queue, \
        approves or denies each (or approves them all), each is recorded on its own, and the answer has one result \
        per call, in order, under `structuredContent.items`. Only tools that act can be batched (not reins_batch, \
        reins_get_result, reins_wait, reins_tickets, reins_cancel or reins_reply). Use it only for calls that belong \
        together."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_BATCH_CALLS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "description": "The tool, as listed."},
                                "arguments": {"type": "object", "description": "Its arguments."}
                            },
                            "required": ["name", "arguments"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["calls"],
                "additionalProperties": false
            },
            // What it does is what its calls do: some may change things.
            "annotations": write_hints(false, true)
        }),
        json!({
            "name": "reins_list_accounts",
            "title": "List connected services and accounts",
            "description": "List the services the user has connected in Reins (for example gmail, github) when called \
        without `service`; needs no approval and shows no accounts. With `service`: asks the user, on their phone, to \
        let you see that service's accounts (for example the Gmail addresses). Once they agree (usually for a month) \
        you can pass the one you want as `account` to that service's tools. Only ask for the accounts when you need \
        to choose between several.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "service": {"type": "string",
                        "description": "An integration from the first call, e.g. `gmail`. Omit to list the integrations."},
                    "ask_for_more": {"type": "boolean",
                        "description": "Set to true when an earlier answer said accounts were withheld and you need one of them: \
                        the user is asked whether to share more."}
                },
                "required": [],
                "additionalProperties": false
            },
            // With `service` it asks the phone for a permission to see the accounts (usually for a month): a write.
            "annotations": write_hints(false, false)
        }),
        json!({
            "name": "reins_get_result",
            "title": "Get the result of an approved request",
            "description": "Get the result of an earlier request that was waiting for the user's approval (pass the \
        request_id or the ticket from that answer). Waits a while if it is still pending. Until this returns the \
        result, the action has not happened.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request_id": {"type": "string", "description": "The request_id to look up."},
                    "ticket": {"type": "string", "description": "The ticket (rt_...) to look up."}
                },
                "required": [],
                "additionalProperties": false
            },
            "annotations": read_hints()
        }),
        json!({
            "name": "reins_wait",
            "title": "Wait for a ticket",
            "description": "Block until a ticket (rt_...) is decided or max_seconds pass (at most 600), then return \
        where it stands, with the result when it was approved.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ticket": {"type": "string"},
                    "max_seconds": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT_SECS,
                        "description": "Default 55."}
                },
                "required": ["ticket"],
                "additionalProperties": false
            },
            "annotations": read_hints()
        }),
        json!({
            "name": "reins_tickets",
            "title": "List your tickets",
            "description": "List this connection's tickets: those waiting for review, and those decided in the last day.",
            "inputSchema": {"type": "object", "properties": {}, "required": [], "additionalProperties": false},
            "annotations": read_hints()
        }),
        json!({
            "name": "reins_cancel",
            "title": "Withdraw a request",
            "description": "Withdraw a request or ticket that is still waiting for review (it is no longer needed).",
            "inputSchema": {
                "type": "object",
                "properties": {"ticket": {"type": "string", "description": "The ticket (rt_...) or request_id."}},
                "required": ["ticket"],
                "additionalProperties": false
            },
            "annotations": write_hints(false, false)
        }),
        json!({
            "name": "reins_reply",
            "title": "Answer the reviewer's question",
            "description": "Answer a reviewer who asked why (the ticket's state is needs_info). The answer is shown \
        to them as yours; it never changes what the request does.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ticket": {"type": "string"},
                    "text": {"type": "string", "minLength": 1, "maxLength": MAX_REPLY_CHARS}
                },
                "required": ["ticket", "text"],
                "additionalProperties": false
            },
            "annotations": write_hints(false, false)
        }),
        json!({
            "name": UPLOAD_TOOL,
            "title": "Upload a file to pass on as a link",
            "description": "Upload a file to pass to another tool as a link (for example a large file another tool \
        should download). The phone answers with an upload link: upload the file there (`curl -T <file> '<upload_url>'`). The \
        user approves it on their phone when it arrives; then the download link from the same answer works. For files \
        a Reins tool takes directly (release assets, repository files, attachments) call that tool instead: it \
        answers with its own upload link when it needs one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 200,
                        "description": "The file name, without a path."},
                    "size": {"type": "integer", "minimum": 1, "maximum": MAX_BLOB_BYTES,
                        "description": "The file size in bytes."},
                    "content_type": {"type": "string", "description": "The media type, if known."},
                    "reason": {"type": "string", "minLength": 1, "maxLength": 300,
                        "description": "What the file is for; shown to the user."}
                },
                "required": ["name", "size", "reason"],
                "additionalProperties": false
            },
            "annotations": write_hints(false, false)
        }),
    ];
    // The other integrations describe their tools as data (reins-proto).
    // Desktop-only tools answer with credentials for the paired desktop app: never offered to an AI.
    tools.extend(connector::specs().iter().filter(|s| !s.desktop_only && !UNLISTED_TOOLS.contains(&s.tool)).map(|s| {
        json!({
            "name": s.tool,
            "title": s.title,
            "description": s.description,
            "inputSchema": s.input_schema(),
            "annotations": s.annotations()
        })
    }));
    for tool in &mut tools {
        let meta = tool["name"].as_str().and_then(permission::for_tool).map(|p| p.meta());
        if let (Some(meta), Some(obj)) = (meta, tool.as_object_mut()) {
            obj.insert("_meta".to_owned(), json!({ permission::META_KEY: meta }));
        }
    }
    tools
}

/// [`tool_definitions`] plus the tools of the MCP servers the user added on the phone, after Reins's own.
pub fn tool_definitions_with(mcp: &[McpServerReport]) -> Vec<Value> {
    let mut tools = tool_definitions();
    tools.extend(mcp_tools::tool_definitions(mcp));
    tools
}

#[derive(Debug, PartialEq)]
pub enum ToolInvocation {
    /// A normalized call to relay to the phone.
    Relay(ToolCall, Option<String>),
    GetResult(RequestId),
    /// `reins_wait`: a ticket and the most seconds to wait.
    Wait(RequestId, u64),
    Cancel(RequestId),
    Tickets,
    /// `reins_reply`: a ticket and the agent's answer.
    Reply(RequestId, String),
    /// `reins_batch`: each call as it would be relayed alone, or why it cannot be.
    Batch(Vec<BatchCall>),
}

/// One call of a `reins_batch`.
#[derive(Debug, PartialEq)]
pub struct BatchCall {
    /// The tool, as the AI named it.
    pub name: String,
    /// The call for the phone and its account, or why it is refused (only this call fails).
    pub call: Result<(ToolCall, Option<String>), String>,
}

/// Several calls at once, shown together on the phone ([`reins_proto::batch`]).
pub const BATCH_TOOL: &str = "reins_batch";

/// The tools that cannot be part of a batch: they do not act, they look after other requests.
const NOT_BATCHABLE: [&str; 6] =
    [BATCH_TOOL, "reins_get_result", "reins_wait", "reins_tickets", "reins_cancel", "reins_reply"];

fn parse_batch(arguments: &Value, mcp: &[McpServerReport]) -> Result<ToolInvocation, ToolArgError> {
    let args = object(arguments)?;
    reject_unknown(args, &["calls"])?;
    let Some(Value::Array(calls)) = args.get("calls") else {
        return invalid("`calls` must be an array of {name, arguments}.");
    };
    if calls.is_empty() || calls.len() > MAX_BATCH_CALLS {
        return invalid(format!("`calls` must hold 1 to {MAX_BATCH_CALLS} calls."));
    }
    let mut out = Vec::with_capacity(calls.len());
    for (index, item) in calls.iter().enumerate() {
        let Some(item) = item.as_object() else {
            return invalid(format!("`calls[{index}]` must be an object with `name` and `arguments`."));
        };
        reject_unknown(item, &["name", "arguments"])?;
        let name = required_string(item, "name")?;
        let call = if NOT_BATCHABLE.contains(&name.as_str()) {
            Err(format!("{name} cannot be part of a batch; call it on its own."))
        } else {
            match parse_invocation_with(&name, item.get("arguments").unwrap_or(&Value::Null), mcp) {
                Ok(ToolInvocation::Relay(call, account)) => Ok((call, account)),
                Ok(_) => Err(format!("{name} cannot be part of a batch; call it on its own.")),
                Err(ToolArgError::UnknownTool(_)) => Err(format!("Unknown tool `{name}`.")),
                Err(ToolArgError::Invalid(why)) => Err(why),
            }
        };
        out.push(BatchCall {
            name,
            call,
        });
    }
    Ok(ToolInvocation::Batch(out))
}

/// What a `reins_request_access` write permission names: the integration and operation of `tool` (a connector tool,
/// or an added MCP server's tool as listed) and the one thing it covers.
fn write_grant_target(
    tool: &str,
    resource: Option<String>,
    mcp: &[McpServerReport],
) -> Result<(String, String, Option<String>), ToolArgError> {
    if tool == "gmail_send" {
        return invalid("For gmail_send ask with `action: \"send\"` and the `recipients`.");
    }
    if let Some(spec) = connector::spec_for_tool(tool).filter(|s| !s.desktop_only) {
        return Ok((spec.service.to_owned(), spec.op.to_owned(), resource));
    }
    match mcp_tools::grant_target(mcp, tool) {
        Some(Ok((service, op))) if resource.is_none() => Ok((service, op, None)),
        Some(Ok(_)) => invalid("An MCP tool's permission covers the tool itself: leave out `resource`."),
        Some(Err(why)) => invalid(why),
        None => invalid(format!("Unknown tool `{tool}`: name a tool as listed.")),
    }
}

/// Default `reins_wait`: below the 60 s tool timeout some clients have.
pub const DEFAULT_WAIT_SECS: u64 = 55;

fn ticket_arg(args: &Map<String, Value>, key: &str) -> Result<RequestId, ToolArgError> {
    let raw = required_string(args, key)?;
    request_of(&raw).map_or_else(|| invalid(format!("`{key}` is not valid.")), Ok)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ToolArgError {
    /// JSON-RPC `-32602`.
    UnknownTool(String),
    /// An `isError` tool result the model can act on.
    Invalid(String),
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ToolArgError> {
    Err(ToolArgError::Invalid(message.into()))
}

fn object(arguments: &Value) -> Result<&Map<String, Value>, ToolArgError> {
    arguments.as_object().ok_or_else(|| ToolArgError::Invalid("Tool arguments must be a JSON object.".to_owned()))
}

fn reject_unknown(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), ToolArgError> {
    for key in args.keys() {
        if !allowed.contains(&key.as_str()) {
            let hint = if key == "bcc" {
                " Reins cannot send Bcc."
            } else {
                ""
            };
            return invalid(format!("Unknown property `{key}`.{hint} Allowed properties: {}.", allowed.join(", ")));
        }
    }
    Ok(())
}

fn required_string(args: &Map<String, Value>, key: &str) -> Result<String, ToolArgError> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => invalid(format!("`{key}` must be a string.")),
        None => invalid(format!("`{key}` is required.")),
    }
}

fn optional_string(args: &Map<String, Value>, key: &str) -> Result<Option<String>, ToolArgError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => invalid(format!("`{key}` must be a string.")),
    }
}

fn string_list(args: &Map<String, Value>, key: &str, required: bool) -> Result<Vec<String>, ToolArgError> {
    match args.get(key) {
        None | Some(Value::Null) if !required => Ok(Vec::new()),
        None | Some(Value::Null) => invalid(format!("`{key}` is required.")),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ToolArgError::Invalid(format!("`{key}` must contain only strings.")))
            })
            .collect(),
        Some(_) => invalid(format!("`{key}` must be an array of strings.")),
    }
}

fn normalize(call: ToolCall, args: &Map<String, Value>) -> Result<ToolInvocation, ToolArgError> {
    let account = optional_string(args, "account")?
        .map(|a| normalize_account(&a).map_err(|e| ToolArgError::Invalid(e.to_string())))
        .transpose()?;
    call.normalized().map(|c| ToolInvocation::Relay(c, account)).map_err(|e| ToolArgError::Invalid(e.to_string()))
}

/// Maps a `tools/call` to something runnable; validation failures are model-visible.
#[cfg(test)]
pub fn parse_invocation(name: &str, arguments: &Value) -> Result<ToolInvocation, ToolArgError> {
    parse_invocation_with(name, arguments, &[])
}

/// [`parse_invocation`] that also knows the tools of the MCP servers the user added (`mcp`).
pub fn parse_invocation_with(
    name: &str,
    arguments: &Value,
    mcp: &[McpServerReport],
) -> Result<ToolInvocation, ToolArgError> {
    match name {
        UPLOAD_TOOL => {
            let args = object(arguments)?;
            reject_unknown(args, &["name", "size", "content_type", "reason"])?;
            let Some(size) = args.get("size").and_then(Value::as_u64) else {
                return invalid("`size` must be the file size in bytes (a positive integer).");
            };
            normalize(
                ToolCall::RequestUpload {
                    name: required_string(args, "name")?,
                    size,
                    content_type: optional_string(args, "content_type")?,
                    reason: required_string(args, "reason")?,
                },
                args,
            )
        }
        "gmail_search" => {
            let args = object(arguments)?;
            reject_unknown(args, &["account", "query", "max_results"])?;
            let query = required_string(args, "query")?;
            let max_results = match args.get("max_results") {
                None | Some(Value::Null) => DEFAULT_MAX_RESULTS,
                Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    Some(n) => n,
                    None => return invalid("`max_results` must be a positive integer."),
                },
            };
            normalize(
                ToolCall::GmailSearch {
                    query,
                    max_results,
                },
                args,
            )
        }
        "gmail_read" => {
            let args = object(arguments)?;
            reject_unknown(args, &["account", "message_ids"])?;
            normalize(
                ToolCall::GmailRead {
                    message_ids: string_list(args, "message_ids", true)?,
                },
                args,
            )
        }
        "gmail_send" => {
            let args = object(arguments)?;
            reject_unknown(args, &["account", "to", "cc", "subject", "body", "reply_to_message_id"])?;
            normalize(
                ToolCall::GmailSend {
                    email: Box::new(OutgoingEmail {
                        to: string_list(args, "to", true)?,
                        cc: string_list(args, "cc", false)?,
                        subject: required_string(args, "subject")?,
                        body: required_string(args, "body")?,
                        reply_to_message_id: optional_string(args, "reply_to_message_id")?,
                    }),
                },
                args,
            )
        }
        "reins_request_access" => {
            let args = object(arguments)?;
            reject_unknown(
                args,
                &[
                    "account",
                    "action",
                    "duration_seconds",
                    "reason",
                    "from",
                    "subject_contains",
                    "recipients",
                    "max_uses",
                    "any",
                    "tool",
                    "resource",
                ],
            )?;
            let action = match required_string(args, "action")?.as_str() {
                "read" => GrantAction::Read,
                "send" => GrantAction::Send,
                "write" => GrantAction::Write,
                _ => return invalid("`action` must be \"read\", \"send\" or \"write\"."),
            };
            let tool = optional_string(args, "tool")?;
            let resource = optional_string(args, "resource")?;
            let (service, op, resource) = match (action, tool.as_deref()) {
                (GrantAction::Write, Some(name)) => {
                    let (service, op, resource) = write_grant_target(name, resource, mcp)?;
                    (Some(service), Some(op), resource)
                }
                (GrantAction::Write, None) => return invalid("`tool` is required for a write permission."),
                (_, None) if resource.is_none() => (None, None, None),
                _ => return invalid("`tool` and `resource` are only for `action: \"write\"`."),
            };
            if action == GrantAction::Write
                && args.contains_key("account")
                && service.as_deref().is_some_and(|s| s.starts_with(permission::MCP_SERVICE_PREFIX))
            {
                return invalid("An MCP tool's permission takes no `account`.");
            }
            let duration_secs = match args.get("duration_seconds") {
                Some(v) => match v.as_u64() {
                    Some(n) => n,
                    None => return invalid("`duration_seconds` must be a positive integer."),
                },
                None => return invalid("`duration_seconds` is required."),
            };
            let max_uses = match args.get("max_uses") {
                None | Some(Value::Null) => None,
                Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    Some(n) => Some(n),
                    None => return invalid("`max_uses` must be a positive integer."),
                },
            };
            let any = match args.get("any") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return invalid("`any` must be true or false."),
            };
            normalize(
                ToolCall::RequestGrant {
                    grant: Box::new(GrantRequest {
                        action,
                        duration_secs,
                        max_uses,
                        reason: required_string(args, "reason")?,
                        any,
                        from: string_list(args, "from", false)?,
                        subject_contains: optional_string(args, "subject_contains")?,
                        recipients: string_list(args, "recipients", false)?,
                        service,
                        op,
                        resource,
                        tool,
                    }),
                },
                args,
            )
        }
        "reins_list_accounts" => {
            let args = object(arguments)?;
            reject_unknown(args, &["service", "ask_for_more"])?;
            let ask_for_more = match args.get("ask_for_more") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return invalid("`ask_for_more` must be true or false."),
            };
            normalize(
                ToolCall::ListAccounts {
                    service: optional_string(args, "service")?,
                    ask_for_more,
                },
                args,
            )
        }
        "reins_get_result" => {
            let args = object(arguments)?;
            reject_unknown(args, &["request_id", "ticket"])?;
            let key = match (args.contains_key("request_id"), args.contains_key("ticket")) {
                (true, false) => "request_id",
                (false, true) => "ticket",
                _ => return invalid("Give either `request_id` or `ticket`."),
            };
            let id = required_string(args, key)?;
            if id.is_empty() || id.len() > 64 {
                return invalid(format!("`{key}` is not valid."));
            }
            Ok(ToolInvocation::GetResult(ticket_arg(args, key)?))
        }
        "reins_wait" => {
            let args = object(arguments)?;
            reject_unknown(args, &["ticket", "max_seconds"])?;
            let secs = match args.get("max_seconds") {
                None => DEFAULT_WAIT_SECS,
                Some(v) => match v.as_u64() {
                    Some(n) if (1..=MAX_WAIT_SECS).contains(&n) => n,
                    _ => return invalid(format!("`max_seconds` must be 1 to {MAX_WAIT_SECS}.")),
                },
            };
            Ok(ToolInvocation::Wait(ticket_arg(args, "ticket")?, secs))
        }
        BATCH_TOOL => parse_batch(arguments, mcp),
        "reins_tickets" => {
            reject_unknown(object(arguments)?, &[])?;
            Ok(ToolInvocation::Tickets)
        }
        "reins_cancel" => {
            let args = object(arguments)?;
            reject_unknown(args, &["ticket"])?;
            Ok(ToolInvocation::Cancel(ticket_arg(args, "ticket")?))
        }
        "reins_reply" => {
            let args = object(arguments)?;
            reject_unknown(args, &["ticket", "text"])?;
            let raw = required_string(args, "text")?;
            if raw.chars().count() > MAX_REPLY_CHARS {
                return invalid(format!("`text` is longer than {MAX_REPLY_CHARS} characters."));
            }
            let text = clean_text(&raw, MAX_REPLY_CHARS).map_or_else(|| invalid("`text` is empty."), Ok)?;
            Ok(ToolInvocation::Reply(ticket_arg(args, "ticket")?, text))
        }
        other => match connector::spec_for_tool(other).filter(|s| !s.desktop_only) {
            Some(spec) => {
                let (call, account) = spec.parse(arguments).map_err(ToolArgError::Invalid)?;
                Ok(ToolInvocation::Relay(ToolCall::Connector(call), account))
            }
            None => match mcp_tools::parse_call(mcp, other, arguments) {
                Some(call) => call.map(|c| ToolInvocation::Relay(c, None)).map_err(ToolArgError::Invalid),
                None => Err(ToolArgError::UnknownTool(other.to_owned())),
            },
        },
    }
}

// ---------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------

/// A successful tool result: `structuredContent` plus the same JSON as a text block.
pub fn success(structured: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": structured.to_string()}],
        "structuredContent": structured,
        "isError": false
    })
}

/// The answer to a `reins_batch`: one text block per call (`[index] tool: ...`) and, structured, each call's own
/// result with its request id. An error only when every call failed.
pub fn render_batch(batch_id: &str, items: Vec<(String, Option<RequestId>, Value)>) -> Value {
    let all_failed = items.iter().all(|(_, _, r)| r["isError"].as_bool().unwrap_or(false));
    let mut content = Vec::with_capacity(items.len());
    let mut structured = Vec::with_capacity(items.len());
    for (index, (tool, request_id, result)) in items.into_iter().enumerate() {
        let text = result["content"]
            .as_array()
            .map(|blocks| blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        content.push(json!({"type": "text", "text": format!("[{index}] {tool}: {text}")}));
        structured.push(json!({
            "index": index,
            "tool": tool,
            "request_id": request_id.map(|id| id.0),
            "result": result
        }));
    }
    json!({
        "content": content,
        "structuredContent": {"batch_id": batch_id, "items": structured},
        "isError": all_failed
    })
}

/// A tool-level error the model can read and relay to the user.
pub fn failure(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn rfc3339(unix: i64) -> String {
    DateTime::from_timestamp(unix, 0).map_or_else(String::new, |d| d.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn summary_json(m: &MessageSummary) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".to_owned(), json!(m.id));
    out.insert("thread_id".to_owned(), json!(m.thread_id));
    out.insert("from".to_owned(), json!(m.from));
    if let Some(name) = &m.from_name {
        out.insert("from_name".to_owned(), json!(name));
    }
    out.insert("to".to_owned(), json!(m.to));
    out.insert("cc".to_owned(), json!(m.cc));
    out.insert("subject".to_owned(), json!(m.subject));
    out.insert("date".to_owned(), json!(rfc3339(m.date)));
    out.insert("snippet".to_owned(), json!(m.snippet));
    out
}

fn structured(result: &ToolResult) -> Value {
    match result {
        ToolResult::Search {
            messages,
        } => {
            json!({"messages": messages.iter().map(|m| Value::Object(summary_json(m))).collect::<Vec<_>>()})
        }
        ToolResult::Read {
            messages,
        } => {
            let items: Vec<Value> = messages
                .iter()
                .map(|m| {
                    let mut obj = summary_json(&m.summary);
                    obj.insert("body_text".to_owned(), json!(m.body_text));
                    Value::Object(obj)
                })
                .collect();
            json!({"messages": items})
        }
        ToolResult::Sent(sent) => json!({"id": sent.id, "thread_id": sent.thread_id}),
        ToolResult::Connector {
            data,
        } => data.clone(),
        // Rendered by `render_outcome` as the server's own result; this arm only keeps the match exhaustive.
        ToolResult::Mcp {
            result,
        } => result.clone(),
        ToolResult::Integrations {
            integrations,
        } => json!({
            "integrations": integrations.iter().map(|i| json!({"service": i.service, "name": i.name})).collect::<Vec<_>>(),
            "note": "Accounts are not shown. Call reins_list_accounts with a service to ask the user to share them."
        }),
        ToolResult::Accounts {
            accounts,
            withheld,
        } => {
            let mut out = json!({
                "accounts": accounts.iter().map(|a| json!({"service": a.service, "account": a.account})).collect::<Vec<_>>()
            });
            if *withheld > 0 {
                out["withheld"] = json!(withheld);
                out["note"] = json!(format!(
                    "The user shared {} account(s) and keeps {withheld} more private. If the task needs another one, \
                     call reins_list_accounts again with ask_for_more=true and the user will be asked.",
                    accounts.len()
                ));
            }
            out
        }
        ToolResult::Granted {
            summary,
            expires_at,
            max_uses,
        } => json!({"granted": true, "summary": summary, "expires_at": rfc3339(*expires_at), "max_uses": max_uses,
            "note": "Nothing has been done yet. Now call the tool itself (gmail_search, gmail_read or gmail_send); \
            the calls this permission covers run without asking the user again until it expires."}),
    }
}

/// What the phone decided or returned.
pub fn render_outcome(outcome: &RelayOutcome) -> Value {
    match outcome {
        RelayOutcome::Result {
            result: ToolResult::Mcp {
                result,
            },
        } => mcp_tools::passthrough(result),
        RelayOutcome::Result {
            result,
        } => success(&structured(result)),
        RelayOutcome::Denied {
            reason,
        } => failure(&denied_text(reason.as_deref())),
        RelayOutcome::Error {
            message,
        } => failure(&format!("The Reins device could not complete the request: {message}")),
    }
}

/// The tool result for one wait on a relay request (spec §4.3).
pub fn render_wait(wait: &WaitResult, id: &RequestId, seen: PhoneSeen) -> Value {
    match wait {
        WaitResult::Answered(outcome) => render_outcome(outcome),
        WaitResult::Offline => failure(&offline_text(id, seen)),
        WaitResult::Pending => failure(&pending_text(id)),
        WaitResult::NotFound => failure("Unknown or expired request_id. Start the request again."),
        WaitResult::Deferred(ticket) => success(&json!({"status": "pending_review", "ticket": ticket,
            "next": "Continue with other work. Call reins_get_result with this ticket later, or reins_wait to block up \
to 10 minutes."})),
    }
}

/// A denial, with the reviewer's note when they wrote one (labelled as theirs).
pub fn denied_text(note: Option<&str>) -> String {
    match note.and_then(|n| clean_text(n, MAX_NOTE_CHARS)) {
        Some(note) => format!("{DENIED_TEXT} Note: \"{note}\""),
        None => DENIED_TEXT.to_owned(),
    }
}

/// The tool result for a ticket after a wait (`reins_get_result`, `reins_wait`).
pub fn render_ticket(view: &AgentView) -> Value {
    match view {
        AgentView::Open(row) => match row.state.as_str() {
            "needs_info" => success(&json!({
                "status": "needs_info",
                "ticket": ticket_of(&RequestId(row.id.clone())),
                "question": row.question,
                "next": "The reviewer asked this. Answer with reins_reply(ticket, text); then wait as before."
            })),
            _ => success(&tickets::pending_review(row, None)),
        },
        AgentView::Approved(row) => success(&json!({
            "status": "approved",
            "ticket": ticket_of(&RequestId(row.id.clone())),
            "next": "Approved. It runs when the user's phone is online; call reins_get_result again in a minute. \
        Nothing has happened yet."
        })),
        AgentView::Outcome(outcome) => render_outcome(outcome),
        AgentView::Done(row) => {
            let result: Value = row.result.as_deref().and_then(|r| serde_json::from_str(r).ok()).unwrap_or(json!({}));
            success(&json!({
                "status": "done",
                "ticket": ticket_of(&RequestId(row.id.clone())),
                "result": result,
                "note": "Approved and done. Only ids were kept: the full result went to whoever waited at the time."
            }))
        }
        AgentView::Closed(row) => failure(&match row.state.as_str() {
            "expired" => "Expired: not approved in time. Nothing happened.".to_owned(),
            "cancelled" => "Cancelled. Nothing happened.".to_owned(),
            "stale" => format!(
                "Stale: the world changed since it was approved ({}). Nothing happened; make the request again.",
                row.stale.as_deref().unwrap_or("details unknown")
            ),
            "failed" => "Approved, but it failed on the user's device.".to_owned(),
            _ => "Approved, but whether it happened is unknown. Check before trying again.".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::{MessageFull, SentMessage};

    use super::*;

    fn invalid_msg(name: &str, args: &Value) -> String {
        match parse_invocation(name, args) {
            Err(ToolArgError::Invalid(m)) => m,
            other => panic!("expected an invalid-argument error, got {other:?}"),
        }
    }

    fn summary() -> MessageSummary {
        MessageSummary {
            id: "m1".into(),
            thread_id: "t1".into(),
            from: "alerts@bank.com".into(),
            from_name: Some("Bank Alerts".into()),
            to: vec!["me@example.com".into()],
            cc: vec![],
            subject: "Statement".into(),
            date: 1_700_000_000,
            snippet: "Your statement is ready".into(),
        }
    }

    #[test]
    fn tool_definitions_are_strict_and_well_formed() {
        let tools = tool_definitions();
        let listed = connector::specs().iter().filter(|s| !s.desktop_only).count() - UNLISTED_TOOLS.len();
        assert_eq!(tools.len(), 12 + listed);
        for tool in &tools {
            assert!(tool["name"].is_string() && tool["description"].is_string(), "{tool}");
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{tool}");
            for required in tool["inputSchema"]["required"].as_array().unwrap() {
                assert!(tool["inputSchema"]["properties"].get(required.as_str().unwrap()).is_some(), "{tool}");
            }
        }
        assert_eq!(tools[1]["inputSchema"]["properties"]["max_results"]["maximum"], 50);
        assert_eq!(tools[0]["inputSchema"]["properties"]["message_ids"]["maxItems"], 20);
        assert!(tools[2]["inputSchema"]["properties"].get("bcc").is_none());
    }

    /// The Claude and ChatGPT directories require a human title and every annotation on each tool.
    #[test]
    fn every_tool_has_a_title_and_all_annotations_and_reads_are_read_only() {
        let tools = tool_definitions();
        for tool in &tools {
            let name = tool["name"].as_str().unwrap();
            let title = tool["title"].as_str().unwrap_or_default();
            assert!(!title.trim().is_empty() && title != name, "{name} needs a human title");
            assert!(name.len() <= 64, "{name} is longer than the directories allow");
            for hint in ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"] {
                assert!(tool["annotations"][hint].is_boolean(), "{name} lacks {hint}");
            }
            let a = &tool["annotations"];
            assert!(
                !(a["readOnlyHint"] == true && a["destructiveHint"] == true),
                "{name} cannot be read-only and destructive"
            );
            // Descriptions start with the verb a user would search for ("Send an email ..."), not "Sends".
            let first = tool["description"].as_str().unwrap().split([' ', ',']).next().unwrap();
            assert!(
                !first.ends_with('s') || first == "Dismiss" || first == "Access",
                "{name}: the description should start with an imperative verb, not `{first}`"
            );
        }
        let annotations = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["annotations"].clone();
        for spec in connector::specs().iter().filter(|s| !s.desktop_only && !UNLISTED_TOOLS.contains(&s.tool)) {
            let read = spec.effect != connector::Effect::Write;
            assert_eq!(annotations(spec.tool)["readOnlyHint"], read, "{}", spec.tool);
        }
        for read in ["gmail_read", "gmail_search", "reins_get_result", "reins_wait", "reins_tickets"] {
            assert_eq!(annotations(read)["readOnlyHint"], true, "{read}");
        }
        // reins_list_accounts with a service asks the phone for a permission to see accounts.
        for write in ["gmail_send", "reins_request_access", "reins_list_accounts", UPLOAD_TOOL] {
            assert_eq!(annotations(write)["readOnlyHint"], false, "{write}");
        }
        assert_eq!(annotations("reins_list_accounts")["destructiveHint"], false);
        for runs_code in ["github_workflow_dispatch", "github_run_rerun", "github_branch_rename"] {
            assert_eq!(annotations(runs_code)["destructiveHint"], true, "{runs_code}");
        }
        assert_eq!(annotations("vault_send_create")["openWorldHint"], true, "a Send is a public link");
        // Sends that cannot be taken back, deletions and overwrites are destructive; adding things is not.
        for destructive in [
            "gmail_send",
            "telegram_send",
            "sms_send",
            "gmail_trash",
            "calendar_delete_event",
            "github_repo_delete",
            "github_file_put",
            "github_pr_merge",
            "vault_item_delete",
            "github_api_write",
            "payments_purchase_request",
        ] {
            assert_eq!(annotations(destructive)["destructiveHint"], true, "{destructive}");
        }
        for additive in ["github_create_issue", "github_star_add", "vault_item_create", "gmail_create_draft"] {
            assert_eq!(annotations(additive)["destructiveHint"], false, "{additive}");
        }
        assert_eq!(annotations("github_star_add")["idempotentHint"], true);
        assert_eq!(annotations("github_create_issue")["idempotentHint"], false);
        assert_eq!(annotations("gmail_send")["openWorldHint"], true);
        assert_eq!(annotations("gmail_search")["openWorldHint"], false);
        assert_eq!(annotations("github_search_repos")["openWorldHint"], true);
    }

    #[test]
    fn every_tool_is_listed_and_a_granted_permission_says_nothing_happened_yet() {
        // Listing never depends on what the phone reported: a chat keeps the list it read first.
        let names: Vec<String> = tool_definitions().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect();
        for expected in ["gmail_send", "github_create_issue", "telegram_send", "vault_search", "reins_request_access"] {
            assert!(names.iter().any(|n| n == expected), "{expected}");
        }
        let granted = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Granted {
                summary: "Send emails to a@b.com".into(),
                expires_at: 1_700_003_600,
                max_uses: Some(1),
            },
        });
        let note = granted["structuredContent"]["note"].as_str().unwrap();
        assert!(note.starts_with("Nothing has been done yet") && note.contains("gmail_send"), "{note}");
        // reins_request_access tells the model when not to use it.
        let access = tool_definitions().into_iter().find(|t| t["name"] == "reins_request_access").unwrap();
        assert!(access["description"].as_str().unwrap().contains("Do NOT use it for a single action"));
    }

    #[test]
    fn search_arguments() {
        assert_eq!(
            parse_invocation("gmail_search", &json!({"query": " from:bank "})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::GmailSearch {
                    query: "from:bank".into(),
                    max_results: 10
                },
                None
            )
        );
        assert_eq!(
            parse_invocation("gmail_search", &json!({"query": "x", "max_results": 50})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::GmailSearch {
                    query: "x".into(),
                    max_results: 50
                },
                None
            )
        );
        assert_eq!(
            parse_invocation("gmail_search", &json!({"query": "x", "account": " Work@Gmail.com "})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::GmailSearch {
                    query: "x".into(),
                    max_results: 10
                },
                Some("work@gmail.com".into())
            )
        );
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "account": "not an address"})).contains("account"));
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "account": 5})).contains("string"));
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "max_results": 51})).contains("max_results"));
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "max_results": 0})).contains("max_results"));
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "max_results": "5"})).contains("max_results"));
        assert!(invalid_msg("gmail_search", &json!({"query": "x", "max_results": -1})).contains("max_results"));
        assert!(invalid_msg("gmail_search", &json!({})).contains("`query` is required"));
        assert!(invalid_msg("gmail_search", &json!({"query": 5})).contains("must be a string"));
        assert!(invalid_msg("gmail_search", &json!({"query": "   "})).contains("query"));
        assert!(invalid_msg("gmail_search", &json!("nope")).contains("JSON object"));
    }

    #[test]
    fn access_requests_are_parsed_and_bounded() {
        let ok = parse_invocation(
            "reins_request_access",
            &json!({"action": "read", "duration_seconds": 3600, "reason": "Summarise the bank statements",
                "from": ["Alerts@Bank.com"], "subject_contains": "statement", "max_uses": 3}),
        )
        .unwrap();
        let ToolInvocation::Relay(
            ToolCall::RequestGrant {
                grant,
            },
            _,
        ) = ok
        else {
            panic!("grant request expected")
        };
        assert_eq!((grant.action, grant.duration_secs, grant.max_uses), (GrantAction::Read, 3600, Some(3)));
        assert_eq!(grant.from, ["alerts@bank.com"]);

        let bad = |args: Value| invalid_msg("reins_request_access", &args);
        assert!(
            bad(json!({"action": "read", "duration_seconds": 30, "reason": "x", "from": ["a@b.com"]}))
                .contains("duration_secs")
        );
        assert!(bad(json!({"action": "delete", "duration_seconds": 600, "reason": "x"})).contains("action"));
        assert!(bad(json!({"action": "read", "duration_seconds": 600, "reason": "x"})).contains("from"));
        assert!(
            bad(json!({"action": "send", "duration_seconds": 600, "reason": "x", "any": true})).contains("everyone")
        );
        assert!(
            bad(json!({"action": "read", "duration_seconds": 600, "reason": "x", "from": ["a@b.com"], "bcc": 1}))
                .contains("Unknown property")
        );
        assert!(bad(json!({"action": "read", "reason": "x", "from": ["a@b.com"]})).contains("duration_seconds"));
    }

    fn github_tool(op: &str) -> &'static str {
        connector::specs().iter().find(|s| s.service == "github" && s.op == op).unwrap().tool
    }

    fn mcp_report() -> [McpServerReport; 1] {
        let tool = |name: &str, read_only: bool, destructive: bool| reins_proto::remote_mcp::McpToolReport {
            name: name.into(),
            title: None,
            description: format!("does {name}"),
            input_schema: json!({"type": "object"}),
            read_only,
            destructive,
        };
        [McpServerReport {
            id: "linear-2".into(),
            name: "Linear".into(),
            tools: vec![tool("search", true, false), tool("label issue", false, false), tool("wipe", false, true)],
        }]
    }

    #[test]
    fn write_permissions_name_one_tool_on_one_thing_for_at_most_30_days() {
        let edit = github_tool("issue_comment_edit");
        let grant =
            |args: Value, mcp: &[McpServerReport]| match parse_invocation_with("reins_request_access", &args, mcp) {
                Ok(ToolInvocation::Relay(
                    ToolCall::RequestGrant {
                        grant,
                    },
                    _,
                )) => Ok(*grant),
                Ok(other) => panic!("{other:?}"),
                Err(ToolArgError::Invalid(why)) => Err(why),
                Err(e) => panic!("{e:?}"),
            };
        let ok = grant(
            json!({"action": "write", "tool": edit, "resource": " Me/App ", "duration_seconds": 86_400,
                "reason": "Fix typos in my comments"}),
            &[],
        )
        .unwrap();
        assert_eq!(ok.action, GrantAction::Write);
        assert_eq!(
            (ok.service.as_deref(), ok.op.as_deref(), ok.tool.as_deref()),
            (Some("github"), Some("issue_comment_edit"), Some(edit))
        );
        assert!(ok.resource.is_some());

        let reports = mcp_report();
        let mcp = grant(
            json!({"action": "write", "tool": "linear-2__label_issue", "duration_seconds": 3600, "reason": "Triage"}),
            &reports,
        )
        .unwrap();
        assert_eq!(
            (mcp.service.as_deref(), mcp.op.as_deref(), mcp.resource.as_deref()),
            (Some("mcp_linear_2"), Some(""), Some("label issue"))
        );

        let base = |extra: Value| {
            let mut args = json!({"action": "write", "duration_seconds": 3600, "reason": "x"});
            args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            args
        };
        let refused = |extra: Value| grant(base(extra), &reports).unwrap_err();
        assert!(refused(json!({})).contains("`tool`"));
        assert!(refused(json!({"tool": edit})).contains("resource"), "a connector write names its resource");
        assert!(!refused(json!({"tool": github_tool("repo_delete"), "resource": "me/app"})).is_empty());
        assert!(refused(json!({"tool": "gmail_send"})).contains("send"));
        assert!(refused(json!({"tool": "linear-2__search"})).contains("only reads"));
        assert!(refused(json!({"tool": "linear-2__wipe"})).contains("every time"));
        assert!(refused(json!({"tool": "linear-2__label_issue", "resource": "x"})).contains("resource"));
        assert!(refused(json!({"tool": "nope"})).contains("Unknown tool"));
        assert!(!refused(json!({"tool": edit, "resource": "me/app", "duration_seconds": 31 * 86_400})).is_empty());
        assert!(!refused(json!({"tool": edit, "resource": "me/app", "any": true})).is_empty());
        let read_with_tool = grant(
            json!({"action": "read", "duration_seconds": 600, "reason": "x", "from": ["a@b.com"], "tool": edit}),
            &[],
        );
        assert!(read_with_tool.unwrap_err().contains("only for"));
    }

    #[test]
    fn a_batch_keeps_each_call_as_it_would_be_alone() {
        let reports = mcp_report();
        let calls = json!({"calls": [
            {"name": "gmail_search", "arguments": {"query": "from:bank"}},
            {"name": "linear-2__label_issue", "arguments": {"id": 1}},
            {"name": "reins_wait", "arguments": {"ticket": "x"}},
            {"name": "nope", "arguments": {}},
            {"name": "gmail_read", "arguments": {"message_ids": []}}
        ]});
        let Ok(ToolInvocation::Batch(items)) = parse_invocation_with(BATCH_TOOL, &calls, &reports) else {
            panic!("a batch was expected")
        };
        assert_eq!(items.len(), 5);
        assert_eq!(
            items[0].call,
            Ok((
                ToolCall::GmailSearch {
                    query: "from:bank".into(),
                    max_results: DEFAULT_MAX_RESULTS
                },
                None
            ))
        );
        assert!(matches!(items[1].call, Ok((ToolCall::Mcp(_), None))));
        assert!(items[2].call.as_ref().unwrap_err().contains("cannot be part of a batch"));
        assert!(items[3].call.as_ref().unwrap_err().contains("Unknown tool"));
        assert!(items[4].call.is_err());

        let bad = |args: Value| invalid_msg(BATCH_TOOL, &args);
        assert!(bad(json!({"calls": []})).contains("1 to 10"));
        let eleven: Vec<Value> =
            (0..11).map(|_| json!({"name": "gmail_search", "arguments": {"query": "a"}})).collect();
        assert!(bad(json!({"calls": eleven})).contains("1 to 10"));
        assert!(
            bad(json!({"calls": [{"name": "gmail_search", "arguments": {}, "extra": 1}]})).contains("Unknown property")
        );
        assert!(bad(json!({})).contains("calls"));
    }

    #[test]
    fn batch_results_have_one_entry_per_call_and_fail_only_when_all_do() {
        let ok = success(&json!({"done": true}));
        let out = render_batch(
            "b1",
            vec![
                ("gmail_search".into(), Some(RequestId("r1".into())), ok),
                ("nope".into(), None, failure("Unknown tool `nope`.")),
            ],
        );
        assert_eq!(out["isError"], false);
        assert_eq!(out["content"].as_array().unwrap().len(), 2);
        assert_eq!(out["content"][1]["text"], "[1] nope: Unknown tool `nope`.");
        assert_eq!(out["structuredContent"]["batch_id"], "b1");
        assert_eq!(out["structuredContent"]["items"][0]["request_id"], "r1");
        assert_eq!(out["structuredContent"]["items"][1]["result"]["isError"], true);
        let all_bad = render_batch("b2", vec![("x".into(), None, failure("no"))]);
        assert_eq!(all_bad["isError"], true);
    }

    #[test]
    fn every_tool_carries_the_same_permission_metadata() {
        let reports = mcp_report();
        for tool in tool_definitions_with(&reports) {
            let name = tool["name"].as_str().unwrap();
            let meta = &tool["_meta"][permission::META_KEY];
            for key in ["access", "approval", "every_time", "standing_grant", "resource_param", "max_grant_seconds"] {
                assert!(meta.get(key).is_some(), "{name} lacks {key}");
            }
            if tool["annotations"]["readOnlyHint"] == true {
                assert_ne!(meta["access"], "write", "{name}");
            }
        }
        let by_name = |n: &str| tool_definitions_with(&reports).into_iter().find(|t| t["name"] == n).unwrap();
        assert_eq!(
            by_name(github_tool("issue_comment_edit"))["_meta"][permission::META_KEY]["standing_grant"],
            "write"
        );
        assert_eq!(by_name("linear-2__wipe")["_meta"][permission::META_KEY]["every_time"], true);
        assert_eq!(by_name("linear-2__label_issue")["_meta"][permission::META_KEY]["standing_grant"], "write");
    }

    #[test]
    fn granted_results_render_for_the_ai() {
        let out = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Granted {
                summary: "Read emails from alerts@bank.com".into(),
                expires_at: 1_700_003_600,
                max_uses: None,
            },
        });
        assert_eq!(out["isError"], false);
        assert_eq!(out["structuredContent"]["granted"], true);
        assert_eq!(out["structuredContent"]["expires_at"], "2023-11-14T23:13:20Z");
    }

    #[test]
    fn listing_accounts_takes_no_arguments() {
        assert_eq!(
            parse_invocation("reins_list_accounts", &json!({})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::ListAccounts {
                    service: None,
                    ask_for_more: false
                },
                None
            )
        );
        assert_eq!(
            parse_invocation("reins_list_accounts", &json!({"service": " Gmail "})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::ListAccounts {
                    service: Some("gmail".into()),
                    ask_for_more: false
                },
                None
            )
        );
        assert_eq!(
            parse_invocation("reins_list_accounts", &json!({"service": "gmail", "ask_for_more": true})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::ListAccounts {
                    service: Some("gmail".into()),
                    ask_for_more: true
                },
                None
            )
        );
        assert!(invalid_msg("reins_list_accounts", &json!({"ask_for_more": "yes"})).contains("true or false"));
        assert!(invalid_msg("reins_list_accounts", &json!({"service": "not an integration"})).contains("service"));
        assert!(invalid_msg("reins_list_accounts", &json!({"account": "a@b.com"})).contains("Unknown property"));
        let listed = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Integrations {
                integrations: vec![reins_proto::relay::IntegrationInfo {
                    service: "gmail".into(),
                    name: "Gmail".into(),
                }],
            },
        });
        assert_eq!(listed["structuredContent"]["integrations"], json!([{"service": "gmail", "name": "Gmail"}]));
        let out = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Accounts {
                accounts: vec![reins_proto::relay::AccountInfo {
                    service: "gmail".into(),
                    account: "me@gmail.com".into(),
                }],
                withheld: 0,
            },
        });
        assert_eq!(out["structuredContent"], json!({"accounts": [{"service": "gmail", "account": "me@gmail.com"}]}));
        let partial = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Accounts {
                accounts: vec![reins_proto::relay::AccountInfo {
                    service: "gmail".into(),
                    account: "me@gmail.com".into(),
                }],
                withheld: 2,
            },
        });
        assert_eq!(partial["structuredContent"]["withheld"], 2);
        assert!(partial["structuredContent"]["note"].as_str().unwrap().contains("ask_for_more=true"));
    }

    #[test]
    fn read_arguments() {
        assert_eq!(
            parse_invocation("gmail_read", &json!({"message_ids": ["a1", "b2", "a1"]})).unwrap(),
            ToolInvocation::Relay(
                ToolCall::GmailRead {
                    message_ids: vec!["a1".into(), "b2".into()]
                },
                None
            )
        );
        assert!(invalid_msg("gmail_read", &json!({"message_ids": []})).contains("message_ids"));
        assert!(invalid_msg("gmail_read", &json!({"message_ids": [1]})).contains("only strings"));
        assert!(invalid_msg("gmail_read", &json!({"message_ids": "a1"})).contains("array"));
        assert!(invalid_msg("gmail_read", &json!({"message_ids": ["../etc"]})).contains("message_id"));
        assert!(invalid_msg("gmail_read", &json!({})).contains("required"));
    }

    #[test]
    fn send_arguments_reject_bcc_and_bad_addresses() {
        let ok = parse_invocation("gmail_send", &json!({"to": ["Alice@Bank.com"], "subject": "Hi", "body": "Text"}))
            .unwrap();
        let ToolInvocation::Relay(
            ToolCall::GmailSend {
                email,
            },
            _,
        ) = ok
        else {
            panic!("wrong invocation")
        };
        assert_eq!((email.to, email.cc.len(), email.reply_to_message_id), (vec!["alice@bank.com".to_owned()], 0, None));
        let with_reply = parse_invocation(
            "gmail_send",
            &json!({"to": ["a@b.com"], "cc": ["c@d.com"], "subject": "s", "body": "b", "reply_to_message_id": "abc123"}),
        )
        .unwrap();
        assert!(matches!(with_reply, ToolInvocation::Relay(ToolCall::GmailSend { .. }, _)));

        let bcc =
            invalid_msg("gmail_send", &json!({"to": ["a@b.com"], "subject": "s", "body": "b", "bcc": ["x@evil.com"]}));
        assert!(bcc.contains("`bcc`") && bcc.contains("cannot send Bcc"), "{bcc}");
        assert!(
            invalid_msg("gmail_send", &json!({"to": ["Bob <b@x.com>"], "subject": "s", "body": "b"}))
                .contains("address")
        );
        assert!(
            invalid_msg("gmail_send", &json!({"to": ["a@b.com\r\nBcc: x@evil.com"], "subject": "s", "body": "b"}))
                .contains("address")
        );
        assert!(
            invalid_msg("gmail_send", &json!({"to": ["a@b.com"], "subject": "s\r\nBcc: x@evil.com", "body": "b"}))
                .contains("subject")
        );
        assert!(invalid_msg("gmail_send", &json!({"to": [], "subject": "s", "body": "b"})).contains("recipient"));
        assert!(invalid_msg("gmail_send", &json!({"to": ["a@b.com"], "body": "b"})).contains("`subject` is required"));
        assert!(
            invalid_msg("gmail_send", &json!({"to": ["a@b.com"], "subject": 1, "body": "b"}))
                .contains("must be a string")
        );
    }

    #[test]
    fn get_result_and_unknown_tools() {
        assert_eq!(
            parse_invocation("reins_get_result", &json!({"request_id": "r-1"})).unwrap(),
            ToolInvocation::GetResult(RequestId("r-1".into()))
        );
        assert!(invalid_msg("reins_get_result", &json!({"request_id": ""})).contains("request_id"));
        assert!(invalid_msg("reins_get_result", &json!({"request_id": "x".repeat(65)})).contains("request_id"));
        assert_eq!(
            parse_invocation("reins_get_result", &json!({"ticket": "rt_r-1"})).unwrap(),
            ToolInvocation::GetResult(RequestId("r-1".into())),
            "a ticket is its request"
        );
        assert!(invalid_msg("reins_get_result", &json!({})).contains("either"));
        assert!(invalid_msg("reins_get_result", &json!({"request_id": "a", "ticket": "rt_a"})).contains("either"));
        assert!(invalid_msg("reins_get_result", &json!({"ticket": "rt_a/b"})).contains("ticket"));
        assert_eq!(
            parse_invocation("delete_everything", &json!({})),
            Err(ToolArgError::UnknownTool("delete_everything".into()))
        );
    }

    #[test]
    fn spec_texts_are_verbatim() {
        let id = RequestId("req-9".into());
        assert_eq!(DENIED_TEXT, "Denied by the user on their Reins device.");
        assert_eq!(
            offline_text(&id, PhoneSeen::Ago(2 * 3_600)),
            "Reins: your approval device is offline (last seen 2 hours ago). Ask the user to open the Reins app; the request waits there for up to 10 minutes. Then call reins_get_result with request_id=req-9."
        );
        assert!(offline_text(&id, PhoneSeen::Never).contains("(last seen not since the Reins server last restarted)"));
        assert_eq!(
            pending_text(&id),
            "Waiting for the user to approve on their phone; nothing has happened yet. Tell the user to approve it in the Reins app. Do not call the tool again: that would ask a second time. After the user responds (they can still approve after this message), call reins_get_result with request_id=req-9."
        );
        let seen = PhoneSeen::Ago(90);
        for wait in [WaitResult::Offline, WaitResult::Pending, WaitResult::NotFound] {
            assert_eq!(render_wait(&wait, &id, seen)["isError"], true);
        }
        assert_eq!(render_wait(&WaitResult::Offline, &id, seen)["content"][0]["text"], offline_text(&id, seen));
        assert_eq!(render_wait(&WaitResult::Pending, &id, seen)["content"][0]["text"], pending_text(&id));
    }

    #[test]
    fn outcomes_render_as_structured_content_with_rfc3339_dates() {
        let search = RelayOutcome::Result {
            result: ToolResult::Search {
                messages: vec![summary()],
            },
        };
        let rendered = render_outcome(&search);
        assert_eq!(rendered["isError"], false);
        let msg = &rendered["structuredContent"]["messages"][0];
        assert_eq!(msg["date"], "2023-11-14T22:13:20Z");
        assert_eq!(
            (msg["from"].as_str(), msg["from_name"].as_str(), msg["cc"].clone()),
            (Some("alerts@bank.com"), Some("Bank Alerts"), json!([]))
        );
        assert!(msg.get("body_text").is_none());
        let text: Value = serde_json::from_str(rendered["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, rendered["structuredContent"], "the text block is exactly the structured JSON");

        let mut nameless = summary();
        nameless.from_name = None;
        let read = RelayOutcome::Result {
            result: ToolResult::Read {
                messages: vec![MessageFull {
                    summary: nameless,
                    body_text: "Hello".into(),
                }],
            },
        };
        let msg = &render_outcome(&read)["structuredContent"]["messages"][0];
        assert_eq!(msg["body_text"], "Hello");
        assert!(msg.get("from_name").is_none());

        let sent = RelayOutcome::Result {
            result: ToolResult::Sent(SentMessage {
                id: "s1".into(),
                thread_id: "t9".into(),
            }),
        };
        assert_eq!(render_outcome(&sent)["structuredContent"], json!({"id": "s1", "thread_id": "t9"}));
    }

    #[test]
    fn ticket_tools_parse_strictly() {
        assert_eq!(
            parse_invocation("reins_wait", &json!({"ticket": "rt_a"})).unwrap(),
            ToolInvocation::Wait(RequestId("a".into()), 55)
        );
        assert_eq!(
            parse_invocation("reins_wait", &json!({"ticket": "rt_a", "max_seconds": 600})).unwrap(),
            ToolInvocation::Wait(RequestId("a".into()), 600)
        );
        assert!(invalid_msg("reins_wait", &json!({"ticket": "rt_a", "max_seconds": 601})).contains("max_seconds"));
        assert!(invalid_msg("reins_wait", &json!({"ticket": "rt_a", "max_seconds": 0})).contains("max_seconds"));
        assert!(invalid_msg("reins_wait", &json!({})).contains("ticket"));
        assert_eq!(parse_invocation("reins_tickets", &json!({})).unwrap(), ToolInvocation::Tickets);
        assert!(invalid_msg("reins_tickets", &json!({"all": true})).contains("all"));
        assert_eq!(
            parse_invocation("reins_cancel", &json!({"ticket": "rt_a"})).unwrap(),
            ToolInvocation::Cancel(RequestId("a".into()))
        );
        assert_eq!(
            parse_invocation("reins_reply", &json!({"ticket": "rt_a", "text": " because CI is red "})).unwrap(),
            ToolInvocation::Reply(RequestId("a".into()), "because CI is red".into())
        );
        assert!(invalid_msg("reins_reply", &json!({"ticket": "rt_a", "text": "  "})).contains("empty"));
        assert!(invalid_msg("reins_reply", &json!({"ticket": "rt_a", "text": "x".repeat(2001)})).contains("longer"));
    }

    #[test]
    fn pending_review_is_not_an_error_and_closed_tickets_are() {
        let wait = render_wait(&WaitResult::Deferred("rt_a".into()), &RequestId("a".into()), PhoneSeen::Never);
        assert_eq!(
            (wait["isError"].clone(), wait["structuredContent"]["status"].clone()),
            (json!(false), json!("pending_review"))
        );
        assert_eq!(wait["structuredContent"]["ticket"], "rt_a");
    }

    #[test]
    fn denials_and_device_errors_are_tool_errors() {
        let denied = render_outcome(&RelayOutcome::Denied {
            reason: None,
        });
        assert_eq!(
            (denied["isError"].clone(), denied["content"][0]["text"].as_str()),
            (json!(true), Some(DENIED_TEXT))
        );
        // The reviewer's note (or the budget's refusal) reaches the agent, labelled, on one clean line.
        let noted = render_outcome(&RelayOutcome::Denied {
            reason: Some("use the\u{7} staging bucket".into()),
        });
        assert_eq!(
            noted["content"][0]["text"],
            "Denied by the user on their Reins device. Note: \"use the  staging bucket\""
        );
        let err = render_outcome(&RelayOutcome::Error {
            message: "Gmail needs consent".into(),
        });
        assert_eq!(err["content"][0]["text"], "The Reins device could not complete the request: Gmail needs consent");
        assert!(err.get("structuredContent").is_none());
    }

    #[test]
    fn the_upload_tool_is_always_listed_and_relays_a_request_upload() {
        assert!(tool_definitions().iter().any(|t| t["name"] == UPLOAD_TOOL));
        assert_eq!(
            parse_invocation(
                UPLOAD_TOOL,
                &json!({"name": " build.zip ", "size": 1234, "reason": "Attach to the issue"})
            )
            .unwrap(),
            ToolInvocation::Relay(
                ToolCall::RequestUpload {
                    name: "build.zip".into(),
                    size: 1234,
                    content_type: None,
                    reason: "Attach to the issue".into()
                },
                None
            )
        );
        let typed = parse_invocation(
            UPLOAD_TOOL,
            &json!({"name": "a.png", "size": 1, "content_type": "image/png", "reason": "r"}),
        )
        .unwrap();
        assert!(matches!(
            typed,
            ToolInvocation::Relay(
                ToolCall::RequestUpload {
                    content_type: Some(_),
                    ..
                },
                None
            )
        ));
        let bad = |args: Value| invalid_msg(UPLOAD_TOOL, &args);
        assert!(bad(json!({"name": "a", "reason": "r"})).contains("size"));
        assert!(bad(json!({"name": "a", "size": -1, "reason": "r"})).contains("size"));
        assert!(bad(json!({"name": "a", "size": MAX_BLOB_BYTES + 1, "reason": "r"})).contains("size"));
        assert!(bad(json!({"name": "../a", "size": 1, "reason": "r"})).contains("name"));
        assert!(bad(json!({"name": "a", "size": 1, "reason": ""})).contains("reason"));
        assert!(bad(json!({"name": "a", "size": 1, "reason": "r", "url": "x"})).contains("Unknown property"));
    }

    #[test]
    fn mcp_server_tools_are_listed_after_ours_and_called_by_their_own_name() {
        let report = McpServerReport {
            id: "linear".into(),
            name: "Linear".into(),
            tools: vec![reins_proto::remote_mcp::McpToolReport {
                name: "search issues".into(),
                title: None,
                description: "Finds issues".into(),
                input_schema: json!({"type": "object"}),
                read_only: true,
                destructive: false,
            }],
        };
        let reports = [report];
        let tools = tool_definitions_with(&reports);
        let added = tools.last().unwrap();
        assert_eq!(added["title"], "Linear: search issues");
        assert_eq!(
            added["annotations"],
            json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true})
        );
        assert_eq!(tools.len(), tool_definitions().len() + 1);
        assert_eq!(tools.last().unwrap()["name"], "linear__search_issues");
        let ToolInvocation::Relay(ToolCall::Mcp(call), None) =
            parse_invocation_with("linear__search_issues", &json!({"q": "bug"}), &reports).unwrap()
        else {
            panic!("an MCP call was expected")
        };
        assert_eq!((call.server.as_str(), call.tool.as_str()), ("linear", "search issues"));
        assert!(matches!(
            parse_invocation_with("linear__search_issues", &json!("x"), &reports),
            Err(ToolArgError::Invalid(_))
        ));
        assert!(matches!(parse_invocation("linear__search_issues", &json!({})), Err(ToolArgError::UnknownTool(_))));
    }

    #[test]
    fn mcp_results_pass_through_instead_of_json_text() {
        let out = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Mcp {
                result: json!({"content": [{"type": "text", "text": "3 issues"}], "isError": false, "_meta": {}}),
            },
        });
        assert_eq!(out, json!({"content": [{"type": "text", "text": "3 issues"}], "isError": false}));
    }

    #[test]
    fn integration_tools_are_listed_and_parsed_from_their_descriptions() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for spec in connector::specs() {
            let listed = !spec.desktop_only && !UNLISTED_TOOLS.contains(&spec.tool);
            assert_eq!(names.contains(&spec.tool), listed, "{} is listed wrongly", spec.tool);
        }
        // Unlisted is not unavailable: a client that knows the name can still call it.
        let read = parse_invocation("github_request_read", &json!({"path": "/user"})).unwrap();
        assert!(matches!(read, ToolInvocation::Relay(ToolCall::Connector(_), _)));
        for unlisted in UNLISTED_TOOLS {
            assert!(connector::spec_for_tool(unlisted).is_some_and(|s| !s.desktop_only), "{unlisted}");
            assert_ne!(parse_invocation(unlisted, &json!({})), Err(ToolArgError::UnknownTool(unlisted.to_owned())));
        }
        for desktop in connector::specs().iter().filter(|s| s.desktop_only) {
            assert_eq!(
                parse_invocation(desktop.tool, &json!({})),
                Err(ToolArgError::UnknownTool(desktop.tool.to_owned())),
                "{} must never be callable over MCP",
                desktop.tool
            );
        }
        let send = tools.iter().find(|t| t["name"] == "telegram_send").unwrap();
        assert_eq!(send["annotations"]["readOnlyHint"], false);
        assert_eq!(send["inputSchema"]["required"], json!(["chat", "text"]));
        let read = tools.iter().find(|t| t["name"] == "telegram_read").unwrap();
        assert_eq!(read["annotations"]["readOnlyHint"], true);

        let parsed = parse_invocation("telegram_read", &json!({"chat": "@anna", "account": " +1555 "})).unwrap();
        let ToolInvocation::Relay(ToolCall::Connector(call), account) = parsed else {
            panic!("a connector call was expected")
        };
        assert_eq!((call.service.as_str(), call.op.as_str(), call.int_arg("limit")), ("telegram", "read", Some(20)));
        assert_eq!(account.as_deref(), Some("+1555"));
        assert!(invalid_msg("telegram_read", &json!({})).contains("`chat` is required"));
        assert!(
            invalid_msg("telegram_send", &json!({"chat": "x", "text": "y", "bcc": 1})).contains("Unknown property")
        );
        assert!(matches!(parse_invocation("telegram_nope", &json!({})), Err(ToolArgError::UnknownTool(_))));

        let out = render_outcome(&RelayOutcome::Result {
            result: ToolResult::Connector {
                data: json!({"items": [{"id": "1"}]}),
            },
        });
        assert_eq!(out["structuredContent"], json!({"items": [{"id": "1"}]}));
    }

    #[test]
    fn a_purchase_that_does_not_add_up_is_refused_before_the_phone_sees_it() {
        let cart = json!({"merchant": "Shop", "merchant_url": "https://shop.example.com/cart", "currency": "USD",
            "items": [{"name": "Mug", "quantity": 2, "unit_price": "8.00"}], "total": "15.00"});
        assert!(invalid_msg("payments_purchase_request", &cart).contains("make 16.00"));
        let mut ok = cart.clone();
        ok["total"] = json!("16.00");
        assert!(matches!(
            parse_invocation("payments_purchase_request", &ok),
            Ok(ToolInvocation::Relay(ToolCall::Connector(_), None))
        ));
    }
}
