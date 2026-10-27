//! Nets in a harness hook: planned when a request is asked, applied only when it is allowed.
//!
//! - A shell command is rewritten (`reins_policy::effects::nets::plan_command`). The question carries the net and the
//!   netted command, so the approval is for exactly that text, and the harness gets it back as the tool's new input
//!   with the `allow` (Claude Code and Codex `updatedInput`, Gemini CLI `tool_input`). Cursor's shell hook can't
//!   change a command, so its nets are advisory: nothing is rewritten, and nothing is lowered.
//! - A file tool (Write, Edit, apply_patch, Delete) gets snapshots, taken right before the hook answers `allow`.
//!
//! The plan is made under a deadline on its own thread, like the effects reading: when it is late, panics or fails,
//! the request simply has no net.

use std::path::Path;

use reins_policy::effects::nets::{self, NetPlan};
use sha2::{Digest as _, Sha256};

use super::{Facts, PlanRecord, Store};
use crate::config::Config;
use crate::harness::Harness;
use crate::hooks::{Action, Event, GitHead, HookCall};
use crate::journal::NetRecord;

/// A net planned for one hook call.
#[derive(Clone, Debug)]
pub struct HookNet {
    pub id: String,
    pub plan: NetPlan,
    /// The netted command (shell), or `None` (file snapshots).
    pub rewritten: Option<String>,
    /// The files to copy before a file tool runs.
    pub snapshots: Vec<String>,
}

/// Whether the harness's hook can hand back a changed command.
#[must_use]
pub fn rewrites(harness: Harness, event: Event) -> bool {
    matches!(
        (harness, event),
        (Harness::ClaudeCode | Harness::Codex, Event::PreToolUse) | (Harness::Gemini, Event::GeminiBeforeTool)
    )
}

/// The failure key of a command run in a folder: the same text in the same folder.
#[must_use]
pub fn failure_key(cwd: &str, command: &str) -> String {
    let mut h = Sha256::new();
    h.update(cwd.as_bytes());
    h.update([0]);
    h.update(command.as_bytes());
    data_encoding::HEXLOWER.encode(&h.finalize())
}

/// Runs `f` on its own thread with a large stack for at most `deadline`; `None` on a panic or when late.
pub fn within_deadline<T: Send + 'static>(
    deadline: std::time::Duration,
    f: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("reins-nets".to_owned()).stack_size(64 << 20).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok().flatten();
        let _sent = tx.send(result);
    });
    if spawned.is_err() {
        return None;
    }
    rx.recv_timeout(deadline).ok().flatten()
}

/// How long planning may take (it reads the command a few times, and checks paths on disk).
pub const DEADLINE: std::time::Duration = std::time::Duration::from_millis(400);

/// Plans the net for `call` (a command or a file change), and records the plan for `reins net run`. `None`: no net.
#[must_use]
pub fn plan(config: &Config, paths: &crate::config::Paths, call: &HookCall, head: Option<&GitHead>) -> Option<HookNet> {
    if !config.nets.enabled || !cfg!(unix) {
        return None;
    }
    let store = Store::open(paths);
    let cwd = call.cwd.clone().unwrap_or_default();
    let reins = std::env::current_exe().ok()?.to_str()?.to_owned();
    let root = store.ensure_root().ok()?.to_str()?.to_owned();
    let max_snapshot = config.nets.snapshot_max_bytes;
    let (config_for, action, head_owned) = (config.clone(), call.action.clone(), head.cloned());
    let rewrites = rewrites(call.harness, call.event);
    let store_for = store.clone();
    let cwd_for = cwd.clone();
    let net = within_deadline(DEADLINE, move || {
        let env = |k: &str| std::env::var(k).ok();
        let ctx = crate::consequences::context(&config_for, Some(&cwd_for), &env, head_owned.as_ref());
        let facts = Facts {
            store: &store_for,
            max_snapshot,
        };
        match &action {
            Action::Command(c) if rewrites => {
                if store_for.failed_recently(&failure_key(&cwd_for, c), crate::now_unix()) {
                    return None;
                }
                let id = format!("n{}", crate::server::random_token(9));
                let runner = nets::Runner {
                    reins: &reins,
                    plan: &id,
                    store: &root,
                };
                let plan = nets::plan_command(c, &ctx, &facts, runner)?;
                Some(HookNet {
                    rewritten: Some(plan.rewritten.clone()),
                    snapshots: Vec::new(),
                    id,
                    plan,
                })
            }
            Action::Files {
                write: true,
                paths,
            } => {
                let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                let plan = nets::plan_files(&refs, &ctx, &facts)?;
                Some(HookNet {
                    id: format!("n{}", crate::server::random_token(9)),
                    rewritten: None,
                    snapshots: plan.netted_paths.clone(),
                    plan,
                })
            }
            _ => None,
        }
    })?;
    if !nets::valid_plan_id(&net.id) {
        return None;
    }
    let command = match &call.action {
        Action::Command(c) => c.clone(),
        _ => String::new(),
    };
    // What `reins net run` goes by: the programs and folders the plan allows, and the scratch roots it deletes for
    // real, as this hook read them (never the environment the command runs in).
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let scratch: Vec<String> =
        config.workspace.scratch.iter().map(|s| reins_policy::effects::normalize(s, "/", &home)).collect();
    store
        .begin(&PlanRecord {
            id: net.id.clone(),
            at: crate::now_unix(),
            key: failure_key(&cwd, &command),
            command,
            cwd,
            lines: net.plan.lines(),
            entry: None,
            programs: net.plan.programs.clone(),
            cwds: net.plan.cwds.clone(),
            home,
            scratch,
            git: find_program("git").unwrap_or_default(),
            env: std::env::vars()
                .filter(|(k, _)| k.starts_with("GIT_") || k.starts_with("XDG_") || k == "HOME")
                .collect(),
        })
        .ok()?;
    Some(net)
}

/// `program` as `PATH` (the hook's, the harness's) finds it, absolute.
fn find_program(program: &str) -> Option<String> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .filter(|d| d.is_absolute())
        .map(|d| d.join(program))
        .find(|p| p.is_file())
        .and_then(|p| p.to_str().map(str::to_owned))
}

/// The question's `net` argument: one line per net.
#[must_use]
pub fn question_net(net: &HookNet) -> String {
    net.plan.lines().iter().map(|l| l.replace('\n', " ")).collect::<Vec<_>>().join("\n")
}

/// Takes the snapshots a file tool's net needs, right before it runs. `Err`: something could not be kept (the request
/// then runs without its net, and Activity says so).
pub fn apply_files(config: &Config, paths: &crate::config::Paths, net: &HookNet) -> Result<(), String> {
    let store = Store::open(paths);
    for p in &net.snapshots {
        store.snapshot(&net.id, Path::new(p), config.nets.snapshot_max_bytes).map_err(|e| format!("{p}: {e}"))?;
    }
    Ok(())
}

/// What Activity keeps of an applied net.
#[must_use]
pub fn record(net: &HookNet) -> NetRecord {
    NetRecord {
        plan: net.id.clone(),
        kinds: net.plan.kinds(),
        lines: net.plan.lines(),
        until: crate::now_unix() + super::store::TRASH_DAYS * 86_400,
        ran: false,
        kept: false,
    }
}
