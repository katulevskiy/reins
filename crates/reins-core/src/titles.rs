//! What a request is, in the same words on every screen: the thing it touches (a terminal for a command, a file, the
//! vault's secrets, an integration's logo), what it does to it (run, read, write, send, delete, push, ...) and the
//! request itself ("Run `git push --force origin main`", "Push 3 commits to acme/web"). Worked out here once, from the
//! call, for the waiting requests and kept with each Activity entry together with what was asked in full (the command
//! with its arguments, where it ran, why it was asked), so no screen has to guess from a tool's generic title.
//!
//! A hook of the desktop app asks with `desktop_ask`: the question "Claude Code wants to run: git push", the detail
//! "Command:\n<the command>\n\nIn: <folder>\nRule: <pattern>" (or "Files:\n<paths>") and the topic `command:<pattern>`
//! (or `file:<pattern>`). Those parts are read back here; a question in other words is shown as it was asked.

use reins_proto::connector::{ConnectorCall, DESKTOP, Effect, VAULT};
use reins_proto::gmail::{MessageSummary, ToolCall};
use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::store::{AuditRecord, AuditRequest};
use crate::text;
use crate::types::{ActivityRequest, RequestLabel};
use crate::views::{self, ParkedRequest};

/// The most bytes kept of each text of a request (the command, what the agent said).
pub const MAX_FIELD: usize = 8 * 1024;
/// The most characters of a subject in a title (the whole command stays in the full request).
const MAX_SUBJECT: usize = 300;

/// What else is known about a call besides its arguments, when it is known.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Who the emails are from (a Gmail read).
    pub senders: Vec<String>,
    /// What it acts on, by its name (a chat, a calendar, a repository).
    pub resource: Option<String>,
    /// An MCP server's name, the tool's title, and whether the tool only reads or deletes.
    pub mcp: Option<McpContext>,
}

#[derive(Clone, Debug, Default)]
pub struct McpContext {
    pub server: String,
    pub title: String,
    pub read_only: bool,
    pub destructive: bool,
}

impl Context {
    /// What a parked request holds: the senders of what a read found, the name of what a call acts on.
    #[must_use]
    pub fn of(parked: &ParkedRequest) -> Self {
        let held = parked.connector.as_ref();
        Self {
            senders: parked.messages.iter().map(sender).collect(),
            resource: held
                .and_then(|h| {
                    h.preview
                        .as_ref()
                        .map(|p| p.resource_label.clone())
                        .or_else(|| h.items.first().map(|i| i.resource_label.clone()))
                })
                .filter(|l| !l.trim().is_empty()),
            mcp: held.and_then(|h| h.mcp.as_ref()).map(|m| McpContext {
                server: m.server_name.clone(),
                title: m.title.clone(),
                read_only: m.read_only,
                destructive: m.destructive,
            }),
        }
    }
}

fn sender(m: &MessageSummary) -> String {
    let from = m.from.trim();
    let name = from.split('<').next().unwrap_or_default().trim().trim_matches('"').trim();
    if name.is_empty() {
        from.trim_start_matches('<').trim_end_matches('>').to_owned()
    } else {
        name.to_owned()
    }
}

fn line(s: &str) -> String {
    text::truncate_chars(&text::mark_invisible(&text::one_line(s)), MAX_SUBJECT)
}

/// Cleaned ([`text::neutralize`], invisible characters shown) and at most [`MAX_FIELD`] bytes, ending in `…` when cut.
fn field(s: &str) -> String {
    let clean = text::mark_invisible(&text::neutralize(s.trim_end()));
    if clean.len() <= MAX_FIELD {
        return clean;
    }
    let mut cut = text::truncate_bytes(&clean, MAX_FIELD - '…'.len_utf8()).to_owned();
    cut.push('…');
    cut
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn label(
    provider: &str,
    action: &str,
    lead: impl Into<String>,
    subject: impl Into<String>,
    code: bool,
) -> RequestLabel {
    RequestLabel {
        provider: provider.to_owned(),
        action: action.to_owned(),
        lead: lead.into(),
        subject: subject.into(),
        code,
        tail: String::new(),
    }
}

/// The label of a waiting request, with what it holds.
#[must_use]
pub fn of_parked(parked: &ParkedRequest) -> RequestLabel {
    of_call(&parked.request.call, &Context::of(parked))
}

/// The label of a call.
#[must_use]
pub fn of_call(call: &ToolCall, cx: &Context) -> RequestLabel {
    match call {
        ToolCall::GmailSearch {
            query,
            ..
        } => {
            let q = line(query);
            if q.is_empty() {
                label("gmail", "search", "Search Gmail", "", false)
            } else {
                label("gmail", "search", "Search Gmail for", q, false)
            }
        }
        ToolCall::GmailRead {
            message_ids,
        } => {
            let what = format!("Read {}", plural(message_ids.len().max(cx.senders.len()), "email", "emails"));
            let mut from: Vec<&String> = cx.senders.iter().filter(|s| !s.trim().is_empty()).collect();
            from.dedup();
            match from.as_slice() {
                [one] => label("gmail", "read", format!("{what} from"), line(one), false),
                _ => label("gmail", "read", what, "", false),
            }
        }
        ToolCall::GmailSend {
            email,
        } => {
            let mut l =
                label("gmail", "send", "Send an email to", line(email.to.first().map_or("", String::as_str)), false);
            let more = email.recipients().count().saturating_sub(1);
            if more > 0 {
                l.tail = format!(" and {more} more");
            }
            l
        }
        ToolCall::RequestGrant {
            grant,
        } => label("gmail", "grant", "Ask for access:", line(&views::grant_request_summary(grant)), false),
        ToolCall::ListAccounts {
            service,
            ..
        } => match service.as_deref() {
            Some(s) => label(s, "read", format!("See your {} accounts", views::service_name(s)), "", false),
            None => label("desktop", "read", "See your integrations", "", false),
        },
        ToolCall::Connector(call) => connector(call, cx),
        ToolCall::Mcp(call) => {
            let m = cx.mcp.clone().unwrap_or_default();
            let action = if m.destructive {
                "delete"
            } else if m.read_only {
                "read"
            } else {
                "write"
            };
            let (subject, code) = if m.title.trim().is_empty() || m.title == call.tool {
                (line(&call.tool), true)
            } else {
                (line(&m.title), false)
            };
            let mut l = label(&format!("mcp:{}", call.server), action, "Use", subject, code);
            if !m.server.trim().is_empty() {
                l.tail = format!(" on {}", line(&m.server));
            }
            l
        }
        ToolCall::RequestUpload {
            name,
            ..
        } => label(crate::blob::SERVICE_FILES, "send", "Share a file:", line(name), false),
    }
}

/// A call to an integration (or to the desktop app itself).
fn connector(call: &ConnectorCall, cx: &Context) -> RequestLabel {
    let op = call.op.as_str();
    match (call.service.as_str(), op) {
        (DESKTOP, "ask") => ask(call),
        (DESKTOP, reins_proto::connector::SESSION_OP) => {
            label(DESKTOP, "grant", "Start a work session:", line(call.str_arg("reason").unwrap_or_default()), false)
        }
        (DESKTOP, reins_proto::connector::SESSION_END_OP) => label(DESKTOP, "grant", "End a work session", "", false),
        (VAULT, "secret_release") => {
            let n = call.args.get("secrets").and_then(Value::as_array).map_or(0, Vec::len);
            let command = line(call.str_arg("command").unwrap_or_default());
            let what = format!("Use {}", plural(n, "secret", "secrets"));
            if command.is_empty() {
                label("secrets", "secrets", what, "", false)
            } else {
                label("secrets", "secrets", format!("{what} for"), command, true)
            }
        }
        (VAULT, "ssh_sign") => {
            let host = call.str_arg("host").or_else(|| call.str_arg("host_key")).map(line).unwrap_or_default();
            if host.is_empty() {
                label("ssh", "secrets", "Sign in with an SSH key", "", false)
            } else {
                label("ssh", "secrets", "Sign in to", host, true)
            }
        }
        (VAULT, "ssh_keys") => label("ssh", "read", "See your SSH keys", "", false),
        (_, reins_proto::desktop::GIT_PUSH_OP | reins_proto::desktop::GIT_TAG_PUSH_OP) => git_push(call),
        (_, reins_proto::desktop::GIT_FETCH_OP) => {
            label(&call.service, "read", "Clone and fetch", line(call.str_arg("repo").unwrap_or_default()), false)
        }
        _ => {
            let spec = call.spec();
            let effect = call.effect();
            let deletes = ["delete", "remove", "trash", "destroy"].iter().any(|w| op.contains(w));
            let action = match effect {
                Some(Effect::Search | Effect::List) => "search",
                Some(Effect::Write) if deletes => "delete",
                Some(Effect::Write) if op == "send" || op.starts_with("send_") => "send",
                Some(Effect::Write) => "write",
                _ => "read",
            };
            let title = spec.map_or_else(|| format!("Use {}", call.op.replace('_', " ")), |s| s.title.to_owned());
            let query = call.str_arg("query").map(line).filter(|q| !q.is_empty());
            let on = cx
                .resource
                .as_deref()
                .map(line)
                .filter(|r| !r.is_empty())
                .or_else(|| call.resource().map(|r| line(&r)));
            match (effect, query, on) {
                (Some(Effect::Search), Some(q), _) => label(&call.service, action, format!("{title} for"), q, false),
                (_, _, Some(r)) if !r.is_empty() => label(&call.service, action, format!("{title} in"), r, false),
                _ => label(&call.service, action, title, "", false),
            }
        }
    }
}

/// "Push 3 commits to acme/web", "Force-push 2 commits to ...", "Push tag v1.2 to ...", "Delete branch old from ...".
fn git_push(call: &ConnectorCall) -> RequestLabel {
    let repo = line(call.str_arg("repo").unwrap_or_default());
    let Some(view) = views::git_push_view(call) else {
        return label(&call.service, "push", "Push to", repo, false);
    };
    let refs = &view.refs;
    if !refs.is_empty() && refs.iter().all(|r| r.change == "delete") {
        let what = match refs.as_slice() {
            [one] => format!("Delete {} {}", one.kind, one.short_name),
            _ => format!("Delete {} refs", refs.len()),
        };
        return label(&call.service, "delete", format!("{what} from"), repo, false);
    }
    if !refs.is_empty() && refs.iter().all(|r| r.kind == "tag") {
        let what = match refs.as_slice() {
            [one] => format!("Push tag {}", one.short_name),
            _ => format!("Push {} tags", refs.len()),
        };
        return label(&call.service, "push", format!("{what} to"), repo, false);
    }
    let commits: u32 = refs.iter().map(|r| r.commit_count).sum();
    let verb = if refs.iter().any(|r| r.force) {
        "Force-push"
    } else {
        "Push"
    };
    let what = if commits == 0 {
        verb.to_owned()
    } else {
        format!("{verb} {}", plural(commits as usize, "commit", "commits"))
    };
    label(&call.service, "push", format!("{what} to"), repo, false)
}

/// What a hook's question holds, read back from its detail (see the module's comment).
#[derive(Debug, Default, PartialEq, Eq)]
struct Hook {
    /// The agent, from "<agent> wants to …".
    agent: String,
    command: Option<String>,
    files: Vec<String>,
    write: bool,
    cwd: String,
    rule: String,
}

fn hook(call: &ConnectorCall) -> Option<Hook> {
    let detail = call.str_arg("detail").unwrap_or_default();
    // A hook's rule lines come first (`reins_desktop::hooks::rule_lines`): the rule, then the rest as before.
    let (reported, detail) = match crate::connector::desktop::reported_rule(detail) {
        Some((rule, rest)) => (Some(rule.pattern), rest),
        None => (None, detail),
    };
    let question = text::one_line(call.str_arg("question").unwrap_or_default());
    let (body, tail) = match detail.rfind("\n\nIn: ").or_else(|| detail.rfind("\nRule: ")) {
        Some(at) => detail.split_at(at),
        None => (detail, ""),
    };
    let mut h = Hook::default();
    for l in tail.lines() {
        if let Some(cwd) = l.strip_prefix("In: ") {
            cwd.trim().clone_into(&mut h.cwd);
        } else if let Some(rule) = l.strip_prefix("Rule: ") {
            rule.trim().clone_into(&mut h.rule);
        }
    }
    if let Some(rule) = reported {
        h.rule = rule;
    }
    if let Some(command) = body.strip_prefix("Command:\n") {
        h.command = Some(command.trim_end().to_owned());
    } else if let Some(files) = body.strip_prefix("Files:\n") {
        h.files = files.lines().map(str::trim).filter(|f| !f.is_empty()).map(str::to_owned).collect();
    }
    let topic = call.str_arg("topic").unwrap_or_default().trim();
    if h.command.is_none() && h.files.is_empty() {
        // A question without the detail (or an older app): what the question and the topic still say.
        if let Some((_, command)) = question.split_once(" wants to run: ") {
            h.command = Some(command.to_owned());
        } else if let Some(pattern) = topic.strip_prefix("command:") {
            h.command = Some(pattern.to_owned());
        } else if topic.starts_with("file:") {
            let path = ["read ", "change "]
                .iter()
                .find_map(|verb| question.split_once(&format!(" wants to {verb}")).map(|(_, p)| p.to_owned()));
            h.files = vec![path.unwrap_or_else(|| topic.trim_start_matches("file:").to_owned())];
        } else {
            return None;
        }
    }
    h.write = question.contains(" wants to change ");
    if let Some((who, _)) = question.split_once(" wants to ") {
        who.trim().clone_into(&mut h.agent);
    }
    Some(h)
}

/// A command that deletes: `rm`, a drop, a destroy, a delete.
fn deletes(command: &str) -> bool {
    let lower = command.to_lowercase();
    let first_words = lower.split(['\n', ';', '&', '|']).filter_map(|c| {
        let word = c.split_whitespace().find(|w| !w.contains('=') && *w != "sudo")?;
        Some(word.rsplit('/').next().unwrap_or(word).to_owned())
    });
    let mut first = first_words.collect::<Vec<_>>();
    first.dedup();
    first.iter().any(|w| matches!(w.as_str(), "rm" | "rmdir" | "del" | "rd" | "unlink" | "shred" | "erase"))
        || [
            "drop table",
            "drop database",
            "drop schema",
            " destroy",
            "kubectl delete",
            "repo delete",
            "branch -d",
            "push --delete",
            "remove-item",
        ]
        .iter()
        .any(|n| lower.contains(n))
}

/// A question from the desktop app.
fn ask(call: &ConnectorCall) -> RequestLabel {
    let question = line(call.str_arg("question").unwrap_or_default());
    match hook(call) {
        Some(Hook {
            command: Some(command),
            ..
        }) => label(
            "terminal",
            if deletes(&command) {
                "delete"
            } else {
                "run"
            },
            "Run",
            line(&command),
            true,
        ),
        Some(h) if !h.files.is_empty() => {
            let mut l = label(
                "file",
                if h.write {
                    "write"
                } else {
                    "read"
                },
                if h.write {
                    "Change"
                } else {
                    "Read"
                },
                line(&h.files[0]),
                true,
            );
            if h.files.len() > 1 {
                l.tail = format!(" and {} more", h.files.len() - 1);
            }
            l
        }
        // A question in its own words: the question is the title.
        _ => label(DESKTOP, "ask", question, "", false),
    }
}

/// Why a hook asked, in a few words: what makes the command or file risky, else the rule it matched.
fn why_asked(call: &ConnectorCall, h: &Hook) -> String {
    let topic = call.str_arg("topic").unwrap_or_default();
    let all = format!(
        "{topic}\n{}\n{}",
        call.str_arg("question").unwrap_or_default(),
        call.str_arg("detail").unwrap_or_default()
    );
    // What cannot be undone, or a secret file (a question about one is asked because of what it reads).
    let risky = crate::connector::desktop::destructive_ask(
        Some(topic).filter(|t| !t.is_empty()),
        call.str_arg("question").unwrap_or_default(),
        call.str_arg("detail").unwrap_or_default(),
    ) || topic.starts_with("file:");
    let lower = all.to_lowercase();
    let reason = if !risky {
        None
    } else if topic.starts_with("file:") || !h.files.is_empty() {
        Some(if h.write {
            "Changing a protected file"
        } else {
            "Reading a secret file"
        })
    } else if lower.contains("reset --hard") || lower.contains("checkout -f") || lower.contains("clean -") {
        Some("Throwing away changes")
    } else if lower.contains("push") && !lower.contains("--delete") && !lower.contains(" :") {
        Some("Rewriting git history")
    } else if h.command.as_deref().is_some_and(deletes) || lower.contains("delete") {
        Some("Deleting for good")
    } else {
        Some("Can't be undone")
    };
    match (reason, h.rule.is_empty()) {
        (Some(r), _) => r.to_owned(),
        (None, false) => format!("Your rule: {}", line(&h.rule)),
        (None, true) => String::new(),
    }
}

/// Argument names never kept in full: keys, nonces, digests, what is sealed, and anything that may hold a secret.
const HIDDEN_ARGS: [&str; 14] = [
    "client_key",
    "nonce",
    "digest",
    "data_base64",
    "sealed",
    "password",
    "notes",
    "fields",
    "custom_fields",
    "totp",
    "token",
    "secret",
    "summary",
    "body_base64",
];

/// The vault's arguments that name things (never a value).
pub(crate) const VAULT_ARGS: [&str; 14] = [
    "item",
    "name",
    "folder",
    "query",
    "secrets",
    "command",
    "purpose",
    "host",
    "host_key",
    "key",
    "field",
    "file_name",
    "lease_secs",
    "kind",
];

fn shown_arg(service: &str, name: &str) -> bool {
    if service == VAULT {
        return VAULT_ARGS.contains(&name);
    }
    let lower = name.to_lowercase();
    !HIDDEN_ARGS.contains(&lower.as_str())
        && !lower.ends_with("_key")
        && !lower.ends_with("apikey")
        && !["password", "secret", "token", "credential", "private", "cvv", "card_number", "authorization"]
            .iter()
            .any(|w| lower.contains(w))
}

/// A tool call as text: its name, then each argument on its own line (`name: value`, a long value on the lines after).
fn call_text(tool: &str, service: &str, args: &Map<String, Value>) -> String {
    let mut out = tool.to_owned();
    for (name, value) in args.iter().filter(|(n, _)| shown_arg(service, n)) {
        let value = match value {
            Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
        if value.contains('\n') {
            let _ = write!(out, "\n{name}:\n{value}");
        } else {
            let _ = write!(out, "\n{name}: {value}");
        }
        if out.len() > MAX_FIELD {
            break;
        }
    }
    out
}

/// A push, ref by ref, with its commits.
fn push_text(call: &ConnectorCall) -> Option<String> {
    let view = views::git_push_view(call)?;
    let mut out = format!("git push {}", view.repo);
    for r in &view.refs {
        let force = if r.force {
            ", force"
        } else {
            ""
        };
        let commits = if r.change == "delete" {
            String::new()
        } else {
            format!(": {}", plural(r.commit_count as usize, "commit", "commits"))
        };
        let _ = write!(out, "\n{} {} {}{commits}{force}", r.change, r.kind, r.short_name);
        for c in &r.commits {
            let _ = write!(out, "\n  {} {}", c.short_sha, c.subject);
        }
    }
    Some(out)
}

/// What a call asked, in full.
#[must_use]
pub fn request(call: &ToolCall, asked_at: i64) -> AuditRequest {
    let mut r = AuditRequest {
        asked_at,
        ..AuditRequest::default()
    };
    match call {
        ToolCall::GmailSearch {
            query,
            ..
        } => r.full = format!("Search Gmail\n{query}"),
        ToolCall::GmailRead {
            message_ids,
        } => r.full = format!("Read {}\n{}", plural(message_ids.len(), "email", "emails"), message_ids.join("\n")),
        // The email has its own section.
        ToolCall::GmailSend {
            ..
        }
        | ToolCall::ListAccounts {
            ..
        } => {}
        ToolCall::RequestGrant {
            grant,
        } => {
            r.full = views::grant_request_summary(grant);
            r.purpose.clone_from(&grant.reason);
        }
        ToolCall::RequestUpload {
            name,
            size,
            reason,
            ..
        } => {
            r.full = format!("Share a file\nname: {name}\nsize: {size} bytes");
            r.purpose.clone_from(reason);
        }
        ToolCall::Mcp(call) => r.full = call_text(&call.tool, "mcp", &call.arguments),
        ToolCall::Connector(call) if call.service == DESKTOP && call.op == "ask" => {
            if let Some(h) = hook(call) {
                r.full = h.command.clone().unwrap_or_else(|| h.files.join("\n"));
                r.why = why_asked(call, &h);
                r.agent.clone_from(&h.agent);
                r.cwd.clone_from(&h.cwd);
            } else {
                call.str_arg("question").unwrap_or_default().clone_into(&mut r.purpose);
                call.str_arg("detail").unwrap_or_default().clone_into(&mut r.full);
                r.why = call.str_arg("topic").map(|t| format!("Asked about {t}")).unwrap_or_default();
            }
        }
        ToolCall::Connector(call) if call.service == VAULT && call.op == "secret_release" => {
            let refs: Vec<&str> = call
                .args
                .get("secrets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            r.full = format!("{}\n\nSecrets:\n{}", call.str_arg("command").unwrap_or_default(), refs.join("\n"));
            call.str_arg("purpose").unwrap_or_default().clone_into(&mut r.purpose);
            "Secrets are asked for every time".clone_into(&mut r.why);
        }
        ToolCall::Connector(call) => {
            let spec = call.spec();
            let tool = spec.map_or(call.op.as_str(), |s| s.tool);
            r.full = call_text(tool, &call.service, &call.args);
            if let Some(push) = push_text(call) {
                r.full = format!("{push}\n\n{}", r.full);
            }
            ["purpose", "reason", "description"]
                .iter()
                .find_map(|k| call.str_arg(k))
                .unwrap_or_default()
                .clone_into(&mut r.purpose);
        }
    }
    clean(&r)
}

fn clean(r: &AuditRequest) -> AuditRequest {
    AuditRequest {
        agent: line(&r.agent),
        cwd: line(&r.cwd),
        why: line(&r.why),
        full: field(&r.full),
        purpose: field(&r.purpose),
        asked_at: r.asked_at,
    }
}

/// What an entry asked, as the app shows it.
#[must_use]
pub fn activity_request(r: &AuditRequest) -> ActivityRequest {
    ActivityRequest {
        agent: r.agent.clone(),
        cwd: r.cwd.clone(),
        why: r.why.clone(),
        full: r.full.clone(),
        purpose: r.purpose.clone(),
        asked_at: r.asked_at,
        git: None,
    }
}

/// The label of an entry: the one kept with it, else (an entry logged before labels were kept) what its service,
/// action and detail still say.
#[must_use]
pub fn of_entry(r: &AuditRecord) -> RequestLabel {
    if let Some(l) = &r.info.label {
        return (**l).clone();
    }
    let service = if r.service.is_empty() {
        views::SERVICE_GMAIL
    } else {
        r.service.as_str()
    };
    let detail = r.detail.strip_prefix("approved: ").unwrap_or(&r.detail);
    if service == DESKTOP && r.op == "ask" {
        if let Some((_, command)) = detail.split_once(" wants to run: ") {
            return label(
                "terminal",
                if deletes(command) {
                    "delete"
                } else {
                    "run"
                },
                "Run",
                line(command),
                true,
            );
        }
        for (verb, action, lead) in [("read", "read", "Read"), ("change", "write", "Change")] {
            if let Some((_, path)) = detail.split_once(&format!(" wants to {verb} ")) {
                return label("file", action, lead, line(path), true);
            }
        }
        return label(DESKTOP, "ask", "A question from your computer", "", false);
    }
    let action = match r.action.as_str() {
        "search" | "list" => "search",
        "send" => "send",
        "write" => "write",
        "grant" => "grant",
        _ => "read",
    };
    if service == views::SERVICE_GMAIL && r.op.is_empty() {
        let fake = match r.action.as_str() {
            "search" => r.info.query.as_ref().map(|q| ToolCall::GmailSearch {
                query: q.clone(),
                max_results: 0,
            }),
            _ => None,
        };
        if let Some(call) = fake {
            return of_call(&call, &Context::default());
        }
        if let Some(email) = &r.info.email {
            let mut l =
                label("gmail", "send", "Send an email to", line(email.to.first().map_or("", String::as_str)), false);
            let more = (email.to.len() + email.cc.len()).saturating_sub(1);
            if more > 0 {
                l.tail = format!(" and {more} more");
            }
            return l;
        }
        if r.action == "read" {
            return label("gmail", "read", format!("Read {}", plural(r.count as usize, "email", "emails")), "", false);
        }
    }
    let title = views::op_title(service, &r.op);
    let lead = if !title.is_empty() {
        title
    } else if r.action == "grant" {
        "Access".to_owned()
    } else {
        let name = views::service_name(service.strip_prefix("mcp:").map_or(service, |_| "MCP server")).to_owned();
        match action {
            "search" => format!("Search {name}"),
            "send" => format!("Send with {name}"),
            "write" => format!("Change {name}"),
            "grant" => "Access".to_owned(),
            _ => format!("Read {name}"),
        }
    };
    label(service, action, lead, "", false)
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::OutgoingEmail;
    use reins_proto::remote_mcp::McpCall;
    use serde_json::json;

    use super::*;

    fn desktop(op: &str, service: &str, args: &Value) -> ToolCall {
        ToolCall::Connector(ConnectorCall {
            service: service.to_owned(),
            op: op.to_owned(),
            args: args.as_object().unwrap().clone(),
        })
    }

    fn hook_ask(command: &str) -> ToolCall {
        desktop(
            "ask",
            DESKTOP,
            &json!({
                "question": format!("Claude Code wants to run: {command}"),
                "detail": format!("Command:\n{command}\n\nIn: /home/me/dev/acme-web\nRule: git push --force*"),
                "topic": "command:git push --force*",
                "client_key": "k", "nonce": "n",
            }),
        )
    }

    fn of(call: &ToolCall) -> RequestLabel {
        of_call(call, &Context::default())
    }

    #[test]
    fn a_hooks_command_is_run_in_a_terminal() {
        let l = of(&hook_ask("git push --force origin main \\\n  --no-verify"));
        assert_eq!((l.provider.as_str(), l.action.as_str(), l.lead.as_str(), l.code), ("terminal", "run", "Run", true));
        assert_eq!(l.subject, "git push --force origin main \\ --no-verify");
        assert_eq!(l.title(), "Run git push --force origin main \\ --no-verify");
        let rm = of(&hook_ask("rm -rf build/"));
        assert_eq!((rm.action.as_str(), rm.subject.as_str()), ("delete", "rm -rf build/"));
        // Never the tool's generic title.
        assert!(!l.title().contains("Ask you on your phone"));
    }

    #[test]
    fn a_hooks_command_is_kept_in_full_with_where_and_why() {
        let r = request(&hook_ask("git push --force origin main \\\n  --no-verify"), 1_700_000_000);
        assert_eq!(r.full, "git push --force origin main \\\n  --no-verify");
        assert_eq!(r.agent, "Claude Code");
        assert_eq!(r.cwd, "/home/me/dev/acme-web");
        assert_eq!(r.why, "Rewriting git history");
        assert_eq!(r.asked_at, 1_700_000_000);
        let tame = desktop(
            "ask",
            DESKTOP,
            &json!({"question": "Codex wants to run: make deploy", "detail": "Command:\nmake deploy\nRule: make deploy",
                "topic": "command:make deploy"}),
        );
        assert_eq!(request(&tame, 0).why, "Your rule: make deploy");
    }

    #[test]
    fn a_hooks_file_read_is_a_file() {
        let call = desktop(
            "ask",
            DESKTOP,
            &json!({"question": "Claude Code wants to read ~/dev/api/.env", "detail": "Files:\n~/dev/api/.env\n\nIn: ~/dev/api\nRule: .env",
                "topic": "file:.env"}),
        );
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.title().as_str(), l.code),
            ("file", "read", "Read ~/dev/api/.env", true)
        );
        let r = request(&call, 0);
        assert_eq!(
            (r.full.as_str(), r.why.as_str(), r.cwd.as_str()),
            ("~/dev/api/.env", "Reading a secret file", "~/dev/api")
        );
        let two = desktop(
            "ask",
            DESKTOP,
            &json!({"question": "Cursor wants to change a.pem and 1 more file(s)", "detail": "Files:\na.pem\nb.pem", "topic": "file:*.pem"}),
        );
        let l = of(&two);
        assert_eq!((l.action.as_str(), l.title().as_str()), ("write", "Change a.pem and 1 more"));
    }

    #[test]
    fn a_question_in_its_own_words_is_its_own_title() {
        let call = desktop(
            "ask",
            DESKTOP,
            &json!({"question": "Deploy the site now?", "detail": "make deploy\n(staging first)"}),
        );
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.title().as_str()),
            ("desktop", "ask", "Deploy the site now?")
        );
        let r = request(&call, 0);
        assert_eq!((r.purpose.as_str(), r.full.as_str()), ("Deploy the site now?", "make deploy\n(staging first)"));
    }

    #[test]
    fn released_secrets_name_the_command_never_a_value() {
        let call = desktop(
            "secret_release",
            VAULT,
            &json!({"secrets": ["vault:Stripe/api key", "vault:DB/password", "vault:Sentry/dsn"], "command": "npm run dev",
                "purpose": "Start the dev server", "lease_secs": 3600, "client_key": "k", "nonce": "n"}),
        );
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.title().as_str(), l.code),
            ("secrets", "secrets", "Use 3 secrets for npm run dev", true)
        );
        let r = request(&call, 0);
        assert_eq!(r.full, "npm run dev\n\nSecrets:\nvault:Stripe/api key\nvault:DB/password\nvault:Sentry/dsn");
        assert_eq!(r.purpose, "Start the dev server");
    }

    #[test]
    fn a_vault_call_keeps_names_only() {
        let call = desktop(
            "item_create",
            VAULT,
            &json!({"name": "Bank", "password": "hunter2", "notes": "pin 1234", "folder": "Money"}),
        );
        let r = request(&call, 0);
        assert!(r.full.contains("name: Bank") && r.full.contains("folder: Money"), "{}", r.full);
        assert!(!r.full.contains("hunter2") && !r.full.contains("1234"), "{}", r.full);
        let other = desktop(
            "send",
            "telegram",
            &json!({"chat": "Family", "text": "hi", "api_token": "t0k", "client_key": "k"}),
        );
        let r = request(&other, 0);
        assert!(r.full.contains("text: hi") && !r.full.contains("t0k") && !r.full.contains("client_key"), "{}", r.full);
    }

    #[test]
    fn an_ssh_sign_in_names_the_host() {
        let call = desktop(
            "ssh_sign",
            VAULT,
            &json!({"key": "SHA256:abc", "data_base64": "AAAA", "flags": 0, "host": "prod.acme.com"}),
        );
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.title().as_str()),
            ("ssh", "secrets", "Sign in to prod.acme.com")
        );
        assert!(!request(&call, 0).full.contains("AAAA"), "the data signed is not kept");
    }

    #[test]
    fn a_git_push_counts_its_commits() {
        let commit = |sha: &str| json!({"sha": sha.repeat(40), "subject": "Fix", "author": "Ada", "date": 1, "files_changed": 1});
        let update = json!({"name": "refs/heads/main", "change": "update", "old": "a".repeat(40), "new": "b".repeat(40),
            "fast_forward": true, "commit_count": 3, "commits": [commit("1"), commit("2"), commit("3")], "files_changed": 2});
        let call = desktop(
            "git_push",
            "github",
            &json!({"repo": "acme/web", "client_key": "k", "nonce": "n", "digest": "d",
                "summary": {"updates": [update], "pack_bytes": 10}}),
        );
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.title().as_str()),
            ("github", "push", "Push 3 commits to acme/web")
        );
        let r = request(&call, 0);
        assert!(r.full.starts_with("git push acme/web\nupdate branch main: 3 commits\n  1111111 Fix"), "{}", r.full);
        assert!(!r.full.contains("digest") && !r.full.contains("\"updates\""), "{}", r.full);
    }

    #[test]
    fn a_gmail_send_names_the_recipient() {
        let call = ToolCall::GmailSend {
            email: Box::new(OutgoingEmail {
                to: vec!["ada@acme.com".into(), "bob@acme.com".into()],
                cc: vec![],
                subject: "Hi".into(),
                body: "Hello".into(),
                reply_to_message_id: None,
            }),
        };
        let l = of(&call);
        assert_eq!(
            (l.provider.as_str(), l.action.as_str(), l.lead.as_str(), l.subject.as_str()),
            ("gmail", "send", "Send an email to", "ada@acme.com")
        );
        assert_eq!(l.title(), "Send an email to ada@acme.com and 1 more");
    }

    #[test]
    fn an_mcp_tool_is_used_on_its_server() {
        let call = ToolCall::Mcp(McpCall {
            server: "s1".into(),
            tool: "create_issue".into(),
            arguments: json!({"title": "Bug", "api_key": "x"}).as_object().unwrap().clone(),
        });
        let bare = of(&call);
        assert_eq!(
            (bare.provider.as_str(), bare.action.as_str(), bare.title().as_str(), bare.code),
            ("mcp:s1", "write", "Use create_issue", true)
        );
        let cx = Context {
            mcp: Some(McpContext {
                server: "Linear".into(),
                title: "Create issue".into(),
                read_only: false,
                destructive: false,
            }),
            ..Context::default()
        };
        assert_eq!(of_call(&call, &cx).title(), "Use Create issue on Linear");
        let r = request(&call, 0);
        assert_eq!(r.full, "create_issue\ntitle: Bug");
    }

    #[test]
    fn long_and_hostile_text_is_bounded_and_cleaned() {
        let long = format!("echo {}\u{202e}", "x".repeat(20_000));
        let r = request(&hook_ask(&long), 0);
        assert!(r.full.len() <= MAX_FIELD && r.full.ends_with('…'), "{}", r.full.len());
        assert!(!r.full.contains('\u{202e}'));
        let l = of(&hook_ask(&long));
        assert_eq!(l.subject.chars().count(), MAX_SUBJECT);
    }

    #[test]
    fn older_entries_still_read_as_what_they_were() {
        let record = |service: &str, op: &str, action: &str, detail: &str| AuditRecord {
            seq: 1,
            at: 1,
            connection_id: "c".into(),
            connection_label: "laptop".into(),
            action: action.into(),
            outcome: "sent".into(),
            detail: detail.into(),
            grant_id: None,
            service: service.into(),
            account: None,
            count: 1,
            info: crate::store::AuditInfo::default(),
            op: op.into(),
        };
        let l = of_entry(&record(DESKTOP, "ask", "write", "approved: Claude Code wants to run: git push -f"));
        assert_eq!((l.provider.as_str(), l.title().as_str()), ("terminal", "Run git push -f"));
        let l = of_entry(&record(DESKTOP, "ask", "write", "denied by you"));
        assert_eq!(l.title(), "A question from your computer");
        let l = of_entry(&record("telegram", "read", "read", "approved"));
        assert_eq!((l.provider.as_str(), l.action.as_str()), ("telegram", "read"));
    }
}
