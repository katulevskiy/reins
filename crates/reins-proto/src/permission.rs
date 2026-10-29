//! Per-tool permission metadata: what each tool an AI sees may touch, whether a standing permission
//! (`reins_request_access`) can cover its calls, and for how long at most. Derived from the tool descriptions
//! ([`crate::connector::ToolSpec`]) and the built-in tools; nothing here decides a call (the phone does), it only
//! tells AI clients, consistently for every tool, what to expect.
//!
//! Kept apart from the connector descriptions on purpose: a per-action risk level belongs to the connector manifest,
//! not here.

use serde::Serialize;
use serde_json::{Value, json};

use crate::connector::{Effect, ToolSpec, spec_for_tool, specs};
use crate::gmail::MAX_GRANT_SECS;

/// The `_meta` key of a `tools/list` entry that carries [`ToolPermission::meta`].
pub const META_KEY: &str = "dev.reins/permission";
/// The longest a standing write permission may last: 30 days.
pub const MAX_WRITE_GRANT_SECS: u64 = MAX_GRANT_SECS;
/// The longest resource a write permission names (a repository, a chat id, a calendar id).
pub const MAX_RESOURCE_LEN: usize = 200;
/// The integration name of an added MCP server's grants: `mcp_<server id with '-' as '_'>`.
pub const MCP_SERVICE_PREFIX: &str = "mcp_";
const MAX_MCP_TOOL_LEN: usize = 128;

/// What a tool may touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Only Reins' own state (tickets, results).
    None,
    List,
    Read,
    Write,
}

/// Which standing permission (`reins_request_access` `action`) can cover the tool's calls, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Read,
    Send,
    Write,
}

/// The permission facts of one tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolPermission {
    pub access: Access,
    /// A call may wait for the user's approval on their phone.
    pub approval: bool,
    /// Asked for every time: no standing permission ever covers it.
    pub every_time: bool,
    /// What `reins_request_access` can ask for to cover it; `None` when nothing can.
    pub standing: Option<Standing>,
    /// The argument that names the one thing a write permission covers.
    pub resource_param: Option<&'static str>,
}

impl ToolPermission {
    const fn own(access: Access, approval: bool) -> Self {
        Self {
            access,
            approval,
            every_time: false,
            standing: None,
            resource_param: None,
        }
    }

    /// The longest standing permission that can cover it, in seconds (0: none can).
    #[must_use]
    pub fn max_grant_secs(&self) -> u64 {
        match self.standing {
            Some(Standing::Write) => MAX_WRITE_GRANT_SECS,
            Some(_) => MAX_GRANT_SECS,
            None => 0,
        }
    }

    /// The `_meta` entry of the tool: the same keys for every tool.
    #[must_use]
    pub fn meta(&self) -> Value {
        json!({
            "access": self.access,
            "approval": self.approval,
            "every_time": self.every_time,
            "standing_grant": self.standing,
            "resource_param": self.resource_param,
            "max_grant_seconds": self.max_grant_secs(),
        })
    }
}

/// The permission facts of a tool of another integration.
#[must_use]
pub fn of_spec(spec: &ToolSpec) -> ToolPermission {
    let access = match spec.effect {
        Effect::List => Access::List,
        Effect::Read | Effect::Search => Access::Read,
        Effect::Write => Access::Write,
    };
    ToolPermission {
        access,
        approval: true,
        every_time: spec.once_only,
        standing: write_grantable(spec).then_some(Standing::Write),
        resource_param: spec.resource_param,
    }
}

/// The permission facts of a tool of an MCP server the user added (its hints are the server's own).
#[must_use]
pub fn of_mcp_tool(read_only: bool, destructive: bool) -> ToolPermission {
    let write = !read_only;
    ToolPermission {
        access: if write {
            Access::Write
        } else {
            Access::Read
        },
        approval: true,
        every_time: write && destructive,
        standing: (write && !destructive).then_some(Standing::Write),
        resource_param: None,
    }
}

/// The permission facts of a built-in tool or a tool of another integration, by the name AI clients see; `None`
/// for an unknown name (an added MCP server's tools: [`of_mcp_tool`]).
#[must_use]
pub fn for_tool(name: &str) -> Option<ToolPermission> {
    let builtin = match name {
        "gmail_search" | "gmail_read" => ToolPermission {
            standing: Some(Standing::Read),
            ..ToolPermission::own(Access::Read, true)
        },
        "gmail_send" => ToolPermission {
            standing: Some(Standing::Send),
            ..ToolPermission::own(Access::Write, true)
        },
        // A permission or an upload is only ever the user's own decision.
        "reins_request_access" | "reins_upload" => ToolPermission {
            every_time: true,
            ..ToolPermission::own(Access::None, true)
        },
        "reins_list_accounts" => ToolPermission::own(Access::List, true),
        // Each call of a batch is decided on its own, as if it had been called alone.
        "reins_batch" => ToolPermission::own(Access::None, true),
        "reins_get_result" | "reins_wait" | "reins_tickets" | "reins_cancel" | "reins_reply" => {
            ToolPermission::own(Access::None, false)
        }
        _ => return spec_for_tool(name).map(of_spec),
    };
    Some(builtin)
}

/// Whether a standing write permission can cover calls of `spec`: a write that is not asked every time, not only for
/// the desktop app, and names the one thing it changes (a write permission is never for everything).
#[must_use]
pub fn write_grantable(spec: &ToolSpec) -> bool {
    spec.effect == Effect::Write && !spec.once_only && !spec.desktop_only && spec.resource_param.is_some()
}

/// The grant integration of an added MCP server (`linear-2` → `mcp_linear_2`).
#[must_use]
pub fn mcp_grant_service(server_id: &str) -> String {
    format!("{MCP_SERVICE_PREFIX}{}", server_id.replace('-', "_"))
}

/// What a standing write permission covers, checked: the tool of another integration (or the MCP tool) and the one
/// thing it may change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteTarget {
    pub service: String,
    /// A connector operation; empty for an MCP tool (whose own name is the resource).
    pub op: String,
    pub resource: String,
}

/// Checks what a write permission asks to cover (`service`, `op`, `resource`) and returns it as stored. For an added
/// MCP server `service` is its [`mcp_grant_service`] and the tool's own name is in `op` (as asked) or in `resource`
/// with `op` empty (as stored).
pub fn write_target(service: &str, op: &str, resource: Option<&str>) -> Result<WriteTarget, String> {
    let resource = resource.map(str::trim).filter(|r| !r.is_empty());
    if let Some(server) = service.strip_prefix(MCP_SERVICE_PREFIX) {
        let server_ok = !server.is_empty()
            && server.len() <= 24
            && server.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !server_ok {
            return Err("not an added MCP server".to_owned());
        }
        // Asked for by its name in `op`; stored (and normalized again) with the name as the resource.
        let tool = match (op.trim(), resource) {
            (op, None) => op,
            ("", Some(stored)) => stored,
            _ => return Err("an MCP tool's permission covers the tool itself; leave out `resource`".to_owned()),
        };
        if tool.is_empty() || tool.len() > MAX_MCP_TOOL_LEN || tool.chars().any(char::is_control) {
            return Err("name the MCP tool".to_owned());
        }
        return Ok(WriteTarget {
            service: service.to_owned(),
            op: String::new(),
            resource: tool.to_owned(),
        });
    }
    let spec =
        specs().iter().find(|s| s.service == service && s.op == op).ok_or_else(|| format!("no tool {service}.{op}"))?;
    if spec.effect != Effect::Write {
        return Err(format!("{} does not change anything; it needs no write permission", spec.tool));
    }
    if spec.once_only || spec.desktop_only {
        return Err(format!("{} is asked for every time; no permission can cover it", spec.tool));
    }
    let Some(param) = spec.resource_param else {
        return Err(format!("{} names no single thing, so no permission can cover it", spec.tool));
    };
    let Some(resource) = resource else {
        return Err(format!("name the `{param}` the permission covers in `resource`"));
    };
    if resource.len() > MAX_RESOURCE_LEN || resource.chars().any(char::is_control) {
        return Err(format!("`resource` must be at most {MAX_RESOURCE_LEN} characters on one line"));
    }
    Ok(WriteTarget {
        service: service.to_owned(),
        op: op.to_owned(),
        resource: resource.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_tool_has_metadata_with_the_same_keys() {
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let expected = keys(&for_tool("gmail_send").unwrap().meta());
        for spec in specs().iter().filter(|s| !s.desktop_only) {
            let meta = for_tool(spec.tool).unwrap().meta();
            assert_eq!(keys(&meta), expected, "{}", spec.tool);
        }
        assert_eq!(keys(&of_mcp_tool(false, false).meta()), expected);
        assert!(for_tool("nope").is_none());
    }

    #[test]
    fn a_write_grant_is_offered_only_where_one_can_cover_the_call() {
        let edit = for_tool("github_issue_comment_edit").unwrap();
        assert_eq!((edit.access, edit.standing, edit.every_time), (Access::Write, Some(Standing::Write), false));
        assert_eq!(edit.resource_param, Some("repo"));
        assert_eq!(edit.max_grant_secs(), 30 * 86_400);
        let delete = for_tool("github_repo_delete").unwrap();
        assert_eq!((delete.standing, delete.every_time), (None, true));
        assert_eq!(delete.max_grant_secs(), 0);
        assert_eq!(for_tool("github_repo_languages").unwrap().standing, None, "a read takes no write grant");
        assert_eq!(for_tool("gmail_send").unwrap().standing, Some(Standing::Send));
        assert_eq!(for_tool("reins_wait").unwrap().meta()["approval"], false);
        assert_eq!(of_mcp_tool(false, true).standing, None, "destructive: every time");
        assert_eq!(of_mcp_tool(true, false).standing, None, "a read takes no write grant");
        assert_eq!(of_mcp_tool(false, false).standing, Some(Standing::Write));
    }

    #[test]
    fn write_targets_are_checked() {
        let ok = write_target("github", "issue_comment_edit", Some(" me/app ")).unwrap();
        assert_eq!(
            ok,
            WriteTarget {
                service: "github".to_owned(),
                op: "issue_comment_edit".to_owned(),
                resource: "me/app".to_owned(),
            }
        );
        assert!(write_target("github", "issue_comment_edit", None).is_err(), "names the repo");
        assert!(write_target("github", "issue_comment_edit", Some("  ")).is_err());
        assert!(write_target("github", "issue_comment_edit", Some(&"x".repeat(201))).is_err());
        assert!(write_target("github", "issue_comment_edit", Some("a\nb")).is_err());
        assert!(write_target("github", "repo_delete", Some("me/app")).is_err(), "asked every time");
        assert!(write_target("github", "repo_languages", Some("me/app")).is_err(), "a read");
        assert!(write_target("github", "nope", Some("me/app")).is_err());
        let mcp = write_target(&mcp_grant_service("linear-2"), "create.issue", None).unwrap();
        assert_eq!(
            (mcp.service.as_str(), mcp.op.as_str(), mcp.resource.as_str()),
            ("mcp_linear_2", "", "create.issue")
        );
        assert!(write_target("mcp_linear", "create", Some("x")).is_err());
        assert!(write_target("mcp_", "create", None).is_err());
        assert!(write_target("mcp_Bad", "create", None).is_err());
        assert!(write_target("mcp_linear", " ", None).is_err());
        assert_eq!(write_target("mcp_linear_2", "", Some("create.issue")).unwrap(), mcp, "stored form is kept");
    }
}
