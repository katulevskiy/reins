//! Pausing: git talks to the hosts directly for a while (15 minutes to 24 hours) or until the user resumes. Hooks and
//! MCP tools still ask the phone; only git's routing stops. A timed pause ends by itself; "Until I resume" never does.

use crate::state::Saved;

/// The pause lengths the window and the tray offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseFor {
    Quarter,
    Hour,
    FourHours,
    Day,
    /// Until the user resumes.
    Manual,
}

impl PauseFor {
    pub const ALL: [Self; 5] = [Self::Quarter, Self::Hour, Self::FourHours, Self::Day, Self::Manual];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Quarter => "15 minutes",
            Self::Hour => "1 hour",
            Self::FourHours => "4 hours",
            Self::Day => "24 hours",
            Self::Manual => "Until I resume",
        }
    }

    /// For buttons side by side.
    #[must_use]
    pub fn short(self) -> &'static str {
        match self {
            Self::Quarter => "15 min",
            Self::Hour => "1 hour",
            Self::FourHours => "4 hours",
            Self::Day => "24 hours",
            Self::Manual => "Until I resume",
        }
    }

    /// How long; `None` for "Until I resume".
    #[must_use]
    pub fn secs(self) -> Option<i64> {
        match self {
            Self::Quarter => Some(15 * 60),
            Self::Hour => Some(3_600),
            Self::FourHours => Some(4 * 3_600),
            Self::Day => Some(24 * 3_600),
            Self::Manual => None,
        }
    }

    /// The tray menu item's id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Quarter => "pause-15m",
            Self::Hour => "pause-1h",
            Self::FourHours => "pause-4h",
            Self::Day => "pause-24h",
            Self::Manual => "pause-manual",
        }
    }
}

/// Whether git is paused now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    Off,
    /// Until these Unix seconds.
    Until(i64),
    /// Until the user resumes.
    Manual,
}

impl Pause {
    #[must_use]
    pub fn of(saved: &Saved, now: i64) -> Self {
        if saved.paused_manual {
            return Self::Manual;
        }
        match saved.paused_until {
            Some(until) if until > now => Self::Until(until),
            _ => Self::Off,
        }
    }

    #[must_use]
    pub fn on(self) -> bool {
        self != Self::Off
    }

    /// "Paused for 3 h 12 min more", "Paused until you resume".
    #[must_use]
    pub fn line(self, now: i64) -> Option<String> {
        match self {
            Self::Off => None,
            Self::Until(until) => Some(format!("Paused for {} more", crate::format::left(until - now))),
            Self::Manual => Some("Paused until you resume".to_owned()),
        }
    }

    /// Short, for the sidebar: "3 h 12 min left", "until you resume".
    #[must_use]
    pub fn short(self, now: i64) -> Option<String> {
        match self {
            Self::Off => None,
            Self::Until(until) => Some(format!("{} left", crate::format::left(until - now))),
            Self::Manual => Some("until you resume".to_owned()),
        }
    }
}

impl Saved {
    /// Starts a pause of `length` from `now`.
    pub fn pause(&mut self, length: PauseFor, now: i64) {
        self.paused_until = length.secs().map(|s| now + s);
        self.paused_manual = length.secs().is_none();
    }

    pub fn unpause(&mut self) {
        self.paused_until = None;
        self.paused_manual = false;
    }

    /// A timed pause ran out (a manual one never does).
    #[must_use]
    pub fn pause_ran_out(&self, now: i64) -> bool {
        !self.paused_manual && self.paused_until.is_some_and(|until| now >= until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_pauses_end_and_manual_ones_do_not() {
        let mut saved = Saved::default();
        assert_eq!(Pause::of(&saved, 100), Pause::Off);
        saved.pause(PauseFor::FourHours, 1_000);
        assert_eq!(Pause::of(&saved, 1_000), Pause::Until(1_000 + 4 * 3_600));
        assert!(!saved.pause_ran_out(1_000 + 3_600));
        assert!(saved.pause_ran_out(1_000 + 4 * 3_600));
        assert_eq!(Pause::of(&saved, 1_000 + 4 * 3_600), Pause::Off);

        saved.pause(PauseFor::Manual, 1_000);
        assert_eq!((saved.paused_until, saved.paused_manual), (None, true));
        assert_eq!(Pause::of(&saved, i64::MAX), Pause::Manual);
        assert!(!saved.pause_ran_out(i64::MAX));
        saved.unpause();
        assert_eq!(Pause::of(&saved, 0), Pause::Off);
    }

    #[test]
    fn pause_lines() {
        let now = 10_000;
        assert_eq!(Pause::Until(now + 3 * 3_600 + 11 * 60 + 20).line(now).unwrap(), "Paused for 3 h 12 min more");
        assert_eq!(Pause::Until(now + 30).line(now).unwrap(), "Paused for 1 min more");
        assert_eq!(Pause::Manual.line(now).unwrap(), "Paused until you resume");
        assert_eq!(Pause::Off.line(now), None);
        assert_eq!(Pause::Until(now + 900).short(now).unwrap(), "15 min left");
    }

    #[test]
    fn lengths_and_ids_are_distinct() {
        let ids: std::collections::HashSet<_> = PauseFor::ALL.iter().map(|p| p.id()).collect();
        assert_eq!(ids.len(), PauseFor::ALL.len());
        assert_eq!(PauseFor::Day.secs(), Some(86_400));
        assert_eq!(PauseFor::Manual.secs(), None);
        assert!(PauseFor::ALL.iter().all(|p| !p.label().is_empty() && !p.short().is_empty()));
    }
}
