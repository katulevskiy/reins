//! The MCP tools Reins exposes (spec §4.2): schemas, strict argument parsing and result
//! rendering (Decisions 9, 10, 14). Unknown properties are rejected, never dropped, so an
//! approved action is exactly what the AI asked for.

use chrono::{DateTime, SecondsFormat};
use reins_proto::{
    blob::MAX_BLOB_BYTES,
    connector,
    gmail::{
        GrantAction, GrantRequest, MAX_ANY_GRANT_SECS, MAX_GRANT_RULES, MAX_GRANT_SECS, MAX_READ_IDS, MAX_REASON_LEN,
        MAX_RECIPIENTS, MAX_SEARCH_RESULTS, MIN_GRANT_SECS, MessageSummary, OutgoingEmail, ToolCall, normalize_account,
    },
    ids::RequestId,
    relay::{RelayOutcome, ToolResult},
    remote_mcp::McpServerReport,
};
use serde_json::{Map, Value, json};

use super::{mcp_tools, relay::WaitResult};

/// Applied by the server when the AI omits `max_results` (contracts C).
pub const DEFAULT_MAX_RESULTS: u32 = 10;
/// Always listed: hands the AI an upload link for a file it wants to pass on (files spec, S3).
pub const UPLOAD_TOOL: &str = "reins_upload";

pub const DENIED_TEXT: &str = "Denied by the user on their Reins device.";

pub fn offline_text(id: &RequestId) -> String {
    format!(
        "Reins: your approval device is offline. Ask the user to open the Reins app; the request is waiting \
there. Then call reins_get_result with request_id={id}."
    )
}

pub fn pending_text(id: &RequestId) -> String {
    format!(
        "Waiting for the user to approve on their phone. When they confirm (the user can approve even after this \
message), call reins_get_result with request_id={id}, or repeat the same request: a one-time approval may \
already cover it."
    )
}

fn account_property() -> Value {
    json!({
        "type": "string",
        "description": "Which connected account to use: an address from reins_list_accounts. Optional when only \
        one account is connected."
    })
}

/// The tools in a fixed order (`tools/list` must be deterministic).
#[cfg(test)]
pub fn tool_definitions() -> Vec<Value> {
    tool_definitions_for(None)
}

/// The tools for a user whose phone reported these integrations (`None`: not reported, so every tool). Gmail and
/// the connector tools are listed only for integrations that have an account; the `reins_*` tools always.
pub fn tool_definitions_for(services: Option<&[String]>) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "gmail_read",
            "title": "Read Gmail messages",
            "description": "Reads the full text of specific Gmail messages by id (ids come from gmail_search). \
        The user may have to approve on their phone.",
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
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "gmail_search",
            "title": "Search Gmail",
            "description": "Searches the user's Gmail with Gmail search syntax (for example `from:bank newer_than:7d`) and \
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
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "gmail_send",
            "title": "Send an email",
            "description": "Sends an email from the user's Gmail account. The user always sees the recipients and the \
        full text on their phone and must approve unless they granted a standing permission. Bcc is not supported.",
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
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}
        }),
        json!({
            "name": "reins_request_access",
            "title": "Ask for a standing permission",
            "description": "Asks the user, on their phone, for a time-limited permission so that later gmail_* calls \
        need no approval. Ask for the NARROWEST permission that completes your task: name the exact senders \
        (`a@b.com`) or domains (`@b.com`) or a subject text for reads, the exact recipients for sends, and the \
        SHORTEST duration you need. Prefer this over repeated single requests when you will make several related \
        calls. `any: true` (all mail, at most 7 days) and long durations are shown to the user as broad requests and \
        are often refused. Always give a short, honest `reason`. Sending to everyone cannot be requested.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account_property(),
                    "action": {"type": "string", "enum": ["read", "send"],
                        "description": "`read` covers gmail_search and gmail_read, `send` covers gmail_send."},
                    "duration_seconds": {"type": "integer", "minimum": MIN_GRANT_SECS, "maximum": MAX_GRANT_SECS,
                        "description": "How long the permission lasts. Ask for as little as you need."},
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
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
        }),
        json!({
            "name": "reins_list_accounts",
            "title": "List integrations and their accounts",
            "description": "Without `service`: lists the integrations the user has connected (for example gmail); \
        needs no approval and shows no accounts. With `service`: asks the user, on their phone, to let you see that \
        integration's accounts (for example the Gmail addresses). Once they agree (usually for a month) you can \
        pass the address you want as `account` to gmail_search, gmail_read, gmail_send or reins_request_access. \
        Only ask for the accounts when you need to choose between several.",
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
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "reins_get_result",
            "title": "Get a pending Reins result",
            "description": "Returns the result of an earlier request that was still waiting for the user's approval \
        (use the request_id from that answer). Waits a while if it is still pending.",
            "inputSchema": {
                "type": "object",
                "properties": {"request_id": {"type": "string", "description": "The request_id to look up."}},
                "required": ["request_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": UPLOAD_TOOL,
            "title": "Upload a file to pass on as a link",
            "description": "For passing a file to another tool as a link (for example a large file another tool should \
        download). The phone answers with an upload link: upload the file there (`curl -T <file> '<upload_url>'`). The \
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
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
        }),
    ];
    // The other integrations describe their tools as data (reins-proto).
    // Desktop-only tools answer with credentials for the paired desktop app: never offered to an AI.
    tools.extend(connector::specs().iter().filter(|s| !s.desktop_only).map(|s| {
        json!({
            "name": s.tool,
            "title": s.title,
            "description": s.description,
            "inputSchema": s.input_schema(),
            "annotations": s.annotations()
        })
    }));
    if let Some(services) = services {
        tools.retain(|t| {
            let name = t["name"].as_str().unwrap_or_default();
            let service = if name.starts_with("gmail_") {
                Some("gmail")
            } else {
                connector::spec_for_tool(name).map(|s| s.service)
            };
            service.is_none_or(|s| services.iter().any(|known| known == s))
        });
    }
    tools
}

/// [`tool_definitions_for`] plus the tools of the MCP servers the user added on the phone, after Reins's own.
pub fn tool_definitions_with(services: Option<&[String]>, mcp: &[McpServerReport]) -> Vec<Value> {
    let mut tools = tool_definitions_for(services);
    tools.extend(mcp_tools::tool_definitions(mcp));
    tools
}

#[derive(Debug, PartialEq)]
pub enum ToolInvocation {
    /// A normalized call to relay to the phone.
    Relay(ToolCall, Option<String>),
    GetResult(RequestId),
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
                ],
            )?;
            let action = match required_string(args, "action")?.as_str() {
                "read" => GrantAction::Read,
                "send" => GrantAction::Send,
                _ => return invalid("`action` must be \"read\" or \"send\"."),
            };
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
            reject_unknown(args, &["request_id"])?;
            let id = required_string(args, "request_id")?;
            if id.is_empty() || id.len() > 64 {
                return invalid("`request_id` is not valid.");
            }
            Ok(ToolInvocation::GetResult(RequestId(id)))
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
        } => json!({"granted": true, "summary": summary, "expires_at": rfc3339(*expires_at), "max_uses": max_uses}),
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
            ..
        } => failure(DENIED_TEXT),
        RelayOutcome::Error {
            message,
        } => failure(&format!("The Reins device could not complete the request: {message}")),
    }
}

/// The tool result for one wait on a relay request (spec §4.3).
pub fn render_wait(wait: &WaitResult, id: &RequestId) -> Value {
    match wait {
        WaitResult::Answered(outcome) => render_outcome(outcome),
        WaitResult::Offline => failure(&offline_text(id)),
        WaitResult::Pending => failure(&pending_text(id)),
        WaitResult::NotFound => failure("Unknown or expired request_id. Start the request again."),
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
        assert_eq!(tools.len(), 7 + connector::specs().iter().filter(|s| !s.desktop_only).count());
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
            parse_invocation("delete_everything", &json!({})),
            Err(ToolArgError::UnknownTool("delete_everything".into()))
        );
    }

    #[test]
    fn spec_texts_are_verbatim() {
        let id = RequestId("req-9".into());
        assert_eq!(DENIED_TEXT, "Denied by the user on their Reins device.");
        assert_eq!(
            offline_text(&id),
            "Reins: your approval device is offline. Ask the user to open the Reins app; the request is waiting there. Then call reins_get_result with request_id=req-9."
        );
        assert_eq!(
            pending_text(&id),
            "Waiting for the user to approve on their phone. When they confirm (the user can approve even after this message), call reins_get_result with request_id=req-9, or repeat the same request: a one-time approval may already cover it."
        );
        for wait in [WaitResult::Offline, WaitResult::Pending, WaitResult::NotFound] {
            assert_eq!(render_wait(&wait, &id)["isError"], true);
        }
        assert_eq!(render_wait(&WaitResult::Offline, &id)["content"][0]["text"], offline_text(&id));
        assert_eq!(render_wait(&WaitResult::Pending, &id)["content"][0]["text"], pending_text(&id));
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
    fn denials_and_device_errors_are_tool_errors() {
        let denied = render_outcome(&RelayOutcome::Denied {
            reason: Some("secret reason".into()),
        });
        assert_eq!(
            (denied["isError"].clone(), denied["content"][0]["text"].as_str()),
            (json!(true), Some(DENIED_TEXT))
        );
        let err = render_outcome(&RelayOutcome::Error {
            message: "Gmail needs consent".into(),
        });
        assert_eq!(err["content"][0]["text"], "The Reins device could not complete the request: Gmail needs consent");
        assert!(err.get("structuredContent").is_none());
    }

    #[test]
    fn the_upload_tool_is_always_listed_and_relays_a_request_upload() {
        let none: Vec<String> = Vec::new();
        let listed = tool_definitions_for(Some(&none));
        assert!(listed.iter().any(|t| t["name"] == UPLOAD_TOOL), "listed even with no integration");
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
        let tools = tool_definitions_with(None, &reports);
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
            assert_eq!(names.contains(&spec.tool), !spec.desktop_only, "{} is listed wrongly", spec.tool);
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
}
