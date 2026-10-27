//! The consequence engine on this computer (Decisions v2, `docs/design/decisions-v2/08-review-resolutions.md` R1):
//! it reads requests where the facts are (the working directory, the environment, the git worktrees) and attaches what
//! a request does to its Activity entry. The matrix (`reins_policy::matrix`) evaluates it with this computer's own
//! facts (`[auto]`): in shadow unless `auto.matrix_decides` is on, when the guard still says what is asked.

use std::collections::BTreeMap;
use std::path::Path;

use reins_policy::context::{self as graph, FactKind, Zone};
use reins_policy::effects::{self, Analysis, Context, Level, Summary, Workspace};
use reins_policy::matrix::{self, Input, Outcome};
use serde::{Deserialize, Serialize};

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
/// Only a repository strictly inside one of `roots` counts, and the search for `.git` stops at that root: a `.git` an
/// agent plants in a folder above (`~/.git`) or in the root itself must not make it a worktree whose deletes git brings
/// back. The engine reads paths as written, so a `cwd` that is not its own resolved form (a symlink in it, `..`) has no
/// worktree: its paths could not be compared with the roots.
#[must_use]
pub fn worktree_of(cwd: &Path, roots: &[String], home: &str) -> Option<String> {
    if cwd.components().any(|c| c == std::path::Component::ParentDir) {
        return None;
    }
    let real = std::fs::canonicalize(cwd).ok()?;
    if real != cwd {
        return None;
    }
    let roots: Vec<std::path::PathBuf> =
        roots.iter().filter_map(|r| std::fs::canonicalize(effects::normalize(r, "/", home)).ok()).collect();
    let root = roots.iter().filter(|r| real.starts_with(r) && real != **r).max_by_key(|r| r.components().count())?;
    real.ancestors()
        .take_while(|d| *d != root.as_path())
        .find(|d| d.join(".git").exists())
        .map(|d| d.to_string_lossy().into_owned())
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

/// What the matrix said of a request on this computer, as kept on its Activity entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shadow {
    pub outcome: Outcome,
    pub zone: Zone,
    pub level: Level,
    /// "Auto · Mine · Undoable".
    pub line: String,
    /// The hash of the snapshot it was evaluated against.
    pub snapshot: String,
    /// It decided (`auto.matrix_decides`); else it was only recorded.
    pub decided: bool,
}

/// `remote.origin.url` of the repository holding `cwd`, read by the hook's strict reader of git's configuration
/// ([`crate::hooks::git_head`]: never by running git; `None` when any configuration has a `pushurl`, an `insteadOf`,
/// an include or anything it doesn't read cleanly). Then `origin` is not known, and what goes to it is Outside.
#[must_use]
pub fn origin_url(cwd: &Path) -> Option<String> {
    origin_of(crate::hooks::git_head(cwd).as_ref())
}

fn origin_of(head: Option<&GitHead>) -> Option<String> {
    head?.remotes.get("origin").cloned().filter(|u| graph::parse_repo(u, None).is_some())
}

/// The matrix's evaluation of `a`, a request run in `cwd` (`command`: its text, when a command), against this
/// computer's `[auto]`. The base repository and the remotes come from the worktree's configuration, unless the
/// command may act on another repository (`git -C`, a `cd` out of it, `GIT_DIR`…).
#[must_use]
pub fn shadow(
    config: &Config,
    a: &Analysis,
    cwd: Option<&str>,
    command: Option<&str>,
    connection: &str,
    now: i64,
) -> Shadow {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let at = match (cwd, command) {
        (Some(c), Some(cmd)) => graph::repo_at(cmd, c, git_top(c).as_deref(), &home),
        _ => graph::RepoAt::Cwd,
    };
    // The repository the command acts in: the worktree's, a leading `cd`'s target, or none.
    let dir = match at {
        graph::RepoAt::Cwd => cwd.map(str::to_owned),
        graph::RepoAt::Dir(d) => Some(d),
        graph::RepoAt::Unknown => None,
    };
    // The agent's environment can point git elsewhere, or set its configuration: then nothing is known.
    let env_redirects = std::env::vars_os().any(|(k, _)| {
        let k = k.to_string_lossy();
        k == "GIT_DIR" || k == "GIT_WORK_TREE" || k.starts_with("GIT_CONFIG")
    });
    let head = dir.filter(|_| !env_redirects).and_then(|d| {
        let head = crate::hooks::git_head(Path::new(&d))?;
        // `origin` as the strict reader read it must be where git itself would push (its push URL, after
        // `pushurl` and `insteadOf`); any difference or error and it isn't known.
        let read = head.remotes.get("origin");
        (read.is_none() || read == git_push_url(&d).as_ref()).then_some(head)
    });
    shadow_with(config, a, head.as_ref(), connection, now)
}

/// `git remote get-url --push origin` in `dir`: where git would push `origin` (no hooks or programs run for it).
fn git_push_url(dir: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-c", "core.fsmonitor=false", "-C", dir, "remote", "get-url", "--push", "origin"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim();
    (out.status.success() && !url.is_empty() && !url.contains('\n')).then(|| url.to_owned())
}

/// The folder holding the repository `cwd` is in (as `git_head` finds it).
fn git_top(cwd: &str) -> Option<String> {
    Path::new(cwd).ancestors().find(|d| d.join(".git").exists()).map(|d| d.to_string_lossy().into_owned())
}

/// [`shadow`] with the configuration of the repository the request acts in (`head`; `None`: not known).
pub(crate) fn shadow_with(config: &Config, a: &Analysis, head: Option<&GitHead>, connection: &str, now: i64) -> Shadow {
    let snap = config.auto.snapshot();
    let remotes = head.map(|h| &h.remotes);
    let base = graph::Base {
        repo: remotes.and_then(|r| r.get("origin")).map(String::as_str),
        remotes,
        bare_host: None,
    };
    let resolved = graph::request_zone(a, &snap.facts, &base);
    let e = matrix::evaluate(
        &Input {
            analysis: a,
            resolved: &resolved,
            connection,
            seen_resource: false,
            now,
        },
        &snap,
    );
    Shadow {
        outcome: e.outcome,
        zone: e.zone,
        level: e.level,
        line: e.line(),
        snapshot: e.snapshot,
        decided: config.auto.matrix_decides && config.auto.setup_done,
    }
}

/// GitHub owners (and other hosts' owners) this computer's projects push to (the `origin` of each repository directly
/// under a workspace root, and of the root itself), for the setup to propose: the owner sets their zones.
#[must_use]
pub fn proposals(config: &Config) -> Vec<(FactKind, String)> {
    let home = crate::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let mut owners = std::collections::BTreeSet::new();
    for root in &config.workspace.roots {
        let root = effects::normalize(root, &home, &home);
        let root = Path::new(&root);
        let mut dirs = vec![root.to_path_buf()];
        if let Ok(rd) = std::fs::read_dir(root) {
            dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.join(".git").exists()).take(500));
        }
        for d in dirs {
            if let Some(r) = origin_url(&d).and_then(|u| graph::parse_repo(&u, None)) {
                owners.insert(r.owner_key());
            }
        }
    }
    owners
        .into_iter()
        .filter(|o| config.auto.facts.get(FactKind::Owner, o).is_none())
        .map(|o| (FactKind::Owner, o))
        .collect()
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

/// What a hook's action does and what the matrix says of it, under [`guarded`] (`None`: no reading in time).
#[must_use]
pub fn evaluate_action(
    config: &Config,
    action: &Action,
    cwd: Option<&str>,
    head: Option<&GitHead>,
    connection: &str,
) -> Option<(Summary, Shadow)> {
    let (config, action, cwd, head, connection) =
        (config.clone(), action.clone(), cwd.map(str::to_owned), head.cloned(), connection.to_owned());
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("reins-matrix".to_owned()).stack_size(STACK).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let a = analysis_of(&config, &action, cwd.as_deref(), head.as_ref())?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
            let command = match &action {
                Action::Command(c) => Some(c.as_str()),
                _ => None,
            };
            let sh = shadow(&config, &a, cwd.as_deref(), command, &connection, now);
            Some((a.summary(), sh))
        }))
        .ok()
        .flatten();
        let _sent = tx.send(result);
    });
    if spawned.is_err() {
        return None;
    }
    rx.recv_timeout(DEADLINE).ok().flatten()
}

/// What the matrix says of a request with its safety net applied (`after`: the plan's analysis with the nets'
/// levels, P3), under the same deadline as [`evaluate_action`]. `None`: no reading in time.
#[must_use]
pub fn evaluate_netted(
    config: &Config,
    call: &crate::hooks::HookCall,
    after: &Analysis,
    connection: &str,
) -> Option<Shadow> {
    let (config, after, cwd, connection) = (config.clone(), after.clone(), call.cwd.clone(), connection.to_owned());
    let command = match &call.action {
        Action::Command(c) => Some(c.clone()),
        _ => None,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("reins-matrix".to_owned()).stack_size(STACK).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
            shadow(&config, &after, cwd.as_deref(), command.as_deref(), &connection, now)
        }))
        .ok();
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
    analysis_of(config, action, cwd, head).map(|a| a.summary())
}

/// The engine's analysis of a hook's action.
#[must_use]
pub fn analysis_of(config: &Config, action: &Action, cwd: Option<&str>, head: Option<&GitHead>) -> Option<Analysis> {
    let env = |k: &str| std::env::var(k).ok();
    let ctx = context(config, cwd, &env, head);
    match action {
        Action::Command(c) => Some(effects::analyze_command(c, &ctx)),
        Action::Files {
            write,
            paths,
        } => {
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            Some(effects::analyze_files(&paths, *write, &ctx))
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
        std::fs::create_dir_all(root.join("app/.git")).unwrap();
        // Resolved, as a hook's cwd is (macOS's temp folder is behind a symlink).
        let root = std::fs::canonicalize(&root).unwrap();
        let dir = root.join("app");
        let mut config = Config::default();
        config.workspace.roots = vec![root.to_string_lossy().into_owned()];
        let cwd = dir.to_string_lossy().into_owned();
        let env = |k: &str| (k == "TMPDIR").then(|| "/tmp".to_owned());
        let ctx = context(&config, Some(&cwd), &env, None);
        assert_eq!(ctx.workspace.worktrees.last(), Some(&cwd));
        assert_eq!(ctx.env.get("TMPDIR").map(String::as_str), Some("/tmp"));
        let s = of_action(&config, &Action::Command("rm -rf target/debug".to_owned()), Some(&cwd), None).unwrap();
        assert_eq!(s.level, Level::L0, "{s:?}");
        let s = of_action(&config, &Action::Command("rm -rf src".to_owned()), Some(&cwd), None).unwrap();
        assert!(s.level <= Level::L1, "inside the worktree: recoverable from git: {s:?}");
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
        // A `.git` planted in a root itself, or a path climbing out with `..`: no worktree either.
        let dev = root.join("dev");
        std::fs::create_dir_all(dev.join(".git")).unwrap();
        std::fs::create_dir_all(dev.join("app")).unwrap();
        assert_eq!(worktree_of(&dev.join("app"), &config.workspace.roots, ""), None);
        std::fs::create_dir_all(dev.join("app/.git")).unwrap();
        assert_eq!(
            worktree_of(&dev.join("app/src/.."), &config.workspace.roots, ""),
            None,
            "a cwd with .. is not read"
        );
        let real_root = std::fs::canonicalize(&root).unwrap();
        config.workspace.roots = vec![real_root.join("dev").to_string_lossy().into_owned()];
        let app = real_root.join("dev/app");
        assert_eq!(worktree_of(&app, &config.workspace.roots, ""), Some(app.to_string_lossy().into_owned()));
        // Through a symlink the engine's paths would not be the worktree's: none.
        std::fs::create_dir_all(app.join("src/deep")).unwrap();
        let link = real_root.join("short");
        std::os::unix::fs::symlink(app.join("src/deep"), &link).unwrap();
        assert_eq!(worktree_of(&link, &config.workspace.roots, ""), None);
        let ctx = context(&config, Some(&inside.to_string_lossy()), &|_| None, None);
        assert!(ctx.workspace.worktrees.is_empty(), "{:?}", ctx.workspace.worktrees);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn origin_is_read_from_the_repositorys_files() {
        let dir = std::env::temp_dir().join(format!("reins-origin-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let origin = |config: &str| {
            std::fs::write(dir.join(".git/config"), config).unwrap();
            origin_of(crate::hooks::git_head_with(&dir.join("src"), &[]).as_ref())
        };
        assert_eq!(
            origin("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:dkat/app.git\n").as_deref(),
            Some("git@github.com:dkat/app.git")
        );
        // Anything that can send a push elsewhere, and nothing is known.
        for odd in [
            "[remote \"origin\"]\nurl = git@github.com:a/b.git\nurl = git@github.com:c/d.git\n",
            "[remote \"origin\"]\nurl = git@github.com:a/b.git\npushurl = https://evil.example/a/b\n",
            "[remote \"origin\"]\nurl = git@github.com:a/b.git\n[url \"https://evil.example/\"]\ninsteadOf = git@github.com:\n",
            "[include]\npath = elsewhere\n[remote \"origin\"]\nurl = git@github.com:a/b.git\n",
        ] {
            assert_eq!(origin(odd), None, "{odd}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn origin_must_be_where_git_pushes() {
        let dir = std::env::temp_dir().join(format!("reins-pushurl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git").args(["init", "-q"]).arg(&dir).status().is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        std::fs::write(dir.join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:dkat/app.git\n").unwrap();
        let d = dir.to_string_lossy().into_owned();
        assert_eq!(git_push_url(&d).as_deref(), Some("git@github.com:dkat/app.git"));
        let mut config = Config::default();
        config.auto.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        let env = |_: &str| None;
        let ctx = context(&config, Some(&d), &env, None);
        let a = effects::analyze_command("git push origin HEAD:refs/heads/feat", &ctx);
        let zone =
            || shadow(&config, &a, Some(&d), Some("git push origin HEAD:refs/heads/feat"), "Claude Code", 0).zone;
        // As long as the user's own git configuration doesn't rewrite it, it's the owner's.
        if git_push_url(&d).as_deref() == Some("git@github.com:dkat/app.git") && crate::hooks::git_head(&dir).is_some()
        {
            assert_eq!(zone(), Zone::Mine);
        }
        // A push URL elsewhere (the strict reader refuses it, and git would push there): Outside.
        std::fs::write(
            dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:dkat/app.git\n\tpushurl = https://evil.example/x/y\n",
        )
        .unwrap();
        assert_eq!(git_push_url(&d).as_deref(), Some("https://evil.example/x/y"));
        assert_eq!(zone(), Zone::Outside);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_command_that_leaves_the_worktree_has_no_origin() {
        let dir = std::env::temp_dir().join(format!("reins-redirect-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/config"), "[remote \"origin\"]\nurl = git@github.com:dkat/app.git\n").unwrap();
        let head = crate::hooks::git_head_with(&dir, &[]);
        let mut config = Config::default();
        config.auto.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        let cwd = dir.to_string_lossy().into_owned();
        let env = |_: &str| None;
        let ctx = context(&config, Some(&cwd), &env, None);
        let zone = |cmd: &str| {
            let a = effects::analyze_command(cmd, &ctx);
            let head = match graph::repo_at(cmd, &cwd, git_top(&cwd).as_deref(), "/home/me") {
                graph::RepoAt::Cwd => head.clone(),
                _ => None,
            };
            shadow_with(&config, &a, head.as_ref(), "Claude Code", 0).zone
        };
        assert_eq!(zone("git push origin HEAD:refs/heads/feat"), Zone::Mine);
        assert_eq!(zone("git -C /elsewhere push origin HEAD:refs/heads/feat"), Zone::Outside);
        assert_eq!(zone("cd /elsewhere && git push origin HEAD:refs/heads/feat"), Zone::Outside);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn desktop_facts_are_its_own() {
        let mut config = Config::default();
        config.workspace.roots = vec!["/home/me".to_owned()];
        config.auto.facts.set(FactKind::Owner, "dkat", Zone::Mine);
        let env = |_: &str| None;
        let ctx = context(&config, Some("/home/me/app"), &env, None);
        let a = effects::analyze_command("git push origin HEAD:refs/heads/feat https://github.com/dkat/app", &ctx);
        let sh = shadow(&config, &a, Some("/home/me/app"), None, "Claude Code", 0);
        assert_eq!(sh.snapshot, config.auto.snapshot().hash(), "evaluated against this computer's [auto] only");
        let mut other = config.clone();
        other.auto.facts = reins_policy::context::Facts::default();
        assert_ne!(shadow(&other, &a, Some("/home/me/app"), None, "Claude Code", 0).snapshot, sh.snapshot);
        assert!(!sh.decided);
    }
}
