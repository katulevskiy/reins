//! The health card (`reins doctor` in the window) and "Send a test" (`reins test`): how the checks add up,
//! what the window can fix itself, and how a test ended, in words.

use std::time::Duration;

use reins_desktop::ask::{Answer, TEST_TIMEOUT};
use reins_desktop::doctor::{Check, Level};
use reins_desktop::harness::Harness;

/// The checks as last run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    /// `None` until the first run ends.
    pub checks: Option<Vec<Check>>,
    pub running: bool,
    /// Unix seconds the last run ended.
    pub at: Option<i64>,
    /// Why the checks could not run (the settings cannot be read).
    pub error: Option<String>,
}

/// How often the checks run while the window is open.
pub const EVERY: i64 = 60;

impl Health {
    /// Whether the checks should run again at `now`.
    #[must_use]
    pub fn due(&self, now: i64) -> bool {
        !self.running && self.at.is_none_or(|at| now - at >= EVERY)
    }
}

/// The card's headline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

/// The checks worth showing: not OK and not skipped, what fails first.
#[must_use]
pub fn to_look_at(checks: &[Check]) -> Vec<&Check> {
    let mut out: Vec<&Check> = checks.iter().filter(|c| matches!(c.level, Level::Warn | Level::Fail)).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.level));
    out
}

/// "All good" / "2 things to look at" / "Needs fixing", and a line under it.
#[must_use]
pub fn summary(checks: &[Check]) -> Summary {
    let passed = checks.iter().filter(|c| c.level == Level::Ok).count();
    let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warns = checks.iter().filter(|c| c.level == Level::Warn).count();
    let things = |n: usize| {
        if n == 1 {
            "1 thing".to_owned()
        } else {
            format!("{n} things")
        }
    };
    if fails > 0 {
        let detail = match warns {
            0 => format!("{} to fix", things(fails)),
            _ => format!("{} to fix, {} to look at", things(fails), things(warns)),
        };
        Summary {
            level: Level::Fail,
            title: "Needs fixing".to_owned(),
            detail,
        }
    } else if warns > 0 {
        Summary {
            level: Level::Warn,
            title: format!("{} to look at", things(warns)),
            detail: String::new(),
        }
    } else {
        Summary {
            level: Level::Ok,
            title: "All good".to_owned(),
            detail: if passed == 1 {
                "1 check passed".to_owned()
            } else {
                format!("{passed} checks passed")
            },
        }
    }
}

/// What the window can do about a check itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Start the background service.
    Start,
    /// Restart it (it runs another version).
    Restart,
    /// Send git through Reins again.
    Resume,
    /// Add Reins to an AI tool.
    Connect(Harness),
    /// Forget the session and pair again.
    PairAgain,
}

impl Fix {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Restart => "Restart",
            Self::Resume => "Resume",
            Self::Connect(_) => "Connect",
            Self::PairAgain => "Pair again",
        }
    }
}

/// The fix the window offers for `check`, by its id (`service`, `git`, `harness:codex`, `pairing`, `session`).
#[must_use]
pub fn fix_for(check: &Check) -> Option<Fix> {
    if !matches!(check.level, Level::Warn | Level::Fail) {
        return None;
    }
    match check.id.as_str() {
        "service" if check.level == Level::Fail => Some(Fix::Start),
        "service" => Some(Fix::Restart),
        "git" => Some(Fix::Resume),
        "pairing" | "session" if check.level == Level::Fail => Some(Fix::PairAgain),
        id => {
            let h = id.strip_prefix("harness:")?;
            Harness::ALL.into_iter().find(|x| x.id() == h).map(Fix::Connect)
        }
    }
}

/// "Send a test".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Test {
    #[default]
    Idle,
    /// Sent at this instant; waiting for the phone.
    Waiting(std::time::Instant),
    /// The answer, and whether the wait ran out (see [`test_result`]).
    Ended(Answer, bool),
}

/// How a test ended, in a line. `checks_above`: the health checks are on the same page, above the line.
#[must_use]
pub fn test_result(answer: &Answer, timed_out: bool, checks_above: bool) -> (bool, String) {
    match answer {
        Answer::Yes => (true, "✓ Approved. It works.".to_owned()),
        Answer::No(_) => (true, "✓ Denied. It works.".to_owned()),
        Answer::Unanswered(_) if timed_out => {
            let secs = TEST_TIMEOUT.as_secs();
            (
                false,
                if checks_above {
                    format!("No answer in {secs} s. See the checks above.")
                } else {
                    format!("No answer in {secs} s. See Health in Overview.")
                },
            )
        }
        Answer::Unanswered(why) => (false, why.clone()),
    }
}

/// The seconds left of the test's wait, after `elapsed`.
#[must_use]
pub fn seconds_left(elapsed: Duration) -> u64 {
    let left = TEST_TIMEOUT.saturating_sub(elapsed);
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// `--demo`: what the checks find on a paired computer, with one AI tool not connected yet (unless `fixed`).
#[must_use]
pub fn demo_checks(fixed: &[String]) -> Vec<Check> {
    let check = |id: &str, label: &str, level: Level, detail: &str, fix: Option<&str>| Check {
        id: id.to_owned(),
        label: label.to_owned(),
        level,
        detail: detail.to_owned(),
        fix: fix.map(str::to_owned),
    };
    // Cursor, as the demo's Connections and AI tools step have it: installed, not connected yet.
    let cursor = if fixed.iter().any(|f| f == "harness:cursor") {
        check("harness:cursor", "Cursor", Level::Ok, "Connected: its tools and risky commands reach your phone", None)
    } else {
        check(
            "harness:cursor",
            "Cursor",
            Level::Warn,
            "Installed but not connected",
            Some("Run `reins harness add cursor` (or connect it in the Reins app)."),
        )
    };
    vec![
        check("pairing", "Paired with your phone", Level::Ok, "Through https://app.reins2fa.com", None),
        check("server", "Server reachable", Level::Ok, "app.reins2fa.com answered in 84 ms", None),
        check("clock", "Clock", Level::Ok, "Within 1 s of the server", None),
        check("session", "Session accepted", Level::Ok, "The server knows this computer", None),
        check("phone", "Phone seen", Level::Ok, "Last checked in 2 min ago", None),
        check("service", "Background service", Level::Ok, "Running on 127.0.0.1:7457", None),
        check("git", "git through Reins", Level::Ok, "github.com", None),
        check(
            "harness:claude-code",
            "Claude Code",
            Level::Ok,
            "Connected: its tools and risky commands reach your phone",
            None,
        ),
        check("harness:codex", "Codex", Level::Ok, "Connected: its tools and risky commands reach your phone", None),
        cursor,
        check("notify", "\"Check your phone\" notifications", Level::Ok, "On", None),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, level: Level) -> Check {
        Check {
            id: id.to_owned(),
            label: id.to_owned(),
            level,
            detail: String::new(),
            fix: None,
        }
    }

    #[test]
    fn the_summary_counts_what_is_not_ok() {
        let ok = [check("a", Level::Ok), check("b", Level::Ok), check("c", Level::Skip)];
        let s = summary(&ok);
        assert_eq!((s.level, s.title.as_str()), (Level::Ok, "All good"));
        assert_eq!(s.detail, "2 checks passed");

        let one_warn = [check("a", Level::Ok), check("b", Level::Warn)];
        assert_eq!(summary(&one_warn).title, "1 thing to look at");
        let warns = [check("a", Level::Warn), check("b", Level::Warn), check("c", Level::Skip)];
        let s = summary(&warns);
        assert_eq!((s.level, s.title.as_str()), (Level::Warn, "2 things to look at"));

        let fail = [check("a", Level::Fail), check("b", Level::Ok)];
        let s = summary(&fail);
        assert_eq!((s.level, s.title.as_str()), (Level::Fail, "Needs fixing"));
        assert_eq!(s.detail, "1 thing to fix");
        let mixed = [check("a", Level::Fail), check("b", Level::Warn), check("c", Level::Warn)];
        assert_eq!(summary(&mixed).detail, "1 thing to fix, 2 things to look at");
        assert_eq!(summary(&[]).title, "All good");
    }

    #[test]
    fn failures_are_shown_first() {
        let checks =
            [check("w", Level::Warn), check("ok", Level::Ok), check("f", Level::Fail), check("s", Level::Skip)];
        let ids: Vec<&str> = to_look_at(&checks).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["f", "w"]);
    }

    #[test]
    fn the_window_fixes_what_it_can() {
        assert_eq!(fix_for(&check("service", Level::Fail)), Some(Fix::Start));
        assert_eq!(fix_for(&check("service", Level::Warn)), Some(Fix::Restart));
        assert_eq!(fix_for(&check("service", Level::Ok)), None);
        assert_eq!(fix_for(&check("git", Level::Warn)), Some(Fix::Resume));
        assert_eq!(fix_for(&check("pairing", Level::Fail)), Some(Fix::PairAgain));
        assert_eq!(fix_for(&check("session", Level::Fail)), Some(Fix::PairAgain));
        assert_eq!(fix_for(&check("session", Level::Warn)), None, "a hiccup is not a reason to pair again");
        assert_eq!(fix_for(&check("harness:codex", Level::Warn)), Some(Fix::Connect(Harness::Codex)));
        assert_eq!(fix_for(&check("harness:claude-code", Level::Warn)), Some(Fix::Connect(Harness::ClaudeCode)));
        assert_eq!(fix_for(&check("harness:nope", Level::Warn)), None);
        assert_eq!(fix_for(&check("clock", Level::Fail)), None);
        assert_eq!(fix_for(&check("phone", Level::Warn)), None);
        assert!(Fix::Connect(Harness::Codex).label() == "Connect" && Fix::PairAgain.label() == "Pair again");
    }

    #[test]
    fn test_results_read_as_sentences() {
        assert_eq!(test_result(&Answer::Yes, false, true), (true, "✓ Approved. It works.".to_owned()));
        let (ok, text) = test_result(&Answer::No("Denied on your phone.".to_owned()), false, true);
        assert!(ok && text.starts_with("✓ Denied"));
        let (ok, text) = test_result(&Answer::Unanswered("late".to_owned()), true, true);
        assert!(!ok);
        assert_eq!(text, "No answer in 90 s. See the checks above.");
        assert!(test_result(&Answer::Unanswered("late".to_owned()), true, false).1.contains("Overview"));
        assert_eq!(
            test_result(&Answer::Unanswered("Cannot ask your phone: offline".to_owned()), false, true).1,
            "Cannot ask your phone: offline"
        );
    }

    #[test]
    fn the_countdown_rounds_up_and_stops_at_zero() {
        assert_eq!(seconds_left(Duration::ZERO), 90);
        assert_eq!(seconds_left(Duration::from_millis(500)), 90);
        assert_eq!(seconds_left(Duration::from_secs(89)), 1);
        assert_eq!(seconds_left(Duration::from_secs(200)), 0);
    }

    #[test]
    fn checks_run_once_a_minute() {
        let mut h = Health::default();
        assert!(h.due(1_000));
        h.at = Some(1_000);
        assert!(!h.due(1_059));
        assert!(h.due(1_060));
        h.running = true;
        assert!(!h.due(5_000));
    }

    #[test]
    fn the_demo_has_one_thing_to_look_at_until_it_is_fixed() {
        let checks = demo_checks(&[]);
        assert_eq!(summary(&checks).title, "1 thing to look at");
        assert_eq!(fix_for(to_look_at(&checks)[0]), Some(Fix::Connect(Harness::Cursor)));
        assert_eq!(summary(&demo_checks(&["harness:cursor".to_owned()])).title, "All good");
    }
}
