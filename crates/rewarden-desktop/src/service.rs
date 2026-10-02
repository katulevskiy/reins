//! `rewarden service install`: a systemd user unit (Linux) or a launchd agent (macOS) that runs `rewarden daemon` at
//! login and restarts it when it fails.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const SYSTEMD_UNIT: &str = "rewarden.service";
pub const LAUNCHD_LABEL: &str = "dev.rewarden.daemon";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Systemd,
    Launchd,
}

impl Manager {
    /// This system's service manager.
    pub fn current() -> Result<Self, String> {
        if cfg!(target_os = "macos") {
            Ok(Self::Launchd)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Systemd)
        } else {
            Err("installing a service is supported on Linux (systemd) and macOS (launchd); run `rewarden daemon` instead".to_owned())
        }
    }

    /// Where the service file goes under `home`.
    #[must_use]
    pub fn file(self, home: &Path) -> PathBuf {
        match self {
            Self::Systemd => home.join(".config/systemd/user").join(SYSTEMD_UNIT),
            Self::Launchd => home.join("Library/LaunchAgents").join(format!("{LAUNCHD_LABEL}.plist")),
        }
    }

    #[must_use]
    pub fn contents(self, exe: &Path, home: &Path) -> String {
        match self {
            Self::Systemd => systemd_unit(exe),
            Self::Launchd => launchd_plist(exe, &home.join("Library/Logs/rewarden.log")),
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
        "[Unit]\nDescription=Rewarden desktop app (git proxy)\n\n[Service]\nExecStart={} daemon\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
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

/// Writes the service file for `exe`; when `start`, enables and starts it. Returns the file.
pub fn install(manager: Manager, home: &Path, exe: &Path, start: bool) -> Result<PathBuf, String> {
    let file = manager.file(home);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&file, manager.contents(exe, home)).map_err(|e| format!("{}: {e}", file.display()))?;
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
            let target = format!("gui/{}/{LAUNCHD_LABEL}", rustix::process::getuid().as_raw());
            run("launchctl", &["kickstart", "-k", &target])?;
        }
    }
    Ok(Some("Restarted the background service.".to_owned()))
}

/// Stops (when `stop`) and removes the service. `Ok(false)` when it was not installed.
pub fn uninstall(manager: Manager, home: &Path, stop: bool) -> Result<bool, String> {
    let file = manager.file(home);
    if !file.exists() {
        return Ok(false);
    }
    if stop {
        let path = file.display().to_string();
        // Already stopped is fine.
        match manager {
            Manager::Systemd => run("systemctl", &["--user", "disable", "--now", SYSTEMD_UNIT]).ok(),
            Manager::Launchd => run("launchctl", &["unload", "-w", &path]).ok(),
        };
    }
    std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
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
        let unit = systemd_unit(Path::new("/opt/my apps/rewarden%1$x"));
        assert!(unit.contains("ExecStart=\"/opt/my apps/rewarden%%1$$x\" daemon\n"), "{unit}");
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn the_launchd_agent_runs_the_daemon_and_escapes_the_path() {
        let plist = launchd_plist(Path::new("/Apps/R&D/rewarden"), Path::new("/Users/me/Library/Logs/rewarden.log"));
        assert!(plist.contains("<string>/Apps/R&amp;D/rewarden</string><string>daemon</string>"), "{plist}");
        assert!(plist.contains("<string>dev.rewarden.daemon</string>"));
    }

    #[test]
    fn install_writes_and_uninstall_removes_the_file_without_touching_the_system() {
        let home = tempfile::tempdir().unwrap();
        for manager in [Manager::Systemd, Manager::Launchd] {
            let file = install(manager, home.path(), Path::new("/usr/bin/rewarden"), false).unwrap();
            assert_eq!(file, manager.file(home.path()));
            assert!(std::fs::read_to_string(&file).unwrap().contains("/usr/bin/rewarden"));
            assert!(uninstall(manager, home.path(), false).unwrap());
            assert!(!file.exists());
            assert!(!uninstall(manager, home.path(), false).unwrap());
        }
        assert!(home.path().join(".config/systemd/user").is_dir());
    }
}
