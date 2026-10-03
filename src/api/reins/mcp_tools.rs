//! Universal MCP (files spec, S2): the tools of the MCP servers the user added on the phone, listed to AI clients as
//! `<server>__<tool>` and relayed back to the phone as [`ToolCall::Mcp`]; their results pass through as the server
//! gave them.
//!
//! Everything about these tools is reported by the phone and, further back, by a third-party server: names, schemas
//! and hints are bounded by the contract and only ever shown, never trusted for a decision here.

use std::collections::HashSet;

use reins_proto::{
    gmail::ToolCall,
    remote_mcp::{MAX_SERVERS, McpCall, McpServerReport, McpToolReport, exposed_name},
};
use serde_json::{Map, Value, json};

/// Keeps the valid reports: each passes the contract's bounds, ids are unique, at most [`MAX_SERVERS`].
pub fn sanitize_reports(reports: Vec<McpServerReport>) -> Vec<McpServerReport> {
    let mut seen = HashSet::new();
    reports
        .into_iter()
        .filter(|r| match r.validate() {
            Ok(()) => true,
            Err(e) => {
                warn!("Ignoring an MCP server the phone reported: {e}");
                false
            }
        })
        .filter(|r| seen.insert(r.id.clone()))
        .take(MAX_SERVERS)
        .collect()
}

/// Every tool with the name AI clients see, in report order; a name that two tools would share goes to the first.
fn exposed(reports: &[McpServerReport]) -> impl Iterator<Item = (String, &McpServerReport, &McpToolReport)> {
    let mut seen = HashSet::new();
    reports
        .iter()
        .flat_map(|server| server.tools.iter().map(move |tool| (exposed_name(&server.id, &tool.name), server, tool)))
        .filter(move |(name, _, _)| seen.insert(name.clone()))
}

/// The `tools/list` entries for the reported servers.
pub fn tool_definitions(reports: &[McpServerReport]) -> Vec<Value> {
    exposed(reports)
        .map(|(name, server, tool)| {
            let title = tool.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&tool.name);
            let mut annotations = json!({"readOnlyHint": tool.read_only, "openWorldHint": true});
            if !tool.read_only {
                annotations["destructiveHint"] = json!(tool.destructive);
            }
            json!({
                "name": name,
                "title": format!("{}: {title}", server.name),
                "description": format!("{}: {}", server.name, tool.description),
                "inputSchema": tool.input_schema,
                "annotations": annotations
            })
        })
        .collect()
}

/// A `tools/call` of an exposed name: `None` when no reported tool has it, else the call for the phone (with the
/// tool's own name) or why the arguments are refused.
pub fn parse_call(reports: &[McpServerReport], name: &str, arguments: &Value) -> Option<Result<ToolCall, String>> {
    let (_, server, tool) = exposed(reports).find(|(exposed, _, _)| exposed == name)?;
    let arguments = match arguments {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => return Some(Err("Tool arguments must be a JSON object.".to_owned())),
    };
    let call = ToolCall::Mcp(McpCall {
        server: server.id.clone(),
        tool: tool.name.clone(),
        arguments,
    });
    Some(call.normalized().map_err(|e| e.to_string()))
}

/// A tool result of an added MCP server as the AI gets it: its `content`, `structuredContent` and `isError`, nothing
/// else (no `_meta` from a third party). A result without content gets its structured content as text, as MCP asks.
pub fn passthrough(result: &Value) -> Value {
    let Some(obj) = result.as_object() else {
        return json!({"content": [{"type": "text", "text": result.to_string()}], "isError": false});
    };
    let structured = obj.get("structuredContent").filter(|s| s.is_object()).cloned();
    let content = match obj.get("content") {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        _ => Value::Array(
            structured.as_ref().map(|s| json!({"type": "text", "text": s.to_string()})).into_iter().collect(),
        ),
    };
    let mut out = Map::new();
    out.insert("content".to_owned(), content);
    if let Some(structured) = structured {
        out.insert("structuredContent".to_owned(), structured);
    }
    out.insert("isError".to_owned(), json!(obj.get("isError").and_then(Value::as_bool).unwrap_or(false)));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use reins_proto::remote_mcp::MAX_DESCRIPTION;

    use super::*;

    fn tool(name: &str, read_only: bool) -> McpToolReport {
        McpToolReport {
            name: name.to_owned(),
            title: None,
            description: format!("does {name}"),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            read_only,
            destructive: !read_only,
        }
    }

    fn server(id: &str, tools: Vec<McpToolReport>) -> McpServerReport {
        McpServerReport {
            id: id.to_owned(),
            name: id.to_uppercase(),
            tools,
        }
    }

    #[test]
    fn invalid_and_duplicate_reports_are_dropped() {
        let mut long = tool("t", true);
        long.description = "x".repeat(MAX_DESCRIPTION + 1);
        let reports = vec![
            server("linear", vec![tool("a", true)]),
            server("Bad_Id", vec![tool("a", true)]),
            server("linear", vec![tool("b", true)]),
            server("long", vec![long]),
            server("ok", vec![]),
        ];
        let kept: Vec<String> = sanitize_reports(reports).into_iter().map(|r| r.id).collect();
        assert_eq!(kept, ["linear", "ok"]);
        let many: Vec<McpServerReport> = (0..MAX_SERVERS + 3).map(|i| server(&format!("s{i}"), vec![])).collect();
        assert_eq!(sanitize_reports(many).len(), MAX_SERVERS);
    }

    #[test]
    fn tools_are_listed_with_the_server_name_and_their_hints() {
        let mut titled = tool("create.issue", false);
        titled.title = Some("Create issue".to_owned());
        let defs = tool_definitions(&[server("linear", vec![tool("search", true), titled])]);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0]["name"], "linear__search");
        assert_eq!(defs[0]["title"], "LINEAR: search");
        assert_eq!(defs[0]["description"], "LINEAR: does search");
        assert_eq!(defs[0]["inputSchema"]["properties"]["q"]["type"], "string");
        assert_eq!(defs[0]["annotations"], json!({"readOnlyHint": true, "openWorldHint": true}));
        assert_eq!(defs[1]["name"], "linear__create_issue");
        assert_eq!(defs[1]["title"], "LINEAR: Create issue");
        assert_eq!(defs[1]["annotations"]["destructiveHint"], true);
    }

    #[test]
    fn calls_map_back_to_the_tools_own_name() {
        let reports = [server("linear", vec![tool("create.issue", false), tool("create_issue", true)])];
        assert_eq!(tool_definitions(&reports).len(), 1, "a clashing exposed name is listed once");
        let call = parse_call(&reports, "linear__create_issue", &json!({"title": "Bug"})).unwrap().unwrap();
        assert_eq!(
            call,
            ToolCall::Mcp(McpCall {
                server: "linear".to_owned(),
                tool: "create.issue".to_owned(),
                arguments: json!({"title": "Bug"}).as_object().unwrap().clone(),
            })
        );
        let empty = parse_call(&reports, "linear__create_issue", &Value::Null).unwrap().unwrap();
        assert!(matches!(empty, ToolCall::Mcp(McpCall { arguments, .. }) if arguments.is_empty()));
        assert!(parse_call(&reports, "linear__create_issue", &json!([1])).unwrap().is_err());
        let huge = json!({"x": "y".repeat(reins_proto::gmail::MAX_MCP_ARGUMENTS)});
        assert!(parse_call(&reports, "linear__create_issue", &huge).unwrap().is_err());
        assert!(parse_call(&reports, "linear__nope", &json!({})).is_none());
        assert!(parse_call(&reports, "gmail_search", &json!({})).is_none());
    }

    /// `ToolCall` is tagged by `tool` and `McpCall` has a `tool` field of its own, so the relayed JSON carries `tool`
    /// twice and the phone cannot read it back. The fix belongs in `reins-proto` (frozen for this batch).
    #[test]
    fn an_mcp_call_survives_the_relay_wire_form() {
        let call = ToolCall::Mcp(McpCall {
            server: "linear".to_owned(),
            tool: "search".to_owned(),
            arguments: Map::new(),
        });
        let text = serde_json::to_string(&call).unwrap();
        assert_eq!(serde_json::from_str::<ToolCall>(&text).unwrap(), call);
    }

    #[test]
    fn results_pass_through_without_foreign_fields() {
        let result = json!({"content": [{"type": "image", "data": "AAAA", "mimeType": "image/png"}],
            "structuredContent": {"n": 1}, "isError": true, "_meta": {"x": 1}});
        assert_eq!(
            passthrough(&result),
            json!({"content": [{"type": "image", "data": "AAAA", "mimeType": "image/png"}],
                "structuredContent": {"n": 1}, "isError": true})
        );
        assert_eq!(
            passthrough(&json!({"structuredContent": {"n": 2}})),
            json!({"content": [{"type": "text", "text": "{\"n\":2}"}], "structuredContent": {"n": 2}, "isError": false})
        );
        assert_eq!(passthrough(&json!({})), json!({"content": [], "isError": false}));
        assert_eq!(
            passthrough(&json!("odd")),
            json!({"content": [{"type": "text", "text": "\"odd\""}], "isError": false})
        );
    }
}
