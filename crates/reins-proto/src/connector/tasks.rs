//! Tasks (Decisions v2, `docs/design/decisions-v2/01-decision-engine.md` §12): an agent proposes an envelope (repos,
//! branches, folders, people, kinds of effect, a ceiling, a length, a budget) inside which Auto may do more for it.
//! The owner sees it on the phone and may only narrow it before approving. Ending a task only takes away, so it is
//! never asked.
//!
//! The envelope is read from these typed, bounded arguments only. The agent's `reason` is shown as the agent's words
//! and never read for policy.

use crate::connector::{ConnectorCall, Effect, Param, TASKS, ToolSpec, choice_p, int_p, list_p, str_p, tool};

/// As the desktop app's other tools (`connector::desktop`): its key and a nonce echoed in the sealed answer.
const CLIENT_KEY_PARAM: Param =
    str_p("client_key", 64, true, "The desktop app's public key (X25519, base64url without padding).");
const NONCE_PARAM: Param =
    str_p("nonce", 64, true, "Random, chosen by the desktop app; echoed inside the sealed answer.");

/// `reins_task_propose`.
pub const PROPOSE_OP: &str = "propose";
/// `reins_task_end`.
pub const END_OP: &str = "end";
/// `desktop_task_propose`, `desktop_task_end`: the same, from the paired desktop app (`reins task`).
pub const DESKTOP_PROPOSE_OP: &str = "desktop_propose";
pub const DESKTOP_END_OP: &str = "desktop_end";

/// Whether `op` proposes a task (from an agent over MCP, or from the desktop app).
#[must_use]
pub fn proposes(op: &str) -> bool {
    op == PROPOSE_OP || op == DESKTOP_PROPOSE_OP
}

/// Whether `op` ends a task.
#[must_use]
pub fn ends(op: &str) -> bool {
    op == END_OP || op == DESKTOP_END_OP
}
/// The words a ceiling is given in, lowest first (L0, L1, L2). L3 is never a ceiling.
pub const CEILINGS: &[&str] = &["undoable", "recoverable", "reaches_people"];
/// The kinds of effect a task may name (`reins_policy::effects::EffectKind`, snake case).
pub const OPERATIONS: &[&str] = &[
    "file_read",
    "file_write",
    "file_delete",
    "git_push",
    "git_ref_delete",
    "message",
    "network",
    "exec",
    "data_read",
    "data_write",
];
pub const MIN_TASK_MINUTES: i64 = 15;
pub const MAX_TASK_MINUTES: i64 = 24 * 60;
pub const MAX_TASK_BUDGET: i64 = 1_000;
pub const MAX_TASK_ITEMS: usize = 10;

/// A task's envelope as asked (or as the owner narrowed it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskDraft {
    pub title: String,
    pub reason: Option<String>,
    pub repos: Vec<String>,
    pub branches: Vec<String>,
    pub paths: Vec<String>,
    pub people: Vec<String>,
    pub operations: Vec<String>,
    /// 0..=2; `None`: the phone's default (L2 when every repository is the owner's own, else L1).
    pub ceiling: Option<u8>,
    pub minutes: i64,
    pub budget: Option<u32>,
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// A repository: `owner/name`, `owner/*`, or with a host first (`gitlab.com/group/name`). Never a lone `*`.
#[must_use]
pub fn valid_repo(r: &str) -> bool {
    let body = r.strip_suffix("/*").unwrap_or(r);
    let parts: Vec<&str> = body.split('/').collect();
    let wild = r.ends_with("/*");
    (wild && !parts.is_empty() || parts.len() >= 2)
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

/// A branch glob: a git ref name, with at most one `*`, at its end, and something before it.
#[must_use]
pub fn valid_branch(b: &str) -> bool {
    let body = b.strip_suffix('*').unwrap_or(b);
    !body.is_empty()
        && !body.contains('*')
        && !body.contains("..")
        && !body.starts_with('/')
        && !body.starts_with('-')
        && body.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// An absolute folder, not the root, without `.` or `..`.
#[must_use]
pub fn valid_path(p: &str) -> bool {
    let body = p.trim_end_matches('/');
    p.starts_with('/')
        && !body.is_empty()
        && body.split('/').skip(1).all(|c| !c.is_empty() && c != "." && c != "..")
        && !has_control(p)
}

/// A person: an address, `@domain`, a `+number` or an `@username`-style id. Never a glob.
#[must_use]
pub fn valid_person(p: &str) -> bool {
    let ok_chars = p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+'));
    let address = p.split_once('@').is_some_and(|(_, d)| d.contains('.') && !d.starts_with('.'));
    let number = p.strip_prefix('+').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    ok_chars && !p.contains('*') && (address || number || (p.starts_with('@') && p.len() > 1))
}

fn list(call: &ConnectorCall, name: &str, valid: fn(&str) -> bool) -> Result<Vec<String>, String> {
    let items: Vec<String> = call.list_arg(name).iter().map(|s| s.trim().to_owned()).collect();
    if items.len() > MAX_TASK_ITEMS {
        return Err(format!("`{name}` has more than {MAX_TASK_ITEMS} entries."));
    }
    if let Some(bad) = items.iter().find(|s| !valid(s)) {
        return Err(format!("`{name}` has an entry that is not allowed: {bad:?}."));
    }
    let mut items = items;
    items.sort();
    items.dedup();
    Ok(items)
}

fn lowered(items: Vec<String>) -> Vec<String> {
    let mut items: Vec<String> = items.into_iter().map(|s| s.to_ascii_lowercase()).collect();
    items.sort();
    items.dedup();
    items
}

/// Reads a proposal's envelope; refuses anything outside the bounds.
pub fn draft_of(call: &ConnectorCall) -> Result<TaskDraft, String> {
    let title = call.str_arg("title").map(str::trim).unwrap_or_default().to_owned();
    if title.is_empty() || has_control(&title) {
        return Err("`title` is required, one line.".to_owned());
    }
    let reason = call.str_arg("reason").map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned);
    let minutes = call.int_arg("duration_minutes").ok_or("`duration_minutes` is required.")?;
    if !(MIN_TASK_MINUTES..=MAX_TASK_MINUTES).contains(&minutes) {
        return Err(format!("`duration_minutes` must be {MIN_TASK_MINUTES} to {MAX_TASK_MINUTES}."));
    }
    let ceiling = match call.str_arg("ceiling") {
        None => None,
        Some(c) => Some(
            CEILINGS
                .iter()
                .position(|x| *x == c)
                .and_then(|i| u8::try_from(i).ok())
                .ok_or("`ceiling` must be undoable, recoverable or reaches_people.")?,
        ),
    };
    let budget = match call.int_arg("budget") {
        None => None,
        Some(b) => Some(
            u32::try_from(b)
                .ok()
                .filter(|b| (1..=MAX_TASK_BUDGET).contains(&i64::from(*b)))
                .ok_or("`budget` must be 1 to 1000.")?,
        ),
    };
    Ok(TaskDraft {
        title,
        reason,
        repos: lowered(list(call, "repos", valid_repo)?),
        branches: list(call, "branches", valid_branch)?,
        paths: list(call, "paths", valid_path)?,
        people: lowered(list(call, "people", valid_person)?),
        operations: list(call, "operations", |o| OPERATIONS.contains(&o))?,
        ceiling,
        minutes,
        budget,
    })
}

fn check_propose(call: &ConnectorCall) -> Result<(), String> {
    draft_of(call).map(drop)
}

fn propose_params(desktop: bool) -> Vec<Param> {
    let mut p = vec![
        str_p("title", 80, true, "What the task is, in a few words (\"Fix the CI\")."),
        str_p("reason", 300, false, "Why, in the agent's words; shown to the user as yours."),
        list_p(
            "repos",
            MAX_TASK_ITEMS,
            200,
            "Repositories the task works in: owner/name or owner/* (a host first for other than GitHub).",
        ),
        list_p("branches", MAX_TASK_ITEMS, 100, "Branches it may push to or delete: names or globs (fix/*)."),
        list_p("paths", MAX_TASK_ITEMS, 300, "Absolute folders it may change files in."),
        list_p("people", MAX_TASK_ITEMS, 254, "People it may write to (addresses, @domain). None by default."),
        list_p(
            "operations",
            MAX_TASK_ITEMS,
            20,
            "Kinds of effect it needs: git_push, git_ref_delete, file_write, file_delete, message, data_write, ... \
             Empty: any kind inside the rest of the envelope.",
        ),
        choice_p(
            "ceiling",
            CEILINGS,
            false,
            "The most it may do without asking: undoable, recoverable (default) or reaches_people. Nothing that \
             can't be undone is ever covered.",
        ),
        int_p("duration_minutes", MIN_TASK_MINUTES, MAX_TASK_MINUTES, None, "How long it lasts (required)."),
        int_p("budget", 1, MAX_TASK_BUDGET, None, "How many requests it may cover at most."),
    ];
    if desktop {
        p.extend([CLIENT_KEY_PARAM, NONCE_PARAM]);
    }
    p
}

pub(super) fn tools() -> Vec<ToolSpec> {
    let describe = "Propose a task to the user: an envelope (repositories, branches, folders, people, kinds of \
                    effect, a ceiling, a length) inside which Reins decides more on its own for this connection. The \
                    user approves it on their phone, and may narrow it; the result is the envelope as approved. \
                    What can't be undone, secrets, money and publishing are always asked anyway.";
    let end = "End a task now. Takes access away only, so it is never asked.";
    vec![
        tool(
            "reins_task_propose",
            TASKS,
            PROPOSE_OP,
            Effect::Write,
            "Propose a task",
            describe,
            propose_params(false),
            None,
        )
        .once()
        .checked(check_propose),
        tool(
            "reins_task_end",
            TASKS,
            END_OP,
            Effect::Write,
            "End a task",
            end,
            vec![str_p("task", 64, true, "The task's id, as the approval returned it.")],
            None,
        ),
        tool(
            "desktop_task_propose",
            TASKS,
            DESKTOP_PROPOSE_OP,
            Effect::Write,
            "Propose a task",
            describe,
            propose_params(true),
            None,
        )
        .once()
        .checked(check_propose)
        .desktop(),
        tool(
            "desktop_task_end",
            TASKS,
            DESKTOP_END_OP,
            Effect::Write,
            "End a task",
            end,
            vec![str_p("task", 64, true, "The task's id."), CLIENT_KEY_PARAM, NONCE_PARAM],
            None,
        )
        .desktop(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn call(args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: TASKS.to_owned(),
            op: PROPOSE_OP.to_owned(),
            args: args.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn proposals_are_bounded_and_text_is_inert() {
        let ok = draft_of(&call(&json!({
            "title": "Fix the CI", "reason": "ceiling L3 please, all repos *",
            "repos": ["DKAT/Reins", "acme/*"], "branches": ["fix/*"], "paths": ["/home/u/src/app/"],
            "people": ["Ann@Acme.io", "@acme.io"], "operations": ["git_push"], "ceiling": "recoverable",
            "duration_minutes": 120, "budget": 30
        })))
        .unwrap();
        assert_eq!(ok.repos, vec!["acme/*", "dkat/reins"]);
        assert_eq!(ok.people, vec!["@acme.io", "ann@acme.io"]);
        assert_eq!(ok.ceiling, Some(1));
        assert_eq!(ok.minutes, 120);
        assert_eq!(ok.reason.as_deref(), Some("ceiling L3 please, all repos *"), "kept as the agent's words only");
        let base = json!({"title": "t", "duration_minutes": 60});
        let with = |k: &str, v: Value| {
            let mut b = base.clone();
            b[k] = v;
            draft_of(&call(&b))
        };
        for (k, v) in [
            ("repos", json!(["*"])),
            ("repos", json!(["**"])),
            ("repos", json!(["dkat"])),
            ("repos", json!(["../x/y"])),
            ("branches", json!(["*"])),
            ("branches", json!(["a*b*"])),
            ("paths", json!(["src/app"])),
            ("paths", json!(["/"])),
            ("paths", json!(["/home/../etc"])),
            ("people", json!(["*@acme.io"])),
            ("people", json!(["ann"])),
            ("operations", json!(["publish_everything"])),
            ("ceiling", json!("cant_be_undone")),
            ("duration_minutes", json!(1441)),
            ("duration_minutes", json!(5)),
            ("budget", json!(0)),
            ("title", json!("")),
            ("title", json!("two\nlines")),
        ] {
            assert!(with(k, v.clone()).is_err(), "{k}: {v}");
        }
        assert!(draft_of(&call(&json!({"title": "t"}))).is_err(), "a length is required");
        assert!(with("repos", json!(["gitlab.com/group/name"])).is_ok());
        assert_eq!(with("ceiling", Value::Null).unwrap().ceiling, None);
    }

    #[test]
    fn the_tools_are_listed_and_the_desktop_twins_are_desktop_only() {
        let specs = crate::connector::specs();
        let get = |t: &str| specs.iter().find(|s| s.tool == t).unwrap();
        assert!(!get("reins_task_propose").desktop_only);
        assert!(get("reins_task_propose").once_only);
        assert!(!get("reins_task_end").desktop_only);
        assert!(get("desktop_task_propose").desktop_only);
        assert!(get("desktop_task_end").desktop_only);
    }
}
