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

/// The planner's facts from this computer: whether a path can go into this store's trash, is a linked worktree, or is a
/// file a snapshot of at most `max_snapshot` bytes can copy.
pub struct Facts<'a> {
    pub store: &'a Store,
    pub max_snapshot: u64,
}

impl reins_policy::effects::nets::NetFacts for Facts<'_> {
    fn trashable(&self, abs: &str) -> bool {
        self.store.trashable(Path::new(abs))
    }

    fn is_worktree(&self, abs: &str) -> bool {
        linked_worktree(Path::new(abs))
    }

    fn snapshottable(&self, abs: &str) -> bool {
        std::fs::metadata(abs).is_ok_and(|m| m.is_file() && m.len() <= self.max_snapshot)
    }
}

/// `dir` is a linked git worktree, where it really is: its `.git` is a file naming a `worktrees/<name>` folder of a
/// repository that exists.
#[must_use]
pub fn linked_worktree(dir: &Path) -> bool {
    if std::fs::canonicalize(dir).ok().as_deref() != Some(dir) {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(dir.join(".git")) else {
        return false;
    };
    text.lines().next().and_then(|l| l.strip_prefix("gitdir: ")).is_some_and(|g| {
        let g = dir.join(g.trim());
        g.parent().and_then(Path::file_name).is_some_and(|n| n == "worktrees") && g.is_dir()
    })
}

/// Whether `path` is scratch: deleting it loses nothing (the OS temp folders, `~/.cache`, the scratch roots), with the
/// home folder and scratch roots `plan` recorded when the hook planned it (never the environment the command runs in).
#[must_use]
pub fn is_scratch(plan: &PlanRecord, path: &Path) -> bool {
    let ws = reins_policy::effects::Workspace {
        home: plan.home.clone(),
        scratch: plan.scratch.clone(),
        ..reins_policy::effects::Workspace::default()
    };
    let p = path.to_string_lossy();
    let abs = reins_policy::effects::normalize(&p, "/", &plan.home);
    ws.area(&abs) == reins_policy::effects::Area::Scratch
}

/// Brings back everything `plan` kept: backup refs first (a worktree may need its branch), then trashed paths and
/// snapshots, newest first. One line per thing; the plan is marked restored when everything came back.
pub fn restore(store: &Store, plan: &str) -> Result<Vec<String>, String> {
    if store.restored(plan) {
        return Ok(vec!["Already restored".to_owned()]);
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

/// How long after its plan a netted command may still run (a plan serves every run of its words: a loop runs them
/// many times, so it is not used up by one).
pub const PLAN_SECS: i64 = 12 * 3_600;

/// Prunes at most once an hour (a stamp file in the store). The daemon calls it; a netted command never does.
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

/// Prunes the Reins trash now and then, as long as the daemon runs (on a blocking thread: it walks folders).
pub async fn prune_loop(paths: crate::config::Paths) {
    loop {
        let store = Store::open(&paths);
        let _done = tokio::task::spawn_blocking(move || prune_now_and_then(&store)).await;
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    }
}

/// Why `reins net run` refuses to run `program` in `cwd` for `plan`, if it does: no plan, an old one, another program,
/// or another folder than the hook planned.
#[must_use]
pub fn refusal(plan: Option<&PlanRecord>, program: &str, cwd: &Path, now: i64) -> Option<String> {
    let Some(p) = plan else {
        return Some("there is no such plan".to_owned());
    };
    if now - p.at > PLAN_SECS || p.at > now + 60 {
        return Some("the plan is too old".to_owned());
    }
    if !p.programs.iter().any(|x| x == program) {
        return Some(format!("the plan does not net {program}"));
    }
    let here = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let known = p.cwds.iter().any(|d| {
        let d = Path::new(d);
        d == cwd || std::fs::canonicalize(d).is_ok_and(|r| r == here)
    });
    if !known {
        return Some(format!("it runs in {}, not where the plan runs", cwd.display()));
    }
    None
}

/// `reins net run --plan PLAN --store STORE -- PROGRAM ARGS…`: runs `rm` or `git` with the net of `plan`, from the
/// store the hook wrote it to (named in the command, never taken from the environment). Its exit code.
#[must_use]
pub fn run(plan: &str, store: &Path, argv: &[OsString]) -> u8 {
    if !reins_policy::effects::nets::valid_plan_id(plan) || !store.is_absolute() {
        eprintln!("reins: {plan:?} is not a plan; nothing was run.");
        return git::REFUSED;
    }
    let store = Store::at(store);
    let Some((program, rest)) = argv.split_first() else {
        eprintln!("reins: net run needs a program; nothing was run.");
        return git::REFUSED;
    };
    let Ok(cwd) = std::env::current_dir() else {
        eprintln!("reins: no working folder; nothing was run.");
        return git::REFUSED;
    };
    let record = store.plan(plan);
    let program = program.to_str().unwrap_or_default();
    if let Some(why) = refusal(record.as_ref(), program, &cwd, crate::now_unix()) {
        eprintln!("reins: {why}, so nothing was run. Run the command again to be asked.");
        store.fail_plan(plan, &why);
        return git::REFUSED;
    }
    let record = record.unwrap_or_default();
    match program {
        "rm" => {
            let scratch = |p: &Path| is_scratch(&record, p);
            rm::rm(&store, plan, rest, &cwd, &rm::Scratch(&scratch), &mut std::io::stdout(), &mut std::io::stderr())
        }
        "git" => {
            git::use_env(git::GitEnv {
                program: record.git.clone(),
                vars: record.env,
            });
            git::git(&store, plan, rest, &cwd, &mut std::io::stderr())
        }
        _ => {
            eprintln!("reins: net run knows rm and git only; nothing was run.");
            git::REFUSED
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
