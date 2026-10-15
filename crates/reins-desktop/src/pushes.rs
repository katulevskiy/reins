//! What this computer keeps of each approved push, for Activity's commit inspection (`design/signal/ASK-TEXT.md`):
//! its commits (SHA, subject, author, lines added and removed, the files with theirs) and the diff text of each file
//! while about 1 MB per push lasts. One JSON file per push, named by its activity entry's id, in `pushes/` in the state
//! directory (0600, the folder 0700); files older than [`KEEP`] go whenever a push is kept.
//!
//! The diff text is the repository's own content: whoever shows it must treat it as untrusted text.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub use reins_proto::desktop::{CommitInfo, FileChange, FileStatus, PushSummary, RefChange, RefUpdate};
use serde::{Deserialize, Serialize};

/// How long a push is kept.
pub const KEEP: Duration = Duration::from_hours(30 * 24);
/// The folder in the state directory.
const DIR: &str = "pushes";
/// The largest file read back (a push's patches are kept under 1 MB; JSON escaping adds some).
const MAX_FILE: u64 = 4 << 20;

/// One approved push.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Push {
    /// Unix seconds when it was approved.
    pub at: i64,
    /// The repository as the proxy names it (`github.com/acme/web`).
    pub repo: String,
    /// Commits and files, with the diff text kept.
    pub summary: PushSummary,
}

/// The file of entry `id`; `None` for an id that is not one an entry has (letters, digits, `-`, `_`).
fn file(state_dir: &Path, id: &str) -> Option<PathBuf> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| state_dir.join(DIR).join(format!("{id}.json")))
}

/// Keeps `push` under entry `id`, then removes the pushes older than [`KEEP`].
pub fn keep(state_dir: &Path, id: &str, push: &Push) -> Result<(), String> {
    let path = file(state_dir, id).ok_or("not an activity id")?;
    let dir = state_dir.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec(push).map_err(|e| e.to_string())?;
    crate::config::write_private(&path, &json).map_err(|e| format!("{}: {e}", path.display()))?;
    prune(state_dir, SystemTime::now());
    Ok(())
}

/// The push kept under entry `id`, if there is one (and it can be read).
#[must_use]
pub fn load(state_dir: &Path, id: &str) -> Option<Push> {
    let path = file(state_dir, id)?;
    if std::fs::metadata(&path).ok()?.len() > MAX_FILE {
        return None;
    }
    serde_json::from_slice(&std::fs::read(&path).ok()?).ok()
}

/// Removes the pushes kept longer than [`KEEP`] before `now`.
pub fn prune(state_dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(state_dir.join(DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        let ours = entry.path().extension().is_some_and(|x| x == "json");
        if old && ours {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push() -> Push {
        Push {
            at: 1_800_000_000,
            repo: "github.com/acme/web".to_owned(),
            summary: PushSummary::default(),
        }
    }

    #[test]
    fn a_push_is_kept_privately_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), "abc_123-x", &push()).unwrap();
        assert_eq!(load(dir.path(), "abc_123-x"), Some(push()));
        assert_eq!(load(dir.path(), "other"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.path().join("pushes/abc_123-x.json")), 0o600);
            assert_eq!(mode(&dir.path().join("pushes")), 0o700);
        }
    }

    #[test]
    fn ids_name_files_in_the_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "../x", "a/b", "a.b", "a\\b", &"x".repeat(65)] {
            assert!(keep(dir.path(), bad, &push()).is_err(), "{bad}");
            assert_eq!(load(dir.path(), bad), None);
        }
    }

    #[test]
    fn pushes_older_than_thirty_days_go() {
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), "old", &push()).unwrap();
        keep(dir.path(), "new", &push()).unwrap();
        prune(dir.path(), SystemTime::now() + KEEP - Duration::from_secs(60));
        assert!(load(dir.path(), "old").is_some(), "not yet thirty days");
        prune(dir.path(), SystemTime::now() + KEEP + Duration::from_secs(60));
        assert!(load(dir.path(), "old").is_none() && load(dir.path(), "new").is_none());
    }
}
