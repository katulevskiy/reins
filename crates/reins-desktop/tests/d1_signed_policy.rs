//! Decisions v2, 08 R1: hook requests decided with the policy the phone signed for this computer. A fresh, valid
//! snapshot decides like the phone (Auto here, Ask on the phone, Deny only by a rule, Overnight parks); anything else
//! falls back to asking as before; this computer's own settings only ever tighten.

mod d1_mock;

use std::collections::BTreeMap;

use d1_mock::{App, Mock, SigningPhone, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::Config;
use reins_desktop::harness::Harness;
use reins_desktop::{hooks, journal, policy};
use reins_policy::context::{FactKind, Zone};
use reins_policy::desktop_policy::{self as dp, Binding, DesktopPolicy, ModeState, PolicyMessage, Tighten, Tightening};
use reins_policy::effects::Level;
use reins_policy::matrix::{Cell, Issuer, Outcome, Rule, Snapshot, Then, When};
use reins_proto::autopilot::Mode;

/// The approval phone, signing for the app.
struct Phone(SigningPhone);

impl Phone {
    fn new() -> Self {
        Self(SigningPhone::default())
    }

    fn send(&self, app: &App, m: &PolicyMessage) -> policy::Applied {
        self.0.send(&app.paths, &app.identity, m)
    }
}

fn binding(app: &App, version: u64, issued_at: i64) -> Binding {
    Binding {
        version,
        issued_at,
        not_after: issued_at + dp::VALIDITY_SECS,
        client_key: app.identity.public_key(),
        connection_id: "conn-desktop".to_owned(),
        profile: dp::PERSONAL.to_owned(),
    }
}

/// The phone's policy: dkat is Mine, acme is Work, the 01 §4 matrix, Auto.
fn signed(app: &App, version: u64, mode: Mode) -> DesktopPolicy {
    let mut snapshot = Snapshot::default();
    snapshot.facts.set(FactKind::Owner, "dkat", Zone::Mine);
    snapshot.facts.set(FactKind::Owner, "acme", Zone::Work);
    DesktopPolicy {
        binding: binding(app, version, reins_desktop::now_unix()),
        decides: true,
        mode: ModeState {
            mode,
            until: None,
        },
        snapshot,
        github_ids: BTreeMap::new(),
        roots: vec!["/home/me".to_owned()],
        scratch: Vec::new(),
    }
}

fn claude(command: &str) -> String {
    serde_json::json!({"session_id": "abc", "cwd": "/home/me/app", "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
    .to_string()
}

fn config() -> Config {
    let mut c = Config::default();
    c.workspace.roots = vec!["/home/me".to_owned()];
    c
}

/// Runs the hook; the decision, and whether the phone was asked.
async fn run(mock: &Mock, app: &App, c: &Config, command: &str) -> (Option<String>, bool) {
    let before = mock.with(|s| s.calls.len());
    let (out, code, _) = hooks::run(Harness::ClaudeCode, claude(command).as_bytes(), &app.paths, c, &NoPrompter).await;
    assert_eq!(code, 0);
    let decision = out.map(|o| {
        let v: serde_json::Value = serde_json::from_str(&o).unwrap();
        v["hookSpecificOutput"]["permissionDecision"].as_str().unwrap_or_default().to_owned()
    });
    (decision, mock.with(|s| s.calls.len()) > before)
}

#[tokio::test]
async fn a_fresh_snapshot_decides_like_the_phone() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    let p = signed(&app, 10, Mode::Auto);
    assert_eq!(phone.send(&app, &PolicyMessage::Snapshot(Box::new(p.clone()))).snapshots, 1);
    // Build output in the workspace (the guard's "deletes", a label now): Auto, allowed here without the phone.
    assert_eq!(run(&mock, &app, &config(), "rm -rf build dist").await, (Some("allow".to_owned()), false));
    let log = journal::read(&app.paths, 10, 3_600);
    let m = log[0].matrix.clone().unwrap();
    assert!(m.decided && m.outcome == Outcome::Auto && m.zone == Zone::Mine, "{m:?}");
    // The same request on the phone's matrix: the same outcome.
    let ctx = reins_desktop::consequences::context(&config(), Some("/home/me/app"), &|_| None, None);
    let a = reins_policy::effects::analyze_command("rm -rf build dist", &ctx);
    let r = reins_policy::context::request_zone(&a, &p.snapshot.facts, &reins_policy::context::Base::default());
    let phone_says = reins_policy::matrix::evaluate(
        &reins_policy::matrix::Input {
            analysis: &a,
            resolved: &r,
            connection: "conn-desktop",
            seen_resource: false,
            now: reins_desktop::now_unix(),
            overnight: false,
            netted: false,
        },
        &p.snapshot,
    );
    assert_eq!(phone_says.outcome, m.outcome);
    // What the matrix asks about goes to the phone, even with no guard rule.
    assert_eq!(run(&mock, &app, &config(), "cp notes.txt /etc/notes.txt").await, (Some("allow".to_owned()), true));
    // Nothing guarded and Auto: no opinion (the harness's own settings apply), no phone.
    assert_eq!(run(&mock, &app, &config(), "ls -la").await, (None, false));
    // The phone makes Mine L0 an Ask: the computer asks too.
    let mut stricter = signed(&app, 11, Mode::Auto);
    stricter.snapshot.matrix.set(Zone::Mine, Level::L0, Cell::Ask);
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(stricter)));
    assert_eq!(run(&mock, &app, &config(), "rm -rf build dist").await, (Some("allow".to_owned()), true));
}

#[tokio::test]
async fn lockdown_refuses_every_tool_use() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(signed(&app, 1, Mode::Lockdown))));
    assert_eq!(run(&mock, &app, &config(), "ls -la").await, (Some("deny".to_owned()), false));
    let log = journal::read(&app.paths, 10, 3_600);
    assert!(log[0].reason.as_deref().unwrap_or_default().contains("Lockdown"), "{:?}", log[0]);
    // A "tighten now" alone, with no snapshot held: refused too.
    let other = logged_in(&mock, 3600);
    let now = reins_desktop::now_unix();
    phone.send(
        &other,
        &PolicyMessage::Tighten(Tighten {
            binding: Binding {
                not_after: now + dp::TIGHTEN_SECS,
                ..binding(&other, 5, now)
            },
            tightening: Tightening {
                lockdown: true,
                ..Tightening::default()
            },
        }),
    );
    assert_eq!(run(&mock, &other, &config(), "cargo test").await, (Some("deny".to_owned()), false));
}

#[tokio::test]
async fn overnight_parks() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(signed(&app, 1, Mode::Overnight))));
    // Rewriting what may be the default branch of an unknown repository can't be undone: parked for the morning.
    let (decision, asked) = run(&mock, &app, &config(), "git push --force origin main").await;
    assert!(asked);
    assert_eq!(decision.as_deref(), Some("deny"), "parked: the agent is told to go on");
    assert_eq!(mock.with(|s| s.defers.last().copied()), Some(true), "parked at once");
}

#[tokio::test]
async fn bypass_allows_but_deny_rules_hold() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    let mut p = signed(&app, 1, Mode::Bypass);
    p.mode.until = Some(reins_desktop::now_unix() + 600);
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(p.clone())));
    assert_eq!(run(&mock, &app, &config(), "git push --force origin main").await, (Some("allow".to_owned()), false));
    p.binding.version = 2;
    p.snapshot.rules.push(Rule {
        id: "never-irreversible".to_owned(),
        issuer: Issuer::Owner,
        when: When {
            levels: vec![Level::L3],
            ..When::default()
        },
        then: Then::Deny,
        not_after: None,
        accepted: false,
        explain: "Nothing irreversible".to_owned(),
    });
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(p)));
    assert_eq!(run(&mock, &app, &config(), "git push --force origin main").await, (Some("deny".to_owned()), false));
}

#[tokio::test]
async fn reins_groups_are_always_asked() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(signed(&app, 1, Mode::Auto))));
    let mut c = config();
    // Even with the group switched off locally.
    c.guard.disabled_groups = vec!["reins".to_owned()];
    for command in [
        "cat /home/me/.local/state/reins/policy.json",
        "sed -i s/a/b/ /home/me/.config/reins/config.toml",
        "cp /home/me/x.json /home/me/.local/state/reins/tickets.json",
    ] {
        assert_eq!(run(&mock, &app, &c, command).await, (Some("allow".to_owned()), true), "{command}");
    }
}

#[tokio::test]
async fn an_agent_edited_config_cannot_loosen() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(signed(&app, 1, Mode::Auto))));
    // An agent's edits: Auto in every cell, acme as Mine, `/` as a root, every built-in group off, the local switch on.
    let mut c = config();
    for z in Zone::ALL {
        for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
            c.auto.matrix.set(z, l, Cell::Auto);
        }
    }
    c.auto.facts.set(FactKind::Owner, "acme", Zone::Mine);
    c.auto.facts.set(FactKind::Owner, "evil", Zone::Mine);
    c.auto.matrix_decides = true;
    c.auto.setup_done = true;
    c.workspace.roots = vec!["/".to_owned(), "/home/me".to_owned()];
    c.workspace.scratch = vec!["/home/me".to_owned()];
    c.guard.disabled_groups = reins_desktop::guard::GROUPS.iter().map(|g| g.id.to_owned()).collect();
    // Still asked: a write outside the workspace, a delete outside it (`/` is no root), a push to someone else's.
    for command in
        ["cp notes.txt /etc/notes.txt", "rm -rf /opt/data", "git push https://github.com/evil/app HEAD:refs/heads/main"]
    {
        let (_, asked) = run(&mock, &app, &c, command).await;
        assert!(asked, "{command} was not asked");
    }
}

#[tokio::test]
async fn a_stale_or_tampered_snapshot_falls_back_to_asking() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = Phone::new();
    // Issued long ago: valid when it arrived, expired now.
    let mut old = signed(&app, 1, Mode::Auto);
    old.binding.issued_at -= dp::VALIDITY_SECS;
    old.binding.not_after -= dp::VALIDITY_SECS;
    assert_eq!(phone.send(&app, &PolicyMessage::Snapshot(Box::new(old))).snapshots, 0, "already expired on arrival");
    // As before: the guard asks the phone about the delete.
    assert_eq!(run(&mock, &app, &config(), "rm -rf build dist").await, (Some("allow".to_owned()), true));
    // A fresh one decides; tampered with on disk, it doesn't.
    phone.send(&app, &PolicyMessage::Snapshot(Box::new(signed(&app, 2, Mode::Auto))));
    assert_eq!(run(&mock, &app, &config(), "rm -rf build dist").await, (Some("allow".to_owned()), false));
    let file = app.paths.state_dir.join("policy.json");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("\\\"decides\\\":true", "\\\"decides\\\":false")).unwrap();
    assert_ne!(std::fs::read_to_string(&file).unwrap(), text);
    assert_eq!(run(&mock, &app, &config(), "rm -rf build dist").await, (Some("allow".to_owned()), true));
}

#[tokio::test]
async fn without_a_snapshot_the_local_matrix_never_allows() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let mut c = config();
    c.auto.matrix_decides = true;
    c.auto.setup_done = true;
    // Guarded: asked as before (P2's local matrix only adds asks).
    assert_eq!(run(&mock, &app, &c, "rm -rf build dist").await, (Some("allow".to_owned()), true));
    assert_eq!(run(&mock, &app, &c, "ls -la").await, (None, false));
}
