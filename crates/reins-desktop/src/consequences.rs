//! The consequence engine on this computer (Decisions v2, `docs/design/decisions-v2/08-review-resolutions.md` R1):
//! it reads requests where the facts are (the working directory, the environment, the git worktrees) and attaches what
//! a request does to its Activity entry. In P1 it decides nothing: the guard still says what is asked.

use std::collections::BTreeMap;
use std::path::Path;

use reins_policy::effects::{self, Context, Summary, Workspace};

use crate::config::Config;
use crate::hooks::{Action, GitHead};

/// Environment variables the engine reads from the agent's environment: where things are, never a secret.
const ENV: &[&str] = &[
    "PATH",
    "TMPDIR",
    "USER",
    "LOGNAME",
    "SHELL",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "RUSTUP_HOME",
    "GOPATH",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "JAVA_HOME",
    "WAYLAND_DISPLAY",
    "DISPLAY",
];

/// The worktree `cwd` is in (the folder holding `.git`), found from its files.
///
/// Only a repository inside one of `roots` counts: a `.git` an agent plants in a folder above (`~/.git`) must not make
/// that folder a worktree whose deletes git brings back.
#[must_use]
pub fn worktree_of(cwd: &Path, roots: &[String], home: &str) -> Option<String> {
    let top = cwd.ancestors().find(|d| d.join(".git").exists())?;
    let top_text = top.to_string_lossy().into_owned();
    roots
        .iter()
        .map(|r| effects::normalize(r, "/", home))
        .any(|r| top_text.starts_with(&format!("{}/", r.trim_end_matches('/'))))
        .then_some(top_text)
}

/// The workspace for a request run in `cwd`: the configured roots and scratch folders, and the worktree `cwd` is in
/// (with the agents' worktrees below it).
#[must_use]
pub fn workspace(config: &Config, cwd: Option<&Path>) -> Workspace {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let mut worktrees = Vec::new();
    if let Some(top) = cwd.and_then(|c| worktree_of(c, &config.workspace.roots, &home)) {
        worktrees.push(format!("{top}/.claude/worktrees/*"));
        worktrees.push(top);
    }
    Workspace {
        home,
        roots: config.workspace.roots.clone(),
        scratch: config.workspace.scratch.clone(),
        worktrees,
        tracked: Vec::new(),
    }
}

/// The engine's context for a request run in `cwd`, with `env` (the agent's environment) and what the hook read of the
/// repository.
#[must_use]
pub fn context(
    config: &Config,
    cwd: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
    head: Option<&GitHead>,
) -> Context {
    let ws = workspace(config, cwd.map(Path::new));
    let mut c = Context::new(cwd.unwrap_or_default(), ws);
    let mut vars = BTreeMap::new();
    for name in ENV {
        if let Some(v) = env(name).filter(|v| !v.is_empty()) {
            vars.insert((*name).to_owned(), v);
        }
    }
    c.env.extend(vars);
    if let Some(h) = head {
        c.branch.clone_from(&h.branch);
        c.default_branch.clone_from(&h.default);
    }
    c
}

/// How long the engine may take for one request before its reading is dropped.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_millis(200);
/// The engine's thread stack: its recursion is bounded well below this (`reins-policy`'s totality tests run it at a
/// quarter of it).
const STACK: usize = 64 << 20;

/// Runs `f` on its own thread with a large stack, catching a panic, for at most [`DEADLINE`]. `None` on a panic, a
/// missed deadline or a thread that could not start: the reading is dropped, nothing else changes.
pub fn guarded<F>(f: F) -> Option<Summary>
where
    F: FnOnce() -> Option<Summary> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("reins-effects".to_owned()).stack_size(STACK).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok().flatten();
        let _sent = tx.send(result);
    });
    if spawned.is_err() {
        return None;
    }
    rx.recv_timeout(DEADLINE).ok().flatten()
}

/// [`of_action`] under [`guarded`].
#[must_use]
pub fn guarded_action(config: &Config, action: &Action, cwd: Option<&str>, head: Option<&GitHead>) -> Option<Summary> {
    let (config, action, cwd, head) = (config.clone(), action.clone(), cwd.map(str::to_owned), head.cloned());
    guarded(move || of_action(&config, &action, cwd.as_deref(), head.as_ref()))
}

/// What a hook's action does.
#[must_use]
pub fn of_action(config: &Config, action: &Action, cwd: Option<&str>, head: Option<&GitHead>) -> Option<Summary> {
    let env = |k: &str| std::env::var(k).ok();
    let ctx = context(config, cwd, &env, head);
    match action {
        Action::Command(c) => Some(effects::analyze_command(c, &ctx).summary()),
        Action::Files {
            write,
            paths,
        } => {
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            Some(effects::analyze_files(&paths, *write, &ctx).summary())
        }
        Action::Other => None,
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    /// A panic or a hang in the engine drops the reading and nothing else.
    #[test]
    fn a_panic_or_a_hang_drops_the_reading() {
        assert_eq!(guarded(|| panic!("the engine broke")), None);
        let started = std::time::Instant::now();
        assert_eq!(
            guarded(|| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                None
            }),
            None
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}

// POSIX paths: the engine reads POSIX shells (Windows commands are not modelled yet, and are asked).
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_command_in_a_worktree_is_read_with_its_facts() {
        let root = std::env::temp_dir().join(format!("reins-consequences-{}", std::process::id()));
        let dir = root.join("app");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let mut config = Config::default();
        config.workspace.roots = vec![root.to_string_lossy().into_owned()];
        let cwd = dir.to_string_lossy().into_owned();
        let env = |k: &str| (k == "TMPDIR").then(|| "/tmp".to_owned());
        let ctx = context(&config, Some(&cwd), &env, None);
        assert_eq!(ctx.workspace.worktrees.last(), Some(&cwd));
        assert_eq!(ctx.env.get("TMPDIR").map(String::as_str), Some("/tmp"));
        let s = of_action(&config, &Action::Command("rm -rf target/debug".to_owned()), Some(&cwd), None).unwrap();
        assert_eq!(s.level, effects::Level::L0, "{s:?}");
        let s = of_action(&config, &Action::Command("rm -rf src".to_owned()), Some(&cwd), None).unwrap();
        assert!(s.level <= effects::Level::L1, "inside the worktree: recoverable from git: {s:?}");
        assert!(of_action(&config, &Action::Other, Some(&cwd), None).is_none());
        assert!(guarded_action(&config, &Action::Command("rm -rf target".to_owned()), Some(&cwd), None).is_some());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_planted_git_folder_outside_the_roots_is_no_worktree() {
        let root = std::env::temp_dir().join(format!("reins-planted-{}", std::process::id()));
        let inside = root.join("Documents");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&inside).unwrap();
        let mut config = Config::default();
        config.workspace.roots = vec![root.join("dev").to_string_lossy().into_owned()];
        assert_eq!(worktree_of(&inside, &config.workspace.roots, ""), None);
        let ctx = context(&config, Some(&inside.to_string_lossy()), &|_| None, None);
        assert!(ctx.workspace.worktrees.is_empty(), "{:?}", ctx.workspace.worktrees);
        std::fs::remove_dir_all(&root).ok();
    }
}
