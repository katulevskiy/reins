//! One-tap answers. A request that is not asked every time can be approved without opening it (from the notification,
//! or with "Approve all" for a burst from one AI), exactly as the approval sheet approves it untouched: everything found
//! is released except what looks like a code or a password, and nothing is remembered. The sheet also offers "Approve
//! and allow for a while": the same approval plus a permission for the same connection, the same kind of request and the
//! same target, for an hour (eight hours once the user approved the same thing a few times that day). Nothing here is
//! offered for the hard floor ([`crate::autopilot::gates::request_floor`]), and [`Engine::approve_quick`] refuses it.

use std::collections::BTreeSet;

use reins_proto::gmail::ToolCall;

use crate::autopilot::gates::request_floor;
use crate::engine::Engine;
use crate::store::{AuditRecord, unix_now};
use crate::types::{
    ApprovalChoice, ApprovalKind, ApprovalView, GitPushView, GrantScopeChoice, QuickApproval, StandingGrant,
};
use crate::views::{ParkedRequest, service_name};
use crate::{CoreError, text};

/// How long "Approve and allow for a while" lasts…
pub const ALLOW_SECS: u64 = 3_600;
/// …and how long once the user approved the same thing [`REPEATS_FOR_LONGER`] times in the last day.
pub const REPEAT_ALLOW_SECS: u64 = 8 * 3_600;
pub const REPEATS_FOR_LONGER: u32 = 2;
/// The window in which identical approvals are counted.
const REPEAT_WINDOW_SECS: i64 = 86_400;
const MAX_HEADLINE_CHARS: usize = 240;

/// Why a request cannot be answered without opening it, or `None`.
pub fn quick_floor(view: &ApprovalView) -> Option<&'static str> {
    request_floor(view)
}

/// What approving untouched releases: every email found; every item except what looks like a code or a password.
pub fn default_selection(view: &ApprovalView) -> Vec<String> {
    match view.kind {
        ApprovalKind::Search | ApprovalKind::Read => view.messages.iter().map(|m| m.id.clone()).collect(),
        ApprovalKind::Fetch => view.messages.iter().filter(|m| !m.sensitive).map(|m| m.id.clone()).collect(),
        _ => Vec::new(),
    }
}

/// Adds the headline and the one-tap answers to a view built from `parked` (repeats are counted by the engine).
pub fn decorate(parked: &ParkedRequest, mut view: ApprovalView) -> ApprovalView {
    view.headline = headline(parked, &view);
    view.quick = quick_view(parked, &view, 0);
    view
}

fn quick_view(parked: &ParkedRequest, view: &ApprovalView, repeats: u32) -> Option<QuickApproval> {
    if quick_floor(view).is_some() {
        return None;
    }
    let (allow, allow_what) = match allow_scope(parked, view) {
        Some((scope, what)) => (
            Some(StandingGrant {
                duration_secs: Some(if repeats >= REPEATS_FOR_LONGER {
                    REPEAT_ALLOW_SECS
                } else {
                    ALLOW_SECS
                }),
                max_uses: None,
                scope,
            }),
            what,
        ),
        None => (None, String::new()),
    };
    Some(QuickApproval {
        from_notification: !view.messages.iter().any(|m| m.sensitive),
        allow,
        allow_what,
        repeats,
    })
}

fn empty_scope() -> GrantScopeChoice {
    GrantScopeChoice {
        all_mail: false,
        selected_messages_only: false,
        sender_addresses: Vec::new(),
        sender_domains: Vec::new(),
        subject_pattern: None,
        recipient_addresses: Vec::new(),
        recipient_domains: Vec::new(),
        resources: Vec::new(),
        classes: Vec::new(),
    }
}

/// The permission "Approve and allow for a while" makes, and what it covers in words: the same connection (every grant
/// is for one), the same kind of request and the same target. `None` when the request cannot be remembered.
fn allow_scope(parked: &ParkedRequest, view: &ApprovalView) -> Option<(GrantScopeChoice, String)> {
    if view.no_standing {
        return None;
    }
    match view.kind {
        // A grant for reading mail covers searching and reading one account (the grant names it).
        ApprovalKind::Search | ApprovalKind::Read => {
            let account = view.account.as_deref().map_or_else(|| "your Gmail".to_owned(), text::one_line);
            Some((
                GrantScopeChoice {
                    all_mail: true,
                    ..empty_scope()
                },
                format!("searching and reading {account}"),
            ))
        }
        // Sending to exactly these people.
        ApprovalKind::Send => {
            let ToolCall::GmailSend {
                email,
            } = &parked.request.call
            else {
                return None;
            };
            let mut seen = BTreeSet::new();
            let recipients: Vec<String> =
                email.recipients().map(str::to_lowercase).filter(|a| seen.insert(a.clone())).collect();
            if recipients.is_empty() {
                return None;
            }
            let what = format!("emails to {}", list(&recipients));
            Some((
                GrantScopeChoice {
                    recipient_addresses: recipients,
                    ..empty_scope()
                },
                what,
            ))
        }
        // The things the request touches (not the wider ones they belong to); a change keeps the kind of this one.
        ApprovalKind::Fetch | ApprovalKind::Write => {
            let narrow: Vec<_> = view.resources.iter().filter(|r| !r.wider).collect();
            if narrow.is_empty() {
                return None;
            }
            let labels: Vec<String> = narrow.iter().map(|r| text::one_line(&r.label)).collect();
            let what = if view.kind == ApprovalKind::Fetch {
                format!("reading {} ({})", list(&labels), service_name(&view.service))
            } else if let Some(mcp) = &view.mcp {
                format!("{} on {}", text::one_line(&mcp.title), text::one_line(&mcp.server_name))
            } else if view.ask.is_some() {
                lower_first(&list(&labels))
            } else {
                let op = if view.op_title.is_empty() {
                    view.action.clone()
                } else {
                    lower_first(&view.op_title)
                };
                format!("{op}: {}", list(&labels))
            };
            Some((
                GrantScopeChoice {
                    resources: narrow.iter().map(|r| r.id.clone()).collect(),
                    ..empty_scope()
                },
                what,
            ))
        }
        ApprovalKind::Grant | ApprovalKind::Accounts => None,
    }
}

/// What approving does, in one sentence, from the AI's point of view.
pub fn headline(parked: &ParkedRequest, view: &ApprovalView) -> String {
    let who = parked.label();
    let line = match &parked.request.call {
        ToolCall::GmailSearch {
            query,
            ..
        } => match view.messages.len() {
            0 => format!("{who} is told that nothing matched \"{}\".", text::one_line(query)),
            n => format!("{who} gets the {} found for \"{}\".", plural(n, "email", "emails"), text::one_line(query)),
        },
        ToolCall::GmailRead {
            ..
        } => format!("{who} gets the full text of {}.", plural(view.messages.len(), "email", "emails")),
        ToolCall::GmailSend {
            email,
        } => {
            let to: Vec<String> = email.recipients().map(str::to_owned).collect();
            let from = view.account.as_deref().map(|a| format!(" from {a}")).unwrap_or_default();
            format!("An email to {} goes out{from}.", list(&to))
        }
        ToolCall::RequestGrant {
            grant,
        } => format!("{who} may {} without asking you.", crate::views::grant_request_summary(grant)),
        ToolCall::ListAccounts {
            service,
            ..
        } => format!(
            "{who} sees the {} addresses you tick, nothing in them.",
            service.as_deref().map_or("connected", service_name)
        ),
        ToolCall::Mcp(_) => match &view.mcp {
            Some(m) => format!("{who} runs {} on {}.", text::one_line(&m.title), text::one_line(&m.server_name)),
            None => format!("{who} runs a tool of an MCP server."),
        },
        ToolCall::RequestUpload {
            ..
        } => format!("{who} shares a file."),
        ToolCall::Connector(_) => connector_headline(&who, view),
    };
    text::truncate_chars(&text::one_line(&line), MAX_HEADLINE_CHARS)
}

fn connector_headline(who: &str, view: &ApprovalView) -> String {
    if let Some(git) = &view.git {
        return git_headline(git);
    }
    if let Some(ask) = &view.ask {
        return format!("The desktop app gets a yes to: {}", text::one_line(&ask.question));
    }
    if let Some(s) = &view.secrets {
        return format!(
            "The desktop app gets {} for {}, for {}.",
            plural(s.items.len(), "secret", "secrets"),
            text::one_line(&s.command),
            crate::views::duration_text(s.lease_secs.max(60))
        );
    }
    if let Some(ssh) = &view.ssh {
        let host = ssh.host.as_deref().map(|h| format!(" to {h}")).unwrap_or_default();
        return format!("Your SSH key {} signs a sign-in{host}.", text::one_line(&ssh.key_name));
    }
    if view.kind == ApprovalKind::Write {
        return view.preview.first().map_or_else(
            || format!("{who} makes a change in {}.", service_name(&view.service)),
            |l| with_period(&text::one_line(l)),
        );
    }
    let place = {
        let narrow: Vec<String> =
            view.resources.iter().filter(|r| !r.wider).map(|r| text::one_line(&r.label)).collect();
        if narrow.is_empty() || narrow.len() > 2 {
            service_name(&view.service).to_owned()
        } else {
            list(&narrow)
        }
    };
    let held = view.messages.iter().filter(|m| m.sensitive).count();
    let shared = view.messages.len() - held;
    let mut line = if view.messages.is_empty() {
        format!("{who} is told that nothing was found in {place}.")
    } else {
        format!("{who} gets {} from {place}.", plural(shared, "item", "items"))
    };
    if held == 1 {
        line.push_str(" One that looks like a code or a password stays private unless you tick it.");
    } else if held > 1 {
        line.push_str(" Some that look like a code or a password stay private unless you tick them.");
    }
    line
}

fn git_headline(git: &GitPushView) -> String {
    let repo = text::one_line(&git.repo);
    let [r] = git.refs.as_slice() else {
        return format!("Pushes to {} in {repo}.", plural(git.refs.len(), "branch or tag", "branches and tags"));
    };
    let what = if r.kind == "tag" {
        format!("the tag {}", r.short_name)
    } else {
        r.short_name.clone()
    };
    match r.change.as_str() {
        "delete" => format!("Deletes {what} in {repo}."),
        "create" if r.kind == "tag" => format!("Publishes {what} in {repo}."),
        "create" => format!("Creates {what} in {repo} with {}.", plural(r.commit_count as usize, "commit", "commits")),
        _ if r.force || r.force_unknown => format!("Rewrites the history of {what} in {repo}."),
        _ => format!("Pushes {} to {what} in {repo}.", plural(r.commit_count as usize, "commit", "commits")),
    }
}

fn with_period(s: &str) -> String {
    if s.ends_with(['.', '!', '?', ':']) {
        s.to_owned()
    } else {
        format!("{s}.")
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "a", "a and b", "a, b and 3 more".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// What makes two approvals "the same thing": the connection, the integration, the operation, the account and the
/// targets the permission would cover.
#[derive(Debug, PartialEq, Eq)]
struct Sameness {
    connection: String,
    service: String,
    /// Gmail: search and read count as one (one permission covers both).
    actions: Vec<&'static str>,
    op: String,
    account: Option<String>,
    targets: Vec<String>,
}

fn sameness(parked: &ParkedRequest) -> Sameness {
    let actions = match parked.kind() {
        ApprovalKind::Search | ApprovalKind::Read
            if matches!(parked.request.call, ToolCall::GmailSearch { .. } | ToolCall::GmailRead { .. }) =>
        {
            vec!["search", "read"]
        }
        _ => vec![parked.action()],
    };
    Sameness {
        connection: parked.request.connection_id.0.clone(),
        service: parked.service(),
        actions,
        op: parked.op(),
        account: parked.account.clone(),
        targets: approval_targets(parked),
    }
}

/// The targets an approval of `parked` is logged with (see `AuditInfo::targets`): the things it touches, sorted.
pub(crate) fn approval_targets(parked: &ParkedRequest) -> Vec<String> {
    let Some(held) = &parked.connector else {
        return Vec::new();
    };
    if held.mcp.is_some() {
        return Vec::new();
    }
    let targets: BTreeSet<String> = match &held.preview {
        Some(p) => std::iter::once(p.resource.clone()).collect(),
        None => held.items.iter().filter(|i| !i.sensitive).map(|i| i.resource.clone()).collect(),
    };
    targets.into_iter().filter(|t| !t.is_empty()).collect()
}

/// Whether `entry` is the user's own approval of the same thing.
fn is_repeat(entry: &AuditRecord, same: &Sameness) -> bool {
    entry.detail.starts_with("approved:")
        && entry.info.decided_by.is_empty()
        && entry.connection_id == same.connection
        && entry.service == same.service
        && same.actions.contains(&entry.action.as_str())
        && entry.op == same.op
        && entry.account == same.account
        && entry.info.targets == same.targets
}

impl Engine {
    /// The approval sheet's view: the request as parked, with how often the user approved the same thing lately.
    pub fn approval_view(&self, request_id: &str) -> Result<ApprovalView, CoreError> {
        let (_, parked) = self.parked(request_id)?;
        let mut view = crate::views::approval_view(&parked);
        if view.quick.is_some() {
            let repeats = self.repeats(&parked).unwrap_or_else(|e| {
                log::warn!("could not count earlier approvals: {e}");
                0
            });
            view.quick = quick_view(&parked, &view, repeats);
        }
        Ok(view)
    }

    fn repeats(&self, parked: &ParkedRequest) -> Result<u32, CoreError> {
        let same = sameness(parked);
        let since = unix_now() - REPEAT_WINDOW_SECS;
        let entries = self.store.audit_since(&same.connection, since)?;
        Ok(u32::try_from(entries.iter().filter(|e| is_repeat(e, &same)).count()).unwrap_or(u32::MAX))
    }

    /// Approves a request as the sheet would untouched (see the module docs), from the notification or "Approve all".
    /// What is asked every time is refused: it must be opened.
    pub async fn approve_quick(&self, request_id: &str) -> Result<(), CoreError> {
        let view = crate::views::approval_view(&self.parked(request_id)?.1);
        if let Some(why) = quick_floor(&view) {
            return Err(CoreError::invalid(format!("Open this request to decide: {why}.")));
        }
        let choice = ApprovalChoice {
            selected_message_ids: default_selection(&view),
            standing: None,
        };
        self.approve(request_id, choice).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(detail: &str, decided_by: &str, targets: &[&str]) -> AuditRecord {
        AuditRecord {
            seq: 0,
            at: 0,
            connection_id: "c1".to_owned(),
            connection_label: "Claude".to_owned(),
            action: "read".to_owned(),
            outcome: "released".to_owned(),
            detail: detail.to_owned(),
            grant_id: None,
            service: "telegram".to_owned(),
            account: Some("+1555".to_owned()),
            count: 1,
            info: crate::store::AuditInfo {
                decided_by: decided_by.to_owned(),
                targets: targets.iter().map(|t| (*t).to_owned()).collect(),
                ..crate::store::AuditInfo::default()
            },
            op: "read".to_owned(),
        }
    }

    fn same(targets: &[&str]) -> Sameness {
        Sameness {
            connection: "c1".to_owned(),
            service: "telegram".to_owned(),
            actions: vec!["read"],
            op: "read".to_owned(),
            account: Some("+1555".to_owned()),
            targets: targets.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn only_the_users_own_approvals_of_the_same_thing_count() {
        assert!(is_repeat(&rec("approved: 2 items", "", &["100"]), &same(&["100"])));
        assert!(!is_repeat(&rec("approved: 2 items", "autopilot", &["100"]), &same(&["100"])), "Autopilot's own");
        assert!(!is_repeat(&rec("Read: 2 items", "", &["100"]), &same(&["100"])), "covered by a grant");
        assert!(!is_repeat(&rec("denied", "", &["100"]), &same(&["100"])));
        assert!(!is_repeat(&rec("approved: 2 items", "", &["200"]), &same(&["100"])), "another chat");
        let mut other = rec("approved: 2 items", "", &["100"]);
        other.connection_id = "c2".to_owned();
        assert!(!is_repeat(&other, &same(&["100"])), "another AI");
    }

    #[test]
    fn lists_read_naturally() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(list(&s(&["a"])), "a");
        assert_eq!(list(&s(&["a", "b"])), "a and b");
        assert_eq!(list(&s(&["a", "b", "c", "d"])), "a, b and 2 more");
        assert_eq!(lower_first("Send a Telegram message"), "send a Telegram message");
        assert_eq!(with_period("Text +1555"), "Text +1555.");
        assert_eq!(with_period("Delete it?"), "Delete it?");
    }
}
