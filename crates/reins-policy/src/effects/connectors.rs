//! Connector tools (Gmail, Calendar, GitHub, Telegram, SMS, payments, vault), MCP tools by their annotations, file
//! hooks and explicit questions: each mapped to typed effects from its exact arguments (01 §2.2).

use reins_proto::connector::{Effect as Access, spec_for_tool};
use serde_json::Value;

use super::level::{ACCOUNTS, AGENT, Analysis};
use super::model::{Certainty, Channel, CodeSource, Effect, InfraChange, NetMethod};
use super::shell::{Context, Exp, Walk};

fn strs(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::String(s)) => {
            s.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
        }
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// The repository a GitHub tool call acts on, as `owner/name`, from the arguments it will use; `None` when they
/// don't name one exactly. With both `owner` and `repo`, `owner` decides: a `repo` of `other/x` with `owner=me` names
/// no repository (zoned Outside), never `me`'s.
#[must_use]
pub fn tool_repo(args: &Value) -> Option<String> {
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match (str_arg(args, "owner"), str_arg(args, "repo")) {
        (Some(o), Some(r)) => match r.split_once('/') {
            Some((ro, name)) if ro.eq_ignore_ascii_case(o) && valid(ro) && valid(name) => Some(format!("{ro}/{name}")),
            None if valid(o) && valid(r) => Some(format!("{o}/{r}")),
            _ => None,
        },
        (None, Some(r)) => match r.split_once('/') {
            Some((o, name)) if valid(o) && valid(name) => Some(r.to_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn repo_of(args: &Value) -> String {
    tool_repo(args).unwrap_or_else(|| "a repository not named exactly".to_owned())
}

fn full(effects: Vec<Effect>) -> Analysis {
    Analysis::new(effects, Certainty::Full, Vec::new())
}

/// A connector tool call with its arguments (as the AI sent them).
#[allow(clippy::too_many_lines, reason = "one arm per tool family")]
#[must_use]
pub fn analyze_tool(tool: &str, args: &Value) -> Analysis {
    let spec = spec_for_tool(tool);
    let service = spec.map_or_else(|| tool.split('_').next().unwrap_or(tool).to_owned(), |s| s.service.to_owned());
    let read = |what: &str| {
        full(vec![Effect::DataRead {
            service: service.clone(),
            resource: what.to_owned(),
        }])
    };
    let write = |what: &str, lost: bool| {
        full(vec![Effect::DataWrite {
            service: service.clone(),
            what: what.to_owned(),
            lost,
        }])
    };
    let message = |channel: Channel, to: Vec<String>, attachments: u32| {
        full(vec![Effect::Message {
            channel,
            to,
            attachments,
        }])
    };
    let repo = repo_of(args);
    match tool {
        "reins_list_accounts" => {
            return full(vec![Effect::DataRead {
                service: "reins".to_owned(),
                resource: ACCOUNTS.to_owned(),
            }]);
        }
        "reins_get_result" => return Analysis::default(),
        "reins_request_access" => {
            return full(vec![Effect::Permission {
                resource: "standing access to a service".to_owned(),
                change: "grant".to_owned(),
            }]);
        }
        "reins_upload" => {
            return full(vec![Effect::DataWrite {
                service: "reins".to_owned(),
                what: "Uploads a file for a later call".to_owned(),
                lost: false,
            }]);
        }
        // Mail (Gmail's own tools are not in the connector specs).
        "gmail_search"
        | "gmail_read"
        | "gmail_list_attachments"
        | "gmail_get_attachment"
        | "gmail_list_labels"
        | "gmail_list_filters"
        | "gmail_get_vacation" => {
            return read("mail");
        }
        "gmail_send" => {
            let mut to = strs(args, "to");
            to.extend(strs(args, "cc"));
            to.extend(strs(args, "bcc"));
            let attachments = u32::try_from(strs(args, "attachments").len()).unwrap_or(u32::MAX);
            return message(Channel::Email, to, attachments);
        }
        "gmail_create_draft" | "gmail_create_label" | "gmail_create_filter" => {
            return write("Creates a draft, label or filter", false);
        }
        "gmail_trash" | "gmail_organize" | "gmail_delete_label" | "gmail_delete_filter" | "gmail_rename_label" => {
            return write("Moves, labels or trashes mail", true);
        }
        "gmail_set_vacation" => return message(Channel::Email, vec!["everyone who writes to you".to_owned()], 0),
        // Messages.
        "telegram_send" => {
            return message(
                Channel::Telegram,
                [strs(args, "chat"), strs(args, "to"), strs(args, "chat_id")].concat(),
                0,
            );
        }
        "sms_send" => {
            return message(Channel::Sms, [strs(args, "to"), strs(args, "number"), strs(args, "thread")].concat(), 0);
        }
        // Calendar.
        "calendar_create_event" | "device_calendar_create_event" => {
            let guests = [strs(args, "attendees"), strs(args, "guests")].concat();
            return if guests.is_empty() {
                write("Creates a calendar event", false)
            } else {
                message(Channel::Calendar, guests, 0)
            };
        }
        "calendar_delete_event" => return write("Deletes a calendar event", true),
        // Payments.
        "payments_purchase_request" | "payments_purchase_complete" => {
            return full(vec![Effect::Money {
                amount: args
                    .get("amount")
                    .or_else(|| args.get("total"))
                    .map(|v| v.to_string().trim_matches('"').to_owned()),
                merchant: str_arg(args, "merchant").map(str::to_owned),
            }]);
        }
        // Vault.
        "vault_get"
        | "vault_item_view"
        | "vault_attachment_get"
        | "vault_generate_check_password"
        | "vault_send_get" => {
            return full(vec![Effect::SecretOut {
                secret: "a vault item".to_owned(),
                destination: AGENT.to_owned(),
            }]);
        }
        "vault_send_create" | "vault_send_update" | "vault_send_remove_password" => {
            return full(vec![Effect::Publish {
                target: "a vault item as a link".to_owned(),
            }]);
        }
        "vault_trash_empty" | "vault_item_delete" | "vault_folder_delete" | "vault_attachment_delete" => {
            return full(vec![Effect::Infra {
                target: "vault items".to_owned(),
                change: InfraChange::Destroy,
                local: false,
            }]);
        }
        // GitHub.
        "github_pr_create"
        | "github_comment"
        | "github_create_issue"
        | "github_issue_update"
        | "github_pr_update"
        | "github_pr_ready"
        | "github_pr_draft"
        | "github_pr_review_create"
        | "github_pr_review_comment_create"
        | "github_issue_comment_edit"
        | "github_pr_review_comment_edit"
        | "github_issue_assignee_add"
        | "github_pr_reviewers_request"
        | "github_issue_label_add"
        | "github_issue_lock"
        | "github_deployment_review"
        | "github_issue_transfer" => {
            return message(Channel::GitHub, vec![repo], 0);
        }
        "github_pr_merge" | "github_branch_merge" => {
            let base = str_arg(args, "base");
            return full(vec![
                Effect::GitPush {
                    repo: repo.clone(),
                    refname: base.map_or_else(|| "the pull request's base".to_owned(), |b| format!("refs/heads/{b}")),
                    default_branch: base.map_or(Some(true), |b| super::git::default_of(b, None)),
                    rewrites: Some(false),
                    commits: 0,
                },
                Effect::Message {
                    channel: Channel::GitHub,
                    to: vec![repo],
                    attachments: 0,
                },
            ]);
        }
        "github_branch_delete" => {
            let branch = str_arg(args, "branch").unwrap_or("a branch");
            return full(vec![Effect::GitRefDelete {
                repo,
                refname: format!("refs/heads/{branch}"),
                default_branch: super::git::default_of(branch, None),
            }]);
        }
        "github_branch_create"
        | "github_tag_create"
        | "github_label_create"
        | "github_milestone_create"
        | "github_star_add"
        | "github_watch_add"
        | "github_follow_add"
        | "github_gist_star"
        | "github_issue_reaction_add"
        | "github_notification_mark_read" => return write("Creates something on GitHub", false),
        "github_file_put" | "github_file_delete" => {
            let branch = str_arg(args, "branch");
            return full(vec![Effect::GitPush {
                repo,
                refname: branch.map_or_else(|| "the default branch".to_owned(), |b| format!("refs/heads/{b}")),
                default_branch: branch.map_or(Some(true), |b| super::git::default_of(b, None)),
                rewrites: Some(false),
                commits: 1,
            }]);
        }
        "github_workflow_dispatch"
        | "github_run_rerun"
        | "github_run_rerun_failed"
        | "github_job_rerun"
        | "github_run_approve"
        | "github_workflow_enable" => {
            return full(vec![Effect::Exec {
                program: format!("a workflow on {repo}"),
                source: CodeSource::Ci,
            }]);
        }
        "github_release_create"
        | "github_release_update"
        | "github_release_asset_upload"
        | "github_gist_create"
        | "github_repo_visibility_set"
        | "github_repo_fork" => {
            let public = tool != "github_gist_create" || args.get("public").and_then(Value::as_bool).unwrap_or(false);
            if public {
                return full(vec![Effect::Publish {
                    target: format!("{} ({repo})", tool.trim_start_matches("github_").replace('_', " ")),
                }]);
            }
            return write("Creates a secret gist", false);
        }
        // A new repository is public unless it is asked private (GitHub's own default).
        "github_repo_create" => {
            let private = args.get("private").and_then(Value::as_bool).unwrap_or(false)
                || str_arg(args, "visibility").is_some_and(|v| v != "public");
            if !private {
                return full(vec![Effect::Publish {
                    target: format!("a public repository ({})", str_arg(args, "name").unwrap_or("new")),
                }]);
            }
            return write("Creates a private repository", false);
        }
        "github_repo_delete"
        | "github_release_delete"
        | "github_tag_delete"
        | "github_environment_delete"
        | "github_repo_transfer"
        | "github_artifact_delete"
        | "github_release_asset_delete"
        | "github_run_delete"
        | "github_gist_delete" => {
            return full(vec![Effect::Infra {
                target: format!("{} ({repo})", tool.trim_start_matches("github_").replace('_', " ")),
                change: InfraChange::Destroy,
                local: false,
            }]);
        }
        "github_collaborator_add"
        | "github_collaborator_remove"
        | "github_deploy_key_add"
        | "github_deploy_key_delete"
        | "github_branch_protection_set"
        | "github_branch_protection_delete"
        | "github_secret_set"
        | "github_secret_delete"
        | "github_repo_team_add"
        | "github_repo_team_remove"
        | "github_environment_set"
        | "github_webhook_create"
        | "github_webhook_update"
        | "github_webhook_delete"
        | "github_invitation_cancel" => {
            return full(vec![Effect::Permission {
                resource: repo,
                change: tool.trim_start_matches("github_").replace('_', " "),
            }]);
        }
        "github_api_write" => {
            let method = str_arg(args, "method").unwrap_or("POST").to_ascii_uppercase();
            let path = str_arg(args, "path").unwrap_or_default();
            return github_api(&method, path);
        }
        _ => {}
    }
    let Some(spec) = spec else {
        return analyze_mcp("", tool, None);
    };
    match spec.effect {
        Access::List | Access::Read | Access::Search => read(&format!("{} ({})", spec.title, spec.service)),
        Access::Write => {
            if spec.once_only {
                full(vec![Effect::Infra {
                    target: spec.title.to_owned(),
                    change: InfraChange::Destroy,
                    local: false,
                }])
            } else {
                full(vec![Effect::DataWrite {
                    service: spec.service.to_owned(),
                    what: spec.title.to_owned(),
                    lost: true,
                }])
            }
        }
    }
}

/// A call to GitHub's REST API by method and path.
#[must_use]
pub fn github_api(method: &str, path: &str) -> Analysis {
    let p = path.trim_start_matches('/').to_ascii_lowercase();
    let parts: Vec<&str> = p.split('/').collect();
    let repo = if parts.first() == Some(&"repos") && parts.len() >= 3 {
        format!("{}/{}", parts[1], parts[2])
    } else {
        "GitHub".to_owned()
    };
    if method == "GET" || method == "HEAD" {
        return full(vec![Effect::DataRead {
            service: "GitHub".to_owned(),
            resource: path.to_owned(),
        }]);
    }
    let has = |w: &str| parts.contains(&w);
    let e = if has("collaborators")
        || has("keys")
        || has("protection")
        || has("secrets")
        || has("hooks")
        || has("rulesets")
        || has("permissions")
        || has("teams")
        || has("invitations")
    {
        Effect::Permission {
            resource: repo,
            change: format!("{method} {path}"),
        }
    } else if (method == "DELETE" && parts.len() == 3 && parts[0] == "repos") || has("transfer") {
        Effect::Infra {
            target: repo,
            change: InfraChange::Destroy,
            local: false,
        }
    } else if has("releases") || (method == "PATCH" && parts.len() == 3 && parts[0] == "repos") {
        Effect::Publish {
            target: format!("{path} ({repo})"),
        }
    } else if has("comments") || has("issues") || has("pulls") || has("reviews") {
        Effect::Message {
            channel: Channel::GitHub,
            to: vec![repo],
            attachments: 0,
        }
    } else if has("refs") || has("git") || has("merges") || has("contents") {
        Effect::GitPush {
            repo,
            refname: path.to_owned(),
            default_branch: None,
            rewrites: None,
            commits: 0,
        }
    } else if has("dispatches") || has("rerun") || has("actions") {
        Effect::Exec {
            program: format!("a workflow ({repo})"),
            source: CodeSource::Ci,
        }
    } else {
        return Analysis::new(
            vec![Effect::Unknown {
                reason: format!("GitHub API {method} {path}"),
            }],
            Certainty::Partial,
            vec![format!("what {method} {path} does on GitHub")],
        );
    };
    full(vec![e])
}

/// An MCP tool's annotations, as its server declares them (untrusted hints: they make certainty `Partial`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolAnnotations {
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    pub open_world: Option<bool>,
}

impl ToolAnnotations {
    /// From an MCP tool definition's `annotations` object.
    #[must_use]
    pub fn from_json(v: &Value) -> Self {
        let b = |k: &str| v.get(k).and_then(Value::as_bool);
        Self {
            read_only: b("readOnlyHint"),
            destructive: b("destructiveHint"),
            open_world: b("openWorldHint"),
        }
    }
}

/// Common MCP servers whose tools Reins knows: (server, tool name fragment, what it does).
const KNOWN_SERVERS: &[(&str, &str, Known)] = &[
    ("slack", "post", Known::Message),
    ("slack", "send", Known::Message),
    ("slack", "reply", Known::Message),
    ("slack", "list", Known::Read),
    ("slack", "get", Known::Read),
    ("slack", "search", Known::Read),
    ("linear", "create", Known::Message),
    ("linear", "comment", Known::Message),
    ("linear", "update", Known::Message),
    ("linear", "get", Known::Read),
    ("linear", "list", Known::Read),
    ("linear", "search", Known::Read),
    ("jira", "create", Known::Message),
    ("jira", "comment", Known::Message),
    ("jira", "transition", Known::Message),
    ("jira", "get", Known::Read),
    ("jira", "search", Known::Read),
    ("notion", "create", Known::Write),
    ("notion", "update", Known::Write),
    ("notion", "append", Known::Write),
    ("notion", "delete", Known::Lost),
    ("notion", "search", Known::Read),
    ("notion", "get", Known::Read),
    ("notion", "retrieve", Known::Read),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Known {
    Read,
    Write,
    Lost,
    Message,
}

/// A tool of an MCP server Reins does not ship: the per-server table, else its annotations, else unknown.
#[must_use]
pub fn analyze_mcp(server: &str, tool: &str, annotations: Option<&ToolAnnotations>) -> Analysis {
    let s = server.to_ascii_lowercase();
    // The tool's words (`delete_posted_message`, `getUser`): matched whole, never inside another word.
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in tool.chars() {
        if (!ch.is_ascii_alphanumeric() || (ch.is_ascii_uppercase() && prev_lower)) && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        if ch.is_ascii_alphanumeric() {
            cur.push(ch.to_ascii_lowercase());
        }
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let has = |frag: &str| words.iter().any(|w| w == frag);
    // A destructive verb wins over any other word in the name.
    let destructive = ["delete", "remove", "drop", "destroy", "purge", "erase", "wipe", "trash", "clear", "revoke"];
    let known = if KNOWN_SERVERS.iter().any(|(srv, ..)| s.contains(srv)) && destructive.iter().any(|d| has(d)) {
        Some(Known::Lost)
    } else {
        KNOWN_SERVERS.iter().find(|(srv, frag, _)| s.contains(srv) && has(frag)).map(|(_, _, k)| *k)
    };
    if let Some(known) = known {
        let e = match known {
            Known::Read => Effect::DataRead {
                service: server.to_owned(),
                resource: tool.to_owned(),
            },
            Known::Write | Known::Lost => Effect::DataWrite {
                service: server.to_owned(),
                what: tool.to_owned(),
                lost: known == Known::Lost,
            },
            Known::Message => Effect::Message {
                channel: Channel::Other(server.to_owned()),
                to: vec![format!("people on {server}")],
                attachments: 0,
            },
        };
        return full(vec![e]);
    }
    let unknown = || {
        Analysis::new(
            vec![Effect::Unknown {
                reason: format!(
                    "the MCP tool {tool}{}",
                    if server.is_empty() {
                        String::new()
                    } else {
                        format!(" on {server}")
                    }
                ),
            }],
            Certainty::Partial,
            vec![format!("what the tool {tool} does")],
        )
    };
    let Some(a) = annotations else {
        return unknown();
    };
    let e = if a.read_only == Some(true) {
        Effect::DataRead {
            service: server.to_owned(),
            resource: tool.to_owned(),
        }
    } else if a.destructive == Some(true) {
        Effect::Infra {
            target: format!("{tool} on {server}"),
            change: InfraChange::Destroy,
            local: false,
        }
    } else if a.open_world == Some(true) {
        Effect::Network {
            host: server.to_owned(),
            method: NetMethod::Write,
        }
    } else if a.destructive == Some(false) {
        Effect::DataWrite {
            service: server.to_owned(),
            what: tool.to_owned(),
            lost: true,
        }
    } else {
        return unknown();
    };
    // The server's own word for it, not read here.
    Analysis::new(vec![e], Certainty::Partial, vec![format!("what the tool {tool} does (as its server describes it)")])
}

/// A file hook (an agent's read or edit tool): the files it reads or changes.
#[must_use]
pub fn analyze_files(paths: &[&str], write: bool, ctx: &Context) -> Analysis {
    let mut w = Walk::new(ctx);
    for p in paths {
        let e = Exp::lit(p);
        if write {
            let path = w.path(&e);
            w.write(path);
        } else {
            let kind = super::shell::file_kind(p);
            let path = w.path(&e);
            w.effect(Effect::FileRead {
                path,
                kind,
            });
        }
    }
    let mut a = Analysis::new(std::mem::take(&mut w.effects), w.certainty, std::mem::take(&mut w.unresolved));
    a.labels =
        crate::guard::labels(&paths.join("\n").lines().collect::<Vec<_>>()).into_iter().map(|g| g.label).collect();
    a
}

/// An explicit question (`reins ask`, a work session): meant for a person, nothing to read.
#[must_use]
pub fn analyze_question() -> Analysis {
    Analysis::new(
        vec![Effect::Unknown {
            reason: "a question for you".to_owned(),
        }],
        Certainty::None,
        vec!["a question meant for you".to_owned()],
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::effects::{Floor, Level};

    #[test]
    fn connector_maps() {
        let level = |t: &str, a: Value| analyze_tool(t, &a).level;
        assert_eq!(level("gmail_search", json!({"query": "x"})), Level::L0);
        assert_eq!(level("gmail_send", json!({"to": "a@example.com", "body": "x"})), Level::L2);
        let many: Vec<String> = (0..12).map(|i| format!("p{i}@example.com")).collect();
        let mass = analyze_tool("gmail_send", &json!({"to": many}));
        assert_eq!((mass.level, mass.floor), (Level::L3, vec![Floor::MassMessage]));
        assert_eq!(level("gmail_create_draft", json!({})), Level::L0);
        assert_eq!(level("gmail_trash", json!({})), Level::L1);
        assert_eq!(level("calendar_create_event", json!({"title": "x"})), Level::L0);
        assert_eq!(level("calendar_create_event", json!({"attendees": ["b@example.com"]})), Level::L2);
        assert_eq!(level("github_pr_create", json!({"repo": "dkat/reins", "head": "f", "base": "main"})), Level::L2);
        assert_eq!(level("github_pr_merge", json!({"repo": "dkat/reins", "number": 1})), Level::L2);
        // The default branch is not known here: any branch may be it.
        assert_eq!(level("github_branch_delete", json!({"repo": "dkat/reins", "branch": "feat/x"})), Level::L3);
        assert_eq!(level("github_pr_list", json!({"repo": "dkat/reins"})), Level::L0);
        assert!(analyze_tool("github_repo_delete", &json!({"repo": "dkat/reins"})).needs_human());
        assert!(
            analyze_tool("github_collaborator_add", &json!({"repo": "dkat/reins"})).floor.contains(&Floor::Permission)
        );
        assert!(analyze_tool("payments_purchase_request", &json!({"amount": 12})).floor.contains(&Floor::Money));
        assert!(analyze_tool("vault_get", &json!({"id": "x"})).floor.contains(&Floor::SecretRelease));
        assert!(analyze_tool("vault_send_create", &json!({})).floor.contains(&Floor::Publish));
        assert!(analyze_tool("reins_list_accounts", &json!({})).floor.contains(&Floor::AccountListing));
        assert_eq!(level("github_api_write", json!({"method": "DELETE", "path": "/repos/dkat/reins"})), Level::L3);
        assert_eq!(
            analyze_tool("github_api_write", &json!({"method": "POST", "path": "/zzz"})).certainty,
            Certainty::Partial
        );
    }

    #[test]
    fn mcp_annotations() {
        let a = |r, d, o| ToolAnnotations {
            read_only: r,
            destructive: d,
            open_world: o,
        };
        // The server's word only: partly read, so at least L1 (01 §2.4).
        assert_eq!(analyze_mcp("x", "t", Some(&a(Some(true), None, None))).level, Level::L1);
        assert_eq!(analyze_mcp("x", "t", Some(&a(Some(false), Some(true), None))).level, Level::L3);
        assert_eq!(analyze_mcp("x", "t", Some(&a(Some(false), Some(false), Some(true)))).level, Level::L2);
        assert_eq!(analyze_mcp("x", "t", Some(&a(Some(false), Some(false), Some(false)))).level, Level::L1);
        assert_eq!(analyze_mcp("slack", "post_message", None).level, Level::L2);
        assert_eq!(analyze_mcp("notion", "search", None).level, Level::L0);
    }

    #[test]
    fn unknown_mcp_tool_is_partial() {
        let a = analyze_mcp("weird", "frobnicate", None);
        assert_eq!((a.certainty, a.level), (Certainty::Partial, Level::L1));
        assert!(!a.unresolved.is_empty());
        assert!(analyze_question().needs_human());
    }
}
