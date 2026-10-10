//! A relayed call to a tool of an added MCP server (`ToolCall::Mcp`). It follows the flow of the other integrations:
//! the grants of service `mcp_<id>` (resource = the tool's name; read when the server says the tool only reads, else
//! write) answer it at once, or it is parked for the user; a destructive tool is asked every time. The result is the
//! server's own `CallToolResult`, passed through (`ToolResult::Mcp`). A result over `HEAVY_RESULT_BYTES` marks the tool
//! heavy and its large items become download links; a heavy tool's calls go through the Reins server.

use std::collections::{BTreeMap, BTreeSet};

use data_encoding::{BASE64, BASE64_NOPAD};
use reins_proto::blob::INLINE_LIMIT;
use reins_proto::gmail::ToolCall;
use reins_proto::ids::ConnectionId;
use reins_proto::relay::{RelayOutcome, RelayRequest, ToolResult};
use reins_proto::remote_mcp::{HEAVY_RESULT_BYTES, McpCall};
use serde_json::{Map, Value, json};

use super::client::McpError;
use super::servers::{Failure, Op, Output};
use super::{
    MAX_ARGUMENTS_SHOWN, McpCallView, ParkedMcp, display_url, grant_service, server_api, service_of, tool_title,
};
use crate::connector::Preview;
use crate::connector::flow::build_service_grant;
use crate::engine::Engine;
use crate::session::Session;
use crate::store::{AuditRecord, McpTool, StoredMcpServer, unix_now};
use crate::types::ApprovalChoice;
use crate::views::{ParkedConnector, ParkedRequest};
use crate::{CoreError, text};

/// "read" for a tool the server says only reads, else "write".
fn access_of(read_only: bool) -> &'static str {
    if read_only {
        "read"
    } else {
        "write"
    }
}

fn outcome_of(read_only: bool) -> &'static str {
    if read_only {
        "released"
    } else {
        "sent"
    }
}

/// The arguments, pretty, safe to show and bounded.
pub fn arguments_text(arguments: &Map<String, Value>, max: usize) -> String {
    let pretty = serde_json::to_string_pretty(arguments).unwrap_or_default();
    let clean = text::neutralize(&pretty);
    if clean.chars().count() > max {
        format!("{}…", text::truncate_chars(&clean, max.saturating_sub(1)))
    } else {
        clean
    }
}

/// What the grant and the approval call the tool ("Linear · Create issue").
fn resource_label(server_name: &str, tool: &McpTool) -> String {
    format!("{} · {}", text::one_line(server_name), tool_title(tool))
}

/// What the AI is told when a call could not be made (never a URL or a token).
fn ai_message(server_name: &str, f: &Failure) -> String {
    let name = text::one_line(server_name);
    match f {
        Failure::NeedsSignIn {
            ..
        } => format!(
            "{name} needs the user to sign in again. Ask them to open the Reins app and sign in to {name} under MCP servers."
        ),
        Failure::Mcp(McpError::Rpc {
            message,
            ..
        }) => format!("{name} answered with an error: {message}"),
        Failure::Mcp(McpError::Core(CoreError::Network {
            ..
        })) => format!("The phone could not reach {name}. Try again in a moment."),
        Failure::Mcp(McpError::TooLarge) => format!(
            "The result from {name} was too large for the phone. The tool now goes through the Reins server; if it \
             changes things, check whether it already did before calling it again."
        ),
        Failure::Mcp(e) => format!("{name} could not complete the request: {}", e.describe()),
    }
}

/// The approval's view of a parked MCP call.
pub fn call_view(parked: &ParkedRequest) -> Option<McpCallView> {
    let ToolCall::Mcp(call) = &parked.request.call else {
        return None;
    };
    let held = parked.connector.as_ref()?.mcp.as_ref()?;
    Some(McpCallView {
        server_name: text::one_line(&held.server_name),
        server_url: held.server_url.clone(),
        tool: text::one_line(&call.tool),
        title: text::one_line(&held.title),
        description: text::neutralize(&held.description),
        arguments_json: arguments_text(&call.arguments, MAX_ARGUMENTS_SHOWN),
        read_only: held.read_only,
        destructive: held.destructive,
    })
}

/// A file name made of safe characters.
fn file_name(tool: &str, part: &str, ext: &str) -> String {
    let clean: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    format!("{clean}-{part}.{ext}")
}

fn clean_mime(mime: Option<&str>, default: &str) -> String {
    mime.map(str::trim)
        .filter(|m| !m.is_empty() && m.len() <= 100 && m.is_ascii() && !m.chars().any(|c| c.is_ascii_control()))
        .unwrap_or(default)
        .to_owned()
}

fn extension(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or_default().trim() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/pdf" => "pdf",
        "application/json" => "json",
        "text/csv" => "csv",
        "text/html" => "html",
        m if m.starts_with("text/") => "txt",
        _ => "bin",
    }
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    let data = data.trim();
    BASE64.decode(data.as_bytes()).ok().or_else(|| BASE64_NOPAD.decode(data.as_bytes()).ok())
}

/// The bytes of one content item and their type: the text, the decoded image or audio, the embedded resource; anything
/// else as its JSON.
fn item_bytes(item: &Value) -> (Vec<u8>, String) {
    let as_json = || (serde_json::to_vec(item).unwrap_or_default(), "application/json".to_owned());
    match item.get("type").and_then(Value::as_str) {
        Some("text") => item
            .get("text")
            .and_then(Value::as_str)
            .map_or_else(as_json, |t| (t.as_bytes().to_vec(), "text/plain; charset=utf-8".to_owned())),
        Some("image" | "audio") => {
            let mime = clean_mime(item.get("mimeType").and_then(Value::as_str), "application/octet-stream");
            item.get("data").and_then(Value::as_str).and_then(decode_base64).map_or_else(as_json, |b| (b, mime))
        }
        Some("resource") => {
            let resource = item.get("resource").cloned().unwrap_or(Value::Null);
            let mime = resource.get("mimeType").and_then(Value::as_str);
            if let Some(t) = resource.get("text").and_then(Value::as_str) {
                (t.as_bytes().to_vec(), clean_mime(mime, "text/plain; charset=utf-8"))
            } else {
                resource
                    .get("blob")
                    .and_then(Value::as_str)
                    .and_then(decode_base64)
                    .map_or_else(as_json, |b| (b, clean_mime(mime, "application/octet-stream")))
            }
        }
        _ => as_json(),
    }
}

fn link(download: &reins_proto::blob::BlobDownload, mime: &str) -> Value {
    json!({
        "type": "resource_link",
        "uri": download.download_url,
        "name": download.name,
        "mimeType": mime,
        "size": download.size,
        "description": format!(
            "Too large to pass inline: download it from this link (it works until {}).",
            text::iso_utc(download.expires_at)
        ),
    })
}

/// Replaces every content item (and a `structuredContent`) larger than `INLINE_LIMIT` by a link to a download the
/// Reins server keeps for the AI.
async fn shrink(session: &Session, connection: &ConnectionId, tool: &str, result: &mut Value) -> Result<(), CoreError> {
    let limit = usize::try_from(INLINE_LIMIT).unwrap_or(usize::MAX);
    let size = |v: &Value| serde_json::to_vec(v).map_or(usize::MAX, |b| b.len());
    let Some(object) = result.as_object_mut() else {
        return Ok(());
    };
    if let Some(content) = object.get_mut("content").and_then(Value::as_array_mut) {
        for (index, item) in content.iter_mut().enumerate() {
            if size(item) <= limit {
                continue;
            }
            let (bytes, mime) = item_bytes(item);
            let name = file_name(tool, &(index + 1).to_string(), extension(&mime));
            let download = server_api::put_output(session, &connection.0, &name, &mime, &bytes).await?;
            *item = link(&download, &mime);
        }
    }
    if object.get("structuredContent").is_some_and(|s| size(s) > limit)
        && let Some(structured) = object.remove("structuredContent")
    {
        let bytes = serde_json::to_vec(&structured).unwrap_or_default();
        let name = file_name(tool, "structured", "json");
        let download = server_api::put_output(session, &connection.0, &name, "application/json", &bytes).await?;
        let item = link(&download, "application/json");
        match object.get_mut("content").and_then(Value::as_array_mut) {
            Some(content) => content.push(item),
            None => {
                object.insert("content".to_owned(), json!([item]));
            }
        }
    }
    Ok(())
}

impl Engine {
    /// Processes a relayed request built in Rust, as a push or a sync does once it has fetched one (Rust integration
    /// tests).
    pub async fn process_relayed(&self, request: RelayRequest) -> Result<(), CoreError> {
        let session = self.session()?;
        let result = self.process_request(&session, request).await;
        self.autopilot_pass().await;
        result
    }

    /// Calls the tool: through the Reins server when it is heavy; on the phone otherwise, marking it heavy (and
    /// linking its large items) when its result is too large.
    async fn mcp_perform(
        &self,
        session: &Session,
        server: &StoredMcpServer,
        tool: &McpTool,
        arguments: &Map<String, Value>,
        connection: &ConnectionId,
    ) -> Result<Value, Failure> {
        let proxy = Op::Proxy {
            tool: &tool.name,
            arguments,
            connection_id: connection,
            session,
        };
        let value = |out: Output| match out {
            Output::Result(v) => Ok(v),
            Output::Tools(_) => Err(Failure::Mcp(McpError::Protocol("unexpected answer".to_owned()))),
        };
        if server.heavy.contains(&tool.name) {
            return value(self.mcp_run(server, &proxy).await?);
        }
        let mark_heavy = || {
            if let Err(e) = self.store.mcp_set_heavy(&server.id, &tool.name, true) {
                log::warn!("could not mark an MCP tool heavy: {e}");
            }
        };
        let call = Op::Call {
            tool: &tool.name,
            arguments,
        };
        match self.mcp_run(server, &call).await {
            Ok(out) => {
                let mut result = value(out)?;
                if serde_json::to_vec(&result).map_or(usize::MAX, |v| v.len()) > HEAVY_RESULT_BYTES {
                    mark_heavy();
                    shrink(session, connection, &tool.name, &mut result).await?;
                }
                Ok(result)
            }
            Err(Failure::Mcp(McpError::TooLarge)) => {
                mark_heavy();
                // Asking again changes nothing for a tool that only reads.
                if tool.read_only {
                    value(self.mcp_run(server, &proxy).await?)
                } else {
                    Err(Failure::Mcp(McpError::TooLarge))
                }
            }
            Err(f) => Err(f),
        }
    }

    fn mcp_audit(
        &self,
        request: &RelayRequest,
        call: &McpCall,
        read_only: bool,
        outcome: &str,
        detail: &str,
        grant_id: Option<String>,
    ) -> AuditRecord {
        let mut audit = self.audit(request, access_of(read_only), outcome, detail, grant_id, 1, &[]);
        audit.service = service_of(&call.server);
        audit.account = Some(call.server.clone());
        audit.op.clone_from(&call.tool);
        audit.info.note = Some(format!("Arguments: {}", text::one_line(&arguments_text(&call.arguments, 2_000))));
        audit
    }

    async fn mcp_fail(
        &self,
        session: &Session,
        request: &RelayRequest,
        call: &McpCall,
        message: &str,
    ) -> Result<(), CoreError> {
        let audit = self.mcp_audit(request, call, false, "error", message, None);
        self.finish(
            session,
            request,
            audit,
            RelayOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }

    /// Handles a call to a tool of an added MCP server received from the Reins server.
    pub(crate) async fn handle_mcp(
        &self,
        session: &Session,
        mut request: RelayRequest,
        call: &McpCall,
    ) -> Result<(), CoreError> {
        let Some(server) = self.store.mcp_server(&call.server)? else {
            let message = "That MCP server is not added on the user's phone (any more).";
            return self.mcp_fail(session, &request, call, message).await;
        };
        request.account = Some(server.id.clone());
        let Some(tool) = server.tools.iter().find(|t| t.name == call.tool).cloned() else {
            let message = format!(
                "{} has no tool named {}. Ask the user to refresh the server in the Reins app if it is new.",
                text::one_line(&server.name),
                text::truncate_chars(&text::one_line(&call.tool), 128)
            );
            return self.mcp_fail(session, &request, call, &message).await;
        };
        let title = tool_title(&tool);
        // A destructive tool is asked for every time and never looks at the permissions.
        let covered = if tool.destructive {
            BTreeMap::new()
        } else {
            self.store.service_reserve(
                &request.connection_id,
                Some(&server.id),
                &grant_service(&server.id),
                access_of(tool.read_only),
                "",
                "",
                std::slice::from_ref(&tool.name),
                true,
                unix_now(),
            )?
        };
        if let Some(grant) = covered.get(&tool.name) {
            return match self.mcp_perform(session, &server, &tool, &call.arguments, &request.connection_id).await {
                Ok(result) => {
                    let detail = format!("{}: {title}", text::one_line(&server.name));
                    let audit = self.mcp_audit(
                        &request,
                        call,
                        tool.read_only,
                        outcome_of(tool.read_only),
                        &detail,
                        Some(grant.0.clone()),
                    );
                    self.finish(
                        session,
                        &request,
                        audit,
                        RelayOutcome::Result {
                            result: ToolResult::Mcp {
                                result,
                            },
                        },
                    )
                    .await
                }
                Err(f) => {
                    // Nothing was done: give the reserved use back.
                    self.store.refund(&BTreeSet::from([grant.clone()]))?;
                    self.mcp_fail(session, &request, call, &ai_message(&server.name, &f)).await
                }
            };
        }
        let preview = Preview {
            resource: tool.name.clone(),
            resource_label: resource_label(&server.name, &tool),
            lines: vec![format!("{}: {title}", text::one_line(&server.name))],
            parents: Vec::new(),
            once_only: tool.destructive,
            ..Preview::default()
        };
        let held = ParkedMcp {
            server_name: server.name.clone(),
            server_url: display_url(&server.url),
            title,
            description: text::truncate_chars(&tool.description, reins_proto::remote_mcp::MAX_DESCRIPTION),
            read_only: tool.read_only,
            destructive: tool.destructive,
        };
        self.park_request(&ParkedRequest {
            request: request.clone(),
            messages: Vec::new(),
            covered: BTreeMap::new(),
            account: request.account.clone(),
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: Some(ParkedConnector {
                items: Vec::new(),
                covered: BTreeMap::new(),
                preview: Some(preview),
                mcp: Some(held),
                purchase: None,
            }),
        })
    }

    /// The user approved a parked MCP call (and maybe a standing permission for its tool).
    pub(crate) async fn approve_mcp(
        &self,
        session: &Session,
        request_id: &str,
        parked: &ParkedRequest,
        choice: &ApprovalChoice,
        now: i64,
    ) -> Result<(), CoreError> {
        let ToolCall::Mcp(call) = &parked.request.call else {
            return Err(CoreError::invalid("not a call to an MCP server"));
        };
        let held = parked
            .connector
            .as_ref()
            .and_then(|c| c.mcp.as_ref())
            .ok_or_else(|| CoreError::storage("corrupt parked request"))?;
        let request = &parked.request;
        let server = self
            .store
            .mcp_server(&call.server)?
            .filter(|s| display_url(&s.url) == held.server_url)
            .ok_or_else(|| CoreError::service("That MCP server was removed or changed. Deny this request."))?;
        let known = server
            .tools
            .iter()
            .find(|t| t.name == call.tool)
            .ok_or_else(|| CoreError::service("The server no longer has this tool. Deny this request."))?;
        // The stricter of what was shown and what the server says now.
        let tool = McpTool {
            read_only: held.read_only && known.read_only,
            destructive: held.destructive || known.destructive,
            ..known.clone()
        };
        if choice.standing.is_some() && tool.destructive {
            return Err(CoreError::invalid("this is asked for every time and cannot be remembered"));
        }
        let new_grant = choice
            .standing
            .as_ref()
            .map(|s| {
                build_service_grant(
                    s,
                    &grant_service(&server.id),
                    access_of(tool.read_only),
                    "",
                    &[(tool.name.clone(), resource_label(&server.name, &tool))],
                    &[],
                    &request.connection_id,
                    &server.id,
                    now,
                )
            })
            .transpose()?;
        let (audit, outcome) =
            match self.mcp_perform(session, &server, &tool, &call.arguments, &request.connection_id).await {
                Ok(result) => {
                    if let Some(grant) = &new_grant {
                        self.store.insert_grant_from(grant, &parked.label(), "approval")?;
                    }
                    let detail = format!("approved: {}: {}", text::one_line(&server.name), tool_title(&tool));
                    let grant_id = new_grant.map(|g| g.id.0);
                    let audit =
                        self.mcp_audit(request, call, tool.read_only, outcome_of(tool.read_only), &detail, grant_id);
                    (
                        audit,
                        RelayOutcome::Result {
                            result: ToolResult::Mcp {
                                result,
                            },
                        },
                    )
                }
                // The server answered (or may have acted): the AI gets the answer, nothing is retried.
                Err(
                    f @ Failure::Mcp(
                        McpError::Rpc {
                            ..
                        }
                        | McpError::TooLarge,
                    ),
                ) => {
                    let message = ai_message(&server.name, &f);
                    let audit = self.mcp_audit(request, call, tool.read_only, "error", &message, None);
                    (
                        audit,
                        RelayOutcome::Error {
                            message,
                        },
                    )
                }
                // The call could not be made: the request stays for another try.
                Err(f) => return Err(f.into_core(&server.name)),
            };
        self.store.append_audit(&audit)?;
        self.store.remove_pending(request_id)?;
        self.store.mark_handled(request_id, unix_now())?;
        self.notifier.item_resolved(request_id.to_owned());
        self.respond(session, request_id, outcome).await
    }

    /// The audit entry for a refused MCP call.
    pub(crate) fn denied_mcp(&self, parked: &ParkedRequest) -> Option<AuditRecord> {
        let ToolCall::Mcp(call) = &parked.request.call else {
            return None;
        };
        let read_only = parked.connector.as_ref().and_then(|c| c.mcp.as_ref()).is_some_and(|m| m.read_only);
        Some(self.mcp_audit(
            &parked.request,
            call,
            read_only,
            "denied",
            &crate::autopilot::context::denied_detail(),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_pretty_clean_and_bounded() {
        let args = json!({"title": "Fix\u{202e} it", "n": 2}).as_object().unwrap().clone();
        let shown = arguments_text(&args, 8_000);
        assert!(shown.contains("\"title\": \"Fix it\""), "{shown}");
        let long = json!({"x": "y".repeat(100)}).as_object().unwrap().clone();
        let cut = arguments_text(&long, 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn large_items_become_bytes_of_the_right_type() {
        let png = BASE64.encode(b"\x89PNG....");
        assert_eq!(
            item_bytes(&json!({"type": "image", "data": png, "mimeType": "image/png"})),
            (b"\x89PNG....".to_vec(), "image/png".to_owned())
        );
        assert_eq!(item_bytes(&json!({"type": "text", "text": "hi"})).0, b"hi");
        let res =
            json!({"type": "resource", "resource": {"uri": "file:///a.csv", "mimeType": "text/csv", "text": "a,b"}});
        assert_eq!(item_bytes(&res), (b"a,b".to_vec(), "text/csv".to_owned()));
        let bad = json!({"type": "image", "data": "%%%", "mimeType": "image/png"});
        assert_eq!(item_bytes(&bad).1, "application/json", "undecodable data is kept as JSON");
        assert_eq!(extension("text/plain; charset=utf-8"), "txt");
        assert_eq!(file_name("a/b c", "1", "txt"), "a_b_c-1.txt");
    }
}
