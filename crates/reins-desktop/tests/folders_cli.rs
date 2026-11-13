//! `reins folders` from the command line, in a stand-in home: listing, adding and removing `[workspace]` folders, what
//! is refused, and the scratch folders Reins suggests from the activity log. Nothing here is paired with a phone, so
//! every folder is only "set here"; what counts with a signed policy is in `d1_folders.rs`.
//!
//! The stand-in home is under Cargo's own temp folder for tests, not the system's: `/tmp` is always scratch, and a
//! home inside it would make every folder of these tests scratch already.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        // Nothing listens on this port: the daemon is "not running".
        std::fs::write(config.join("config.toml"), "# mine\nlisten = \"127.0.0.1:9\"\n[notify]\nphone = false\n")
            .unwrap();
        let env = Self {
            dir,
        };
        std::fs::create_dir_all(env.home().join("Documents/dev/tmp")).unwrap();
        env
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("config").join("config.toml")).unwrap()
    }

    fn command(&self, args: &[&str]) -> Command {
        let home = self.home();
        let mut c = Command::new(env!("CARGO_BIN_EXE_reins"));
        c.args(args)
            .current_dir(&home)
            .envs(reins_desktop::config::home_env(&home))
            .env("REINS_CONFIG_DIR", self.dir.path().join("config"))
            .env("REINS_STATE_DIR", self.dir.path().join("state"))
            .env_remove("TMPDIR");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Runs it and expects it to work; what it printed.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.status.success(), "`reins {}`: {}", args.join(" "), text(&out));
        text(&out)
    }

    /// Runs it and expects it to refuse; what it said.
    fn refused(&self, args: &[&str]) -> String {
        let before = self.config();
        let out = self.run(args);
        assert!(!out.status.success(), "`reins {}` worked: {}", args.join(" "), text(&out));
        assert_eq!(self.config(), before, "a refused change writes nothing");
        text(&out)
    }

    /// Writes the activity log: one command per line of `entries` (`outcome`, `decider`, the command, more JSON
    /// fields), each asked in `~/Documents/dev` a minute ago.
    fn journal(&self, entries: &[(&str, &str, &str, &str)]) {
        let cwd = self.home().join("Documents/dev");
        let at = reins_desktop::now_unix() - 60;
        let mut log = String::new();
        for (i, (outcome, decider, command, more)) in entries.iter().enumerate() {
            let detail = format!(
                "Rule: Recursive deletes · rm -r\nMatched: {command}\nGroup: deletes (destructive)\n\nCommand:\n\
                 {command}\n\nIn: {}",
                cwd.display()
            );
            let mut entry = serde_json::json!({"id": format!("e{i}"), "at": at, "kind": "command",
                "source": "Claude Code", "what": format!("Claude Code wants to run: {command}"), "detail": detail,
                "decider": decider, "outcome": outcome});
            if !more.is_empty() {
                let more: serde_json::Value = serde_json::from_str(more).unwrap();
                entry.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
            }
            log.push_str(&entry.to_string());
            log.push('\n');
        }
        let state = self.dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("activity.jsonl"), log).unwrap();
    }
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn mkdir(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
}

#[test]
fn folders_are_listed_added_and_removed() {
    let env = Env::new();
    let listed = env.ok(&["folders"]);
    for expected in [
        "Work folders",
        "~/Documents/dev ",
        "~/src ",
        "set here (the phone has not signed a policy for this computer yet)",
        "/tmp, /var/tmp, /dev/shm, ~/.cache",
        "built in",
    ] {
        assert!(listed.contains(expected), "{expected}:\n{listed}");
    }
    assert_eq!(env.ok(&["folders", "list"]), listed);

    // A temp folder of this computer's own: a scratch folder. The file keeps its lines; the roots stay the standard
    // ones, unnamed.
    let said = env.ok(&["folders", "temp", "~/Documents/dev/tmp"]);
    assert!(said.contains("Added ~/Documents/dev/tmp as a scratch folder."), "{said}");
    assert!(said.contains("Settings › Computers › this computer › Folders"), "{said}");
    let config = env.config();
    assert!(config.starts_with("# mine\nlisten = \"127.0.0.1:9\"\n[notify]\nphone = false\n"), "{config}");
    assert!(config.contains("scratch = [\"~/Documents/dev/tmp\"]") && !config.contains("roots"), "{config}");

    // A work folder, given relative to where the command runs (the home folder here): written as `~/…`, with the
    // standard roots kept.
    mkdir(&env.home().join("work"));
    let said = env.ok(&["folders", "add", "work"]);
    assert!(said.contains("Added ~/work as a work folder."), "{said}");
    let config = env.config();
    for root in ["~/Documents/dev", "~/src", "~/code", "~/projects", "~/work"] {
        assert!(config.contains(&format!("\"{root}\"")), "{root}: {config}");
    }
    // An absolute path outside the home folder stays as it is; one that isn't there yet is added when the folder it
    // would be in exists.
    let outside = env.dir.path().join("data");
    mkdir(&outside);
    let later = outside.join("scratch-later");
    let said = env.ok(&["folders", "add", "--scratch", later.to_str().unwrap()]);
    assert!(said.contains("as a scratch folder"), "{said}");
    assert!(env.config().contains(later.to_str().unwrap()));

    let listed = env.ok(&["folders"]);
    for expected in ["~/work ", "~/Documents/dev/tmp ", later.to_str().unwrap()] {
        assert!(listed.contains(expected), "{expected}:\n{listed}");
    }

    // Once each, however it is written.
    assert!(env.refused(&["folders", "add", "~/work"]).contains("already a work folder"));
    assert!(env.refused(&["folders", "add", "./work/"]).contains("already a work folder"));
    assert!(env.refused(&["folders", "temp", "Documents/dev/tmp"]).contains("already a scratch folder"));
    assert!(env.refused(&["folders", "add", "~/Documents/dev"]).contains("already a work folder"));
    assert!(env.refused(&["folders", "temp", "~/Documents/dev/tmp/sub"]).contains("scratch already"));

    // Removing: a folder added here, a scratch folder, a standard root (the others are then written out).
    assert!(env.ok(&["folders", "remove", "~/work"]).contains("Removed ~/work from this computer's work folders."));
    assert!(env.ok(&["folders", "remove", "Documents/dev/tmp"]).contains("scratch folders"));
    assert!(env.ok(&["folders", "remove", "~/src"]).contains("Removed ~/src"));
    let config = env.config();
    assert!(!config.contains("~/work") && !config.contains("~/src"), "{config}");
    assert!(!config.contains("\"~/Documents/dev/tmp\""), "{config}");
    assert!(config.contains("\"~/Documents/dev\"") && config.contains("\"~/code\""), "{config}");
    assert!(config.starts_with("# mine\nlisten = \"127.0.0.1:9\"\n[notify]\nphone = false\n"), "{config}");
    let listed = env.ok(&["folders"]);
    assert!(!listed.contains("~/src") && !listed.contains("~/work"), "{listed}");
    assert!(env.refused(&["folders", "remove", "~/work"]).contains("not one of this computer's folders"));
    assert!(env.refused(&["folders", "remove", "/tmp"]).contains("always scratch"));
}

#[test]
fn broad_sensitive_and_missing_folders_are_refused() {
    let env = Env::new();
    let home = env.home();
    for dir in [".ssh", ".config/reins", "Documents/notes", "Pictures"] {
        mkdir(&home.join(dir));
    }
    // A link to the home folder, and one to the keys: read as what they are.
    std::os::unix::fs::symlink(&home, home.join("everything")).unwrap();
    std::os::unix::fs::symlink(home.join(".ssh"), home.join("Pictures/keys")).unwrap();
    for scratch in [&[][..], &["--scratch"][..]] {
        for (path, why) in [
            ("~", "too broad"),
            ("/", "too broad"),
            ("/home", "too broad"),
            (".", "too broad"),
            ("~/everything", "too broad"),
            ("~/.ssh", "holds keys"),
            ("~/Pictures/keys", "holds keys"),
            ("~/.config/reins", "Reins' own files"),
            ("~/.config", "Reins' own files"),
            ("~/nowhere/deeper", "does not exist"),
            ("/tmp", "always scratch"),
            ("/tmp/reins-x", "always scratch"),
            ("~/.cache/x", "always scratch"),
            ("", "no folder given"),
        ] {
            let args: Vec<&str> = ["folders", "add"].into_iter().chain(scratch.iter().copied()).chain([path]).collect();
            let said = env.refused(&args);
            assert!(said.contains(why), "`{}`: {said}", args.join(" "));
        }
    }
    // A scratch folder that is a work folder, or holds one: its projects would become scratch.
    assert!(env.refused(&["folders", "temp", "~/Documents/dev"]).contains("work folder"));
    assert!(env.refused(&["folders", "temp", "~/Documents"]).contains("work folder ~/Documents/dev"));
    // A file is no folder.
    std::fs::write(home.join("Documents/notes/todo.txt"), "x").unwrap();
    assert!(env.refused(&["folders", "add", "~/Documents/notes/todo.txt"]).contains("not a folder"));
    // And a work folder inside a scratch folder is scratch whatever it is called.
    env.ok(&["folders", "temp", "~/Documents/dev/tmp"]);
    mkdir(&home.join("Documents/dev/tmp/projects"));
    assert!(env.refused(&["folders", "add", "~/Documents/dev/tmp/projects"]).contains("scratch folder"));
}

/// The owner approved deleting three different things in `~/Documents/dev/tmp`: it is suggested, with the command
/// that accepts it. Running that command is the owner's consent; before it, nothing is written anywhere.
#[test]
fn approved_deletes_suggest_a_scratch_folder_and_the_command_accepts_it() {
    let env = Env::new();
    let rm = |x: &str| format!("rm -rf ~/Documents/dev/tmp/{x}");
    let (a, b, c) = (rm("unpacked"), rm("old-build"), rm("iso"));
    env.journal(&[("approved", "phone", &a, ""), ("approved", "phone", &b, ""), ("approved", "phone", &c, "")]);
    let before = env.config();
    let listed = env.ok(&["folders"]);
    assert!(listed.contains("Suggested scratch folders:"), "{listed}");
    assert!(listed.contains("~/Documents/dev/tmp  (you approved 3 deletes in it in the last 14 days)"), "{listed}");
    assert!(listed.contains("reins folders add --scratch ~/Documents/dev/tmp"), "{listed}");
    assert_eq!(env.config(), before, "a suggestion writes nothing");
    assert!(!listed.contains("~/Documents/dev/tmp  set here"), "suggested is not set: {listed}");

    // Accepted: it is a scratch folder of this computer, and no longer a suggestion.
    env.ok(&["folders", "add", "--scratch", "~/Documents/dev/tmp"]);
    let listed = env.ok(&["folders"]);
    assert!(!listed.contains("Suggested"), "{listed}");
    assert!(listed.contains("~/Documents/dev/tmp "), "{listed}");
}

/// What must not become a suggestion: approvals nobody gave, words in a command, a folder in a repository.
#[test]
fn nothing_is_suggested_from_what_the_owner_did_not_approve() {
    let env = Env::new();
    let rm = |x: &str| format!("rm -rf ~/Documents/dev/tmp/{x}");
    let (a, b, c, d) = (rm("a"), rm("b"), rm("c"), rm("d"));
    let suggested = |env: &Env| env.ok(&["folders"]).contains("Suggested");
    // Let through by the settings, decided by a mode or Autopilot, timed out, approved at the computer.
    let bypass = r#"{"autopilot":{"by":"bypass","correctable":true}}"#;
    let autopilot = r#"{"autopilot":{"by":"autopilot","correctable":true}}"#;
    env.journal(&[
        ("allowed", "settings", &a, ""),
        ("approved", "phone", &b, bypass),
        ("approved", "phone", &c, autopilot),
        ("timed_out", "phone", &d, ""),
        ("approved", "local", &a, ""),
        ("approved", "phone", &a, ""),
        ("approved", "phone", &b, ""),
    ]);
    assert!(!suggested(&env), "two approvals by the owner are not three");
    // Three approvals, and one "no" in the same folder.
    env.journal(&[
        ("approved", "phone", &a, ""),
        ("approved", "phone", &b, ""),
        ("approved", "phone", &c, ""),
        ("denied", "phone", &d, ""),
    ]);
    assert!(!suggested(&env));
    // Commands that only say the words.
    let (e1, e2, e3) = (format!("echo '{a}'"), format!("echo {b}"), format!("git commit -m '{c}'"));
    env.journal(&[("approved", "phone", &e1, ""), ("approved", "phone", &e2, ""), ("approved", "phone", &e3, "")]);
    assert!(!suggested(&env));
    // Real approvals, but the folder is a git repository, or is gone.
    env.journal(&[("approved", "phone", &a, ""), ("approved", "phone", &b, ""), ("approved", "phone", &c, "")]);
    assert!(suggested(&env));
    mkdir(&env.home().join("Documents/dev/tmp/.git"));
    assert!(!suggested(&env));
    std::fs::remove_dir_all(env.home().join("Documents/dev/tmp")).unwrap();
    assert!(!suggested(&env));
    // The projects folder itself, though three projects were deleted from it; and the home folder.
    let projects: Vec<String> = ["x", "y", "z"].iter().map(|p| format!("rm -rf ~/Documents/dev/{p}")).collect();
    let homes: Vec<String> = ["x", "y", "z"].iter().map(|p| format!("rm -rf ~/{p}")).collect();
    let all: Vec<(&str, &str, &str, &str)> =
        projects.iter().chain(&homes).map(|c| ("approved", "phone", c.as_str(), "")).collect();
    env.journal(&all);
    assert!(!suggested(&env));
}

/// The temp folder the command runs with (`$TMPDIR`) is suggested when it is none of the built-in ones.
#[test]
fn the_temp_folder_of_this_shell_is_suggested() {
    let env = Env::new();
    let tmp = env.home().join("scratch/tmp");
    mkdir(&tmp);
    let out = env.command(&["folders"]).env("TMPDIR", &tmp).output().unwrap();
    let listed = text(&out);
    assert!(listed.contains("~/scratch/tmp  (the temp folder here ($TMPDIR))"), "{listed}");
    assert!(listed.contains("reins folders add --scratch ~/scratch/tmp"), "{listed}");
    // A built-in one, or one that is set already, is not.
    let out = env.command(&["folders"]).env("TMPDIR", "/tmp").output().unwrap();
    assert!(!text(&out).contains("Suggested"), "{}", text(&out));
    env.ok(&["folders", "temp", "~/scratch/tmp"]);
    let out = env.command(&["folders"]).env("TMPDIR", &tmp).output().unwrap();
    assert!(!text(&out).contains("Suggested"), "{}", text(&out));
}
