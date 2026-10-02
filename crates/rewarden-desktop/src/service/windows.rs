//! The background service on Windows: `rewarden service install` (and `resume`) puts a windowless copy of this program
//! in the state directory (`%LOCALAPPDATA%\rewarden\rewarden-daemon.exe`), adds it to the user's `Run` key
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Reins`) so it runs `daemon` at every logon, and starts
//! it at once. It logs to `daemon.log` next to it. Stopping it (`service uninstall`, the restart after `rewarden update`)
//! goes through the daemon's control API (`POST shutdown`), since Windows has no signal to send it.
//!
//! Why this and not a scheduled task or a Windows service: a Windows service needs an administrator and runs outside
//! the user's desktop session (no approval dialogs); creating a logon-triggered scheduled task is refused to standard
//! users by some Windows versions and policies. Every user may write their own `Run` key, it runs in the desktop
//! session where the approval dialogs appear, and it shows (and can be switched off) in Task Manager's Startup apps.
//! What `Run` lacks is systemd's restart on failure; `rewarden resume` starts the daemon again when it is not running.
//!
//! Why a copy: `rewarden.exe` is a console program, and Windows gives a console program started at logon a console
//! window. The copy differs only in the PE header's subsystem field ([`crate::win::gui_copy`]), so Windows starts it
//! without a window, the way `pythonw.exe` relates to `python.exe`. It is made again from the running program at every
//! install and after every update.

use std::path::{Path, PathBuf};

use crate::config::Paths;

/// The user's `Run` key.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// The value in it (what Task Manager's Startup apps lists).
pub const RUN_VALUE: &str = "Reins";
/// The windowless copy, in the state directory.
pub const DAEMON_EXE: &str = "rewarden-daemon.exe";
pub const LOG_FILE: &str = "daemon.log";

/// The windowless copy that runs the daemon.
#[must_use]
pub fn daemon_exe(paths: &Paths) -> PathBuf {
    paths.state_dir.join(DAEMON_EXE)
}

/// Where the background daemon logs.
#[must_use]
pub fn log_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join(LOG_FILE)
}

/// The command line in the `Run` value: `"…\rewarden-daemon.exe" daemon --log-file "…\daemon.log"`.
#[must_use]
pub fn run_command(daemon: &Path, log: &Path) -> String {
    let arg = |p: &Path| crate::win::command_line_arg(&p.to_string_lossy());
    format!("{} daemon --log-file {}", arg(daemon), arg(log))
}

/// `HKCU\…\Run\Reins`, for messages.
#[must_use]
pub fn shown() -> String {
    format!(r"{RUN_KEY}\{RUN_VALUE}")
}

/// Whether the `Run` value (as `reg query` printed it) starts this state directory's daemon copy.
#[must_use]
pub fn starts_our_daemon(value: &str, paths: &Paths) -> bool {
    value.to_ascii_lowercase().contains(&daemon_exe(paths).to_string_lossy().to_ascii_lowercase())
}

/// The line `rewarden status` shows for the service, from the `Run` value (`None`: not there).
#[must_use]
pub fn describe(value: Option<&str>, paths: &Paths) -> String {
    match value {
        Some(v) if starts_our_daemon(v, paths) => format!("starts at logon ({})", shown()),
        Some(v) => format!("{} starts another program: {v} (`rewarden service install` replaces it)", shown()),
        None => "not installed (`rewarden resume` or `rewarden service install`)".to_owned(),
    }
}

/// What starts the daemon: `Start-Process` goes through the Windows shell, which starts it without this process's
/// handles. The program, its arguments and its directory come from the environment, never from the script.
pub const START_SCRIPT: &str = "Start-Process -FilePath $env:REWARDEN_START_PROGRAM -ArgumentList \
     $env:REWARDEN_START_ARGS -WorkingDirectory $env:REWARDEN_START_DIR";

#[cfg(windows)]
pub use self::imp::{install, installed, restart_if_installed, uninstall};

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::{Paths, RUN_KEY, RUN_VALUE, START_SCRIPT, daemon_exe, log_file, run_command, shown, starts_our_daemon};
    use crate::config::Config;
    use crate::control::{Client, ClientError};
    use crate::win;

    fn reg(args: &[&str]) -> Result<std::process::Output, String> {
        win::hidden(Command::new(win::system32("reg.exe")).args(args))
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run reg.exe: {e}"))
    }

    /// The `Run` value, when it is there.
    pub fn installed() -> Result<Option<String>, String> {
        let out = reg(&["query", RUN_KEY, "/v", RUN_VALUE])?;
        if !out.status.success() {
            // Exit 1: no such value.
            return Ok(None);
        }
        Ok(win::reg_value(&String::from_utf8_lossy(&out.stdout), RUN_VALUE))
    }

    fn register(command: &str) -> Result<(), String> {
        let out = reg(&["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", command, "/f"])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("cannot write {}: {}", shown(), String::from_utf8_lossy(&out.stderr).trim()))
        }
    }

    /// (Re)makes the windowless copy of `exe`; it may be running (it is then moved aside).
    fn write_daemon(paths: &Paths, exe: &Path) -> Result<PathBuf, String> {
        paths.ensure().map_err(|e| e.to_string())?;
        let bytes = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
        let gui = win::gui_copy(&bytes).map_err(|e| format!("{}: {e}", exe.display()))?;
        let daemon = daemon_exe(paths);
        crate::update::replace_moving_aside(&daemon, &gui)?;
        Ok(daemon)
    }

    /// Starts the daemon copy on its own, through the shell (`Start-Process`). Not as a child of this process: Rust
    /// starts children with every inheritable handle of this process, and when this command's output is captured (an
    /// agent running `rewarden resume`) the daemon would keep the capturing pipe open and whoever reads it would wait
    /// forever. The shell starts programs without inheriting handles. Should PowerShell fail, the daemon is started
    /// directly ([`start_child`]).
    fn start(paths: &Paths) -> Result<(), String> {
        let args = format!("daemon --log-file {}", win::command_line_arg(&log_file(paths).to_string_lossy()));
        let started = win::hidden(Command::new(win::system32(r"WindowsPowerShell\v1.0\powershell.exe")).args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            START_SCRIPT,
        ]))
        .env("REWARDEN_START_PROGRAM", daemon_exe(paths))
        .env("REWARDEN_START_ARGS", args)
        .env("REWARDEN_START_DIR", &paths.state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
        match started {
            Ok(s) if s.success() => Ok(()),
            _ => start_child(paths),
        }
    }

    /// Starts the daemon copy as a child: no window, out of this console's process group (Ctrl-C here does not stop
    /// it), and out of a job the terminal may kill on close when the job allows that.
    fn start_child(paths: &Paths) -> Result<(), String> {
        let daemon = daemon_exe(paths);
        let mut cmd = Command::new(&daemon);
        cmd.arg("daemon")
            .arg("--log-file")
            .arg(log_file(paths))
            // Not the terminal's directory, which Windows would then refuse to delete while the daemon runs.
            .current_dir(&paths.state_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let flags = win::CREATE_NO_WINDOW | win::CREATE_NEW_PROCESS_GROUP;
        if cmd.creation_flags(flags | win::CREATE_BREAKAWAY_FROM_JOB).spawn().is_ok() {
            return Ok(());
        }
        cmd.creation_flags(flags).spawn().map(drop).map_err(|e| format!("cannot start {}: {e}", daemon.display()))
    }

    async fn running(paths: &Paths, config: &Config) -> bool {
        match Client::new(paths, config.listen) {
            Ok(c) => c.status().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Stops the running daemon (whichever started it); `false` when none runs.
    async fn stop(paths: &Paths, config: &Config) -> Result<bool, String> {
        match Client::new(paths, config.listen) {
            Ok(c) => c.shutdown(Duration::from_secs(10)).await.map_err(|e| e.to_string()),
            Err(ClientError::NotRunning) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Makes the windowless copy of `exe`, registers it to run at logon and, when `start` and no daemon runs yet,
    /// starts it. Returns what was installed, for the message.
    pub async fn install(paths: &Paths, config: &Config, exe: &Path, start_now: bool) -> Result<String, String> {
        let daemon = write_daemon(paths, exe)?;
        register(&run_command(&daemon, &log_file(paths)))?;
        if start_now && !running(paths, config).await {
            start(paths)?;
        }
        Ok(format!("{} ({})", shown(), daemon.display()))
    }

    /// After `rewarden update` replaced `exe`: when the service is installed, makes its copy again from the new
    /// program and, when the daemon was running, restarts it. `None` when no service is installed.
    pub async fn restart_if_installed(paths: &Paths, config: &Config, exe: &Path) -> Result<Option<String>, String> {
        if !installed()?.is_some_and(|v| starts_our_daemon(&v, paths)) {
            return Ok(None);
        }
        let was_running = stop(paths, config).await?;
        write_daemon(paths, exe)?;
        crate::update::remove_set_aside(&daemon_exe(paths));
        if was_running {
            start(paths)?;
            return Ok(Some("Restarted the background service.".to_owned()));
        }
        Ok(Some("The background service runs the new version from the next logon.".to_owned()))
    }

    /// Stops the daemon (when `stop_now`), removes the `Run` value and the copy. `Ok(false)` when it was not installed.
    pub async fn uninstall(paths: &Paths, config: &Config, stop_now: bool) -> Result<bool, String> {
        let daemon = daemon_exe(paths);
        if installed()?.is_none() {
            // A copy an earlier uninstall could not delete yet.
            std::fs::remove_file(&daemon).ok();
            return Ok(false);
        }
        if stop_now {
            stop(paths, config).await?;
        }
        let out = reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])?;
        if !out.status.success() {
            return Err(format!("cannot remove {}: {}", shown(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        // The stopped daemon may take a moment to exit; still running (not stopped), the copy goes with the next
        // uninstall, install or update.
        for _ in 0..20 {
            if std::fs::remove_file(&daemon).is_ok() || !daemon.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        crate::update::remove_set_aside(&daemon);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_starts_the_windowless_copy_with_its_log() {
        let paths = Paths {
            config_dir: PathBuf::from(r"C:\Users\Jo Doe\AppData\Roaming\rewarden"),
            state_dir: PathBuf::from(r"C:\Users\Jo Doe\AppData\Local\rewarden"),
        };
        let command = run_command(&daemon_exe(&paths), &log_file(&paths));
        let state = r"C:\Users\Jo Doe\AppData\Local\rewarden";
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            command,
            format!("\"{state}{sep}rewarden-daemon.exe\" daemon --log-file \"{state}{sep}daemon.log\"")
        );
        assert!(starts_our_daemon(&command, &paths));
        assert!(starts_our_daemon(&command.to_ascii_uppercase(), &paths), "paths compare ignoring case");
        assert!(describe(Some(&command), &paths).starts_with("starts at logon"));
        assert!(describe(Some(r"C:\other.exe"), &paths).contains("another program"));
        assert!(describe(None, &paths).starts_with("not installed"));
        assert_eq!(shown(), r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Reins");
    }

    #[test]
    fn the_start_script_takes_everything_from_the_environment() {
        for var in ["$env:REWARDEN_START_PROGRAM", "$env:REWARDEN_START_ARGS", "$env:REWARDEN_START_DIR"] {
            assert!(START_SCRIPT.contains(var), "{var}");
        }
        assert!(!START_SCRIPT.contains("-NoNewWindow"), "that would start it as a child, with this process's handles");
    }
}
