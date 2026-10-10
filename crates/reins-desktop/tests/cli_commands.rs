//! The newer commands from the command line, on every platform CI runs (Linux, macOS, Windows), in a temporary home:
//! what `--help` shows, `reins doctor` on a computer that is not set up, work sessions without one running, `reins
//! test` and `reins vault` without a phone, and `reins uninstall`. None of them may wait for a phone or a prompt.

use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Nothing listens on this port: the daemon is "not running".
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("config.toml"), "listen = \"127.0.0.1:9\"\n[notify]\nphone = false\n").unwrap();
        Self {
            dir,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let home = self.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let started = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_reins"))
            .args(args)
            .current_dir(&home)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", home.join("AppData").join("Local"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("REINS_CONFIG_DIR", self.dir.path().join("config"))
            .env("REINS_STATE_DIR", self.dir.path().join("state"))
            .output()
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(60), "`reins {}` waited for something", args.join(" "));
        out
    }
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn help_starts_with_what_a_new_user_needs_and_hides_the_plumbing() {
    let out = Env::new().run(&["--help"]);
    assert!(out.status.success());
    let help = text(&out);
    let commands: Vec<&str> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(&commands[..3], ["setup", "test", "doctor"], "{help}");
    for shown in ["allow", "session", "uninstall", "vault"] {
        assert!(commands.contains(&shown), "{shown}: {help}");
    }
    for hidden in ["daemon", "hook", "mcp"] {
        assert!(!commands.contains(&hidden), "{hidden}: {help}");
    }
    assert!(help.contains("New here?"), "{help}");
}

#[test]
fn doctor_on_a_computer_that_is_not_set_up_says_what_to_do_and_exits_1() {
    let out = Env::new().run(&["doctor"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains("Paired with your phone") && said.contains("reins setup"), "{said}");
    assert!(said.contains("Background service") && said.contains("reins resume"), "{said}");
    assert!(said.contains("Something needs fixing"), "{said}");
}

#[test]
fn work_sessions_need_something_to_allow_and_a_phone() {
    let env = Env::new();
    let status = env.run(&["session"]);
    assert!(status.status.success());
    assert!(text(&status).contains("No work session runs here"), "{}", text(&status));
    let end = env.run(&["session", "end"]);
    assert!(end.status.success() && text(&end).contains("No work session runs here"), "{}", text(&end));
    // Not in a repository, nothing to read: refused before anything is asked.
    let nothing = env.run(&["allow", "2h"]);
    assert!(!nothing.status.success());
    assert!(text(&nothing).contains("nothing to allow"), "{}", text(&nothing));
    let bad = env.run(&["allow", "soon", "--read", "mail"]);
    assert!(!bad.status.success() && text(&bad).contains("duration"), "{}", text(&bad));
    // Something to allow, but no phone: said at once.
    let unpaired = env.run(&["allow", "1h", "--read", "mail"]);
    assert!(!unpaired.status.success());
    assert!(text(&unpaired).contains("reins login"), "{}", text(&unpaired));
}

#[test]
fn test_and_vault_need_a_phone_and_say_so_at_once() {
    let env = Env::new();
    let test = env.run(&["test"]);
    assert!(!test.status.success());
    assert!(text(&test).contains("reins setup"), "{}", text(&test));
    let vault = env.run(&["vault", "list"]);
    assert!(!vault.status.success(), "{}", text(&vault));
}

#[test]
fn uninstall_with_nothing_installed_reports_every_step() {
    // On Windows the service step reads and writes this user's real `Run` key: only where CI allows it.
    if cfg!(windows) && std::env::var_os("REINS_TEST_WINDOWS_SERVICE").is_none() {
        return;
    }
    let out = Env::new().run(&["uninstall"]);
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    for step in ["Send git directly again", "SSH agent", "Background service", "Sign out: was not paired"] {
        assert!(said.contains(step), "{step}: {said}");
    }
}
