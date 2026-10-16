//! The user's own git config is never touched by a flow run on a temporary home. A shell that exports
//! `XDG_CONFIG_HOME` (and has no `~/.gitconfig`) sends `git config --global` to `$XDG_CONFIG_HOME/git/config`, so a
//! test that sets only `HOME` would set up, pause or uninstall the user's real git routing.
//!
//! The flows run in a copy of this test binary started as such a shell would start it: `HOME` and `XDG_CONFIG_HOME`
//! point at a stand-in for the user's own, whose `git/config` holds Reins' rules. Afterwards that stand-in must be
//! byte for byte as it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use reins_desktop::config::{Config, home_env};
use reins_desktop::setup::{Git, Scope};

/// Set in the copy that runs the flows.
const CHILD: &str = "REINS_TEST_REAL_HOME_CHILD";

const REAL_RULES: &str = "[user]\n\tname = Real User\n[url \"http://127.0.0.1:7457/github.com/\"]\n\tinsteadOf = \
                          https://github.com/\n\tinsteadOf = git@github.com:\n\tinsteadOf = ssh://git@github.com/\n";

/// Every file under `dir` and its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path.clone());
                files.insert(path, Vec::new());
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn flows_on_a_temporary_home_leave_the_real_git_config_alone() {
    if std::env::var_os(CHILD).is_some() {
        flows();
        return;
    }
    let real = tempfile::tempdir().unwrap();
    let xdg = real.path().join(".config");
    std::fs::create_dir_all(xdg.join("git")).unwrap();
    let sentinel = xdg.join("git").join("config");
    std::fs::write(&sentinel, REAL_RULES).unwrap();
    let before = snapshot(real.path());
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "flows_on_a_temporary_home_leave_the_real_git_config_alone", "--nocapture"])
        .env(CHILD, "1")
        .env("HOME", real.path())
        .env("USERPROFILE", real.path())
        .env("XDG_CONFIG_HOME", &xdg)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("REINS_CONFIG_DIR")
        .env_remove("REINS_STATE_DIR")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the flows failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"), "the flows did not run: {out:?}");
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), REAL_RULES, "the real git config changed");
    assert_eq!(snapshot(real.path()), before, "something under the real home changed");
}

/// What the tests do on a temporary home, while `HOME` and `XDG_CONFIG_HOME` name the "real" one.
fn flows() {
    // The library's `Git` given only `HOME` (as the app's test backend was).
    let home = tempfile::tempdir().unwrap();
    let git = Git::default().env("HOME", home.path());
    let config = Config::default();
    assert!(git.hosts_set_up(&Scope::Global, &config).unwrap().is_empty(), "the real rules are not seen");
    assert_eq!(git.setup_hosts(&Scope::Global, &config).unwrap().0.len(), 3);
    let written = std::fs::read_to_string(home.path().join(".gitconfig")).unwrap();
    assert!(written.contains("insteadOf = https://github.com/"), "{written}");
    assert_eq!(git.hosts_set_up(&Scope::Global, &config).unwrap(), ["github.com"]);
    assert_eq!(git.unsetup_hosts(&Scope::Global, &config).unwrap(), 3, "only the temporary home's rules");

    // The `reins` binary on a temporary home, as the command line tests run it.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::write(config_dir.join("config.toml"), format!("listen = \"127.0.0.1:{port}\"\nmode = \"local\"\n"))
        .unwrap();
    let run = |args: &[&str]| -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_reins"))
            .args(args)
            .envs(home_env(&home))
            .env("REINS_CONFIG_DIR", &config_dir)
            .env("REINS_STATE_DIR", dir.path().join("state"))
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(out.status.success(), "reins {args:?}: {out:?}");
        out
    };
    let gitconfig = || std::fs::read_to_string(home.join(".gitconfig")).unwrap_or_default();
    run(&["git", "setup"]);
    assert!(gitconfig().contains(&format!("http://127.0.0.1:{port}/github.com/")), "{}", gitconfig());
    run(&["status"]);
    run(&["pause"]);
    assert!(!gitconfig().contains("insteadOf"), "{}", gitconfig());
    run(&["git", "setup"]);
    run(&["git", "unsetup"]);
    run(&["ssh", "setup"]);
    run(&["ssh", "unsetup"]);
    // On Windows `uninstall` would edit this user's real `Run` key.
    #[cfg(not(windows))]
    {
        run(&["git", "setup"]);
        let said = String::from_utf8_lossy(&run(&["uninstall"]).stdout).into_owned();
        assert!(said.contains("Send git directly again: removed 3 rule(s)"), "{said}");
    }
}
