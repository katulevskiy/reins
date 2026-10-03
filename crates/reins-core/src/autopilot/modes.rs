//! Which mode applies to a request (spec §1): the global mode, a connection's own mode, bypasses and their expiry.
//!
//! Rules, most important first:
//! - a global Lockdown applies to every connection, whatever it is set to;
//! - a connection's own setting (its bypass, its mode) wins over the global one;
//! - otherwise the global setting applies (its bypass, its mode);
//! - a global mode never picked is Assisted once a model is installed, Manual before.
//!
//! A bypass is stored as an end time next to the mode it returns to; an expired bypass is simply not in force (the
//! core enforces the end itself, whatever the app's alarm does).

use super::types::AutopilotMode;

/// Longest bypass, and the default one.
pub const MAX_BYPASS_MINUTES: u32 = 60;
pub const DEFAULT_BYPASS_MINUTES: u32 = 15;

/// One stored setting: the global one (`scope` = "") or a connection's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeRow {
    /// Never `Bypass` (that is `bypass_until`). `None`: the default (global), or follow the global mode (connection).
    pub mode: Option<AutopilotMode>,
    pub bypass_until: Option<i64>,
    /// The connection's profile; for the global row, the default profile.
    pub profile_id: Option<String>,
}

impl ModeRow {
    pub fn bypassing(&self, now: i64) -> bool {
        self.bypass_until.is_some_and(|t| t > now)
    }
}

/// The global mode when no bypass runs.
pub fn global_base(global: &ModeRow, model_installed: bool) -> AutopilotMode {
    global.mode.unwrap_or(if model_installed {
        AutopilotMode::Assisted
    } else {
        AutopilotMode::Manual
    })
}

/// The global mode in force now.
pub fn global_mode(global: &ModeRow, model_installed: bool, now: i64) -> AutopilotMode {
    if global.mode == Some(AutopilotMode::Lockdown) {
        AutopilotMode::Lockdown
    } else if global.bypassing(now) {
        AutopilotMode::Bypass
    } else {
        global_base(global, model_installed)
    }
}

/// The mode for a request of a connection now.
pub fn effective(global: &ModeRow, conn: Option<&ModeRow>, model_installed: bool, now: i64) -> AutopilotMode {
    if global.mode == Some(AutopilotMode::Lockdown) {
        return AutopilotMode::Lockdown;
    }
    if let Some(c) = conn {
        if c.mode == Some(AutopilotMode::Lockdown) {
            return AutopilotMode::Lockdown;
        }
        if c.bypassing(now) {
            return AutopilotMode::Bypass;
        }
        if let Some(mode) = c.mode {
            return mode;
        }
    }
    global_mode(global, model_installed, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mode: Option<AutopilotMode>, bypass_until: Option<i64>) -> ModeRow {
        ModeRow {
            mode,
            bypass_until,
            profile_id: None,
        }
    }

    #[test]
    fn defaults_follow_the_model() {
        let g = ModeRow::default();
        assert_eq!(effective(&g, None, false, 0), AutopilotMode::Manual);
        assert_eq!(effective(&g, None, true, 0), AutopilotMode::Assisted);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), None, false, 0), AutopilotMode::Auto);
    }

    #[test]
    fn bypass_ends_by_itself() {
        let g = row(Some(AutopilotMode::Auto), Some(100));
        assert_eq!(effective(&g, None, true, 99), AutopilotMode::Bypass);
        assert_eq!(effective(&g, None, true, 100), AutopilotMode::Auto, "expired at its end time");
        let c = row(None, Some(50));
        assert_eq!(effective(&ModeRow::default(), Some(&c), true, 49), AutopilotMode::Bypass);
        assert_eq!(effective(&ModeRow::default(), Some(&c), true, 50), AutopilotMode::Assisted);
    }

    #[test]
    fn global_lockdown_wins_and_connections_win_otherwise() {
        let lock = row(Some(AutopilotMode::Lockdown), None);
        let bypassing = row(Some(AutopilotMode::Auto), Some(1_000));
        assert_eq!(effective(&lock, Some(&bypassing), true, 0), AutopilotMode::Lockdown);
        let manual = row(Some(AutopilotMode::Manual), None);
        assert_eq!(effective(&row(None, Some(1_000)), Some(&manual), true, 0), AutopilotMode::Manual);
        assert_eq!(
            effective(&ModeRow::default(), Some(&row(Some(AutopilotMode::Lockdown), Some(9))), true, 0),
            AutopilotMode::Lockdown
        );
        assert_eq!(effective(&row(None, Some(1_000)), Some(&row(None, None)), true, 0), AutopilotMode::Bypass);
    }
}
