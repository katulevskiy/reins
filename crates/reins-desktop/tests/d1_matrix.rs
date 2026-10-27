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
