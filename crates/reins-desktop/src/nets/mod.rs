//! Safety nets on this computer (`docs/design/decisions-v2/01-decision-engine.md` §5, 08 R2): the Reins trash, git
//! backup refs and file snapshots, and `reins net run`, which a netted command calls in place of `rm` or `git`.
//!
//! The planner is `reins_policy::effects::nets` (pure). This module is what really happens: [`Store`] keeps what a net
//! saved, [`rm()`](rm::rm) and [`git()`](git::git) run the command with its net, [`restore`] brings a plan's
//! saved things back, and [`Store::prune`] drops them after their keeping time.

pub mod git;
pub mod hook;
pub mod rm;
pub mod store;

use std::ffi::OsString;
use std::path::Path;

pub use store::{Fault, Item, ItemKind, NetError, NetSummary, PlanRecord, RefBackup, Store, WorktreeInfo};

use crate::config::Config;

/// The planner's facts from this computer: whether a path can go into this store's trash.
pub struct Facts<'a>(pub &'a Store);

impl reins_policy::effects::nets::NetFacts for Facts<'_> {
    fn trashable(&self, abs: &str) -> bool {
        self.0.trashable(Path::new(abs))
    }
}

/// Whether `path` is scratch: deleting it loses nothing (the OS temp folders, `~/.cache`, `[workspace] scratch`).
#[must_use]
pub fn is_scratch(config: &Config, path: &Path) -> bool {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let ws = reins_policy::effects::Workspace {
        home: home.clone(),
        scratch: config.workspace.scratch.clone(),
        ..reins_policy::effects::Workspace::default()
    };
    let p = path.to_string_lossy();
    let abs = reins_policy::effects::normalize(&p, "/", &home);
    ws.area(&abs) == reins_policy::effects::Area::Scratch
}

/// Brings back everything `plan` kept: backup refs first (a worktree may need its branch), then trashed paths and
/// snapshots, newest first. One line per thing; the plan is marked restored when everything came back.
pub fn restore(store: &Store, plan: &str) -> Result<Vec<String>, String> {
    if store.restored(plan) {
        return Err("Already restored".to_owned());
    }
    let refs = store.refs(plan);
    let items = store.items(plan);
    if refs.is_empty() && items.is_empty() {
        return Err("Nothing is kept for it any more".to_owned());
    }
    let mut lines = Vec::new();
    let mut failed = Vec::new();
    for r in &refs {
        match git::restore_ref(plan, r) {
            Ok(l) => lines.push(l),
            Err(e) => failed.push(format!("{}: {e}", r.name)),
        }
    }
    for i in &items {
        let done = match &i.worktree {
            Some(wt) => git::restore_worktree(store, plan, i, wt),
            None => store.restore_item(plan, i),
        };
        match done {
            Ok(l) => lines.push(l),
            Err(e) => failed.push(e),
        }
    }
    if failed.is_empty() {
        store.mark_restored(plan)?;
        Ok(lines)
    } else {
        lines.extend(failed.iter().map(|f| format!("Not restored: {f}")));
        Err(lines.join("\n"))
    }
}

/// Deletes a backup ref from its repository (pruning).
#[must_use]
pub fn drop_ref(r: &RefBackup) -> bool {
    std::process::Command::new("git")
        .args(["--git-dir", &r.repo, "update-ref", "-d", &r.backup])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
        // A repository that is gone took its ref with it.
        || !Path::new(&r.repo).exists()
}

/// Prunes at most once an hour (a stamp file in the store).
pub fn prune_now_and_then(store: &Store) {
    let stamp = store.root().join(".pruned");
    let now = crate::now_unix();
    let last = std::fs::read_to_string(&stamp).ok().and_then(|t| t.trim().parse::<i64>().ok()).unwrap_or(0);
    if now - last < 3_600 {
        return;
    }
    if std::fs::create_dir_all(store.root()).is_ok() && std::fs::write(&stamp, now.to_string()).is_ok() {
        store.prune(now, &drop_ref);
    }
}

/// `reins net run --plan PLAN -- PROGRAM ARGS…`: runs `rm` or `git` with the net of `plan`. Its exit code.
#[must_use]
pub fn run(config: &Config, plan: &str, argv: &[OsString]) -> u8 {
    let Ok(store) = Store::open_default() else {
        eprintln!("reins: no place for the Reins trash; nothing was run.");
        return git::REFUSED;
    };
    if !reins_policy::effects::nets::valid_plan_id(plan) {
        eprintln!("reins: {plan:?} is not a plan; nothing was run.");
        return git::REFUSED;
    }
    let Some((program, rest)) = argv.split_first() else {
        eprintln!("reins: net run needs a program; nothing was run.");
        return git::REFUSED;
    };
    let Ok(cwd) = std::env::current_dir() else {
        eprintln!("reins: no working folder; nothing was run.");
        return git::REFUSED;
    };
    let code = match program.to_str() {
        Some("rm") => {
            let scratch = |p: &Path| is_scratch(config, p);
            rm::rm(&store, plan, rest, &cwd, &rm::Scratch(&scratch), &mut std::io::stdout(), &mut std::io::stderr())
        }
        Some("git") => git::git(&store, plan, rest, &cwd, &mut std::io::stderr()),
        _ => {
            eprintln!("reins: net run knows rm and git only; nothing was run.");
            git::REFUSED
        }
    };
    prune_now_and_then(&store);
    code
}

#[cfg(all(test, unix))]
mod tests;
