//! MCP servers the user added on the phone ("universal MCP").
//!
//! The phone is their MCP client: it signs in (OAuth or a token), lists their tools, and runs every call after the
//! usual grants and approvals. It reports the tools (never a token) in [`ServicesReport::mcp`](crate::device::ServicesReport),
//! and the server lists them to AI clients as `<server>__<tool>` ([`exposed_name`]). A call comes back to the phone as
//! [`McpCall`].
//!
//! Large results: a tool that once answered with more than [`HEAVY_RESULT_BYTES`] is marked heavy on the phone, and
//! later calls to it go through the server ([`ProxyCall`]): the server makes that one call with the headers the phone
//! gives, keeps large content as download links, and hands the phone a small result.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most MCP servers a phone reports.
pub const MAX_SERVERS: usize = 20;
/// Most tools per server.
pub const MAX_TOOLS: usize = 200;
/// Longest JSON schema of one tool (serialized).
pub const MAX_SCHEMA_BYTES: usize = 32 * 1024;
pub const MAX_DESCRIPTION: usize = 2_000;
/// A result larger than this marks the tool heavy.
pub const HEAVY_RESULT_BYTES: usize = 1 << 20;
/// Separator between server id and tool name in the name AI clients see.
pub const SEPARATOR: &str = "__";

/// One tool of an added MCP server, as reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolReport {
    /// The tool's own name on its server.
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// The server's `readOnlyHint` (an untrusted hint: it only decides read vs write grants).
    #[serde(default)]
    pub read_only: bool,
    /// The server's `destructiveHint`: such calls are asked every time.
    #[serde(default)]
    pub destructive: bool,
}

/// One MCP server the user added, as reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpServerReport {
    /// Short id chosen on the phone: `[a-z0-9-]{1,24}` (see [`server_id_ok`]).
    pub id: String,
    /// Display name ("Linear").
    pub name: String,
    pub tools: Vec<McpToolReport>,
}

/// A call to a tool of an added MCP server, relayed to the phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCall {
    /// [`McpServerReport::id`].
    pub server: String,
    /// The tool's own name. On the wire `name`: `tool` already says which kind of call a relayed `ToolCall` is.
    #[serde(rename = "name")]
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Map<String, Value>,
}

/// `POST /reins/api/mcp/call`: the server makes one MCP request for the phone (a heavy tool).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyCall {
    pub v: u32,
    /// For the download links of large content.
    pub connection_id: crate::ids::ConnectionId,
    /// The MCP endpoint (https).
    pub endpoint: String,
    /// Sent as given (`Authorization`, `Mcp-Session-Id`, `MCP-Protocol-Version`), never stored or logged.
    pub headers: Vec<(String, String)>,
    /// The JSON-RPC request (`tools/call`).
    pub request: Value,
}

/// The answer to [`ProxyCall`]: the JSON-RPC response, with every content item larger than
/// [`crate::blob::INLINE_LIMIT`] replaced by a `resource_link` to a download the AI can fetch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyCallResult {
    pub status: u16,
    #[serde(default)]
    pub response: Option<Value>,
    /// `Mcp-Session-Id` the endpoint returned, if any.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Downloads made for large content.
    #[serde(default)]
    pub downloads: Vec<crate::blob::BlobDownload>,
}

#[must_use]
pub fn server_id_ok(id: &str) -> bool {
    (1..=24).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The name AI clients see: `<server>__<tool>` with characters MCP clients reject replaced by `_`, at most 64.
#[must_use]
pub fn exposed_name(server: &str, tool: &str) -> String {
    let clean: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut name = format!("{server}{SEPARATOR}{clean}");
    name.truncate(64);
    name
}

impl McpServerReport {
    /// Bounds; the server ignores a report that does not pass.
    pub fn validate(&self) -> Result<(), String> {
        if !server_id_ok(&self.id) {
            return Err(format!("{} is not a server id", self.id));
        }
        if self.name.is_empty() || self.name.chars().count() > 100 {
            return Err("a server name is 1..=100 characters".to_owned());
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(format!("at most {MAX_TOOLS} tools per server"));
        }
        for t in &self.tools {
            if t.name.is_empty() || t.name.len() > 128 || t.name.chars().any(char::is_control) {
                return Err("a tool name is 1..=128 characters".to_owned());
            }
            if t.description.chars().count() > MAX_DESCRIPTION {
                return Err("a tool description is too long".to_owned());
            }
            if serde_json::to_vec(&t.input_schema).map_or(usize::MAX, |v| v.len()) > MAX_SCHEMA_BYTES
                || !t.input_schema.is_object()
            {
                return Err(format!("the schema of {} is not an object or is too large", t.name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn exposed_names_are_safe_and_bounded() {
        assert_eq!(exposed_name("linear", "create_issue"), "linear__create_issue");
        assert_eq!(exposed_name("x", "a.b c/d"), "x__a_b_c_d");
        assert_eq!(exposed_name("srv", &"t".repeat(100)).len(), 64);
        assert!(server_id_ok("my-mcp-2"));
        assert!(!server_id_ok("Bad_Id"));
        assert!(!server_id_ok(""));
    }

    #[test]
    fn a_relayed_mcp_call_round_trips_inside_a_tool_call() {
        let call = crate::gmail::ToolCall::Mcp(McpCall {
            server: "linear".into(),
            tool: "create_issue".into(),
            arguments: json!({"title": "x"}).as_object().unwrap().clone(),
        });
        let wire = serde_json::to_value(&call).unwrap();
        assert_eq!(
            wire,
            json!({"tool": "mcp", "server": "linear", "name": "create_issue", "arguments": {"title": "x"}})
        );
        assert_eq!(serde_json::from_value::<crate::gmail::ToolCall>(wire).unwrap(), call);
    }

    #[test]
    fn reports_are_bounded() {
        let tool = McpToolReport {
            name: "t".into(),
            title: None,
            description: "d".into(),
            input_schema: json!({"type": "object"}),
            read_only: true,
            destructive: false,
        };
        let ok = McpServerReport {
            id: "linear".into(),
            name: "Linear".into(),
            tools: vec![tool],
        };
        ok.validate().unwrap();
        let mut bad = ok.clone();
        bad.tools[0].input_schema = json!("not an object");
        assert!(bad.validate().is_err());
        let mut big = ok;
        big.tools[0].input_schema = json!({"x": "y".repeat(MAX_SCHEMA_BYTES)});
        assert!(big.validate().is_err());
    }
}
