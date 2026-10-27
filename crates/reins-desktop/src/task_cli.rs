//! `reins task start|end` and `reins task` (Decisions v2, `docs/design/decisions-v2/01-decision-engine.md` §12): this
//! computer's agent connection proposes a task, an envelope inside which the phone decides more on its own (pushes to
//! the listed branches of the listed repositories, up to a ceiling, for a while). The phone's owner approves it, and
//! may narrow it; what comes back is the envelope as approved, kept here so `reins task` can show it and
//! `reins task end` can end it. Ending only takes away: the phone never asks.

use std::path::PathBuf;
use std::time::Duration;

use reins_proto::connector::tasks::{self as proto, CEILINGS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Config, Paths};
use crate::journal::{Entry, Kind};
use crate::work_session::{parse_duration, phone_link, span};

pub const PROPOSE_TOOL: &str = "desktop_task_propose";
pub const END_TOOL: &str = "desktop_task_end";

/// What `reins task start` asks for.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct TaskArgs {
    /// What the task is, in a few words.
    #[arg(long)]
    pub title: String,
    /// Repositories (`owner/name` or `owner/*`).
    #[arg(long = "repo")]
    pub repos: Vec<String>,
    /// Branches it may push to or delete (names, or prefixes ending in `*`).
    #[arg(long = "branch")]
    pub branches: Vec<String>,
    /// Absolute folders it may change files in.
    #[arg(long = "path")]
    pub paths: Vec<String>,
    /// People it may write to (none by default).
    #[arg(long = "person")]
    pub people: Vec<String>,
    /// The most it may do without asking: undoable, recoverable or reaches_people.
    #[arg(long)]
    pub ceiling: Option<String>,
    /// How long: 2h, 90m (15 minutes to 24 hours).
    #[arg(long = "for", default_value = "2h")]
    pub length: String,
    /// How many requests it may cover at most.
    #[arg(long)]
    pub budget: Option<u32>,
}

/// The task as the phone approved it, kept in the state folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTask {
    pub id: String,
    pub title: String,
    pub ends_at: i64,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub ceiling: String,
}

fn file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("task.json")
}

/// The arguments of the proposal, checked with the same bounds the phone and the server check (so a typo fails here,
/// before anything is asked).
pub fn propose_args(a: &TaskArgs) -> Result<Value, String> {
    let secs = parse_duration(&a.length)?;
    let minutes = i64::try_from(secs / 60).map_err(|_| "too long".to_owned())?;
    let mut args = json!({
        "title": a.title.trim(),
        "repos": a.repos,
        "branches": a.branches,
        "paths": a.paths,
        "people": a.people,
        "duration_minutes": minutes,
    });
    if let Some(c) = &a.ceiling {
        let c = c.trim().to_ascii_lowercase().replace(' ', "_");
        if !CEILINGS.contains(&c.as_str()) {
            return Err("--ceiling is undoable, recoverable or reaches_people".to_owned());
        }
        args["ceiling"] = json!(c);
    }
    if let Some(b) = a.budget {
        args["budget"] = json!(b);
    }
    let call = reins_proto::connector::ConnectorCall {
        service: reins_proto::connector::TASKS.to_owned(),
        op: proto::DESKTOP_PROPOSE_OP.to_owned(),
        args: args.as_object().cloned().unwrap_or_default(),
    };
    proto::draft_of(&call)?;
    Ok(args)
}

/// The task the phone's answer describes.
pub fn task_from(data: &Value) -> Result<LocalTask, String> {
    let text = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let list = |k: &str| {
        data.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(reins_proto::desktop::printable).collect())
            .unwrap_or_default()
    };
    let id = text("task");
    if !id.starts_with("tk_") || id.len() > 64 {
        return Err("the phone's answer names no task".to_owned());
    }
    Ok(LocalTask {
        id,
        title: reins_proto::desktop::printable(&text("title")),
        ends_at: data.get("ends_at").and_then(Value::as_i64).ok_or("the phone's answer has no end time")?,
        repos: list("repos"),
        branches: list("branches"),
        ceiling: text("ceiling"),
    })
}

/// The task running here, if it hasn't ended.
#[must_use]
pub fn current(paths: &Paths) -> Option<LocalTask> {
    let t: LocalTask = serde_json::from_slice(&std::fs::read(file(paths)).ok()?).ok()?;
    (t.ends_at > crate::now_unix()).then_some(t)
}

/// Proposes the task and waits for the owner (up to the approval timeout); on approval keeps it.
pub async fn start(paths: &Paths, config: &Config, a: &TaskArgs) -> Result<LocalTask, String> {
    let args = propose_args(a)?;
    let link = phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| r.message().to_owned())?;
    let entry = Entry::new(Kind::Ask, &format!("task: {}", a.title.trim())).source(Some("task"));
    let answer = crate::phone::awaiting(
        link.journal(),
        entry,
        None,
        link.timeout(),
        phone.ask(
            PROPOSE_TOOL,
            None,
            |_| args.clone(),
            "No answer from your phone in time. Approve it there, then try again.",
        ),
    )
    .await
    .map_err(|r| crate::work_session::refusal_text(&r))?;
    let task = task_from(&answer.data)?;
    let bytes = serde_json::to_vec_pretty(&task).map_err(|e| e.to_string())?;
    crate::config::write_private(&file(paths), &bytes).map_err(|e| e.to_string())?;
    Ok(task)
}

/// Ends the task running here now (never asked). `Ok(false)` when none runs.
pub async fn end(paths: &Paths, config: &Config) -> Result<bool, String> {
    let Some(task) = serde_json::from_slice::<LocalTask>(&std::fs::read(file(paths)).unwrap_or_default()).ok() else {
        return Ok(false);
    };
    let link = phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| r.message().to_owned())?;
    let id = task.id.clone();
    tokio::time::timeout(
        Duration::from_secs(60),
        phone.ask(END_TOOL, None, |_| json!({"task": id}), "The phone did not answer in time."),
    )
    .await
    .map_err(|_| "the phone did not answer in time; stop the task on the phone".to_owned())?
    .map_err(|r| crate::work_session::refusal_text(&r))?;
    std::fs::remove_file(file(paths)).ok();
    Ok(true)
}

/// One line for `reins task`.
#[must_use]
pub fn describe(t: &LocalTask, now: i64) -> String {
    use std::fmt::Write as _;
    let left = u64::try_from(t.ends_at - now).unwrap_or(0);
    let mut s = format!("Task: {} ({} left", t.title, span(left));
    if !t.ceiling.is_empty() {
        let _w = write!(s, ", up to {}", t.ceiling.replace('_', " "));
    }
    s.push(')');
    if !t.repos.is_empty() {
        let _w = write!(s, "\n  repos: {}", t.repos.join(", "));
    }
    if !t.branches.is_empty() {
        let _w = write!(s, "\n  branches: {}", t.branches.join(", "));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> TaskArgs {
        TaskArgs {
            title: "Fix the CI".to_owned(),
            repos: vec!["dkat/reins".to_owned()],
            branches: vec!["fix/*".to_owned()],
            length: "2h".to_owned(),
            ..TaskArgs::default()
        }
    }

    #[test]
    fn task_cli_builds_a_bounded_call() {
        let v = propose_args(&args()).unwrap();
        assert_eq!(v["duration_minutes"], 120);
        assert_eq!(v["repos"], json!(["dkat/reins"]));
        assert!(v.get("ceiling").is_none(), "the phone's default");
        let mut a = args();
        a.ceiling = Some("Reaches people".to_owned());
        assert_eq!(propose_args(&a).unwrap()["ceiling"], "reaches_people");
        for bad in [
            TaskArgs {
                ceiling: Some("cant_be_undone".to_owned()),
                ..args()
            },
            TaskArgs {
                repos: vec!["*".to_owned()],
                ..args()
            },
            TaskArgs {
                paths: vec!["src".to_owned()],
                ..args()
            },
            TaskArgs {
                length: "25h".to_owned(),
                ..args()
            },
            TaskArgs {
                title: String::new(),
                ..args()
            },
        ] {
            assert!(propose_args(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_answer_is_read_strictly() {
        let t = task_from(&json!({"task": "tk_0123456789abcdef", "title": "Fix\u{202e} CI", "ends_at": 2_000,
            "repos": ["dkat/reins"], "branches": ["fix/*"], "ceiling": "recoverable"}))
        .unwrap();
        assert_eq!(t.id, "tk_0123456789abcdef");
        assert!(!t.title.contains('\u{202e}'), "invisible characters never reach the terminal as they are");
        assert!(task_from(&json!({"task": "x", "ends_at": 1})).is_err());
        assert!(task_from(&json!({"task": "tk_1"})).is_err());
        let line = describe(&t, 2_000 - 3_600);
        assert!(line.starts_with("Task: ") && line.contains("left, up to recoverable)"), "{line}");
        assert!(line.contains("repos: dkat/reins") && line.contains("branches: fix/*"), "{line}");
    }
}
