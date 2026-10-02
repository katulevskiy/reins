//! The `rewarden` command line: git setup on a temporary `HOME`, and status / pending / approve against a daemon
//! started by the binary itself.

use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.toml"),
            format!("listen = \"127.0.0.1:{port}\"\nmode = \"local\"\n[github]\ntoken = \"env:REWARDEN_TEST_TOKEN\"\n"),
        )
        .unwrap();
        Self {
            dir,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let home = self.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_rewarden"));
        c.args(args)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("REWARDEN_CONFIG_DIR", self.dir.path().join("config"))
            .env("REWARDEN_STATE_DIR", self.dir.path().join("state"))
            .env("REWARDEN_TEST_TOKEN", "t0ken");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn gitconfig(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("home/.gitconfig")).unwrap_or_default()
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn git_setup_and_unsetup_edit_only_the_given_home() {
    let env = Env::new(7457);
    let out = env.run(&["git", "setup"]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("http://127.0.0.1:7457/github.com/"));
    let config = env.gitconfig();
    for source in ["https://github.com/", "git@github.com:", "ssh://git@github.com/"] {
        assert!(config.contains(&format!("insteadOf = {source}")), "{config}");
    }
    assert!(stdout(&env.run(&["git", "setup"])).contains("already"));
    assert_eq!(env.gitconfig(), config);
    // git itself now rewrites GitHub URLs to the proxy.
    let rewritten = Command::new("git")
        .args(["ls-remote", "--get-url", "git@github.com:me/app.git"])
        .env("HOME", env.dir.path().join("home"))
        .env("GIT_CONFIG_GLOBAL", env.dir.path().join("home/.gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(env.dir.path())
        .output()
        .unwrap();
    assert_eq!(stdout(&rewritten).trim(), "http://127.0.0.1:7457/github.com/me/app.git");
    let out = env.run(&["git", "unsetup"]);
    assert!(stdout(&out).contains("removed 3 rule(s)"), "{out:?}");
    assert!(!env.gitconfig().contains("insteadOf"), "{}", env.gitconfig());
}

#[test]
fn status_says_when_the_daemon_is_not_running() {
    let env = Env::new(free_port());
    let out = env.run(&["status"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("not running"), "{}", stdout(&out));
    let out = env.run(&["pending"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not running"));
}

fn wait_for(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{} did not appear", path.display());
}

#[test]
fn the_daemon_binary_serves_the_cli_and_cleans_up_on_sigterm() {
    let env = Env::new(free_port());
    let mut daemon = env.command(&["daemon"]).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let token = env.dir.path().join("state/control.token");
    wait_for(&token);
    std::thread::sleep(Duration::from_millis(100));
    let out = env.run(&["status"]);
    let text = stdout(&out);
    assert!(text.contains("running on 127.0.0.1:"), "{text}");
    assert!(text.contains("the local policy on this computer"), "{text}");
    assert!(text.contains("not logged in"), "{text}");
    assert!(stdout(&env.run(&["pending"])).contains("Nothing is waiting."));
    let out = env.run(&["approve", "abcd1234"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("No pending approval abcd1234"));
    #[cfg(unix)]
    {
        let killed = Command::new("kill").args(["-TERM", &daemon.id().to_string()]).status().unwrap();
        assert!(killed.success());
        let status = daemon.wait().unwrap();
        assert!(status.success(), "{status:?}");
        assert!(!token.exists());
        let out = env.run(&["status"]);
        assert!(stdout(&out).contains("not running"));
    }
    #[cfg(not(unix))]
    {
        daemon.kill().ok();
        daemon.wait().ok();
    }
}

/// How the Windows background service is stopped (`service uninstall`, the restart after an update).
#[test]
fn the_daemon_stops_cleanly_when_the_control_api_asks() {
    let port = free_port();
    let env = Env::new(port);
    let mut daemon = env.command(&["daemon"]).stderr(std::process::Stdio::null()).spawn().unwrap();
    let token = env.dir.path().join("state/control.token");
    wait_for(&token);
    let paths = rewarden_desktop::config::Paths {
        config_dir: env.dir.path().join("config"),
        state_dir: env.dir.path().join("state"),
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let client = rewarden_desktop::control::Client::new(&paths, ([127, 0, 0, 1], port).into()).unwrap();
    let stopped = rt.block_on(client.shutdown(Duration::from_secs(10)));
    if stopped.is_err() {
        daemon.kill().ok();
    }
    let status = daemon.wait().unwrap();
    assert!(stopped.unwrap(), "it was running and stopped");
    assert!(status.success(), "{status:?}");
    assert!(!token.exists(), "the control token goes with it");
    assert!(!rt.block_on(client.shutdown(Duration::from_secs(1))).unwrap(), "not running any more");
}
