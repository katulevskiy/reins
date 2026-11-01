//! `reins service install`: a systemd user unit (Linux) or a launchd agent (macOS) that runs `reins daemon` at
//! login and restarts it when it fails. On Windows, see [`windows`]: an entry in the user's `Run` key that starts a
//! windowless copy of the program at logon, started at once as well.

pub mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

pub const SYSTEMD_UNIT: &str = "reins.service";
pub const LAUNCHD_LABEL: &str = "dev.reins.daemon";
/// The name Reins had before, spelled in two parts: the branding check keeps it out of everything else, and only the
/// retirement of the old service below needs it.
pub const LEGACY_NAME: &str = concat!("re", "warden");
/// The service Reins installed under its old name: it listens on the same port, so it goes first.
pub const LEGACY_SYSTEMD_UNIT: &str = concat!("re", "warden", ".service");
pub const LEGACY_LAUNCHD_LABEL: &str = concat!("dev.", "re", "warden", ".daemon");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Systemd,
    Launchd,
}

impl Manager {
    /// This system's service manager (Windows has none: [`windows`] does the same job).
    pub fn current() -> Result<Self, String> {
        if cfg!(target_os = "macos") {
            Ok(Self::Launchd)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Systemd)
        } else {
            Err("installing a service is supported on Linux (systemd), macOS (launchd) and Windows; run `reins daemon` instead".to_owned())
        }
    }

    /// Where the service file goes under `home`.
    #[must_use]
    pub fn file(self, home: &Path) -> PathBuf {
        self.file_named(home, SYSTEMD_UNIT, LAUNCHD_LABEL)
    }

    /// Where the old-name service file is under `home`.
    #[must_use]
    pub fn legacy_file(self, home: &Path) -> PathBuf {
        self.file_named(home, LEGACY_SYSTEMD_UNIT, LEGACY_LAUNCHD_LABEL)
    }

    fn file_named(self, home: &Path, unit: &str, label: &str) -> PathBuf {
        match self {
            Self::Systemd => home.join(".config/systemd/user").join(unit),
            Self::Launchd => home.join("Library/LaunchAgents").join(format!("{label}.plist")),
        }
    }

    #[must_use]
    pub fn contents(self, exe: &Path, home: &Path) -> String {
        match self {
            Self::Systemd => systemd_unit(exe),
            Self::Launchd => launchd_plist(exe, &home.join("Library/Logs/reins.log")),
        }
    }
}

/// A systemd `ExecStart` word: quoted, with systemd's specifiers (`%`) and variables (`$`) escaped.
fn systemd_word(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{escaped}\"")
}

#[must_use]
pub fn systemd_unit(exe: &Path) -> String {
    format!(
        "[Unit]\nDescription=Reins desktop app (git proxy)\n\n[Service]\nExecStart={} daemon\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        systemd_word(&exe.display().to_string())
    )
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[must_use]
pub fn launchd_plist(exe: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>daemon</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        xml(&exe.display().to_string()),
        xml(&log.display().to_string())
    )
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program).args(args).status().map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}

/// Whether a systemd user manager answers here (`systemctl --user` works). Containers, WSL and minimal systems often
/// have none, even when `XDG_RUNTIME_DIR` and a session bus are set, so it is asked rather than guessed.
#[must_use]
pub fn systemd_user_available() -> bool {
    user_manager_answers("systemctl")
}

fn user_manager_answers(systemctl: &str) -> bool {
    Command::new(systemctl)
        .args(["--user", "show-environment"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What `reins setup` says when it installs no service because there is no systemd user session.
#[must_use]
pub fn no_systemd_note() -> String {
    "No background service: this system has no systemd user session (common in containers, WSL and minimal \
     installs). AI tools still ask through Reins without it. To send git through Reins too, keep `reins daemon` \
     running (in a terminal, or from your own startup scripts), then run `reins resume`."
        .to_owned()
}

/// How `reins resume` gets a daemon to route git through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// One already runs (the service, or `reins daemon` started by hand).
    Running,
    /// Install and start the service.
    Install,
    /// No service manager to start it with: the user starts `reins daemon` by hand.
    ByHand,
}

/// What `reins resume` does: `running` whether a daemon answers; `user_manager` asked only on Linux, and only when
/// none runs.
#[must_use]
pub fn start(running: bool, linux: bool, user_manager: impl FnOnce() -> bool) -> Start {
    if running {
        Start::Running
    } else if linux && !user_manager() {
        Start::ByHand
    } else {
        Start::Install
    }
}

/// Why `reins resume` stopped when there is nothing to start the daemon with.
#[must_use]
pub fn start_by_hand_note() -> String {
    "this system has no systemd user session, so Reins cannot start its background service. Start `reins daemon` \
     first (in another terminal, or from your own startup scripts), then run `reins resume` again. git was left as \
     it was."
        .to_owned()
}

/// Writes the service file for `exe`; when `start`, enables and starts it. Returns the file.
pub fn install(manager: Manager, home: &Path, exe: &Path, start: bool) -> Result<PathBuf, String> {
    let file = manager.file(home);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&file, manager.contents(exe, home)).map_err(|e| format!("{}: {e}", file.display()))?;
    retire_legacy(manager, home, start)?;
    if start {
        let path = file.display().to_string();
        match manager {
            Manager::Systemd => {
                run("systemctl", &["--user", "daemon-reload"])?;
                run("systemctl", &["--user", "enable", "--now", SYSTEMD_UNIT])?;
            }
            Manager::Launchd => run("launchctl", &["load", "-w", &path])?,
        }
    }
    Ok(file)
}

/// Restarts the installed, running service so it runs the binary that was just installed. Says what it did; `None`
/// when no service is installed.
pub fn restart_if_installed(manager: Manager, home: &Path) -> Result<Option<String>, String> {
    if !manager.file(home).exists() {
        return Ok(None);
    }
    match manager {
        Manager::Systemd => run("systemctl", &["--user", "try-restart", SYSTEMD_UNIT])?,
        Manager::Launchd => {
            #[cfg(unix)]
            let uid = rustix::process::getuid().as_raw();
            #[cfg(not(unix))]
            let uid = 0;
            let target = format!("gui/{uid}/{LAUNCHD_LABEL}");
            run("launchctl", &["kickstart", "-k", &target])?;
        }
    }
    Ok(Some("Restarted the background service.".to_owned()))
}

/// Stops (when `stop`) and removes the service. `Ok(false)` when it was not installed.
pub fn uninstall(manager: Manager, home: &Path, stop: bool) -> Result<bool, String> {
    remove(manager, &manager.file(home), SYSTEMD_UNIT, stop)
}

/// Stops (when `stop`) and removes the service Reins installed under its old name, which would hold the
/// daemon's port. `Ok(false)` when there is none.
pub fn retire_legacy(manager: Manager, home: &Path, stop: bool) -> Result<bool, String> {
    remove(manager, &manager.legacy_file(home), LEGACY_SYSTEMD_UNIT, stop)
}

fn remove(manager: Manager, file: &Path, unit: &str, stop: bool) -> Result<bool, String> {
    if !file.exists() {
        return Ok(false);
    }
    if stop {
        let path = file.display().to_string();
        // Already stopped is fine.
        match manager {
            Manager::Systemd => run("systemctl", &["--user", "disable", "--now", unit]).ok(),
            Manager::Launchd => run("launchctl", &["unload", "-w", &path]).ok(),
        };
    }
    std::fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if stop && manager == Manager::Systemd {
        run("systemctl", &["--user", "daemon-reload"]).ok();
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systemd_unit_runs_the_daemon_and_quotes_the_path() {
        let unit = systemd_unit(Path::new("/opt/my apps/reins%1$x"));
        assert!(unit.contains("ExecStart=\"/opt/my apps/reins%%1$$x\" daemon\n"), "{unit}");
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn the_launchd_agent_runs_the_daemon_and_escapes_the_path() {
        let plist = launchd_plist(Path::new("/Apps/R&D/reins"), Path::new("/Users/me/Library/Logs/reins.log"));
        assert!(plist.contains("<string>/Apps/R&amp;D/reins</string><string>daemon</string>"), "{plist}");
        assert!(plist.contains("<string>dev.reins.daemon</string>"));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_or_failing_systemctl_means_no_user_session() {
        assert!(user_manager_answers("true"));
        assert!(!user_manager_answers("false"), "systemctl --user fails: no user manager");
        assert!(!user_manager_answers("/nonexistent/systemctl"), "no systemctl at all");
        let note = no_systemd_note();
        assert!(note.contains("`reins daemon`") && note.contains("`reins resume`"), "{note}");
    }

    #[test]
    fn resume_starts_the_daemon_by_hand_only_without_a_user_manager() {
        let never = || -> bool { panic!("asked the service manager needlessly") };
        assert_eq!(start(true, true, never), Start::Running);
        assert_eq!(start(true, false, never), Start::Running);
        assert_eq!(start(false, false, never), Start::Install, "macOS: launchd");
        assert_eq!(start(false, true, || true), Start::Install, "systemd as before");
        assert_eq!(start(false, true, || false), Start::ByHand);
        let note = start_by_hand_note();
        assert!(note.contains("`reins daemon`") && note.contains("`reins resume`"), "{note}");
    }

    #[test]
    fn install_writes_and_uninstall_removes_the_file_without_touching_the_system() {
        let home = tempfile::tempdir().unwrap();
        for manager in [Manager::Systemd, Manager::Launchd] {
            let file = install(manager, home.path(), Path::new("/usr/bin/reins"), false).unwrap();
            assert_eq!(file, manager.file(home.path()));
            assert!(std::fs::read_to_string(&file).unwrap().contains("/usr/bin/reins"));
            assert!(uninstall(manager, home.path(), false).unwrap());
            assert!(!file.exists());
            assert!(!uninstall(manager, home.path(), false).unwrap());
        }
        assert!(home.path().join(".config/systemd/user").is_dir());
    }

    #[test]
    fn install_removes_the_legacy_service_that_holds_the_port() {
        let home = tempfile::tempdir().unwrap();
        for manager in [Manager::Systemd, Manager::Launchd] {
            let legacy = manager.legacy_file(home.path());
            std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
            std::fs::write(&legacy, "old").unwrap();
            install(manager, home.path(), Path::new("/usr/bin/reins"), false).unwrap();
            assert!(!legacy.exists(), "{}", legacy.display());
            assert!(manager.file(home.path()).exists());
            assert!(!retire_legacy(manager, home.path(), false).unwrap());
        }
    }
}
