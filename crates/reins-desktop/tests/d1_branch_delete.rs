//! Decisions v2, 08 R1: a local branch delete decided on this computer.
//!
//! The hook reads what the repository says of its default branch for every command that names git (it used to read it
//! only for a push), so `git branch -D feature/x` is read as what it is: one local ref, recoverable, the owner's. With
//! no default recorded, a local branch is read by its name. The long-lived names, the default the hook read and a name
//! that is not in the command are still asked about.
#![cfg(unix)]

mod d1_mock;

use std::path::Path;

use d1_mock::{Mock, SigningPhone, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::{Config, Paths};
use reins_desktop::harness::Harness;
use reins_desktop::{hooks, journal};
use reins_policy::context::Zone;
use reins_policy::desktop_policy::PolicyMessage;
use reins_policy::effects::{Certainty, Level};
use reins_policy::matrix::Outcome;

/// A repository under `root`, written as files (git never runs): on `feature/x`, its origin the owner's. `default` is
/// the branch `refs/remotes/origin/HEAD` names; `None`: the repository records none.
fn repo(root: &Path, name: &str, default: Option<&str>) -> String {
    let dir = root.join(name);
    let git = dir.join(".git");
    std::fs::create_dir_all(git.join("refs/remotes/origin")).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
    std::fs::write(git.join("config"), "[remote \"origin\"]\n\turl = git@github.com:dkat/app.git\n").unwrap();
    if let Some(d) = default {
        std::fs::write(git.join("refs/remotes/origin/HEAD"), format!("ref: refs/remotes/origin/{d}\n")).unwrap();
    }
    dir.to_str().unwrap().to_owned()
}

/// A computer whose phone signed the default matrix for the projects under a fresh folder.
struct Computer {
    mock: Mock,
    paths: Paths,
    config: Config,
    root: std::path::PathBuf,
    _dirs: (d1_mock::App, tempfile::TempDir),
}

async fn computer() -> Computer {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    // Resolved, as a hook's cwd is.
    let dir = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let roots = [root.to_str().unwrap()];
    let snapshot = SigningPhone::auto_snapshot(&app.identity, 1, &roots);
    let applied = SigningPhone::default().send(&app.paths, &app.identity, &PolicyMessage::Snapshot(Box::new(snapshot)));
    assert_eq!(applied.snapshots, 1, "{applied:?}");
    let mut config = Config::default();
    config.workspace.roots = vec![roots[0].to_owned()];
    Computer {
        mock,
        paths: app.paths.clone(),
        config,
        root,
        _dirs: (app, dir),
    }
}

impl Computer {
    /// Runs the hook for `command` in `cwd`: the decision, and whether the phone was asked.
    async fn run(&self, cwd: &str, command: &str) -> (Option<String>, bool) {
        let input = serde_json::json!({"session_id": "abc", "cwd": cwd, "hook_event_name": "PreToolUse",
            "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
        .to_string();
        let before = self.mock.with(|s| s.calls.len());
        let (out, code, _) =
            hooks::run(Harness::ClaudeCode, input.as_bytes(), &self.paths, &self.config, &NoPrompter).await;
        assert_eq!(code, 0);
        let decision = out.map(|o| {
            let v: serde_json::Value = serde_json::from_str(&o).unwrap();
            v["hookSpecificOutput"]["permissionDecision"].as_str().unwrap_or_default().to_owned()
        });
        (decision, self.mock.with(|s| s.calls.len()) > before)
    }

    /// The newest Activity entry about `command`: its effects and what the matrix said.
    fn last(&self, command: &str) -> (reins_policy::effects::Summary, reins_desktop::consequences::Shadow) {
        // Newest first by the second, in the order written within one: the last of the newest second.
        let log = journal::read(&self.paths, 100, 3_600);
        let entry = log
            .iter()
            .filter(|e| e.what.ends_with(command))
            .max_by_key(|e| e.at)
            .unwrap_or_else(|| panic!("no entry about `{command}`: {log:?}"));
        (entry.effects.clone().expect("the effects are kept"), entry.matrix.clone().expect("the matrix is kept"))
    }

    /// The detail of the last question the phone got.
    fn asked(&self) -> String {
        let call = self.mock.with(|s| s.calls.last().cloned()).expect("a question");
        call["detail"].as_str().unwrap_or_default().to_owned()
    }
}

const ALLOWED: (Option<&str>, bool) = (Some("allow"), false);
const ASKED: (Option<&str>, bool) = (Some("allow"), true);

fn said(got: &(Option<String>, bool)) -> (Option<&str>, bool) {
    (got.0.as_deref(), got.1)
}

#[tokio::test]
async fn a_local_branch_delete_is_recoverable_and_mine() {
    let c = computer().await;
    for (name, default) in [("with-default", Some("main")), ("no-default", None)] {
        let cwd = repo(&c.root, name, default);
        // Decided here, without the phone (the guard's "Rewriting git history" is a label where the phone signed).
        let delete = format!("git branch -D feature/{name}");
        assert_eq!(said(&c.run(&cwd, &delete).await), ALLOWED, "{name}");
        let (effects, matrix) = c.last(&delete);
        assert_eq!((effects.level, effects.certainty), (Level::L1, Certainty::Full), "{name}: {effects:?}");
        assert!(effects.floor.is_empty() && effects.unresolved.is_empty(), "{name}: {effects:?}");
        assert!(matrix.decided && matrix.outcome == Outcome::Auto, "{name}: {matrix:?}");
        assert_eq!((matrix.zone, matrix.level), (Zone::Mine, Level::L1), "{name}: {matrix:?}");
        assert_eq!(matrix.line, "Auto · Mine · Recoverable", "{name}");
        // `-d` (git refuses a branch that is not merged) is never worse: nothing asks, the guard has no rule for it.
        assert_eq!(c.run(&cwd, &format!("git branch -d feature/{name}")).await, (None, false), "{name}");
        // Hostile: a long-lived name, alone or after another, and the remote's branch.
        for command in [
            format!("git branch -D feature/{name} main"),
            format!("git branch -D feature/{name} MASTER"),
            format!("git branch -D feature/{name}; git push origin :main"),
            format!("git branch -D feature/{name} && git push origin --delete main"),
        ] {
            let command = command.as_str();
            assert_eq!(said(&c.run(&cwd, command).await), ASKED, "{name}: {command}");
            let (effects, matrix) = c.last(command);
            assert_eq!(effects.level, Level::L3, "{name}: {command}: {effects:?}");
            assert!(matrix.outcome.reviews(), "{name}: {command}: {matrix:?}");
            // A question reports the repository's branch and default only for a push to origin, as before.
            let detail = c.asked();
            assert!(
                !detail.lines().any(|l| l.starts_with("Branch: ") || l.starts_with("Default branch: ")),
                "{name}: {command}: {detail}"
            );
        }
    }
    // With no default known, a name from elsewhere may be the default: not read.
    let cwd = repo(&c.root, "no-default-2", None);
    assert_eq!(said(&c.run(&cwd, "git branch -D \"$B\"").await), ASKED);
    let (effects, _) = c.last("git branch -D \"$B\"");
    assert_eq!(effects.certainty, Certainty::None, "{effects:?}");
}

/// The default the hook reads is used for a branch delete: the branch `origin/HEAD` names is protected like `main`.
/// Before the hook read the repository for it, `git branch -D next` was a branch like any other.
#[tokio::test]
async fn the_default_the_hook_reads_is_protected() {
    let c = computer().await;
    let cwd = repo(&c.root, "app", Some("next"));
    // The strict reader reports nothing when this computer's own git configuration holds what it does not read (an
    // `insteadOf`, an include): then the default is not known here, and only the built-in names are protected.
    let read = hooks::git_head(Path::new(&cwd)).and_then(|h| h.default);
    if read.as_deref() != Some("next") {
        eprintln!("skipped: this computer's git configuration keeps the hook from reading the repository");
        return;
    }
    assert_eq!(said(&c.run(&cwd, "git branch -D next").await), ASKED);
    let (effects, matrix) = c.last("git branch -D next");
    assert_eq!((effects.level, effects.certainty), (Level::L3, Certainty::Full), "{effects:?}");
    assert_eq!(matrix.line, "Ask · Mine · Can't be undone");
    assert!(effects.lines.iter().any(|l| l.contains("next (the default branch)")), "{effects:?}");
    // Still only for a push to origin: the question about a branch delete reports no branch.
    assert!(!c.asked().contains("Default branch: "), "{}", c.asked());
    assert_eq!(said(&c.run(&cwd, "git branch -D NEXT").await), ASKED, "whatever the case");
    assert_eq!(said(&c.run(&cwd, "git branch -D feature/y").await), ALLOWED);
}
