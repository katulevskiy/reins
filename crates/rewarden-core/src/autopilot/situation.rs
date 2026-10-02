//! The situation text (spec §4): what the model reads about a request, rendered from exactly what the user would see.
//!
//! Two texts per request: `S_facts`, only what the phone worked out or verified, and `S_full`, the same plus the
//! AI-written (or otherwise untrusted) part after a separator line. The training data (`tools/laya/sequence.py`) is
//! rendered by the same rules: fixed key order, one `key: value` per line, empty keys omitted, values cleaned by
//! [`clean_value`]. Keep both in sync.

use std::collections::BTreeSet;

use crate::text;
use crate::types::{ApprovalKind, ApprovalView, BlobView};

/// Values are cut to this many characters (`content` to [`CONTENT_MAX_CHARS`]).
pub const VALUE_MAX_CHARS: usize = 300;
pub const CONTENT_MAX_CHARS: usize = 600;
/// The line between the facts and the AI-written part of `S_full`.
pub const AI_SEPARATOR: &str = "--- written by the AI ---";
/// Literal special tokens of every shipped tokenizer: defused in every value so that no text can add sequence
/// structure ("[SEP]" becomes "(SEP)", "<|padding|>" becomes "(padding)").
pub const SPECIAL_LITERALS: [&str; 13] = [
    "[CLS]",
    "[SEP]",
    "[MASK]",
    "[PAD]",
    "[UNK]",
    "<|padding|>",
    "<pad>",
    "<eos>",
    "<bos>",
    "<unk>",
    "<mask>",
    "<start_of_turn>",
    "<end_of_turn>",
];
/// Most labels listed in one value (recipients, chats, senders).
const MAX_LISTED: usize = 5;

fn defused(literal: &str) -> String {
    let inner = literal.strip_prefix("<|").and_then(|l| l.strip_suffix("|>")).unwrap_or_else(|| {
        let mut chars = literal.chars();
        chars.next();
        chars.next_back();
        chars.as_str()
    });
    format!("({inner})")
}

/// One value as one trimmed line: special-token literals defused, every whitespace run one space, other C0 controls
/// and DEL dropped, trimmed, and cut to `limit` characters (the last one then being "…").
pub fn clean_value(value: &str, limit: usize) -> String {
    let mut s = value.to_owned();
    for literal in SPECIAL_LITERALS {
        if s.contains(literal) {
            s = s.replace(literal, &defused(literal));
        }
    }
    let mut collapsed = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            in_space = false;
            collapsed.push(c);
        }
    }
    let kept: String =
        collapsed.chars().filter(|c| !matches!(u32::from(*c), 0x00..=0x08 | 0x0e..=0x1f | 0x7f)).collect();
    let trimmed = kept.trim_matches(' ');
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", head.trim_end_matches(' '))
}

/// "N minutes" under an hour, "N hours" under 48 hours, else "N days" (rounded down; singular for 1).
pub fn fmt_age(seconds: i64) -> String {
    let s = seconds.max(0);
    let (n, unit) = if s < 3_600 {
        (s / 60, "minute")
    } else if s < 48 * 3_600 {
        (s / 3_600, "hour")
    } else {
        (s / 86_400, "day")
    };
    format!(
        "{n} {unit}{}",
        if n == 1 {
            ""
        } else {
            "s"
        }
    )
}

/// The facts part, keys in the order they are rendered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub connection: String,
    /// Seconds since the connection was paired.
    pub connection_age: Option<i64>,
    /// (approved, denied) in this phone's activity.
    pub connection_history: Option<(u32, u32)>,
    pub service: String,
    pub action: String,
    pub operation: String,
    pub class: String,
    pub account: String,
    pub target: String,
    pub target_is_new: Option<bool>,
    /// Rendered only when above 1.
    pub count: u32,
    /// Joined with "; ".
    pub details: Vec<String>,
}

/// The AI-written (or otherwise untrusted) part: the reason the AI gave and the content it carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiPart {
    pub reason: String,
    pub content: String,
}

/// `S_facts`.
pub fn render_facts(f: &Facts) -> String {
    let mut lines = Vec::new();
    let mut push = |key: &str, value: &str| {
        let v = clean_value(value, VALUE_MAX_CHARS);
        if !v.is_empty() {
            lines.push(format!("{key}: {v}"));
        }
    };
    push("connection", &f.connection);
    push("connection age", &f.connection_age.map(fmt_age).unwrap_or_default());
    push(
        "connection history",
        &f.connection_history.map(|(a, d)| format!("{a} approved, {d} denied")).unwrap_or_default(),
    );
    push("service", &f.service);
    push("action", &f.action);
    push("operation", &f.operation);
    push("class", &f.class);
    push("account", &f.account);
    push("target", &f.target);
    push(
        "target is new",
        f.target_is_new
            .map(|n| {
                if n {
                    "yes"
                } else {
                    "no"
                }
            })
            .unwrap_or_default(),
    );
    if f.count > 1 {
        push("count", &f.count.to_string());
    }
    let details: Vec<String> =
        f.details.iter().map(|d| clean_value(d, VALUE_MAX_CHARS)).filter(|d| !d.is_empty()).collect();
    push("details", &details.join("; "));
    lines.join("\n")
}

/// The AI part after the separator line; empty when it has nothing.
pub fn render_ai(ai: &AiPart) -> String {
    let mut lines = Vec::new();
    for (key, value, limit) in [("reason", &ai.reason, VALUE_MAX_CHARS), ("content", &ai.content, CONTENT_MAX_CHARS)] {
        let v = clean_value(value, limit);
        if !v.is_empty() {
            lines.push(format!("{key}: {v}"));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("{AI_SEPARATOR}\n{}", lines.join("\n"))
}

/// (`S_facts`, `S_full`); the two are equal when there is no AI part.
pub fn render(f: &Facts, ai: &AiPart) -> (String, String) {
    let facts = render_facts(f);
    let extra = render_ai(ai);
    if extra.is_empty() {
        (facts.clone(), facts)
    } else {
        let full = format!("{facts}\n{extra}");
        (facts, full)
    }
}

/// Splits a situation typed in the playground into (`S_facts`, `S_full`) at the separator line.
pub fn split(situation: &str) -> (String, String) {
    let full = situation.trim().to_owned();
    match full.split_once(AI_SEPARATOR) {
        Some((facts, _)) => (facts.trim_end().to_owned(), full),
        None => (full.clone(), full),
    }
}

/// A request as Autopilot knows it, before the connection facts are added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Described {
    pub facts: Facts,
    pub ai: AiPart,
    /// `service/action[/class]` (spec §5.4).
    pub class_key: String,
    /// "Push to a branch · dkat/rewarden": shown for neighbours and in notifications.
    pub label: String,
}

/// `service/action/class`, without a trailing slash when there is no class.
pub fn class_key(service: &str, action: &str, class: &str) -> String {
    let mut key = format!("{service}/{action}");
    if !class.is_empty() {
        key = format!("{key}/{class}");
    }
    key.to_lowercase()
}

fn listed(labels: impl IntoIterator<Item = String>) -> String {
    let mut seen = BTreeSet::new();
    let unique: Vec<String> =
        labels.into_iter().map(|l| text::one_line(&l)).filter(|l| !l.is_empty() && seen.insert(l.clone())).collect();
    let more = unique.len().saturating_sub(MAX_LISTED);
    let mut out = unique.into_iter().take(MAX_LISTED).collect::<Vec<_>>().join(", ");
    if more > 0 {
        out = format!("{out} and {more} more");
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn label_of(operation: &str, target: &str) -> String {
    let label = if target.is_empty() {
        operation.to_owned()
    } else {
        format!("{operation} · {target}")
    };
    clean_value(&label, 100)
}

/// What a parked request is, from the view the approval screen shows.
pub fn describe_request(view: &ApprovalView) -> Described {
    let mut f = Facts {
        service: if view.service.starts_with("mcp:") {
            "mcp".to_owned()
        } else {
            view.service.clone()
        },
        action: view.action.clone(),
        operation: view.op_title.clone(),
        class: view.class.clone(),
        account: view.account.clone().unwrap_or_default(),
        count: view.count,
        ..Facts::default()
    };
    let mut ai = AiPart::default();
    let mut content: Vec<String> = Vec::new();

    // What the request is about, kind by kind; the untrusted text goes to `content`.
    if let Some(mcp) = &view.mcp {
        f.operation = if mcp.title.is_empty() {
            mcp.tool.clone()
        } else {
            mcp.title.clone()
        };
        f.class.clone_from(&mcp.tool);
        f.account.clone_from(&mcp.server_name);
        f.target = format!("{} · {}", mcp.server_name, mcp.tool);
        f.details.push(format!("server {} ({})", mcp.server_name, mcp.server_url));
        f.details.push(
            if mcp.read_only {
                "read-only tool"
            } else {
                "the tool changes things"
            }
            .to_owned(),
        );
        if mcp.destructive {
            f.details.push("the server marks the tool destructive".to_owned());
        }
        content.push(mcp.arguments_json.clone());
    } else if let Some(ask) = &view.ask {
        "ask".clone_into(&mut f.action);
        f.account.clear();
        let (kind, subject) = match ask.topic.as_deref().and_then(|t| t.split_once(':')) {
            Some((kind, subject)) => (kind.trim().to_owned(), subject.trim().to_owned()),
            None => ("question".to_owned(), ask.topic.clone().unwrap_or_default()),
        };
        f.class = kind;
        f.target = subject;
        if f.operation.is_empty() {
            "Answer a question from the desktop app".clone_into(&mut f.operation);
        }
        ai.reason.clone_from(&ask.question);
        content.extend(ask.detail.clone());
    } else if let Some(secrets) = &view.secrets {
        f.target.clone_from(&secrets.command);
        f.details.push(plural(secrets.items.len(), "secret", "secrets"));
        f.details.push(format!("kept {} seconds", secrets.lease_secs));
        ai.reason = secrets.purpose.clone().unwrap_or_default();
    } else if let Some(ssh) = &view.ssh {
        f.target = ssh.host.clone().unwrap_or_else(|| ssh.key_name.clone());
        f.details.push(format!("key {} {}", ssh.key_name, ssh.key_fingerprint));
    } else if let Some(git) = &view.git {
        f.target.clone_from(&git.repo);
        for r in &git.refs {
            f.details.push(format!("{} {} ({})", r.kind, r.short_name, r.change));
            if r.change != "delete" {
                f.details.push(plural(r.commit_count as usize, "commit", "commits"));
                f.details.push(plural(r.files_changed as usize, "file changed", "files changed"));
            }
            f.details.push(
                if r.force_unknown {
                    "history rewrite not ruled out"
                } else if r.force {
                    "force push (rewrites history)"
                } else {
                    "no force"
                }
                .to_owned(),
            );
            if let (Some(a), Some(d)) = (r.additions, r.deletions) {
                f.details.push(format!("+{a} -{d} lines"));
            }
            content.extend(r.commits.iter().map(|c| c.subject.clone()));
            content.extend(r.files.iter().map(|file| format!("{} {}", file.status, file.path)));
        }
        if let Some(owner) = git.repo.split('/').next()
            && !owner.is_empty()
            && owner.eq_ignore_ascii_case(&f.account)
        {
            f.details.push("repository of the account".to_owned());
        }
    } else {
        match view.kind {
            ApprovalKind::Search => {
                "Search emails".clone_into(&mut f.operation);
                f.target = view.query.clone().unwrap_or_default();
                f.details.push(plural(view.messages.len(), "message found", "messages found"));
            }
            ApprovalKind::Read => {
                "Read emails".clone_into(&mut f.operation);
                f.target = listed(view.messages.iter().map(|m| m.from.clone()));
            }
            ApprovalKind::Send => {
                "Send an email".clone_into(&mut f.operation);
                if let Some(email) = &view.email {
                    f.target = listed(email.to.iter().chain(&email.cc).cloned());
                    f.details.push(plural(email.to.len() + email.cc.len(), "recipient", "recipients"));
                    content.push(format!("subject: {}", email.subject));
                    content.push(email.body.clone());
                }
            }
            ApprovalKind::Grant => {
                "Ask for a standing permission".clone_into(&mut f.operation);
                if let Some(grant) = &view.grant {
                    f.target.clone_from(&grant.summary);
                    f.details.push(format!("{} access", grant.breadth));
                    f.details.extend(grant.lines.iter().cloned());
                    ai.reason.clone_from(&grant.reason);
                }
            }
            ApprovalKind::Accounts => {
                "See which accounts are connected".clone_into(&mut f.operation);
                crate::views::service_name(&view.service).clone_into(&mut f.target);
                f.details.push(plural(view.accounts.len(), "account", "accounts"));
            }
            ApprovalKind::Fetch | ApprovalKind::Write => {
                let own: Vec<String> = view.resources.iter().filter(|r| !r.wider).map(|r| r.label.clone()).collect();
                f.target = if own.is_empty() {
                    view.query.clone().unwrap_or_default()
                } else {
                    listed(own)
                };
                let wider: Vec<String> = view.resources.iter().filter(|r| r.wider).map(|r| r.label.clone()).collect();
                if !wider.is_empty() {
                    f.details.push(format!("in {}", listed(wider)));
                }
                content.extend(view.preview.iter().cloned());
            }
        }
    }

    // Items found (emails, messages, issues): their text is third-party content, never a fact.
    if !view.messages.is_empty() {
        let sensitive = view.messages.iter().filter(|m| m.sensitive).count();
        let covered = view.messages.iter().filter(|m| m.covered_by_grant).count();
        if !matches!(view.kind, ApprovalKind::Search) {
            f.details.push(plural(view.messages.len(), "item", "items"));
        }
        if covered > 0 {
            f.details.push(format!("{covered} already allowed"));
        }
        if sensitive > 0 {
            f.details.push(format!("{sensitive} sensitive"));
        }
        content.extend(view.messages.iter().take(MAX_LISTED).map(|m| format!("{} — {}", m.subject, m.snippet)));
    }
    if let Some(blob) = &view.blob {
        f.details.push(format!(
            "uses an uploaded file: {}, {}",
            crate::blob::size_text(blob.size),
            text::one_line(&blob.content_type)
        ));
    }
    if view.no_standing {
        f.details.push("asked every time".to_owned());
    }
    if f.operation.is_empty() {
        f.operation = if view.op.is_empty() {
            view.action.clone()
        } else {
            view.op.clone()
        };
    }
    ai.content = content.join(" | ");
    Described {
        // The service as the view names it: classes of MCP tools are per server (`mcp:<id>`).
        class_key: class_key(&view.service, &f.action, &f.class),
        label: label_of(&f.operation, &f.target),
        facts: f,
        ai,
    }
}

/// What an uploaded file waiting for the user is.
pub fn describe_blob(view: &BlobView, flagged: bool) -> Described {
    let mut f = Facts {
        service: crate::blob::SERVICE_FILES.to_owned(),
        action: "upload".to_owned(),
        operation: "Share a file".to_owned(),
        class: "upload".to_owned(),
        target: view.name.clone(),
        count: 1,
        ..Facts::default()
    };
    f.details.push(crate::blob::size_text(view.size));
    f.details.push(view.content_type.clone());
    if flagged {
        f.details.push("executable or archive".to_owned());
    }
    let ai = AiPart {
        reason: view.purpose.clone(),
        content: view.preview_text.clone().unwrap_or_default(),
    };
    Described {
        class_key: class_key(&f.service, &f.action, &f.class),
        label: label_of(&f.operation, &f.target),
        facts: f,
        ai,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_one_clean_trimmed_line() {
        assert_eq!(clean_value("  a\t\tb \r\n c ", 300), "a b c");
        assert_eq!(clean_value("x\u{0}y\u{7}z\u{7f}", 300), "xyz");
        assert_eq!(clean_value("a \u{0} b", 300), "a  b", "controls go after the whitespace runs, as in Python");
        assert_eq!(clean_value("[SEP] <|padding|> <eos>", 300), "(SEP) (padding) (eos)");
        assert_eq!(clean_value("abcdef", 4), "abc…");
        assert_eq!(clean_value("ab   cdef", 4), "ab…", "the cut is right-trimmed");
        assert_eq!(clean_value("é😀ü", 3), "é😀ü", "characters, not bytes");
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(fmt_age(59), "0 minutes");
        assert_eq!(fmt_age(60), "1 minute");
        assert_eq!(fmt_age(3_600), "1 hour");
        assert_eq!(fmt_age(47 * 3_600), "47 hours");
        assert_eq!(fmt_age(12 * 86_400), "12 days");
        assert_eq!(fmt_age(-5), "0 minutes");
    }

    #[test]
    fn the_spec_example_renders_as_in_the_spec() {
        let f = Facts {
            connection: "Claude Code (laptop)".to_owned(),
            connection_age: Some(12 * 86_400),
            connection_history: Some((140, 3)),
            service: "github".to_owned(),
            action: "write".to_owned(),
            operation: "Push to a branch".to_owned(),
            class: "push".to_owned(),
            account: "dkat".to_owned(),
            target: "dkat/rewarden".to_owned(),
            target_is_new: Some(false),
            count: 1,
            details: vec![
                "branch feature/laya (not the default branch)".to_owned(),
                "3 commits".to_owned(),
                "7 files changed".to_owned(),
                "no force".to_owned(),
            ],
        };
        let ai = AiPart {
            reason: "fix flaky test".to_owned(),
            content: String::new(),
        };
        let (facts, full) = render(&f, &ai);
        assert_eq!(
            facts,
            "connection: Claude Code (laptop)\nconnection age: 12 days\nconnection history: 140 approved, 3 denied\n\
             service: github\naction: write\noperation: Push to a branch\nclass: push\naccount: dkat\n\
             target: dkat/rewarden\ntarget is new: no\ndetails: branch feature/laya (not the default branch); \
             3 commits; 7 files changed; no force"
        );
        assert_eq!(full, format!("{facts}\n--- written by the AI ---\nreason: fix flaky test"));
        assert_eq!(split(&full), (facts.clone(), full.clone()));
        assert_eq!(render(&f, &AiPart::default()), (facts.clone(), facts));
    }

    #[test]
    fn class_keys() {
        assert_eq!(class_key("github", "write", "push"), "github/write/push");
        assert_eq!(class_key("gmail", "read", ""), "gmail/read");
    }
}
