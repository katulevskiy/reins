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
#[must_use]
pub fn worktree_of(cwd: &Path) -> Option<String> {
    cwd.ancestors().find(|d| d.join(".git").exists()).map(|d| d.to_string_lossy().into_owned())
}

/// The workspace for a request run in `cwd`: the configured roots and scratch folders, and the worktree `cwd` is in
/// (with the agents' worktrees below it).
#[must_use]
pub fn workspace(config: &Config, cwd: Option<&Path>) -> Workspace {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let mut worktrees = Vec::new();
    if let Some(top) = cwd.and_then(worktree_of) {
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
mod tests {
    use super::*;

    #[test]
    fn a_command_in_a_worktree_is_read_with_its_facts() {
        let dir = std::env::temp_dir().join(format!("reins-consequences-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let config = Config::default();
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
        std::fs::remove_dir_all(&dir).ok();
    }
}
