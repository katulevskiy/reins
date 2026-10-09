//! Pure builders for the records Kotlin displays (contracts §D), plus the payload
//! that is parked for a request awaiting the user. Every string that came from
//! outside (client names, subjects, addresses) is neutralized here.

use std::collections::BTreeMap;

use reins_policy::{AddrRule, ReadScope, Scope, SendScope, ServiceScope};
use reins_proto::connector::{ConnectorCall, Effect};
use reins_proto::desktop::{self, FileStatus, RefChange};
use reins_proto::gmail::{GrantAction, GrantRequest, MessageSummary, ToolCall};
use reins_proto::pairing::PairingRequest;
use reins_proto::relay::RelayRequest;
use serde::{Deserialize, Serialize};

use crate::connector::{Item, Preview};
use crate::store::{AuditEmail, AuditInfo, AuditMessage, AuditRecord, StoredGrant, unix_now};
use crate::text;
use crate::types::{
    ActivityEntry, ActivityInfo, ActivityMessage, ApprovalKind, ApprovalView, ClassOption, EmailView, GitCommitView,
    GitFileView, GitPushView, GitRefView, GrantRequestView, GrantScopeChoice, GrantView, MessageView, PairingView,
    PendingItem, PendingKind, ResourceView,
};

/// The only connector so far.
pub const SERVICE_GMAIL: &str = "gmail";

const MAX_LABEL_CHARS: usize = 64;
const MAX_QUERY_CHARS: usize = 200;

/// A request parked for the user. Holds message *summaries* only, never bodies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkedRequest {
    #[serde(with = "crate::mcp::wire::parked_request")]
    pub request: RelayRequest,
    /// What a search/read found, in order.
    #[serde(default)]
    pub messages: Vec<MessageSummary>,
    /// Message id → id of the grant that already covers it.
    #[serde(default)]
    pub covered: BTreeMap<String, String>,
    /// The connected account this concerns (e.g. the Gmail address), when known.
    #[serde(default)]
    pub account: Option<String>,
    /// Accounts request: every account of the integration.
    #[serde(default)]
    pub accounts: Vec<String>,
    /// Accounts request: those the AI may already see.
    #[serde(default)]
    pub shared_accounts: Vec<String>,
    /// A call to another integration: what it found or would do, and what already covers it.
    #[serde(default)]
    pub connector: Option<ParkedConnector>,
}

/// What a parked call to another integration holds. The text of the items stays here, sealed with the rest of the
/// parked request, so that what the user saw is exactly what is released.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkedConnector {
    pub items: Vec<Item>,
    /// Item id → id of the grant that already covers it.
    #[serde(default)]
    pub covered: BTreeMap<String, String>,
    /// A write: what it would do.
    #[serde(default)]
    pub preview: Option<Preview>,
    /// A call to an MCP server the user added: the server and tool as shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::ParkedMcp>,
}

impl ParkedRequest {
    pub fn kind(&self) -> ApprovalKind {
        match &self.request.call {
            ToolCall::GmailSearch {
                ..
            }
            // Listing the integrations is answered at once and never waits; only their accounts are asked about.
            | ToolCall::ListAccounts {
                service: None,
                ..
            } => ApprovalKind::Search,
            ToolCall::ListAccounts {
                service: Some(_),
                ..
            } => ApprovalKind::Accounts,
            ToolCall::Connector(call) => {
                if call.effect() == Some(Effect::Write) {
                    ApprovalKind::Write
                } else {
                    ApprovalKind::Fetch
                }
            }
            ToolCall::GmailRead {
                ..
            } => ApprovalKind::Read,
            ToolCall::GmailSend {
                ..
            } => ApprovalKind::Send,
            ToolCall::RequestGrant {
                ..
            } => ApprovalKind::Grant,
            ToolCall::Mcp(_)
            | ToolCall::RequestUpload {
                ..
            } => ApprovalKind::Write,
        }
    }

    /// "search" | "read" | "send" | "grant"
    pub fn action(&self) -> &'static str {
        match self.kind() {
            ApprovalKind::Search => "search",
            ApprovalKind::Read => "read",
            ApprovalKind::Send => "send",
            ApprovalKind::Grant => "grant",
            ApprovalKind::Accounts => "accounts",
            ApprovalKind::Fetch | ApprovalKind::Write => match &self.request.call {
                ToolCall::Connector(call) => match call.effect() {
                    Some(Effect::List) => "list",
                    Some(Effect::Search) => "search",
                    Some(Effect::Write) if call.op == "send" => "send",
                    Some(Effect::Write) => "write",
                    _ => "read",
                },
                ToolCall::Mcp(_)
                    if !self.connector.as_ref().and_then(|c| c.mcp.as_ref()).is_some_and(|m| m.read_only) =>
                {
                    "write"
                }
                _ => "read",
            },
        }
    }

    /// The operation on another integration ("read", "send", ...), else empty.
    pub fn op(&self) -> String {
        match &self.request.call {
            ToolCall::Connector(call) => call.op.clone(),
            ToolCall::Mcp(call) => call.tool.clone(),
            _ => String::new(),
        }
    }

    /// The integration an accounts request is about; Gmail for everything else (the only integration so far).
    pub fn service(&self) -> String {
        match &self.request.call {
            ToolCall::ListAccounts {
                service: Some(service),
                ..
            } => service.clone(),
            ToolCall::Connector(call) => call.service.clone(),
            ToolCall::Mcp(call) => format!("mcp:{}", call.server),
            _ => SERVICE_GMAIL.to_owned(),
        }
    }

    /// How many messages or recipients the operation covers.
    pub fn count(&self) -> u32 {
        match &self.request.call {
            ToolCall::GmailSearch {
                ..
            } => u32::try_from(self.messages.len()).unwrap_or(u32::MAX),
            ToolCall::ListAccounts {
                ..
            } => u32::try_from(self.accounts.len().saturating_sub(self.shared_accounts.len())).unwrap_or(u32::MAX),
            ToolCall::Connector(_) => {
                let n = self.connector.as_ref().map_or(1, |c| {
                    if c.preview.is_some() {
                        1
                    } else {
                        c.items.len()
                    }
                });
                u32::try_from(n).unwrap_or(u32::MAX)
            }
            call => u32::try_from(requested_count(call)).unwrap_or(u32::MAX),
        }
    }

    /// The label shown for the AI, safe to display.
    pub fn label(&self) -> String {
        text::truncate_chars(&text::one_line(&self.request.connection_label), MAX_LABEL_CHARS)
    }
}

/// How many messages or recipients a call names; a search names none (its count is what it finds).
pub fn requested_count(call: &ToolCall) -> usize {
    match call {
        ToolCall::GmailSearch {
            ..
        }
        | ToolCall::ListAccounts {
            ..
        } => 0,
        ToolCall::GmailRead {
            message_ids,
        } => message_ids.len(),
        ToolCall::GmailSend {
            email,
        } => email.recipients().count(),
        ToolCall::RequestGrant {
            ..
        }
        | ToolCall::Connector(_)
        | ToolCall::Mcp(_)
        | ToolCall::RequestUpload {
            ..
        } => 1,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

pub fn request_title(parked: &ParkedRequest) -> String {
    let label = parked.label();
    match &parked.request.call {
        ToolCall::GmailSearch {
            ..
        } => format!("{label} wants to search your Gmail"),
        ToolCall::GmailRead {
            message_ids,
        } => format!("{label} wants to read {}", plural(message_ids.len(), "email", "emails")),
        ToolCall::GmailSend {
            email,
        } => format!("{label} wants to send an email to {}", email.to.first().map_or("", String::as_str)),
        ToolCall::RequestGrant {
            grant,
        } => format!("{label} asks for permission: {}", grant_request_summary(grant)),
        ToolCall::ListAccounts {
            service,
            ..
        } => format!("{label} wants to see your {} accounts", service.as_deref().map_or("connected", service_name)),
        ToolCall::Connector(call) => {
            format!("{label} wants to {}", call.spec().map_or_else(|| call.op.clone(), |s| s.title.to_lowercase()))
        }
        ToolCall::Mcp(call) => format!("{label} wants to use {} ({})", call.tool, call.server),
        ToolCall::RequestUpload {
            name,
            ..
        } => format!("{label} wants to share a file: {name}"),
    }
}

pub fn request_subtitle(parked: &ParkedRequest) -> String {
    match &parked.request.call {
        ToolCall::GmailSearch {
            query,
            ..
        } => text::truncate_chars(&text::one_line(query), MAX_QUERY_CHARS),
        ToolCall::GmailRead {
            ..
        } => plural(parked.messages.len(), "message found", "messages found"),
        ToolCall::GmailSend {
            email,
        } => text::truncate_chars(&text::one_line(&email.subject), MAX_QUERY_CHARS),
        ToolCall::RequestGrant {
            grant,
        } => text::truncate_chars(&text::one_line(&grant.reason), MAX_QUERY_CHARS),
        ToolCall::ListAccounts {
            ..
        } => plural(parked.accounts.len().saturating_sub(parked.shared_accounts.len()), "account", "accounts"),
        ToolCall::Connector(call) => connector_subject(call, parked.connector.as_ref()),
        ToolCall::Mcp(call) => call.server.clone(),
        ToolCall::RequestUpload {
            reason,
            ..
        } => text::truncate_chars(&text::one_line(reason), MAX_QUERY_CHARS),
    }
}

pub fn request_item(parked: &ParkedRequest) -> PendingItem {
    let view = approval_view(parked);
    PendingItem {
        kind: PendingKind::Request,
        id: parked.request.id.0.clone(),
        title: request_title(parked),
        subtitle: request_subtitle(parked),
        created_at: parked.request.created_at,
        connection_id: parked.request.connection_id.0.clone(),
        connection_label: parked.label(),
        action: parked.action().to_owned(),
        count: parked.count(),
        service: parked.service(),
        account: parked.account.clone(),
        wait_until: parked.request.wait_until,
        op: parked.op(),
        op_title: op_title(&parked.service(), &parked.op()),
        suggestion: None,
        headline: view.headline,
        quick: view.quick.is_some_and(|q| q.from_notification),
    }
}

/// The name of an operation of another integration, else empty.
pub fn op_title(service: &str, op: &str) -> String {
    reins_proto::connector::specs()
        .iter()
        .find(|s| s.service == service && s.op == op)
        .map(|s| s.title.to_owned())
        .unwrap_or_default()
}

/// The one line under a request to another integration: what it is about.
fn connector_subject(call: &ConnectorCall, held: Option<&ParkedConnector>) -> String {
    let named = call.str_arg("query").or_else(|| call.resource().as_deref().map(|_| "")).unwrap_or_default();
    let resource = held
        .and_then(|h| {
            h.preview
                .as_ref()
                .map(|p| p.resource_label.clone())
                .or_else(|| h.items.first().map(|i| i.resource_label.clone()))
        })
        .filter(|l| !l.is_empty());
    text::truncate_chars(
        &text::one_line(&match (resource, named) {
            (Some(r), q) if !q.is_empty() => format!("{r} · {q}"),
            (Some(r), _) => r,
            (None, q) if !q.is_empty() => q.to_owned(),
            _ => call.resource().unwrap_or_default(),
        }),
        MAX_QUERY_CHARS,
    )
}

/// "gmail" → "Gmail".
pub fn service_name(service: &str) -> &str {
    match service {
        "gmail" => "Gmail",
        "telegram" => "Telegram",
        "gcalendar" => "Google Calendar",
        "gcontacts" => "Google Contacts",
        "device_calendar" => "Phone calendar",
        "device_contacts" => "Phone contacts",
        "sms" => "Text messages",
        "github" => "GitHub",
        "gitlab" => "GitLab",
        "codeberg" => "Codeberg",
        "bitbucket" => "Bitbucket",
        "desktop" => "Desktop app",
        "vault" => "Password vault",
        crate::blob::SERVICE_FILES => "Files",
        other => other,
    }
}

/// Sanitizes a pairing request received from the server before it is stored or shown.
pub fn clean_pairing(mut p: PairingRequest) -> PairingRequest {
    p.client_name = text::truncate_chars(&text::one_line(&p.client_name), MAX_LABEL_CHARS);
    p.client_host = text::truncate_chars(&text::one_line(&p.client_host), 253);
    // A key that cannot be used is no key: no fingerprint is shown and nothing is pinned.
    p.client_key = p.client_key.as_deref().and_then(desktop::decode_key).map(|k| desktop::encode_key(&k));
    p
}

pub fn pairing_item(p: &PairingRequest) -> PendingItem {
    PendingItem {
        kind: PendingKind::Pairing,
        id: p.id.0.clone(),
        title: format!("Connect {} to Reins?", p.client_name),
        subtitle: p.client_host.clone(),
        created_at: p.created_at,
        connection_id: String::new(),
        connection_label: p.client_name.clone(),
        action: "pair".to_owned(),
        count: 1,
        service: String::new(),
        account: None,
        wait_until: None,
        op: String::new(),
        op_title: String::new(),
        suggestion: None,
        headline: String::new(),
        quick: false,
    }
}

pub fn pairing_view(p: &PairingRequest) -> PairingView {
    PairingView {
        id: p.id.0.clone(),
        client_name: p.client_name.clone(),
        client_host: p.client_host.clone(),
        choices: p.choices.to_vec(),
        created_at: p.created_at,
        key_fingerprint: p.client_key.as_deref().and_then(desktop::key_fingerprint),
    }
}

/// A push from the desktop app, ref by ref, from the summary the app sent (`None` for anything else).
pub fn git_push_view(call: &ConnectorCall) -> Option<GitPushView> {
    if !matches!(call.op.as_str(), desktop::GIT_PUSH_OP | desktop::GIT_TAG_PUSH_OP) {
        return None;
    }
    let summary = crate::connector::github::git::summary_arg(call).ok()?;
    let line = |s: &str| text::one_line(s);
    let refs = summary
        .updates
        .iter()
        .map(|u| {
            let (kind, short_name) = match (u.branch(), u.tag()) {
                (Some(b), _) => ("branch", b),
                (_, Some(t)) => ("tag", t),
                _ => ("other", u.name.as_str()),
            };
            let update = u.change == RefChange::Update;
            GitRefView {
                name: line(&u.name),
                kind: kind.to_owned(),
                short_name: line(short_name),
                change: match u.change {
                    RefChange::Create => "create",
                    RefChange::Update => "update",
                    RefChange::Delete => "delete",
                }
                .to_owned(),
                force: update && u.fast_forward != Some(true),
                force_unknown: update && u.fast_forward.is_none(),
                commit_count: u.commit_count,
                commits: u
                    .commits
                    .iter()
                    .map(|c| GitCommitView {
                        short_sha: c.sha.chars().take(7).collect(),
                        subject: line(&c.subject),
                        author: line(&c.author),
                    })
                    .collect(),
                files_changed: u.files_changed,
                files: u
                    .files
                    .iter()
                    .map(|f| GitFileView {
                        path: line(&f.path),
                        status: match f.status {
                            FileStatus::Added => "added",
                            FileStatus::Modified => "modified",
                            FileStatus::Deleted => "deleted",
                            FileStatus::TypeChanged => "type_changed",
                        }
                        .to_owned(),
                        additions: f.additions,
                        deletions: f.deletions,
                        binary: f.binary,
                    })
                    .collect(),
                additions: u.additions,
                deletions: u.deletions,
            }
        })
        .collect();
    Some(GitPushView {
        repo: line(call.str_arg("repo").unwrap_or_default()),
        pack_bytes: summary.pack_bytes,
        notes: summary.notes.iter().map(|n| line(n)).collect(),
        refs,
    })
}

fn sender_line(m: &MessageSummary) -> String {
    match &m.from_name {
        Some(name) if !m.from.is_empty() => format!("{name} <{}>", m.from),
        _ if m.from.is_empty() => "(unknown sender)".to_owned(),
        _ => m.from.clone(),
    }
}

pub fn message_view(m: &MessageSummary, covered: bool) -> MessageView {
    MessageView {
        id: m.id.clone(),
        from: sender_line(m),
        subject: m.subject.clone(),
        date: m.date,
        snippet: m.snippet.clone(),
        covered_by_grant: covered,
        sensitive: false,
    }
}

fn item_view(item: &Item, covered: bool) -> MessageView {
    MessageView {
        id: item.id.clone(),
        from: text::one_line(&item.from),
        subject: text::one_line(&item.title),
        date: item.date,
        snippet: text::truncate_chars(&item.text(), 400),
        covered_by_grant: covered,
        sensitive: item.sensitive,
    }
}

pub fn approval_view(parked: &ParkedRequest) -> ApprovalView {
    let (query, email, grant) = match &parked.request.call {
        ToolCall::GmailSearch {
            query,
            ..
        } => (Some(text::one_line(query)), None, None),
        ToolCall::GmailRead {
            ..
        }
        | ToolCall::ListAccounts {
            ..
        }
        | ToolCall::Mcp(_)
        | ToolCall::RequestUpload {
            ..
        } => (None, None, None),
        ToolCall::GmailSend {
            email,
        } => (
            None,
            Some(EmailView {
                to: email.to.clone(),
                cc: email.cc.clone(),
                subject: text::one_line(&email.subject),
                body: text::neutralize(&email.body),
            }),
            None,
        ),
        ToolCall::RequestGrant {
            grant,
        } => (None, None, Some(grant_request_view(grant))),
        ToolCall::Connector(call) => (call.str_arg("query").map(text::one_line), None, None),
    };
    let held = parked.connector.as_ref();
    let messages: Vec<MessageView> = match held {
        Some(h) => h.items.iter().map(|i| item_view(i, h.covered.contains_key(&i.id))).collect(),
        None => parked.messages.iter().map(|m| message_view(m, parked.covered.contains_key(&m.id))).collect(),
    };
    let mut resources: Vec<ResourceView> = Vec::new();
    if let Some(h) = held {
        let own = h.preview.iter().map(|p| (&p.resource, &p.resource_label, &p.parents));
        let found = h.items.iter().map(|i| (&i.resource, &i.resource_label, &i.parents));
        let mut push = |id: &String, label: &String, wider: bool| {
            if !id.is_empty() && !resources.iter().any(|r| &r.id == id) {
                resources.push(ResourceView {
                    id: id.clone(),
                    label: text::one_line(label),
                    wider,
                });
            }
        };
        // What was found first, then the wider things it is part of (a branch, then its repository, then its owner).
        for (id, label, _) in own.clone().chain(found.clone()) {
            push(id, label, false);
        }
        for (_, _, parents) in own.chain(found) {
            for (id, label) in parents {
                push(id, label, true);
            }
        }
    }
    let spec = match &parked.request.call {
        ToolCall::Connector(call) => call.spec(),
        _ => None,
    };
    let write = spec.is_some_and(|s| s.effect == Effect::Write) || matches!(parked.request.call, ToolCall::Mcp(_));
    let view = ApprovalView {
        request_id: parked.request.id.0.clone(),
        connection_id: parked.request.connection_id.0.clone(),
        connection_label: parked.label(),
        kind: parked.kind(),
        query,
        messages,
        email,
        created_at: parked.request.created_at,
        service: parked.service(),
        account: parked.account.clone(),
        wait_until: parked.request.wait_until,
        grant,
        count: parked.count(),
        accounts: parked.accounts.clone(),
        shared_accounts: parked.shared_accounts.clone(),
        op: parked.op(),
        resources,
        preview: held
            .and_then(|h| h.preview.as_ref())
            .map(|p| p.lines.iter().map(|l| text::neutralize(l)).collect())
            .unwrap_or_default(),
        no_standing: held.is_some_and(|h| {
            (!h.items.is_empty() && h.items.iter().all(|i| i.sensitive))
                || (write && (spec.is_some_and(|s| s.once_only) || h.preview.as_ref().is_some_and(|p| p.once_only)))
        }),
        op_title: op_title(&parked.service(), &parked.op()),
        action: parked.action().to_owned(),
        class: held
            .and_then(|h| h.preview.as_ref())
            .and_then(|p| p.class.clone())
            .or_else(|| spec.map(|s| s.class.to_owned()))
            .unwrap_or_default(),
        classes: if write {
            reins_proto::connector::classes(&parked.service())
                .iter()
                .map(|c| ClassOption {
                    id: c.id.to_owned(),
                    label: c.label.to_owned(),
                })
                .collect()
        } else {
            Vec::new()
        },
        git: match &parked.request.call {
            ToolCall::Connector(call) => git_push_view(call),
            _ => None,
        },
        ask: match &parked.request.call {
            ToolCall::Connector(call) => crate::connector::desktop::ask_view(call),
            _ => None,
        },
        secrets: match (&parked.request.call, held.and_then(|h| h.preview.as_ref())) {
            (ToolCall::Connector(call), Some(p)) => crate::connector::vault::secret_release_view(call, p),
            _ => None,
        },
        ssh: match (&parked.request.call, held.and_then(|h| h.preview.as_ref())) {
            (ToolCall::Connector(call), Some(p)) => crate::connector::vault::ssh_sign_view(call, p),
            _ => None,
        },
        mcp: crate::mcp::flow::call_view(parked),
        blob: held.and_then(|h| h.preview.as_ref()).and_then(|p| p.blob.as_ref()).map(crate::blob::blob_view),
        headline: String::new(),
        quick: None,
    };
    crate::quick::decorate(parked, view)
}

/// "Read emails from alerts@bank.com for 1 hour" — one line describing what an AI asked for.
pub fn grant_request_summary(g: &GrantRequest) -> String {
    let what = match g.action {
        GrantAction::Read if g.any => "read any email".to_owned(),
        GrantAction::Read => format!("read emails {}", request_limits(g).join(", ")),
        GrantAction::Send => format!("send emails to {}", g.recipients.join(", ")),
    };
    text::one_line(&format!("{what} for {}", duration_text(g.duration_secs)))
}

fn request_limits(g: &GrantRequest) -> Vec<String> {
    let mut parts = Vec::new();
    if !g.from.is_empty() {
        parts.push(format!("from {}", g.from.join(", ")));
    }
    if let Some(s) = &g.subject_contains {
        parts.push(format!("with \"{s}\" in the subject"));
    }
    parts
}

pub fn duration_text(secs: u64) -> String {
    match secs {
        s if s < 3_600 => plural((s / 60) as usize, "minute", "minutes"),
        s if s < 86_400 => plural((s / 3_600) as usize, "hour", "hours"),
        s => plural((s / 86_400) as usize, "day", "days"),
    }
}

pub fn grant_request_view(g: &GrantRequest) -> GrantRequestView {
    let mut lines = Vec::new();
    match g.action {
        GrantAction::Read if g.any => lines.push("Every email, including anything that arrives meanwhile".to_owned()),
        GrantAction::Read => {
            if !g.from.is_empty() {
                lines.push(format!("From {}", g.from.join(", ")));
            }
            if let Some(s) = &g.subject_contains {
                lines.push(format!("Subject contains \"{s}\""));
            }
        }
        GrantAction::Send => {
            lines.push(format!("To {}", g.recipients.join(", ")));
            if let Some(s) = &g.subject_contains {
                lines.push(format!("Subject contains \"{s}\""));
            }
        }
    }
    if let Some(n) = g.max_uses {
        lines.push(format!("At most {}", plural(n as usize, "use", "uses")));
    }
    let exact_only = g.from.iter().chain(&g.recipients).all(|r| !r.starts_with('@'));
    let breadth = if g.any {
        "everything"
    } else if exact_only && g.duration_secs <= 86_400 {
        "narrow"
    } else {
        "broad"
    };
    GrantRequestView {
        action: match g.action {
            GrantAction::Read => "read",
            GrantAction::Send => "send",
        }
        .to_owned(),
        summary: grant_request_summary(g),
        reason: text::neutralize(&g.reason),
        duration_secs: g.duration_secs,
        max_uses: g.max_uses,
        breadth: breadth.to_owned(),
        lines: lines.into_iter().map(|l| text::one_line(&l)).collect(),
    }
}

/// The parts of an audit entry that describe the call and what was released.
pub fn audit_info(call: &ToolCall, released: &[&MessageSummary]) -> AuditInfo {
    let mut info = AuditInfo::default();
    match call {
        ToolCall::GmailSearch {
            query,
            ..
        } => info.query = Some(text::one_line(query)),
        ToolCall::GmailRead {
            ..
        }
        | ToolCall::ListAccounts {
            ..
        }
        | ToolCall::Connector(_)
        | ToolCall::Mcp(_)
        | ToolCall::RequestUpload {
            ..
        } => {}
        ToolCall::GmailSend {
            email,
        } => {
            info.email = Some(AuditEmail {
                to: email.to.clone(),
                cc: email.cc.clone(),
                subject: text::one_line(&email.subject),
                body: text::neutralize(&email.body),
            });
        }
        ToolCall::RequestGrant {
            grant,
        } => {
            info.grant_summary = Some(grant_request_summary(grant));
            info.note = Some(text::neutralize(&grant.reason));
        }
    }
    info.messages = released
        .iter()
        .map(|m| AuditMessage {
            id: m.id.clone(),
            text: String::new(),
            from: sender_line(m),
            subject: text::one_line(&m.subject),
            date: m.date,
        })
        .collect();
    info
}

fn rule_text(rule: &AddrRule) -> String {
    match rule {
        AddrRule::Exact(a) => a.clone(),
        AddrRule::Domain(d) => format!("@{d}"),
        AddrRule::Regex(p) => format!("/{}/", p.source()),
    }
}

fn rules_text(rules: &[AddrRule]) -> String {
    rules.iter().map(rule_text).collect::<Vec<_>>().join(", ")
}

fn read_summary(s: &ReadScope) -> String {
    if s.any {
        return "Read any email".to_owned();
    }
    let mut parts = Vec::new();
    if let Some(ids) = &s.message_ids {
        parts.push(plural(ids.len(), "specific email", "specific emails"));
    }
    if !s.from.is_empty() {
        parts.push(format!("from {}", rules_text(&s.from)));
    }
    if !s.to.is_empty() {
        parts.push(format!("to {}", rules_text(&s.to)));
    }
    if let Some(p) = &s.subject {
        parts.push(format!("subject contains \"{}\"", p.source()));
    }
    if let Some(p) = &s.body {
        parts.push(format!("body matches \"{}\"", p.source()));
    }
    if !s.labels.is_empty() {
        parts.push(format!("labels {}", s.labels.join(", ")));
    }
    if s.after.is_some() || s.before.is_some() {
        parts.push("within a date range".to_owned());
    }
    format!("Read emails {}", parts.join(", "))
}

fn send_summary(s: &SendScope) -> String {
    let mut text = format!("Send emails to {}", rules_text(&s.recipients));
    if let Some(p) = &s.subject {
        text = format!("{text}, subject contains \"{}\"", p.source());
    }
    text
}

fn service_resource_names(s: &ServiceScope) -> Vec<String> {
    if s.labels.len() == s.resources.len() {
        s.labels.clone()
    } else {
        s.resources.clone()
    }
}

/// "Read Telegram: Family, Anna" — one line for a grant on another integration.
fn service_summary(s: &ServiceScope) -> String {
    let verb = match s.access.as_str() {
        "list" => "List",
        "write" => "Write to",
        _ => "Read",
    };
    let what = if s.any {
        "everything".to_owned()
    } else {
        service_resource_names(s).join(", ")
    };
    let kinds = if s.classes.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = s
            .classes
            .iter()
            .map(|c| {
                reins_proto::connector::classes(&s.service).iter().find(|k| k.id == c).map_or(c.as_str(), |k| k.label)
            })
            .collect();
        format!(" ({})", names.join(", "))
    };
    format!("{verb} {}{kinds}: {what}", service_name(&s.service))
}

pub fn grant_summary(scope: &Scope) -> String {
    text::one_line(&match scope {
        Scope::Read(s) => read_summary(s),
        Scope::Send(s) => send_summary(s),
        Scope::Service(s) => service_summary(s),
        Scope::Accounts(s) if s.accounts.is_empty() => format!("See your {} accounts", service_name(&s.service)),
        Scope::Accounts(s) => format!("See {} of your {} accounts", s.accounts.len(), service_name(&s.service)),
    })
}

/// One line per limit of a stored scope, for the grant details.
pub fn scope_lines(scope: &Scope) -> Vec<String> {
    let mut lines = Vec::new();
    match scope {
        Scope::Read(s) if s.any => lines.push("Every email".to_owned()),
        Scope::Read(s) => {
            if let Some(ids) = &s.message_ids {
                lines.push(plural(ids.len(), "specific email", "specific emails"));
            }
            if !s.from.is_empty() {
                lines.push(format!("From {}", rules_text(&s.from)));
            }
            if !s.to.is_empty() {
                lines.push(format!("To {}", rules_text(&s.to)));
            }
            if let Some(p) = &s.subject {
                lines.push(format!("Subject contains \"{}\"", p.source()));
            }
            if let Some(p) = &s.body {
                lines.push(format!("Body matches \"{}\"", p.source()));
            }
            if !s.labels.is_empty() {
                lines.push(format!("Labels {}", s.labels.join(", ")));
            }
            if s.after.is_some() || s.before.is_some() {
                lines.push("Within a date range".to_owned());
            }
        }
        Scope::Send(s) => {
            lines.push(format!("To {}", rules_text(&s.recipients)));
            if let Some(p) = &s.subject {
                lines.push(format!("Subject contains \"{}\"", p.source()));
            }
            if let Some(p) = &s.body {
                lines.push(format!("Body matches \"{}\"", p.source()));
            }
        }
        Scope::Service(s) => {
            lines.push(match s.access.as_str() {
                "list" => "Lists names only, not what is inside".to_owned(),
                "write" => "May send or change things there".to_owned(),
                _ => "May read there".to_owned(),
            });
            for c in &s.classes {
                if let Some(k) = reins_proto::connector::classes(&s.service).iter().find(|k| k.id == c) {
                    lines.push(format!("Only: {}", k.label));
                }
            }
            if s.any {
                lines.push(format!("Every part of {}", service_name(&s.service)));
            }
            lines.extend(service_resource_names(s));
        }
        Scope::Accounts(s) => {
            if s.accounts.is_empty() {
                lines.push(format!("The addresses of all your {} accounts", service_name(&s.service)));
            } else {
                lines.extend(s.accounts.iter().cloned());
            }
            lines.push("Not what is in them".to_owned());
        }
    }
    lines.into_iter().map(|l| text::one_line(&l)).collect()
}

pub fn grant_view(g: &StoredGrant) -> GrantView {
    let now = unix_now();
    GrantView {
        id: g.grant.id.0.clone(),
        connection_id: g.grant.connection_id.0.clone(),
        connection_label: text::truncate_chars(&text::one_line(&g.connection_label), MAX_LABEL_CHARS),
        action: match &g.grant.scope {
            Scope::Read(_) => "read".to_owned(),
            Scope::Send(_) => "send".to_owned(),
            Scope::Accounts(_) => "accounts".to_owned(),
            Scope::Service(s) => s.access.clone(),
        },
        summary: grant_summary(&g.grant.scope),
        expires_at: g.grant.expires_at,
        max_uses: g.grant.max_uses,
        uses: g.grant.uses,
        created_at: g.grant.created_at,
        last_used_at: g.last_used_at,
        origin: g.origin.clone(),
        service: match &g.grant.scope {
            Scope::Service(s) => s.service.clone(),
            Scope::Accounts(s) => s.service.clone(),
            _ => SERVICE_GMAIL.to_owned(),
        },
        account: g.grant.account.clone(),
        lines: scope_lines(&g.grant.scope),
        active: g.grant.is_active(now),
        state: if g.grant.revoked {
            "revoked"
        } else if g.grant.expires_at.is_some_and(|t| now >= t) {
            "expired"
        } else if g.grant.max_uses.is_some_and(|m| g.grant.uses >= m) {
            "used_up"
        } else {
            "active"
        }
        .to_owned(),
        all_mail: matches!(&g.grant.scope, Scope::Read(s) if s.any),
        editable_scope: editable_scope(&g.grant.scope),
    }
}

/// The scope as the editor shows it, or `None` when the editor could not say what the grant covers.
fn editable_scope(scope: &Scope) -> Option<GrantScopeChoice> {
    fn split(rules: &[AddrRule]) -> Option<(Vec<String>, Vec<String>)> {
        let (mut addresses, mut domains) = (Vec::new(), Vec::new());
        for rule in rules {
            match rule {
                AddrRule::Exact(a) => addresses.push(a.clone()),
                AddrRule::Domain(d) => domains.push(d.clone()),
                AddrRule::Regex(_) => return None,
            }
        }
        Some((addresses, domains))
    }
    let empty = GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: Vec::new(),
        sender_domains: Vec::new(),
        subject_pattern: None,
        recipient_addresses: Vec::new(),
        recipient_domains: Vec::new(),
        resources: Vec::new(),
        classes: Vec::new(),
    };
    match scope {
        Scope::Read(s) if s.any => Some(GrantScopeChoice {
            all_mail: true,
            ..empty
        }),
        Scope::Read(s) => {
            if s.message_ids.is_some() || !s.to.is_empty() || s.body.is_some() || !s.labels.is_empty() {
                return None;
            }
            if s.after.is_some() || s.before.is_some() {
                return None;
            }
            let (addresses, domains) = split(&s.from)?;
            Some(GrantScopeChoice {
                sender_addresses: addresses,
                sender_domains: domains,
                subject_pattern: s.subject.as_ref().map(|p| p.source().to_owned()),
                ..empty
            })
        }
        Scope::Accounts(_) | Scope::Service(_) => None,
        Scope::Send(s) => {
            if s.body.is_some() {
                return None;
            }
            let (addresses, domains) = split(&s.recipients)?;
            Some(GrantScopeChoice {
                recipient_addresses: addresses,
                recipient_domains: domains,
                subject_pattern: s.subject.as_ref().map(|p| p.source().to_owned()),
                ..empty
            })
        }
    }
}

pub fn activity_entry(r: &AuditRecord) -> ActivityEntry {
    ActivityEntry {
        id: r.seq,
        at: r.at,
        connection_id: r.connection_id.clone(),
        connection_label: text::truncate_chars(&text::one_line(&r.connection_label), MAX_LABEL_CHARS),
        action: r.action.clone(),
        outcome: r.outcome.clone(),
        detail: text::one_line(&r.detail),
        grant_id: r.grant_id.clone(),
        service: r.service.clone(),
        account: r.account.clone(),
        count: r.count,
        info: ActivityInfo {
            query: r.info.query.clone(),
            messages: r
                .info
                .messages
                .iter()
                .map(|m| ActivityMessage {
                    id: m.id.clone(),
                    text: m.text.clone(),
                    from: m.from.clone(),
                    subject: m.subject.clone(),
                    date: m.date,
                })
                .collect(),
            email: r.info.email.as_ref().map(|e| EmailView {
                to: e.to.clone(),
                cc: e.cc.clone(),
                subject: e.subject.clone(),
                body: e.body.clone(),
            }),
            note: r.info.note.clone(),
            grant_summary: r.info.grant_summary.clone(),
            accounts: r.info.accounts.clone(),
        },
        op: r.op.clone(),
        op_title: op_title(&r.service, &r.op),
        decided_by: r.info.decided_by.clone(),
        autopilot: crate::autopilot::activity_note(r),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use reins_policy::{Grant, Pattern};
    use reins_proto::gmail::OutgoingEmail;

    use super::*;

    fn summary(id: &str, from: &str, name: Option<&str>) -> MessageSummary {
        MessageSummary {
            id: id.to_owned(),
            thread_id: "t".to_owned(),
            from: from.to_owned(),
            from_name: name.map(str::to_owned),
            to: vec![],
            cc: vec![],
            subject: format!("Subject {id}"),
            date: 5,
            snippet: "snip".to_owned(),
        }
    }

    fn parked(call: ToolCall, messages: Vec<MessageSummary>, covered: &[(&str, &str)]) -> ParkedRequest {
        ParkedRequest {
            request: RelayRequest {
                v: 1,
                id: "r1".into(),
                connection_id: "c1".into(),
                connection_label: "Chat\u{202E}GPT".to_owned(),
                created_at: 9,
                wait_until: Some(54),
                account: Some("me@gmail.com".to_owned()),
                call,
            },
            messages,
            covered: covered.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect(),
            account: Some("me@gmail.com".to_owned()),
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: None,
        }
    }

    #[test]
    fn search_view_marks_covered_messages() {
        let p = parked(
            ToolCall::GmailSearch {
                query: "from:bank\n".to_owned(),
                max_results: 10,
            },
            vec![summary("m1", "a@bank.com", Some("Bank")), summary("m2", "", None)],
            &[("m1", "g1")],
        );
        let v = approval_view(&p);
        assert_eq!(
            (v.kind, v.query.as_deref(), v.connection_label.as_str()),
            (ApprovalKind::Search, Some("from:bank"), "ChatGPT")
        );
        assert_eq!((v.messages[0].from.as_str(), v.messages[0].covered_by_grant), ("Bank <a@bank.com>", true));
        assert_eq!((v.messages[1].from.as_str(), v.messages[1].covered_by_grant), ("(unknown sender)", false));
        assert_eq!(request_title(&p), "ChatGPT wants to search your Gmail");
        assert_eq!(request_subtitle(&p), "from:bank");
        assert_eq!(request_item(&p).id, "r1");
    }

    #[test]
    fn send_view_shows_the_whole_email() {
        let email = OutgoingEmail {
            to: vec!["a@work.com".to_owned()],
            cc: vec!["b@work.com".to_owned()],
            subject: "Hi\u{202E}".to_owned(),
            body: "Line1\nLine2\u{202E}".to_owned(),
            reply_to_message_id: None,
        };
        let p = parked(
            ToolCall::GmailSend {
                email: Box::new(email),
            },
            vec![],
            &[],
        );
        let v = approval_view(&p);
        let e = v.email.unwrap();
        assert_eq!((e.subject.as_str(), e.body.as_str(), e.cc.len()), ("Hi", "Line1\nLine2", 1));
        assert_eq!(v.kind, ApprovalKind::Send);
        assert_eq!(request_title(&p), "ChatGPT wants to send an email to a@work.com");
    }

    #[test]
    fn read_titles_count_emails() {
        let one = parked(
            ToolCall::GmailRead {
                message_ids: vec!["a".into()],
            },
            vec![],
            &[],
        );
        assert_eq!(request_title(&one), "ChatGPT wants to read 1 email");
        let many = parked(
            ToolCall::GmailRead {
                message_ids: vec!["a".into(), "b".into()],
            },
            vec![summary("a", "x@y.com", None)],
            &[],
        );
        assert_eq!(request_title(&many), "ChatGPT wants to read 2 emails");
        assert_eq!(request_subtitle(&many), "1 message found");
    }

    #[test]
    fn parked_payload_round_trips() {
        let p = parked(
            ToolCall::GmailSearch {
                query: "q".to_owned(),
                max_results: 5,
            },
            vec![summary("m1", "a@b.com", None)],
            &[("m1", "g")],
        );
        let json = serde_json::to_vec(&p).unwrap();
        assert_eq!(serde_json::from_slice::<ParkedRequest>(&json).unwrap(), p);
    }

    #[test]
    fn pairing_is_cleaned_and_shown() {
        let p = clean_pairing(PairingRequest {
            v: 1,
            id: "p1".into(),
            client_name: "  Cla\u{202E}ude \n".to_owned(),
            client_host: "claude.ai".to_owned(),
            choices: [12, 47, 83],
            created_at: 3,
            client_key: None,
        });
        assert_eq!(p.client_name, "Claude");
        assert_eq!(pairing_item(&p).title, "Connect Claude to Reins?");
        let v = pairing_view(&p);
        assert_eq!((v.choices.clone(), v.client_host.as_str()), (vec![12, 47, 83], "claude.ai"));
        assert_eq!(v.key_fingerprint, None);

        let key = desktop::encode_key(&[7u8; 32]);
        let desk = clean_pairing(PairingRequest {
            client_key: Some(key.clone()),
            ..p.clone()
        });
        assert_eq!(pairing_view(&desk).key_fingerprint, desktop::key_fingerprint(&key));
        let bad = clean_pairing(PairingRequest {
            client_key: Some(format!("{key}A")),
            ..p
        });
        assert_eq!(pairing_view(&bad).key_fingerprint, None);
        assert_eq!(bad.client_key, None, "an unusable key is dropped");
    }

    #[test]
    fn grant_summaries_are_readable() {
        let read = Scope::Read(ReadScope {
            from: vec![AddrRule::domain("bank.com").unwrap(), AddrRule::exact("a@x.com").unwrap()],
            subject: Some(Pattern::literal("statement").unwrap()),
            message_ids: Some(BTreeSet::from(["m1".to_owned()])),
            ..ReadScope::default()
        });
        assert_eq!(
            grant_summary(&read),
            "Read emails 1 specific email, from @bank.com, a@x.com, subject contains \"statement\""
        );
        let send = Scope::Send(SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        });
        assert_eq!(grant_summary(&send), "Send emails to @work.com");
        let stored = StoredGrant {
            grant: Grant::new("g1".into(), "c1".into(), send, 10, Some(99), Some(3)).unwrap(),
            connection_label: "Claude".to_owned(),
            last_used_at: Some(50),
            origin: "user".to_owned(),
        };
        let v = grant_view(&stored);
        assert_eq!((v.action.as_str(), v.expires_at, v.max_uses, v.uses), ("send", Some(99), Some(3), 0));
        assert_eq!(
            (v.origin.as_str(), v.last_used_at, v.lines.clone()),
            ("user", Some(50), vec!["To @work.com".to_owned()])
        );
        assert!(!v.active, "expired long ago");
    }

    #[test]
    fn activity_is_cleaned() {
        let e = activity_entry(&AuditRecord {
            seq: 7,
            at: 1,
            connection_id: "c1".to_owned(),
            connection_label: "Bad\u{202E}Label".to_owned(),
            action: "send".to_owned(),
            outcome: "sent".to_owned(),
            detail: "to a@b.com\n\u{202E}".to_owned(),
            grant_id: None,
            service: "gmail".to_owned(),
            account: None,
            count: 1,
            info: AuditInfo::default(),
            op: String::new(),
        });
        assert_eq!((e.connection_label.as_str(), e.detail.as_str()), ("BadLabel", "to a@b.com"));
    }
}
