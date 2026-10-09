//! What the app remembers between runs (`app.json` in the state directory): who approved the pairing, whether the
//! first-time setup ran (and the step it reached), a pause (with its end, or until the user resumes), a server other
//! than the default one, and where the window was.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::welcome::Stage;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// The phone that approved the pairing ("Daniel's iPhone"), when the pairing said.
    pub phone: Option<String>,
    /// The account the computer is paired with, when the pairing said.
    pub account: Option<String>,
    /// The first-time setup (harnesses, service, git) ran once; later starts open the status window.
    pub setup_done: bool,
    /// Unix seconds: git goes through Reins again then (a timed pause).
    pub paused_until: Option<i64>,
    /// Paused until the user resumes (no end).
    pub paused_manual: bool,
    /// A self-hosted server picked under "Other server".
    pub server: Option<String>,
    /// The step of the welcome flow the setup reached (quitting half-way resumes there).
    pub setup_step: Stage,
    /// Where the window was and how big, to open it there again.
    pub window: Option<Place>,
}

/// A window's place on the screen, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// Pixels as the window system gives them; the file only ever holds finite numbers.
impl Eq for Place {}

/// A rectangle on the screen: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

impl Place {
    /// Where to open the window again: the size at least `min` and at most the display it was on, and the position
    /// kept only when enough of the window is on a display (`None`: centre it). `displays` are the displays' visible
    /// bounds.
    #[must_use]
    pub fn fit(&self, displays: &[Rect], min: (f32, f32)) -> (Option<(f32, f32)>, (f32, f32)) {
        let finite = [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite());
        let (mut w, mut h) = if finite {
            (self.width, self.height)
        } else {
            (0.0, 0.0)
        };
        // The display the window's top left is on, else the first one.
        let on = displays
            .iter()
            .copied()
            .find(|&(dx, dy, dw, dh)| finite && self.x >= dx && self.x < dx + dw && self.y >= dy && self.y < dy + dh);
        if let Some((_, _, dw, dh)) = on.or_else(|| displays.first().copied()) {
            w = w.min(dw);
            h = h.min(dh);
        }
        w = w.max(min.0);
        h = h.max(min.1);
        // Keep the place when the title bar (its top 40 px, 120 px across) can still be grabbed.
        let origin = on
            .filter(|&(dx, dy, dw, dh)| {
                self.x + w.min(120.0) <= dx + dw && self.y + 40.0 <= dy + dh && self.x >= dx && self.y >= dy
            })
            .map(|_| (self.x, self.y));
        (origin, (w, h))
    }
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
        reins_desktop::config::write_private(&file, &json).map_err(|e| format!("{}: {e}", file.display()))
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
            paused_manual: true,
            ..Saved::default()
        };
        saved.save(dir.path()).unwrap();
        assert_eq!(Saved::load(dir.path()), saved);
        std::fs::write(Saved::file(dir.path()), b"{not json").unwrap();
        assert_eq!(Saved::load(dir.path()), Saved::default());
        // A file from before manual pauses reads as not paused manually.
        std::fs::write(Saved::file(dir.path()), br#"{"setup_done":true,"paused_until":5}"#).unwrap();
        let old = Saved::load(dir.path());
        assert!(old.setup_done && !old.paused_manual && old.paused_until == Some(5));
        assert_eq!((old.setup_step, old.window), (Stage::Pair, None));
    }

    #[test]
    fn the_setup_step_and_the_window_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let saved = Saved {
            setup_step: Stage::TurnOn,
            window: Some(Place {
                x: 40.0,
                y: 30.0,
                width: 1100.0,
                height: 720.0,
            }),
            ..Saved::default()
        };
        saved.save(dir.path()).unwrap();
        assert_eq!(Saved::load(dir.path()), saved);
    }

    #[test]
    fn a_window_reopens_where_it_fits() {
        let display = [(0.0, 0.0, 1920.0, 1080.0)];
        let min = (780.0, 560.0);
        let place = |x, y, width, height| Place {
            x,
            y,
            width,
            height,
        };
        // Where it was.
        assert_eq!(place(100.0, 80.0, 1100.0, 700.0).fit(&display, min), (Some((100.0, 80.0)), (1100.0, 700.0)));
        // Too small: the smallest it can be.
        assert_eq!(place(100.0, 80.0, 300.0, 200.0).fit(&display, min).1, (780.0, 560.0));
        // Bigger than the display: the display's size.
        assert_eq!(place(0.0, 0.0, 4000.0, 3000.0).fit(&display, min).1, (1920.0, 1080.0));
        // On a display that is gone: centred, its size kept.
        assert_eq!(place(3000.0, 200.0, 1000.0, 700.0).fit(&display, min), (None, (1000.0, 700.0)));
        // Its title bar off the bottom of the display: centred.
        assert_eq!(place(100.0, 1060.0, 1000.0, 700.0).fit(&display, min).0, None);
        // On the second of two displays.
        let two = [(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 1280.0, 800.0)];
        assert_eq!(place(2000.0, 50.0, 1500.0, 900.0).fit(&two, min), (Some((2000.0, 50.0)), (1280.0, 800.0)));
        // Nonsense from a damaged file.
        assert_eq!(place(f32::NAN, 0.0, f32::INFINITY, 10.0).fit(&display, min), (None, (780.0, 560.0)));
        // No display known: the size alone.
        assert_eq!(place(10.0, 10.0, 900.0, 600.0).fit(&[], min), (None, (900.0, 600.0)));
    }
}
