//! What the app remembers between runs (`app.json` in the state directory): who approved the pairing, whether the
//! first-time setup ran, a pause with its end, and a server other than the default one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// The phone that approved the pairing ("Daniel's iPhone"), when the pairing said.
    pub phone: Option<String>,
    /// The account the computer is paired with, when the pairing said.
    pub account: Option<String>,
    /// The first-time setup (harnesses, service, git) ran once; later starts open the status window.
    pub setup_done: bool,
    /// Unix seconds: git goes through Reins again then.
    pub paused_until: Option<i64>,
    /// A self-hosted server picked under "Use another server".
    pub server: Option<String>,
}

impl Saved {
    #[must_use]
    pub fn file(state_dir: &Path) -> PathBuf {
        state_dir.join("app.json")
    }

    /// The saved state; the defaults when there is none or it cannot be read.
    #[must_use]
    pub fn load(state_dir: &Path) -> Self {
        match std::fs::read(Self::file(state_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!("{}: {e}; starting over", Self::file(state_dir).display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, state_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
        let file = Self::file(state_dir);
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        rewarden_desktop::config::write_private(&file, &json).map_err(|e| format!("{}: {e}", file.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_and_survives_damage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Saved::load(dir.path()), Saved::default());
        let saved = Saved {
            phone: Some("Pixel 9".to_owned()),
            setup_done: true,
            paused_until: Some(1_900_000_000),
            ..Saved::default()
        };
        saved.save(dir.path()).unwrap();
        assert_eq!(Saved::load(dir.path()), saved);
        std::fs::write(Saved::file(dir.path()), b"{not json").unwrap();
        assert_eq!(Saved::load(dir.path()), Saved::default());
    }
}
