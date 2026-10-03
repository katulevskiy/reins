//! "Open Reins at login": the app starts with `--background` (tray only) when the user logs in.
//!
//! | system  | how                                                                                   |
//! |---------|---------------------------------------------------------------------------------------|
//! | macOS   | a launch agent, `~/Library/LaunchAgents/com.reins2fa.desktop.plist` (loaded at next login) |
//! | Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Reins app`               |
//! | Linux   | an XDG autostart entry, `~/.config/autostart/reins.desktop`                           |
//!
//! This is the app. The daemon starts at login on its own, as the background service (`reins service`); on Windows
//! that is the `Reins` value in the same `Run` key, which this one does not touch.

use std::path::{Path, PathBuf};

/// The macOS launch agent's label (the app's bundle id).
pub const LABEL: &str = "com.reins2fa.desktop";
/// The value in the Windows `Run` key.
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_VALUE: &str = "Reins app";

/// What starts at login: the AppImage itself when running from one (its mount point changes every run), else this
/// program.
fn program() -> Result<PathBuf, String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|a| !a.is_empty()) {
        return Ok(PathBuf::from(appimage));
    }
    reins_desktop::update::current_executable()
}

#[cfg_attr(windows, allow(dead_code))]
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The macOS launch agent.
#[must_use]
#[cfg_attr(windows, allow(dead_code))]
pub fn launch_agent(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>--background</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
</dict>
</plist>
"#,
        xml(&exe.display().to_string())
    )
}

/// A Desktop Entry `Exec` word: quoted, with `"`, `` ` ``, `$` and `\` escaped and `%` doubled.
#[cfg_attr(windows, allow(dead_code))]
fn desktop_word(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The XDG autostart entry.
#[must_use]
#[cfg_attr(windows, allow(dead_code))]
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Reins\nComment=Keeps your AI agents on a leash\nExec={} --background\nIcon=reins\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        desktop_word(&exe.display().to_string())
    )
}

/// The file that makes the app start at login (macOS and Linux).
#[cfg_attr(windows, allow(dead_code))]
fn file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home.join(".config"), PathBuf::from);
        config.join("autostart").join("reins.desktop")
    }
}

/// Whether the app starts at login.
#[must_use]
pub fn enabled(home: &Path) -> bool {
    #[cfg(windows)]
    {
        let _ = home;
        imp::get().is_some()
    }
    #[cfg(not(windows))]
    {
        file(home).exists()
    }
}

/// Makes the app start at login, or stops it.
pub fn set(home: &Path, on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = home;
        if on {
            let exe = program()?;
            imp::put(&format!("{} --background", reins_desktop::win::command_line_arg(&exe.to_string_lossy())))
        } else {
            imp::delete()
        }
    }
    #[cfg(not(windows))]
    {
        let file = file(home);
        if !on {
            return match std::fs::remove_file(&file) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("{}: {e}", file.display())),
            };
        }
        let exe = program()?;
        let contents = if cfg!(target_os = "macos") {
            launch_agent(&exe)
        } else {
            desktop_entry(&exe)
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&file, contents).map_err(|e| format!("{}: {e}", file.display()))
    }
}

#[cfg(windows)]
mod imp {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

    use super::RUN_VALUE;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn get() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_READ).ok()?.get_value(RUN_VALUE).ok()
    }

    pub fn put(command: &str) -> Result<(), String> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN).map_err(|e| format!(r"HKCU\{RUN}: {e}"))?;
        key.set_value(RUN_VALUE, &command).map_err(|e| format!(r"HKCU\{RUN}\{RUN_VALUE}: {e}"))
    }

    pub fn delete() -> Result<(), String> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN, KEY_SET_VALUE)
            .map_err(|e| format!(r"HKCU\{RUN}: {e}"))?;
        match key.delete_value(RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(r"HKCU\{RUN}\{RUN_VALUE}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_agent_starts_the_app_in_the_background() {
        let plist = launch_agent(Path::new("/Applications/R&D/Reins.app/Contents/MacOS/reins-app"));
        assert!(
            plist.contains(
                "<string>/Applications/R&amp;D/Reins.app/Contents/MacOS/reins-app</string><string>--background</string>"
            ),
            "{plist}"
        );
        assert!(plist.contains("<string>com.reins2fa.desktop</string>"));
    }

    #[test]
    fn the_desktop_entry_quotes_the_program() {
        let entry = desktop_entry(Path::new("/home/me/Apps/Reins 100%.AppImage"));
        assert!(entry.contains("Exec=\"/home/me/Apps/Reins 100%%.AppImage\" --background\n"), "{entry}");
        assert!(desktop_entry(Path::new("/a/$x")).contains("Exec=\"/a/\\\\$x\" --background"));
    }

    #[cfg(not(windows))]
    #[test]
    fn set_writes_and_removes_the_file() {
        let home = tempfile::tempdir().unwrap();
        assert!(!enabled(home.path()));
        // XDG_CONFIG_HOME, when set, would point elsewhere.
        if std::env::var_os("XDG_CONFIG_HOME").is_none() {
            set(home.path(), true).unwrap();
            assert!(enabled(home.path()));
            assert!(file(home.path()).starts_with(home.path()));
            set(home.path(), false).unwrap();
            assert!(!enabled(home.path()));
            set(home.path(), false).unwrap();
        }
    }
}
