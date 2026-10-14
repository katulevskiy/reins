//! Which mode applies to a request (spec §1): the global mode, a connection's own mode, bypasses and their expiry.
//!
//! Rules, most important first:
//! - a global Lockdown applies to every connection, whatever it is set to;
//! - a connection's own Lockdown, then its own bypass;
//! - a global bypass in force applies to every connection not locked down, whatever mode it has of its own (a global
//!   Bypass that some AIs silently ignored would surprise);
//! - then Overnight, the connection's own or the global one, for the same reason (an AI left on Manual would stall
//!   all night under a global Overnight);
//! - otherwise the connection's own mode, else the global one;
//! - a global mode never picked is Manual.
//!
//! Overnight is a mode like Auto, stored as such, with an optional end and the mode it interrupted kept beside it (in
//! `meta`, so the settings table keeps its columns): at its end it returns to that mode. An older version reads the
//! stored "overnight" as a mode it does not know, and so as Manual, never as something more permissive.
//!
//! A bypass is stored as an end time next to the mode it returns to; an expired bypass is simply not in force (the
//! core enforces the end itself, whatever the app's alarm does). A bypass that runs until it is turned off ends at
//! [`BYPASS_UNTIL_OFF`], a time that never comes: it is in force, survives restarts and is stopped like any other.
//! A bypass approves everything, the hard floor (spec §2) included; only new connections and other phones joining the
//! account still reach the user.

use super::types::AutopilotMode;

/// Longest timed bypass (a year), and the default one.
pub const MAX_BYPASS_MINUTES: u32 = 365 * 24 * 60;
pub const DEFAULT_BYPASS_MINUTES: u32 = 15;
/// The end of a bypass that runs until the user turns it off.
pub const BYPASS_UNTIL_OFF: i64 = i64::MAX;

/// The origin of a permission a bypass granted: `bypass` for the global one, `bypass:<connection>` for one AI's. The
/// bypass takes them all back when it ends.
pub const BYPASS_ORIGIN: &str = "bypass";

/// The origin of a permission granted during Overnight (`overnight`, `overnight:<connection>`): taken back when it ends.
pub const OVERNIGHT_ORIGIN: &str = "overnight";

/// What granted a permission that ends with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temporary {
    Bypass,
    Overnight,
}

impl Temporary {
    fn origin(self) -> &'static str {
        match self {
            Self::Bypass => BYPASS_ORIGIN,
            Self::Overnight => OVERNIGHT_ORIGIN,
        }
    }
}

fn origin_of(kind: Temporary, scope: &str) -> String {
    if scope.is_empty() {
        kind.origin().to_owned()
    } else {
        format!("{}:{scope}", kind.origin())
    }
}

pub fn bypass_origin(scope: &str) -> String {
    origin_of(Temporary::Bypass, scope)
}

pub fn overnight_origin(scope: &str) -> String {
    origin_of(Temporary::Overnight, scope)
}

/// A permission granted by any bypass.
pub fn is_bypass_origin(origin: &str) -> bool {
    bypass_scope(origin).is_some()
}

fn scope_of(kind: Temporary, origin: &str) -> Option<&str> {
    if origin == kind.origin() {
        Some("")
    } else {
        origin.strip_prefix(kind.origin()).and_then(|r| r.strip_prefix(':')).filter(|s| !s.is_empty())
    }
}

/// The bypass a permission's origin belongs to: "" for the global one, a connection id for an AI's own; `None` when a
/// bypass did not grant it.
pub fn bypass_scope(origin: &str) -> Option<&str> {
    scope_of(Temporary::Bypass, origin)
}

/// What granted a permission that ends with it, and its scope ("" global, else a connection); `None` for any other
/// permission.
pub fn temporary_scope(origin: &str) -> Option<(Temporary, &str)> {
    [Temporary::Bypass, Temporary::Overnight].into_iter().find_map(|k| scope_of(k, origin).map(|s| (k, s)))
}

/// When a bypass started `now` ends: after `minutes` (1 minute to a year), or never (`None`, until turned off).
pub fn bypass_end(minutes: Option<u32>, now: i64) -> Result<i64, &'static str> {
    match minutes {
        None => Ok(BYPASS_UNTIL_OFF),
        Some(m) if m == 0 || m > MAX_BYPASS_MINUTES => Err("a bypass lasts between 1 minute and 365 days"),
        Some(m) => Ok(now.saturating_add(i64::from(m) * 60)),
    }
}

/// When an Overnight started `now` ends: after `minutes` (1 minute to a year), or never (`None`, until turned off).
pub fn overnight_end(minutes: Option<u32>, now: i64) -> Result<i64, &'static str> {
    bypass_end(minutes, now).map_err(|_| "Overnight lasts between 1 minute and 365 days")
}

/// One stored setting: the global one (`scope` = "") or a connection's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeRow {
    /// Never `Bypass` (that is `bypass_until`). `None`: the default (global), or follow the global mode (connection).
    pub mode: Option<AutopilotMode>,
    /// [`BYPASS_UNTIL_OFF`] for a bypass without an end.
    pub bypass_until: Option<i64>,
    /// When the bypass started (to show how far along it is, or how long it has run).
    pub bypass_since: Option<i64>,
    /// The connection's profile; for the global row, the default profile.
    pub profile_id: Option<String>,
    /// When `mode` is Overnight: its end ([`BYPASS_UNTIL_OFF`] without one; none read means none), its start, and the
    /// mode it returns to (`None`: the default, or following the global mode).
    pub overnight_until: Option<i64>,
    pub overnight_since: Option<i64>,
    pub overnight_return: Option<AutopilotMode>,
}

impl ModeRow {
    pub fn bypassing(&self, now: i64) -> bool {
        self.bypass_until.is_some_and(|t| t > now)
    }

    /// A bypass runs until the user turns it off.
    pub fn bypassing_until_off(&self) -> bool {
        self.bypass_until == Some(BYPASS_UNTIL_OFF)
    }

    /// Overnight is set and has not reached its end.
    pub fn overnighting(&self, now: i64) -> bool {
        self.mode == Some(AutopilotMode::Overnight) && self.overnight_until.is_none_or(|t| t > now)
    }

    /// Overnight is set but reached its end (the core then returns to the mode it interrupted).
    pub fn overnight_ended(&self, now: i64) -> bool {
        self.mode == Some(AutopilotMode::Overnight) && !self.overnighting(now)
    }

    /// The mode set when no bypass runs: an Overnight that ended reads as the mode it interrupted.
    pub fn base(&self, now: i64) -> Option<AutopilotMode> {
        if self.overnight_ended(now) {
            self.overnight_return.filter(|m| *m != AutopilotMode::Overnight)
        } else {
            self.mode
        }
    }
}

/// The global mode when no bypass runs.
pub fn global_base(global: &ModeRow, now: i64) -> AutopilotMode {
    global.base(now).unwrap_or(AutopilotMode::Manual)
}

/// The global mode in force now.
pub fn global_mode(global: &ModeRow, now: i64) -> AutopilotMode {
    if global.mode == Some(AutopilotMode::Lockdown) {
        AutopilotMode::Lockdown
    } else if global.bypassing(now) {
        AutopilotMode::Bypass
    } else {
        global_base(global, now)
    }
}

/// The mode for a request of a connection now.
pub fn effective(global: &ModeRow, conn: Option<&ModeRow>, now: i64) -> AutopilotMode {
    if global.mode == Some(AutopilotMode::Lockdown) {
        return AutopilotMode::Lockdown;
    }
    if let Some(c) = conn {
        if c.mode == Some(AutopilotMode::Lockdown) {
            return AutopilotMode::Lockdown;
        }
        if c.bypassing(now) || global.bypassing(now) {
            return AutopilotMode::Bypass;
        }
        if c.overnighting(now) || global.overnighting(now) {
            return AutopilotMode::Overnight;
        }
        if let Some(mode) = c.base(now) {
            return mode;
        }
    }
    global_mode(global, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mode: Option<AutopilotMode>, bypass_until: Option<i64>) -> ModeRow {
        ModeRow {
            mode,
            bypass_until,
            ..ModeRow::default()
        }
    }

    fn overnight(until: Option<i64>, back: Option<AutopilotMode>) -> ModeRow {
        ModeRow {
            mode: Some(AutopilotMode::Overnight),
            overnight_until: until,
            overnight_since: Some(0),
            overnight_return: back,
            ..ModeRow::default()
        }
    }

    #[test]
    fn overnight_runs_until_its_end_then_returns_to_the_mode_it_interrupted() {
        let g = overnight(Some(100), Some(AutopilotMode::Auto));
        assert_eq!(effective(&g, None, 99), AutopilotMode::Overnight);
        assert_eq!(effective(&g, None, 100), AutopilotMode::Auto, "ended at its end time");
        assert_eq!(global_base(&g, 100), AutopilotMode::Auto);
        assert_eq!(global_mode(&overnight(Some(100), None), 100), AutopilotMode::Manual, "back to the default");
        // Without an end it runs until turned off; a row without the end (written elsewhere) counts as such.
        for until in [Some(BYPASS_UNTIL_OFF), None] {
            assert_eq!(effective(&overnight(until, None), None, i64::MAX - 1), AutopilotMode::Overnight);
        }
        // A connection's own: back to following the global mode.
        let c = overnight(Some(50), None);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), Some(&c), 49), AutopilotMode::Overnight);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), Some(&c), 50), AutopilotMode::Auto);
    }

    #[test]
    fn lockdown_and_bypass_beat_overnight_and_a_global_overnight_beats_an_ais_own_mode() {
        let g = overnight(None, Some(AutopilotMode::Auto));
        assert_eq!(effective(&g, Some(&row(Some(AutopilotMode::Manual), None)), 0), AutopilotMode::Overnight);
        assert_eq!(effective(&g, Some(&row(Some(AutopilotMode::Lockdown), None)), 0), AutopilotMode::Lockdown);
        assert_eq!(effective(&g, Some(&row(None, Some(10))), 0), AutopilotMode::Bypass);
        let mut both = overnight(None, None);
        both.bypass_until = Some(10);
        assert_eq!(effective(&both, None, 0), AutopilotMode::Bypass, "a bypass on top of it");
        assert_eq!(effective(&both, None, 10), AutopilotMode::Overnight, "and Overnight once it ends");
        assert_eq!(
            effective(&row(Some(AutopilotMode::Lockdown), None), Some(&overnight(None, None)), 0),
            AutopilotMode::Lockdown
        );
    }

    #[test]
    fn overnight_is_stored_by_name_and_unknown_to_nothing_else() {
        assert_eq!(AutopilotMode::parse("overnight"), Some(AutopilotMode::Overnight));
        assert_eq!(AutopilotMode::Overnight.as_str(), "overnight");
        assert_eq!(serde_json::to_string(&AutopilotMode::Overnight).unwrap(), "\"Overnight\"");
        assert_eq!(overnight_end(Some(60), 0), Ok(3_600));
        assert_eq!(overnight_end(None, 0), Ok(BYPASS_UNTIL_OFF));
        assert!(overnight_end(Some(0), 0).is_err());
    }

    #[test]
    fn grants_of_a_bypass_or_an_overnight_know_their_scope() {
        assert_eq!(temporary_scope(&overnight_origin("")), Some((Temporary::Overnight, "")));
        assert_eq!(temporary_scope(&overnight_origin("c1")), Some((Temporary::Overnight, "c1")));
        assert_eq!(temporary_scope(&bypass_origin("c1")), Some((Temporary::Bypass, "c1")));
        assert_eq!(temporary_scope("overnight:"), None);
        assert_eq!(temporary_scope("overnights"), None);
        assert_eq!(temporary_scope("user"), None);
        assert!(!is_bypass_origin(&overnight_origin("")));
    }

    #[test]
    fn a_bypass_lasts_a_minute_to_a_year_or_until_turned_off() {
        assert_eq!(bypass_end(Some(1), 100), Ok(160));
        assert_eq!(bypass_end(Some(3 * 24 * 60), 0), Ok(3 * 86_400));
        assert_eq!(bypass_end(Some(MAX_BYPASS_MINUTES), 0), Ok(365 * 86_400));
        assert!(bypass_end(Some(0), 0).is_err());
        assert!(bypass_end(Some(MAX_BYPASS_MINUTES + 1), 0).is_err());
        assert_eq!(bypass_end(None, 100), Ok(BYPASS_UNTIL_OFF));
    }

    #[test]
    fn a_bypass_until_off_never_ends_by_itself() {
        let g = row(Some(AutopilotMode::Auto), Some(BYPASS_UNTIL_OFF));
        assert!(g.bypassing_until_off());
        for now in [0, 1_700_000_000, 4_000_000_000, i64::MAX - 1] {
            assert_eq!(effective(&g, None, now), AutopilotMode::Bypass, "{now}");
        }
        assert!(!row(None, Some(100)).bypassing_until_off());
        // Lockdown still wins over it, and stopping it returns the mode it interrupted.
        assert_eq!(effective(&row(Some(AutopilotMode::Lockdown), None), Some(&g), 0), AutopilotMode::Lockdown);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), None, 0), AutopilotMode::Auto);
    }

    #[test]
    fn the_default_is_manual() {
        let g = ModeRow::default();
        assert_eq!(effective(&g, None, 0), AutopilotMode::Manual);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), None, 0), AutopilotMode::Auto);
    }

    #[test]
    fn assisted_from_older_versions_reads_as_manual() {
        assert_eq!(AutopilotMode::parse("assisted"), Some(AutopilotMode::Manual));
        assert_eq!(AutopilotMode::parse("manual"), Some(AutopilotMode::Manual));
        assert_eq!(AutopilotMode::Manual.as_str(), "manual");
        let old: AutopilotMode = serde_json::from_str("\"Assisted\"").unwrap();
        assert_eq!(old, AutopilotMode::Manual);
        assert_eq!(serde_json::to_string(&AutopilotMode::Manual).unwrap(), "\"Manual\"");
    }

    #[test]
    fn bypass_ends_by_itself() {
        let g = row(Some(AutopilotMode::Auto), Some(100));
        assert_eq!(effective(&g, None, 99), AutopilotMode::Bypass);
        assert_eq!(effective(&g, None, 100), AutopilotMode::Auto, "expired at its end time");
        let c = row(None, Some(50));
        assert_eq!(effective(&ModeRow::default(), Some(&c), 49), AutopilotMode::Bypass);
        assert_eq!(effective(&ModeRow::default(), Some(&c), 50), AutopilotMode::Manual);
    }

    #[test]
    fn global_lockdown_wins_and_connections_win_otherwise() {
        let lock = row(Some(AutopilotMode::Lockdown), None);
        let bypassing = row(Some(AutopilotMode::Auto), Some(1_000));
        assert_eq!(effective(&lock, Some(&bypassing), 0), AutopilotMode::Lockdown);
        let manual = row(Some(AutopilotMode::Manual), None);
        assert_eq!(effective(&row(Some(AutopilotMode::Auto), None), Some(&manual), 0), AutopilotMode::Manual);
        assert_eq!(
            effective(&row(None, Some(1_000)), Some(&manual), 0),
            AutopilotMode::Bypass,
            "a global bypass beats a connection's own Manual or Auto"
        );
        assert_eq!(effective(&row(None, Some(1_000)), Some(&manual), 1_000), AutopilotMode::Manual, "until it ends");
        assert_eq!(
            effective(&ModeRow::default(), Some(&row(Some(AutopilotMode::Lockdown), Some(9))), 0),
            AutopilotMode::Lockdown
        );
        assert_eq!(effective(&row(None, Some(1_000)), Some(&row(None, None)), 0), AutopilotMode::Bypass);
    }
}
