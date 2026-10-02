//! Which harnesses are on this computer: their settings directory under the home directory, their program on the
//! `PATH` or in the usual install places (an app started from the desktop gets a short `PATH`), or, for Cursor, the
//! installed app. `rewarden harness add --all` and the Reins app add Reins to exactly these.

use std::path::{Path, PathBuf};

use super::Harness;

/// What marks `h` as installed: files under `home`, and program names.
fn marks(h: Harness) -> (&'static [&'static str], &'static [&'static str]) {
    match h {
        Harness::ClaudeCode => (&[".claude", ".claude.json"], &["claude"]),
        Harness::Codex => (&[".codex"], &["codex"]),
        Harness::Gemini => (&[".gemini"], &["gemini"]),
        Harness::Cursor => (&[".cursor"], &["cursor", "cursor-agent"]),
    }
}

/// Where command line tools are usually installed, besides the `PATH`.
fn usual_bin_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin"]
        .iter()
        .map(|d| home.join(d))
        .collect();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from));
    if cfg!(windows)
        && let Some(appdata) = std::env::var_os("APPDATA")
    {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

/// Installed apps that count, for harnesses that are apps.
fn app_paths(h: Harness, home: &Path) -> Vec<PathBuf> {
    match h {
        Harness::Cursor if cfg!(target_os = "macos") => {
            vec![PathBuf::from("/Applications/Cursor.app"), home.join("Applications/Cursor.app")]
        }
        Harness::Cursor if cfg!(windows) => std::env::var_os("LOCALAPPDATA")
            .map(|d| vec![PathBuf::from(d).join("Programs").join("cursor")])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn program_in(dir: &Path, name: &str) -> bool {
    if cfg!(windows) {
        ["exe", "cmd", "bat", "ps1"].iter().any(|ext| dir.join(format!("{name}.{ext}")).is_file())
    } else {
        dir.join(name).is_file()
    }
}

/// Whether `h` is installed for the user whose home directory is `home`.
#[must_use]
pub fn found(h: Harness, home: &Path) -> bool {
    let (files, programs) = marks(h);
    if files.iter().any(|f| home.join(f).exists()) || app_paths(h, home).iter().any(|p| p.exists()) {
        return true;
    }
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(usual_bin_dirs(home));
    dirs.iter().any(|dir| programs.iter().any(|p| program_in(dir, p)))
}

/// The harnesses installed for `home`, in [`Harness::ALL`] order.
#[must_use]
pub fn all_found(home: &Path) -> Vec<Harness> {
    Harness::ALL.into_iter().filter(|&h| found(h, home)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_directory_marks_a_harness_as_installed() {
        let home = tempfile::tempdir().unwrap();
        assert!(!home.path().join(".codex").exists());
        std::fs::create_dir(home.path().join(".codex")).unwrap();
        assert!(found(Harness::Codex, home.path()));
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        assert!(found(Harness::ClaudeCode, home.path()));
        let found = all_found(home.path());
        assert!(found.contains(&Harness::Codex) && found.contains(&Harness::ClaudeCode), "{found:?}");
    }

    #[test]
    fn a_program_in_the_home_bin_directory_counts() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) {
            "gemini.cmd"
        } else {
            "gemini"
        };
        std::fs::write(bin.join(name), "").unwrap();
        assert!(found(Harness::Gemini, home.path()));
    }
}
