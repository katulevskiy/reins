//! Decisions v2 on this computer: the matrix runs on every hook request. Off (the default) it only records what it
//! would have decided on the Activity entry; on (`auto.matrix_decides`) it decides: Auto without the phone, Ask even
//! without a guard rule.

mod d1_mock;

use d1_mock::{Mock, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::Config;
use reins_desktop::harness::Harness;
use reins_desktop::{hooks, journal};
use reins_policy::context::Zone;
use reins_policy::matrix::Outcome;

fn claude(command: &str) -> String {
    serde_json::json!({"session_id": "abc", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
    .to_string()
}

fn config(decides: bool) -> Config {
    let mut c = Config::default();
    c.workspace.roots = vec!["/home/me".to_owned()];
    c.auto.matrix_decides = decides;
    c.auto.setup_done = decides;
    c
}

#[tokio::test]
async fn shadow_off_changes_nothing() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let before = mock.with(|s| s.calls.len());
    let (out, code, _) = hooks::run(
        Harness::ClaudeCode,
        claude("git push --force origin main").as_bytes(),
        &app.paths,
        &config(false),
        &NoPrompter,
    )
    .await;
    assert_eq!(code, 0);
    assert!(out.unwrap().contains("Reins 2FA: approved"));
    assert!(mock.with(|s| s.calls.len()) > before, "asked the phone, as before");
    let log = journal::read(&app.paths, 10, 3_600);
    let m = log[0].matrix.clone().expect("the shadow is kept");
    assert!(!m.decided);
    assert_eq!(
        m.outcome,
        Outcome::Ask {
            hard: true
        }
    );
    // Not risky: no question and no entry, as before (the shadow is kept only with entries).
    let (out, _, _) =
        hooks::run(Harness::ClaudeCode, claude("ls -la").as_bytes(), &app.paths, &config(false), &NoPrompter).await;
    assert_eq!(out, None);
    assert_eq!(journal::read(&app.paths, 10, 3_600).len(), 1);
}

#[tokio::test]
async fn on_a_guard_match_is_still_asked() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let before = mock.with(|s| s.calls.len());
    // The guard asks about `rm -rf`; build output in the workspace is Undoable and Mine (Auto): still asked, the
    // matrix only adds asks on this computer.
    let (out, _, _) =
        hooks::run(Harness::ClaudeCode, claude("rm -rf build dist").as_bytes(), &app.paths, &config(true), &NoPrompter)
            .await;
    assert!(out.unwrap().contains("Reins 2FA: approved"));
    assert!(mock.with(|s| s.calls.len()) > before, "the phone was asked");
    let log = journal::read(&app.paths, 10, 3_600);
    let m = log[0].matrix.clone().unwrap();
    assert!(m.decided && m.outcome == Outcome::Auto && m.zone == Zone::Mine, "{m:?}");
    // Nothing the guard or the matrix asks about: no opinion, no entry.
    let (out, _, _) =
        hooks::run(Harness::ClaudeCode, claude("ls -la").as_bytes(), &app.paths, &config(true), &NoPrompter).await;
    assert_eq!(out, None);
}

#[tokio::test]
async fn without_the_setup_the_matrix_decides_nothing() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let mut c = config(true);
    c.auto.setup_done = false;
    let before = mock.with(|s| s.calls.len());
    let (out, _, _) =
        hooks::run(Harness::ClaudeCode, claude("cp notes.txt /etc/notes.txt").as_bytes(), &app.paths, &c, &NoPrompter)
            .await;
    assert_eq!(out, None, "no guard rule, and the matrix isn't set up: as before");
    assert_eq!(mock.with(|s| s.calls.len()), before);
}

#[tokio::test]
async fn on_ask_asks_without_a_guard_rule() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    // No guard rule for `cp`; a write outside the home folder can't be undone: asked.
    let (out, _, _) = hooks::run(
        Harness::ClaudeCode,
        claude("cp notes.txt /etc/notes.txt").as_bytes(),
        &app.paths,
        &config(true),
        &NoPrompter,
    )
    .await;
    assert!(out.unwrap().contains("Reins 2FA: approved"));
    let asked = mock.with(|s| s.calls.last().cloned()).unwrap();
    assert_eq!(asked["question"], "Claude Code wants to run: cp notes.txt /etc/notes.txt");
    assert!(asked["detail"].as_str().unwrap().starts_with("Auto: Ask · Outside · Can't be undone"), "{asked}");
    // Off: the same command isn't asked (no guard rule), as before.
    let before = mock.with(|s| s.calls.len());
    let (out, _, _) = hooks::run(
        Harness::ClaudeCode,
        claude("cp notes.txt /etc/notes.txt").as_bytes(),
        &app.paths,
        &config(false),
        &NoPrompter,
    )
    .await;
    assert_eq!(out, None);
    assert_eq!(mock.with(|s| s.calls.len()), before);
}

/// Decisions v2 P2 with P3's nets: where only the matrix asks and a net is planned, it decides on the netted request
/// (`NetPlan::after`). Deleting an untracked file in the workspace can't be undone (asked); kept in the Reins trash it
/// can, so it's allowed without the phone and the harness gets the netted command.
#[cfg(unix)]
#[tokio::test]
async fn on_a_net_that_lowers_it_the_matrix_decides_on_the_netted_request() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    // Not under the OS temp folder: `/tmp` is always scratch, where a delete is Auto without any net. Reins's own
    // state (and its trash) moves next to it: the trash must be on the same disk, and `/tmp` may be a RAM disk.
    let root = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let state = root.path().join("state");
    copy_dir(app.dir.path(), &state);
    let paths = reins_desktop::config::Paths::under(&state);
    let ws = root.path().join("ws");
    let project = ws.join("app");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("notes.txt"), "kept").unwrap();
    let mut c = config(true);
    c.workspace.roots = vec![ws.to_str().unwrap().to_owned()];
    let input = serde_json::json!({"session_id": "abc", "cwd": project.to_str().unwrap(),
        "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "rm notes.txt"},
        "tool_use_id": "t"})
    .to_string();
    let before = mock.with(|s| s.calls.len());
    let (out, code, _) = hooks::run(Harness::ClaudeCode, input.as_bytes(), &paths, &c, &NoPrompter).await;
    assert_eq!(code, 0);
    let out: serde_json::Value = serde_json::from_str(&out.expect("an answer")).unwrap();
    let hso = &out["hookSpecificOutput"];
    assert_eq!(hso["permissionDecision"], "allow", "{out}");
    let netted = hso["updatedInput"]["command"].as_str().unwrap();
    assert!(netted.contains("net run"), "{netted}");
    assert_eq!(mock.with(|s| s.calls.len()), before, "the phone wasn't asked");
    let log = journal::read(&paths, 10, 3_600);
    let m = log[0].matrix.clone().unwrap();
    assert!(m.decided && m.outcome == Outcome::Auto, "{m:?}");
    assert!(log[0].nets.is_some());
    assert!(project.join("notes.txt").exists(), "nothing ran yet");
}

#[cfg(unix)]
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).unwrap();
        }
    }
}
