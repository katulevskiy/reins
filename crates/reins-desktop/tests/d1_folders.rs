//! This computer's folders with a policy the phone signed for it: a scratch folder the phone signed counts here even
//! when no configuration names it (the computer only suggested it, the owner confirmed it on the phone), a folder only
//! the configuration names does not, and `reins folders` says of each how it stands.
#![cfg(unix)]

mod d1_mock;

use std::path::PathBuf;

use d1_mock::{App, Mock, SigningPhone, logged_in};
use reins_desktop::auth::prompt::NoPrompter;
use reins_desktop::config::Config;
use reins_desktop::harness::Harness;
use reins_desktop::settings::WorkspaceList;
use reins_desktop::{folders_cli, hooks, policy};
use reins_policy::desktop_policy::PolicyMessage;

/// Folders of this test, resolved: `work` (a root), `elsewhere` and `confirmed` (outside it).
struct Place {
    _dir: tempfile::TempDir,
    base: PathBuf,
}

impl Place {
    fn new() -> Self {
        // Not the system temp folder: `/tmp` is always scratch.
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        for folder in ["work/app", "work/tmp", "other", "elsewhere", "confirmed"] {
            std::fs::create_dir_all(base.join(folder)).unwrap();
        }
        Self {
            _dir: dir,
            base,
        }
    }

    fn path(&self, folder: &str) -> String {
        self.base.join(folder).to_str().unwrap().to_owned()
    }
}

fn config(place: &Place) -> Config {
    let mut c = Config::default();
    c.workspace.roots = vec![place.path("work")];
    c
}

/// Sends a fresh Auto snapshot signing `work` as a root and `scratch` as scratch folders.
fn sign(phone: &SigningPhone, app: &App, place: &Place, version: u64, scratch: &[String]) {
    let mut p = SigningPhone::auto_snapshot(&app.identity, version, &[&place.path("work")]);
    p.scratch = scratch.to_vec();
    assert_eq!(phone.send(&app.paths, &app.identity, &PolicyMessage::Snapshot(Box::new(p))).snapshots, 1);
}

/// Runs the hook for `command` in `work/app`; the decision, and whether the phone was asked.
async fn run(mock: &Mock, app: &App, place: &Place, c: &Config, command: &str) -> (Option<String>, bool) {
    let input = serde_json::json!({"session_id": "abc", "cwd": place.path("work/app"), "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": "t"})
    .to_string();
    let before = mock.with(|s| s.calls.len());
    let (out, code, _) = hooks::run(Harness::ClaudeCode, input.as_bytes(), &app.paths, c, &NoPrompter).await;
    assert_eq!(code, 0);
    let decision = out.map(|o| {
        let v: serde_json::Value = serde_json::from_str(&o).unwrap();
        v["hookSpecificOutput"]["permissionDecision"].as_str().unwrap_or_default().to_owned()
    });
    (decision, mock.with(|s| s.calls.len()) > before)
}

/// The owner confirmed a suggested scratch folder on the phone: it is signed, and deletes in it are decided here,
/// though this computer's configuration never named it. Before that, and for a folder only the configuration names,
/// the phone is asked.
#[tokio::test]
async fn a_scratch_folder_the_phone_signed_counts_without_a_local_entry() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = SigningPhone::default();
    let place = Place::new();
    let c = config(&place);
    let delete = format!("rm -rf {}/old-build", place.path("confirmed"));

    // Signed roots only: the delete is asked about (the owner's approvals of such asks are what suggest the folder).
    sign(&phone, &app, &place, 1, &[]);
    let (e, _) = policy::effective(&app.paths, &c, reins_desktop::now_unix());
    assert!(e.scratch.is_empty(), "{:?}", e.scratch);
    assert_eq!(run(&mock, &app, &place, &c, &delete).await, (Some("allow".to_owned()), true));

    // An agent writes the folder into the configuration: outside the signed roots, that changes nothing.
    let mut edited = c.clone();
    edited.workspace.scratch = vec![place.path("confirmed"), place.path("elsewhere")];
    let (e, _) = policy::effective(&app.paths, &edited, reins_desktop::now_unix());
    assert!(e.scratch.is_empty(), "{:?}", e.scratch);
    assert!(run(&mock, &app, &place, &edited, &delete).await.1, "still asked");

    // The owner confirmed it on the phone: the snapshot signs it as scratch.
    sign(&phone, &app, &place, 2, &[place.path("confirmed")]);
    let (e, _) = policy::effective(&app.paths, &c, reins_desktop::now_unix());
    assert_eq!(e.scratch, [place.path("confirmed")]);
    assert_eq!(run(&mock, &app, &place, &c, &delete).await, (Some("allow".to_owned()), false), "decided here");
    // Only that folder: its neighbour is asked about as before.
    let other = format!("rm -rf {}/old-build", place.path("elsewhere"));
    assert!(run(&mock, &app, &place, &c, &other).await.1);

    // An agent replaces the folder with a link to another one: it is no longer the folder the owner confirmed, and
    // neither its name nor what it points to counts.
    std::fs::remove_dir(place.base.join("confirmed")).unwrap();
    std::os::unix::fs::symlink(place.base.join("elsewhere"), place.base.join("confirmed")).unwrap();
    let (e, _) = policy::effective(&app.paths, &c, reins_desktop::now_unix());
    assert!(e.scratch.is_empty(), "{:?}", e.scratch);
    assert!(run(&mock, &app, &place, &c, &delete).await.1);
    assert!(run(&mock, &app, &place, &c, &other).await.1);

    // The phone removes it: the next snapshot no longer has it.
    std::fs::remove_file(place.base.join("confirmed")).unwrap();
    std::fs::create_dir(place.base.join("confirmed")).unwrap();
    sign(&phone, &app, &place, 3, &[]);
    assert!(run(&mock, &app, &place, &c, &delete).await.1);
}

/// `reins folders` with a signed policy: what counts, what waits for the owner, and what only the phone has.
#[tokio::test]
async fn the_list_says_how_each_folder_stands() {
    let mock = Mock::start().await;
    let app = logged_in(&mock, 3600);
    let phone = SigningPhone::default();
    let place = Place::new();
    let mut c = config(&place);
    c.workspace.roots.push(place.path("other"));
    c.workspace.scratch = vec![place.path("work/tmp"), place.path("elsewhere")];
    let home = reins_desktop::config::home_dir().unwrap().to_string_lossy().into_owned();
    let row = |listed: &[String], folder: &str| {
        let shown = folders_cli::shown(&place.path(folder), &home);
        listed
            .iter()
            .find(|l| l.trim_start().starts_with(&format!("{shown} ")))
            .unwrap_or_else(|| panic!("no row for {shown}: {listed:#?}"))
            .clone()
    };

    // No policy yet: everything is only set here.
    let listed = folders_cli::list(&app.paths, &c, reins_desktop::now_unix()).unwrap();
    for folder in ["work", "other", "work/tmp", "elsewhere"] {
        assert!(row(&listed, folder).contains("set here"), "{listed:#?}");
    }

    sign(&phone, &app, &place, 1, &[place.path("confirmed")]);
    let listed = folders_cli::list(&app.paths, &c, reins_desktop::now_unix()).unwrap();
    assert!(row(&listed, "work").contains("counts (your phone signed it)"), "{listed:#?}");
    assert!(row(&listed, "other").contains("waiting for you to confirm it on your phone"), "{listed:#?}");
    assert!(row(&listed, "work/tmp").contains("counts (inside a work folder your phone signed)"), "{listed:#?}");
    assert!(row(&listed, "elsewhere").contains("waiting for you to confirm it on your phone"), "{listed:#?}");
    assert!(row(&listed, "confirmed").contains("counts (confirmed on your phone; not in config.toml here)"));
    assert!(listed.iter().any(|l| l.contains("Confirm on your phone")), "{listed:#?}");

    // Adding says what happens next, and what the phone still holds is said when removing.
    std::fs::write(app.paths.config_file(), "[notify]\nphone = false\n").unwrap();
    let now = reins_desktop::now_unix();
    std::fs::create_dir_all(place.base.join("work/scratch")).unwrap();
    // (The standard roots are this computer's roots again; `work` is signed, so it is added as a root first.)
    let said = folders_cli::add(&app.paths, &place.path("work"), WorkspaceList::Roots, now).unwrap();
    assert!(said[1].contains("counts from now on"), "{said:?}");
    let said = folders_cli::add(&app.paths, &place.path("work/scratch"), WorkspaceList::Scratch, now).unwrap();
    assert!(said[1].contains("counts from now on: it is inside a work folder your phone signed"), "{said:?}");
    let said = folders_cli::add(&app.paths, &place.path("elsewhere"), WorkspaceList::Scratch, now).unwrap();
    assert!(said[1].contains("counts once you confirm it on your phone"), "{said:?}");
    let said = folders_cli::add(&app.paths, &place.path("confirmed"), WorkspaceList::Scratch, now).unwrap();
    assert!(said[1].contains("your phone signed it already"), "{said:?}");
    let said = folders_cli::remove(&app.paths, &place.path("confirmed"), now).unwrap();
    assert!(said.last().unwrap().contains("still has it signed"), "{said:?}");
    let said = folders_cli::remove(&app.paths, &place.path("elsewhere"), now).unwrap();
    assert!(said.last().unwrap().contains("no longer counts"), "{said:?}");
    // Not in the configuration any more, but signed: the phone is where it is removed.
    let gone = folders_cli::remove(&app.paths, &place.path("confirmed"), now).unwrap_err();
    assert!(gone.contains("Your phone signed it as a scratch folder"), "{gone}");
}
