//! Universal MCP: MCP servers the user adds on the phone. The phone is their MCP client (Streamable HTTP, signed in
//! with OAuth or a token), reports their tools to the Reins server, which lists them to the AIs, and runs every call
//! after the usual grants and approvals. Tokens stay on the phone: never in a log, a view, the activity or a report.
//!
//! - [`client`]: the MCP client (initialize, tools/list, tools/call; JSON and SSE answers; sessions).
//! - [`oauth`]: the MCP authorization spec (RFC 9728, RFC 8414 / OIDC discovery, RFC 7591, PKCE, `resource`).
//! - [`servers`]: adding, signing in, refreshing and removing servers (the app's MCP screens).
//! - [`flow`]: a relayed `ToolCall::Mcp`: grants, approval, the call, large results.

pub mod api;
pub mod client;
pub mod flow;
pub mod oauth;
mod server_api;
pub mod servers;
pub mod wire;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::atomic::AtomicU64;

use reins_proto::remote_mcp::{
    MAX_DESCRIPTION, MAX_SCHEMA_BYTES, MAX_SERVERS, MAX_TOOLS, McpServerReport, McpToolReport, exposed_name,
    server_id_ok,
};
use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::Zeroize;

use crate::store::{McpTool, StoredMcpServer};
use crate::{CoreError, text};

/// Where the authorization server sends the browser back to (the app's intent filter).
pub const REDIRECT_URI: &str = "com.reins2fa.app://mcp-oauth";

/// Hosts an MCP server or authorization server may be reached on over plain HTTP (development, the emulator's host).
const PLAIN_HTTP_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "10.0.2.2", "[::1]"];

/// Longest arguments shown in an approval.
pub const MAX_ARGUMENTS_SHOWN: usize = 8_000;

/// The service id an MCP server's calls are logged under (`mcp:<id>`).
pub fn service_of(server_id: &str) -> String {
    format!("mcp:{server_id}")
}

/// The service id its permissions name. Permissions name integrations as `[a-z0-9_]{1,32}`, so `mcp:my-srv` becomes
/// `mcp_my_srv` (one-to-one: server ids have no `_`).
pub fn grant_service(server_id: &str) -> String {
    format!("mcp_{}", server_id.replace('-', "_"))
}

/// An MCP or OAuth endpoint the phone may talk to: https, or http on a local host; no credentials, no fragment.
pub fn checked_url(raw: &str) -> Result<Url, CoreError> {
    let bad = |why: &str| CoreError::invalid(format!("the address {why}"));
    let url = Url::parse(raw.trim()).map_err(|_| bad("is not a valid URL"))?;
    let host = url.host_str().ok_or_else(|| bad("has no host"))?;
    match url.scheme() {
        "https" => {}
        "http" if PLAIN_HTTP_HOSTS.contains(&host) => {}
        _ => return Err(bad("must start with https://")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("must not contain a user name or password"));
    }
    if url.fragment().is_some() {
        return Err(bad("must not contain #"));
    }
    Ok(url)
}

/// The address as shown: without a query (which could carry a key).
pub fn display_url(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut url) => {
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => text::one_line(raw),
    }
}

/// A display name for a server: the one the user gave, else from its host (`mcp.linear.app` → `Linear`).
pub fn display_name(given: Option<&str>, url: &Url) -> String {
    let given = given.map(text::one_line).filter(|n| !n.is_empty());
    let name = given.unwrap_or_else(|| {
        let host = url.host_str().unwrap_or("mcp");
        let labels: Vec<&str> = host.split('.').collect();
        let pick = labels
            .iter()
            .find(|l| !matches!(**l, "mcp" | "www" | "api" | "app") && l.chars().any(|c| c.is_ascii_alphabetic()))
            .copied()
            .unwrap_or(host);
        let mut chars = pick.chars();
        chars.next().map_or_else(|| "MCP".to_owned(), |first| first.to_uppercase().chain(chars).collect())
    });
    text::truncate_chars(&name, 100)
}

/// A new server id from its name: `[a-z0-9-]`, at most 24, unique among `taken`.
pub fn new_server_id(name: &str, taken: &[String]) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let mut slug: String = slug.trim_matches('-').chars().take(20).collect();
    slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "mcp".clone_into(&mut slug);
    }
    let mut id = slug.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{slug}-{n}");
        n += 1;
    }
    debug_assert!(server_id_ok(&id));
    id
}

/// The tokens of one server, sealed in the secrets table. Never printed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// When the authorization server gave the client a secret at registration.
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("access_token", &self.access_token.as_ref().map(|_| "<redacted>"))
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.client_secret.zeroize();
    }
}

/// What the phone knows of an MCP session (not secret; kept in memory only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    pub session_id: Option<String>,
    pub protocol: String,
}

/// MCP state that lives as long as the engine: sessions, and a lock so that a token is refreshed once at a time.
#[derive(Default)]
pub struct McpState {
    pub(crate) sessions: tokio::sync::Mutex<HashMap<String, Connected>>,
    pub(crate) refreshing: tokio::sync::Mutex<()>,
    pub(crate) next_id: AtomicU64,
}

impl fmt::Debug for McpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpState").finish_non_exhaustive()
    }
}

// ---- views (UniFFI) ------------------------------------------------------------------------------------------------

/// One added MCP server, as the app lists it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    /// The address, without a query.
    pub url: String,
    /// "ok" | "needs_sign_in" | "error"
    pub status: String,
    pub error: Option<String>,
    pub tools: Vec<McpToolView>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct McpToolView {
    pub name: String,
    /// The tool's title, else its name.
    pub title: String,
    pub description: String,
    /// The server says it only reads (its calls need read permission; the hint is not trusted for anything else).
    pub read_only: bool,
    /// The server says it may destroy things: asked every time, never remembered.
    pub destructive: bool,
    /// Its results go through the Reins server (they were too large for the phone once).
    pub heavy: bool,
}

/// Where adding (or refreshing) a server stands.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum McpAddStep {
    Added {
        server: McpServerView,
    },
    /// Open `authorize_url` in a browser tab; the redirect to `com.reins2fa.app://mcp-oauth` goes to
    /// `mcp_finish_sign_in`.
    NeedsSignIn {
        server_id: String,
        authorize_url: String,
    },
}

/// A call to a tool of an added MCP server, spelled out for the approval.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct McpCallView {
    pub server_name: String,
    pub server_url: String,
    pub tool: String,
    pub title: String,
    pub description: String,
    /// The arguments, pretty-printed, at most 8,000 characters.
    pub arguments_json: String,
    pub read_only: bool,
    pub destructive: bool,
}

/// What a parked MCP call keeps of the server and tool as they were when it was parked (shown, then performed).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkedMcp {
    pub server_name: String,
    pub server_url: String,
    pub title: String,
    pub description: String,
    pub read_only: bool,
    pub destructive: bool,
}

pub fn tool_title(tool: &McpTool) -> String {
    tool.title.as_deref().map(text::one_line).filter(|t| !t.is_empty()).unwrap_or_else(|| text::one_line(&tool.name))
}

pub fn server_view(s: &StoredMcpServer) -> McpServerView {
    McpServerView {
        id: s.id.clone(),
        name: text::one_line(&s.name),
        url: display_url(&s.url),
        status: s.status.clone(),
        error: s.error.as_deref().map(text::one_line),
        tools: s
            .tools
            .iter()
            .map(|t| McpToolView {
                name: text::one_line(&t.name),
                title: tool_title(t),
                description: text::truncate_chars(&text::neutralize(&t.description), MAX_DESCRIPTION),
                read_only: t.read_only,
                destructive: t.destructive,
                heavy: s.heavy.contains(&t.name),
            })
            .collect(),
    }
}

/// The servers as reported to the Reins server: only those connected at least once, only tools that pass the
/// contract's bounds (descriptions cut to size, duplicates of a name AI clients see dropped), at most
/// [`MAX_SERVERS`]. Each report passes [`McpServerReport::validate`].
pub fn reports(servers: &[StoredMcpServer]) -> Vec<McpServerReport> {
    servers
        .iter()
        .filter(|s| s.listed_at.is_some() && server_id_ok(&s.id))
        .filter_map(|s| {
            let mut exposed = BTreeSet::new();
            let tools: Vec<McpToolReport> = s
                .tools
                .iter()
                .filter(|t| !t.name.is_empty() && t.name.len() <= 128 && !t.name.chars().any(char::is_control))
                .filter(|t| {
                    t.input_schema.is_object()
                        && serde_json::to_vec(&t.input_schema).is_ok_and(|v| v.len() <= MAX_SCHEMA_BYTES)
                })
                .filter(|t| exposed.insert(exposed_name(&s.id, &t.name)))
                .take(MAX_TOOLS)
                .map(|t| McpToolReport {
                    name: t.name.clone(),
                    title: t.title.as_deref().map(|x| text::truncate_chars(&text::one_line(x), 200)),
                    description: text::truncate_chars(&text::neutralize(&t.description), MAX_DESCRIPTION),
                    input_schema: t.input_schema.clone(),
                    read_only: t.read_only,
                    destructive: t.destructive,
                })
                .collect();
            let name = text::truncate_chars(&text::one_line(&s.name), 100);
            let report = McpServerReport {
                id: s.id.clone(),
                name: if name.is_empty() {
                    s.id.clone()
                } else {
                    name
                },
                tools,
            };
            report.validate().ok().map(|()| report)
        })
        .take(MAX_SERVERS)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn addresses_must_be_https_or_local() {
        for ok in ["https://mcp.linear.app/mcp", "http://127.0.0.1:9000/mcp", "http://localhost/x?k=v"] {
            assert!(checked_url(ok).is_ok(), "{ok}");
        }
        for bad in ["http://mcp.example.com/mcp", "ftp://x", "https://u:p@x.com/", "https://x.com/#f", "nope"] {
            assert!(checked_url(bad).is_err(), "{bad}");
        }
        assert_eq!(display_url("https://x.com/mcp?api_key=SECRET"), "https://x.com/mcp");
    }

    #[test]
    fn names_and_ids_are_derived_and_unique() {
        let url = Url::parse("https://mcp.linear.app/mcp").unwrap();
        assert_eq!(display_name(None, &url), "Linear");
        assert_eq!(display_name(Some("  My\u{202e} Notes "), &url), "My Notes");
        assert_eq!(new_server_id("Linear", &[]), "linear");
        assert_eq!(new_server_id("Linear", &["linear".to_owned()]), "linear-2");
        assert_eq!(new_server_id("My Notes & Co!", &[]), "my-notes-co");
        assert_eq!(new_server_id("!!!", &[]), "mcp");
        let long = new_server_id(&"x".repeat(40), &[]);
        assert!(server_id_ok(&long) && long.len() == 20);
        assert_eq!(grant_service("my-srv-2"), "mcp_my_srv_2");
        assert_eq!(service_of("linear"), "mcp:linear");
    }

    #[test]
    fn credentials_never_print_their_tokens() {
        let c = Credentials {
            access_token: Some("AT-SECRET".to_owned()),
            refresh_token: Some("RT-SECRET".to_owned()),
            expires_at: Some(5),
            client_secret: Some("CS-SECRET".to_owned()),
        };
        let shown = format!("{c:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
    }

    #[test]
    fn reports_keep_only_connected_servers_and_valid_tools() {
        let tool = |name: &str, schema: serde_json::Value| McpTool {
            name: name.to_owned(),
            title: Some("T".to_owned()),
            description: "d".repeat(MAX_DESCRIPTION + 10),
            input_schema: schema,
            read_only: false,
            destructive: true,
        };
        let server = StoredMcpServer {
            id: "srv".to_owned(),
            name: "Srv".to_owned(),
            url: "https://x.com/mcp".to_owned(),
            auth: "none".to_owned(),
            client: None,
            tools: vec![
                tool("ok", json!({"type": "object"})),
                tool("bad", json!("string")),
                tool("a.b", json!({"type": "object"})),
                tool("a_b", json!({"type": "object"})),
            ],
            heavy: Vec::new(),
            status: "ok".to_owned(),
            error: None,
            listed_at: Some(1),
            added_at: 1,
        };
        let pending = StoredMcpServer {
            id: "later".to_owned(),
            listed_at: None,
            ..server.clone()
        };
        let reports = reports(&[server, pending]);
        assert_eq!(reports.len(), 1, "a server never connected is not reported");
        let names: Vec<&str> = reports[0].tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["ok", "a.b"], "invalid schemas and clashing exposed names are dropped");
        assert_eq!(reports[0].tools[0].description.chars().count(), MAX_DESCRIPTION);
        reports[0].validate().unwrap();
    }
}
