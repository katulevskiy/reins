//! Decisions v2, 08 R1: a hook's question carries what this computer read of the request
//! (`reins_policy::desktop_reading`), so the phone's own matrix can decide it when the copy of the policy kept here ran
//! out. It is read with the folders the phone signed, never this computer's own unsigned ones; nothing is sent for what
//! must be asked here anyway, and an older server gets the question without it.

mod d1_mock;

use d1_mock::{App, Mock, SigningPhone, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::Config;
use reins_desktop::harness::Harness;
use reins_desktop::{hooks, journal, policy};
use reins_policy::context::{FactKind, Zone};
use reins_policy::desktop_policy::{self as dp, Fallback, Gate, PolicyMessage};
use reins_policy::desktop_reading::{Asked, FORMAT, Folders, MAX_BYTES, Reading, about, accept};
use reins_policy::effects::{Area, Effect, Level};
use reins_policy::matrix::Cell;
use serde_json::Value;

fn home() -> String {
    reins_desktop::config::home_dir().unwrap().to_string_lossy().into_owned()
}

fn claude(cwd: &str, command: &str) -> String {
    serde_json::json!({"session_id": "abc", "cwd": cwd, "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
    .to_string()
}

/// Runs the hook for `command` in `cwd`: what the phone was asked (its arguments), if it was.
async fn asked(mock: &Mock, app: &App, c: &Config, cwd: &str, command: &str) -> Option<Value> {
    let before = mock.with(|s| s.calls.len());
    let (_, code, _) =
        hooks::run(Harness::ClaudeCode, claude(cwd, command).as_bytes(), &app.paths, c, &NoPrompter).await;
    assert_eq!(code, 0);
    mock.with(|s| (s.calls.len() > before).then(|| s.calls.last().cloned()).flatten())
}

fn reading_of(args: &Value) -> Option<Reading> {
    args.get("reading").and_then(Value::as_str).and_then(Reading::from_wire)
}

fn deletes(r: &Reading) -> Vec<(String, Area)> {
    r.analysis
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileDelete {
                paths,
                ..
            } => Some(paths.iter().map(|p| (p.path.clone(), p.area.clone()))),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The phone signed `roots` for the app an hour ago: that copy of the policy ran out since, so the phone is asked.
fn a_copy_that_ran_out(phone: &SigningPhone, app: &App, roots: &[&str]) {
    let past = reins_desktop::now_unix() - 3_600;
    let mut p = SigningPhone::auto_snapshot(&app.identity, 5, roots);
    p.binding.issued_at = past;
    p.binding.not_after = past + dp::VALIDITY_SECS;
    assert_eq!(phone.send_at(&app.paths, &app.identity, &PolicyMessage::Snapshot(Box::new(p)), past).snapshots, 1);
}

/// The roots every phone signs, as it signs them.
const STANDARD: [&str; 4] = ["~/Documents/dev", "~/src", "~/code", "~/projects"];

#[tokio::test]
async fn a_hook_question_carries_what_this_computer_read() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    a_copy_that_ran_out(&SigningPhone::default(), &app, &STANDARD);
    let config = Config::default();
    assert_eq!(
        policy::effective(&app.paths, &config, reins_desktop::now_unix()).0.gate,
        Gate::Fallback(Fallback::NoSnapshot)
    );
    let home = home();
    let cwd = format!("{home}/Documents/dev/reins-reading-test-app");
    let args = asked(&mock, &app, &config, &cwd, "rm -rf build dist").await.expect("the guard asks, as before");
    let r = reading_of(&args).expect("with what this computer read");
    assert!(args["reading"].as_str().unwrap().len() <= MAX_BYTES);
    assert_eq!(r.v, FORMAT);
    // About exactly the command and the folder the phone is shown.
    assert_eq!(r.about, about(&Asked::Command("rm -rf build dist"), Some(&cwd), None));
    assert!(args["detail"].as_str().unwrap().ends_with(&format!("Command:\nrm -rf build dist\n\nIn: {cwd}")));
    // Read with the roots the phone signed, as absolute folders.
    let standard: Vec<String> = STANDARD.iter().map(|r| r.replacen('~', &home, 1)).collect();
    assert_eq!((r.home.as_str(), &r.roots, r.scratch.len()), (home.as_str(), &standard, 0));
    assert_eq!(deletes(&r), vec![(format!("{cwd}/build"), Area::Generated), (format!("{cwd}/dist"), Area::Generated)]);
    assert_eq!((r.netted, r.matrix, r.facts), (None, None, None), "no net, and nothing of its own to tighten with");
    // Activity keeps the effects and the matrix's line, as before (read once, before asking).
    let entry = journal::read(&app.paths, 1, 3_600).remove(0);
    assert_eq!(entry.effects.as_ref().map(|e| e.level), Some(Level::L0), "{entry:?}");
    assert!(entry.matrix.as_ref().is_some_and(|m| !m.decided), "{entry:?}");
}

#[tokio::test]
async fn it_is_read_with_the_folders_the_phone_signed_never_this_computers_own() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = SigningPhone::default();
    a_copy_that_ran_out(&phone, &app, &["/home/me/dev"]);
    // An agent rewrote this computer's settings: everything a root, the home folder scratch, Auto everywhere.
    let mut edited = Config::default();
    edited.workspace.roots = vec!["/home/me/dev".to_owned(), "/".to_owned(), "/home/me".to_owned()];
    edited.workspace.scratch = vec!["~".to_owned(), "/home/me/dev/tmp".to_owned(), "/home/me/elsewhere".to_owned()];
    for z in Zone::ALL {
        for l in [Level::L0, Level::L1, Level::L2, Level::L3] {
            edited.auto.matrix.set(z, l, Cell::Auto);
        }
    }
    edited.auto.facts.set(FactKind::Owner, "evil", Zone::Mine);
    assert_eq!(
        policy::effective(&app.paths, &edited, reins_desktop::now_unix()).0.gate,
        Gate::Fallback(Fallback::NoSnapshot)
    );
    let args = asked(&mock, &app, &edited, "/home/me/dev/app", "rm -rf /home/me/elsewhere/photos").await.unwrap();
    let r = reading_of(&args).expect("a reading");
    assert_eq!(r.roots, ["/home/me/dev"], "only what the phone signed");
    assert_eq!(r.scratch, ["/home/me/dev/tmp"], "its own scratch folder strictly inside a signed root, nothing else");
    let (path, area) = deletes(&r).remove(0);
    assert_eq!(path, "/home/me/elsewhere/photos");
    assert!(!area.in_workspace(), "read as the phone's folders place it: {area:?}");
    assert_eq!(r.analysis.level, Level::L3);
    // Its own `[auto]` travels, for the phone to tighten with (it can't loosen: `tighten_with_local`).
    assert_eq!(r.matrix.as_ref(), Some(&edited.auto.matrix));
    assert_eq!(r.facts.as_ref().map(|f| f.owners.len()), Some(1));
    // The phone, with the same folders signed, takes it as it is: nothing in it was read with a folder it didn't sign.
    let signed_roots = vec!["/home/me/dev".to_owned()];
    let signed = Folders {
        home: &r.home,
        roots: &signed_roots,
        scratch: &[],
    };
    assert_eq!(accept(&r, &signed).map(|a| a.analysis.level), Some(Level::L3));
    // A fresh copy whose matrix asks: the question carries the reading too (the phone reaches the same answer).
    let mut fresh = SigningPhone::auto_snapshot(&app.identity, 6, &["/home/me/dev"]);
    fresh.snapshot.matrix.set(Zone::Mine, Level::L0, Cell::Ask);
    phone.send(&app.paths, &app.identity, &PolicyMessage::Snapshot(Box::new(fresh)));
    let mut config = Config::default();
    config.workspace.roots = vec!["/home/me/dev".to_owned()];
    assert!(matches!(policy::effective(&app.paths, &config, reins_desktop::now_unix()).0.gate, Gate::Matrix { .. }));
    let args = asked(&mock, &app, &config, "/home/me/dev/app", "rm -rf build").await.expect("the matrix asks");
    let r = reading_of(&args).expect("a reading");
    assert_eq!((r.roots.as_slice(), r.analysis.level), (&["/home/me/dev".to_owned()][..], Level::L0));
}

/// A phone re-checks every call against its own description of the tool and refuses an argument it doesn't know. So
/// a reading goes only to a phone whose signed snapshot says it takes them: never to one that signed nothing kept
/// here, nor to one older than readings.
#[tokio::test]
async fn a_phone_that_never_said_it_takes_readings_gets_none() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let cwd = format!("{}/Documents/dev/reins-reading-test-app", home());
    let config = Config::default();
    // Nothing signed is kept here.
    let args = asked(&mock, &app, &config, &cwd, "rm -rf build dist").await.expect("asked, as before");
    assert!(args.get("reading").is_none(), "{args}");
    // A fresh snapshot of a phone older than readings, whose matrix asks.
    let phone = SigningPhone::default();
    let mut p = SigningPhone::auto_snapshot(&app.identity, 5, &STANDARD);
    p.snapshot.matrix.set(Zone::Mine, Level::L0, Cell::Ask);
    let m = PolicyMessage::Snapshot(Box::new(p));
    assert_eq!(phone.send_as_older_phone(&app.paths, &app.identity, &m).snapshots, 1);
    assert!(matches!(policy::effective(&app.paths, &config, reins_desktop::now_unix()).0.gate, Gate::Matrix { .. }));
    let args = asked(&mock, &app, &config, &cwd, "rm -rf build dist").await.expect("the matrix asks");
    assert!(args.get("reading").is_none(), "{args}");
    // The same phone, updated: its next snapshot says so.
    let mut p = SigningPhone::auto_snapshot(&app.identity, 6, &STANDARD);
    p.snapshot.matrix.set(Zone::Mine, Level::L0, Cell::Ask);
    assert_eq!(phone.send(&app.paths, &app.identity, &PolicyMessage::Snapshot(Box::new(p))).snapshots, 1);
    let args = asked(&mock, &app, &config, &cwd, "rm -rf build dist").await.unwrap();
    assert!(reading_of(&args).is_some(), "{args}");
}

#[tokio::test]
async fn nothing_is_read_for_what_must_be_asked_here() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    a_copy_that_ran_out(&SigningPhone::default(), &app, &STANDARD);
    let home = home();
    let cwd = format!("{home}/Documents/dev/reins-reading-test-app");
    let mut config = Config::default();
    config.guard.commands = vec!["make deploy*".to_owned()];
    for command in [
        // The user's own rule.
        "make deploy-production".to_owned(),
        // Reins's own state, secrets and payments: asked whatever the matrix says.
        format!("cat {home}/.config/reins/session.json"),
        "rm -rf build && cat ~/.local/state/reins/tickets.json".to_owned(),
        "reins secrets get github-token".to_owned(),
        "reins payments-trust abc".to_owned(),
    ] {
        let args = asked(&mock, &app, &config, &cwd, &command).await.unwrap_or_else(|| panic!("{command}: asked"));
        assert!(args.get("reading").is_none(), "{command}: {args}");
    }
    let args = asked(&mock, &app, &config, &cwd, "rm -rf build").await.unwrap();
    assert!(reading_of(&args).is_some(), "{args}");
}

#[tokio::test]
async fn an_older_server_gets_the_question_without_the_reading() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    a_copy_that_ran_out(&SigningPhone::default(), &app, &STANDARD);
    let cwd = format!("{}/Documents/dev/reins-reading-test-app", home());
    for unknown in [&["reading"][..], &["reading", "net"]] {
        mock.with(|s| s.unknown_arguments = unknown.iter().map(|a| (*a).to_owned()).collect());
        let before = mock.with(|s| s.calls.len());
        let (out, code, _) = hooks::run(
            Harness::ClaudeCode,
            claude(&cwd, "rm -rf build dist").as_bytes(),
            &app.paths,
            &Config::default(),
            &NoPrompter,
        )
        .await;
        assert_eq!(code, 0);
        assert!(out.unwrap().contains("Reins 2FA: approved"), "asked again without it, and answered");
        assert_eq!(mock.with(|s| s.calls.len()), before + 1, "the refused call never reached the phone");
        let args = mock.with(|s| s.calls.last().cloned()).unwrap();
        assert!(args.get("reading").is_none(), "{args}");
        assert_eq!(args["question"], "Claude Code wants to run: rm -rf build dist");
    }
}

/// A delete the hook nets: the reading names the netted command the question shows and the paths the net keeps, and
/// the phone levels the request with them itself.
#[cfg(unix)]
#[tokio::test]
async fn a_netted_delete_says_which_paths_its_net_keeps() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    // A project outside the OS temp folder (which is scratch: nothing there is netted), with the app's state (and
    // so the Reins trash) on the same disk.
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let proj = root.join("work/proj");
    std::fs::create_dir_all(proj.join("notes")).unwrap();
    std::fs::write(proj.join("notes/todo.md"), "keep me").unwrap();
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    for e in std::fs::read_dir(&app.paths.state_dir).unwrap().flatten() {
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), state.join(e.file_name())).unwrap();
        }
    }
    let paths = reins_desktop::config::Paths {
        config_dir: app.paths.config_dir.clone(),
        state_dir: state,
    };
    let signed_root = root.join("work").to_string_lossy().into_owned();
    // The phone signed the folder an hour ago: the copy ran out, the question goes to the phone.
    let past = reins_desktop::now_unix() - 3_600;
    let mut p = SigningPhone::auto_snapshot(&app.identity, 5, &[&signed_root]);
    p.binding.issued_at = past;
    p.binding.not_after = past + dp::VALIDITY_SECS;
    SigningPhone::default().send_at(&paths, &app.identity, &PolicyMessage::Snapshot(Box::new(p)), past);
    let mut config = Config::default();
    config.workspace.roots = vec![signed_root.clone()];
    let cwd = proj.to_string_lossy().into_owned();
    let (out, code, _) =
        hooks::run(Harness::ClaudeCode, claude(&cwd, "rm -rf notes").as_bytes(), &paths, &config, &NoPrompter).await;
    assert_eq!(code, 0);
    let out: Value = serde_json::from_str(&out.unwrap()).unwrap();
    let rewritten =
        out["hookSpecificOutput"]["updatedInput"]["command"].as_str().expect("the netted command").to_owned();
    let args = mock.with(|s| s.calls.last().cloned()).unwrap();
    let r = reading_of(&args).expect("a reading");
    let kept = format!("{cwd}/notes");
    assert_eq!(r.netted.as_deref(), Some(&[kept.clone()][..]));
    assert_eq!(r.about, about(&Asked::Command("rm -rf notes"), Some(&cwd), Some(&rewritten)));
    assert_eq!(deletes(&r), vec![(kept, Area::Source)]);
    let signed_roots = vec![signed_root];
    let signed = Folders {
        home: &r.home,
        roots: &signed_roots,
        scratch: &[],
    };
    let taken = accept(&r, &signed).expect("its folders are the signed ones");
    assert_eq!((taken.analysis.level, taken.netted.map(|n| n.level)), (Level::L3, Some(Level::L1)));
}
