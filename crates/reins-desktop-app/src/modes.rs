//! The phone's Autopilot as this app shows it: the mode that runs now (Bypass and Overnight with their clocks,
//! Lockdown), which of Manual and Auto the picker shows, and the words for who decided a request and where Autopilot
//! leans on one that waits.
//!
//! Everything here is shown, never acted on: the phone decides with its own state. A change of mode from this app is
//! a request to the phone (`reins_desktop::autopilot`), which applies what is stricter and asks its owner for the rest.

use reins_desktop::journal::{Attribution, Entry, Lean, Outcome};
use reins_proto::autopilot::{DecidedBy, DesktopModes, Leaning, Mode};

/// A mode that runs on the phone now, for this computer's requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    /// Everything approved until `until` (unix seconds).
    Bypass {
        until: i64,
    },
    /// Auto for agents left working, until `until` when it has an end.
    Overnight {
        until: Option<i64>,
    },
    /// Everything denied.
    Lockdown,
}

impl Running {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Bypass {
                ..
            } => "Bypass",
            Self::Overnight {
                ..
            } => "Overnight",
            Self::Lockdown => "Lockdown",
        }
    }

    /// When it ends, if it does on its own.
    #[must_use]
    pub fn until(self) -> Option<i64> {
        match self {
            Self::Bypass {
                until,
            } => Some(until),
            Self::Overnight {
                until,
            } => until,
            Self::Lockdown => None,
        }
    }

    /// The time left, as a clock ("47:12", "5:59:12").
    #[must_use]
    pub fn clock(self, now: i64) -> Option<String> {
        self.until().map(|until| clock(until - now))
    }
}

/// Seconds as a clock: "47:12" under an hour, "5:59:12" from an hour.
#[must_use]
pub fn clock(secs: i64) -> String {
    let secs = secs.max(0);
    let (h, m, s) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// An end the phone gave (`i64::MAX` or none: until it is turned off), if still ahead.
fn ahead(until: Option<i64>, now: i64) -> Result<Option<i64>, ()> {
    match until {
        None | Some(i64::MAX) => Ok(None),
        Some(t) if t > now => Ok(Some(t)),
        Some(_) => Err(()),
    }
}

/// The mode that runs now for this computer's requests (`None`: Manual or Auto, or not known). A bypass or Overnight
/// whose end has passed since the phone last said is over.
#[must_use]
pub fn running(modes: Option<&DesktopModes>, now: i64) -> Option<Running> {
    let m = modes?;
    match m.mode {
        Mode::Lockdown => Some(Running::Lockdown),
        // A bypass from this app always ends; one without an end shows as ending never.
        Mode::Bypass => match ahead(m.span.until, now) {
            Ok(Some(until)) => Some(Running::Bypass {
                until,
            }),
            Ok(None) => Some(Running::Bypass {
                until: i64::MAX,
            }),
            Err(()) => None,
        },
        Mode::Overnight => ahead(m.span.until, now).ok().map(|until| Running::Overnight {
            until,
        }),
        Mode::Manual | Mode::Auto => None,
    }
}

/// Which of Manual and Auto the picker shows as picked (`None`: Lockdown, or not known).
#[must_use]
pub fn picked(modes: Option<&DesktopModes>) -> Option<Mode> {
    modes.map(|m| m.base).filter(|b| matches!(b, Mode::Manual | Mode::Auto))
}

/// The tag a decided row carries: "Auto", "Overnight", "Bypass", "Lockdown".
#[must_use]
pub fn tag(a: &Attribution) -> &'static str {
    match a.by {
        DecidedBy::Autopilot => "Auto",
        DecidedBy::Overnight => "Overnight",
        DecidedBy::Bypass => "Bypass",
        DecidedBy::Lockdown => "Lockdown",
    }
}

fn who(a: &Attribution) -> &'static str {
    match a.by {
        DecidedBy::Autopilot => "Autopilot",
        DecidedBy::Overnight => "Overnight",
        DecidedBy::Bypass => "Bypass",
        DecidedBy::Lockdown => "Lockdown",
    }
}

/// Whether the phone let it through.
fn allowed(outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Approved | Outcome::Allowed)
}

/// What the phone did, by whom: "Allowed by Autopilot", "Denied by Lockdown".
#[must_use]
pub fn said(a: &Attribution, outcome: Outcome) -> String {
    let did = if allowed(outcome) {
        "Allowed"
    } else {
        "Denied"
    };
    format!("{did} by {}", who(a))
}

/// The panel's title: "Why Autopilot allowed", "Why Lockdown denied".
#[must_use]
pub fn why_title(a: &Attribution, outcome: Outcome) -> String {
    let did = if allowed(outcome) {
        "allowed"
    } else {
        "denied"
    };
    format!("Why {} {did}", who(a))
}

/// The verdict pill beside it: "Allowed · 95% sure".
#[must_use]
pub fn verdict(a: &Attribution, outcome: Outcome) -> String {
    let did = if allowed(outcome) {
        "Allowed"
    } else {
        "Denied"
    };
    match a.confidence {
        Some(c) => format!("{did} · {c}% sure"),
        None => did.to_owned(),
    }
}

/// "This was wrong" sends this: what Autopilot should have done instead (`true`: approve). `None` when it cannot be
/// sent for `e`: not correctable, not answered, or no request id.
#[must_use]
pub fn correction(e: &Entry) -> Option<(String, bool)> {
    e.autopilot.as_ref().filter(|a| a.correctable)?;
    let id = e.request_id.clone()?;
    match e.outcome {
        Outcome::Approved | Outcome::Allowed => Some((id, false)),
        Outcome::Denied | Outcome::Blocked => Some((id, true)),
        _ => None,
    }
}

/// Where Autopilot leans on a request that waits, in a few words: "Autopilot would allow", with its confidence.
#[must_use]
pub fn leaning(l: Lean) -> (&'static str, u8) {
    let words = match l.leaning {
        Leaning::Approve => "Autopilot would allow",
        Leaning::Deny => "Autopilot would deny",
        Leaning::Ask => "Autopilot leaves it to you",
    };
    (words, l.confidence)
}

/// `--demo` with `REINS_DEMO_MODE`: the phone's modes as `name` says ("manual", "auto", "bypass": 47 minutes left,
/// "overnight": six hours, "lockdown").
#[must_use]
pub fn demo(name: &str, now: i64) -> Option<DesktopModes> {
    use reins_proto::autopilot::Span;
    let (mode, base, span) = match name.trim() {
        "manual" => (Mode::Manual, Mode::Manual, Span::default()),
        "auto" => (Mode::Auto, Mode::Auto, Span::default()),
        "bypass" => (
            Mode::Bypass,
            Mode::Manual,
            Span {
                since: Some(now - 13 * 60),
                until: Some(now + 47 * 60 + 12),
            },
        ),
        "overnight" => (
            Mode::Overnight,
            Mode::Auto,
            Span {
                since: Some(now - 48),
                until: Some(now + 6 * 3_600 - 48),
            },
        ),
        "lockdown" => (Mode::Lockdown, Mode::Lockdown, Span::default()),
        _ => return None,
    };
    Some(DesktopModes {
        mode,
        span,
        global: mode,
        base,
        reported_at: now,
    })
}

/// `--demo`: the modes after `choice`, as the phone would apply it (the owner approving what it asks).
#[must_use]
pub fn demo_after(
    choice: reins_desktop::autopilot::ModeChoice,
    now: i64,
    before: Option<DesktopModes>,
) -> DesktopModes {
    use reins_desktop::autopilot::ModeChoice;
    let base = before.map_or(Mode::Manual, |m| m.base);
    let name = match choice {
        ModeChoice::Manual => "manual",
        ModeChoice::Auto => "auto",
        ModeChoice::Lockdown => "lockdown",
        ModeChoice::Bypass(minutes) => {
            let mut m = demo("bypass", now).unwrap_or_else(|| unreachable!("a demo mode"));
            m.base = if base == Mode::Lockdown {
                Mode::Manual
            } else {
                base
            };
            m.span.since = Some(now);
            m.span.until = Some(now + i64::from(minutes) * 60);
            return m;
        }
        ModeChoice::Stop => {
            if base == Mode::Auto {
                "auto"
            } else {
                "manual"
            }
        }
    };
    demo(name, now).unwrap_or_else(|| unreachable!("a demo mode"))
}

#[cfg(test)]
mod tests {
    use reins_proto::autopilot::Span;

    use super::*;

    fn modes(mode: Mode, base: Mode, until: Option<i64>) -> DesktopModes {
        DesktopModes {
            mode,
            span: Span {
                since: Some(0),
                until,
            },
            global: mode,
            base,
            reported_at: 0,
        }
    }

    #[test]
    fn what_runs_and_its_clock() {
        assert_eq!(running(None, 0), None);
        let bypass = modes(Mode::Bypass, Mode::Manual, Some(1_000 + 47 * 60 + 12));
        let r = running(Some(&bypass), 1_000).unwrap();
        assert_eq!((r.name(), r.clock(1_000).as_deref()), ("Bypass", Some("47:12")));
        assert_eq!(running(Some(&bypass), 9_999), None, "its end has passed");
        let night = modes(Mode::Overnight, Mode::Auto, Some(6 * 3_600));
        assert_eq!(running(Some(&night), 48).unwrap().clock(48).as_deref(), Some("5:59:12"));
        let endless = modes(Mode::Overnight, Mode::Auto, Some(i64::MAX));
        assert_eq!(
            running(Some(&endless), 5),
            Some(Running::Overnight {
                until: None
            })
        );
        assert_eq!(running(Some(&modes(Mode::Lockdown, Mode::Lockdown, None)), 0), Some(Running::Lockdown));
        assert_eq!(running(Some(&modes(Mode::Auto, Mode::Auto, None)), 0), None);
        assert_eq!(picked(Some(&bypass)), Some(Mode::Manual));
        assert_eq!(picked(Some(&modes(Mode::Lockdown, Mode::Lockdown, None))), None);
    }

    #[test]
    fn who_decided_in_words_and_what_this_was_wrong_sends() {
        let a = Attribution {
            by: DecidedBy::Autopilot,
            confidence: Some(95),
            reasons: vec![],
            correctable: true,
        };
        assert_eq!(said(&a, Outcome::Approved), "Allowed by Autopilot");
        assert_eq!(why_title(&a, Outcome::Approved), "Why Autopilot allowed");
        assert_eq!(verdict(&a, Outcome::Approved), "Allowed · 95% sure");
        assert_eq!(tag(&a), "Auto");
        let mut e = Entry::new(reins_desktop::journal::Kind::Command, "ls");
        e.autopilot = Some(a);
        e.end(Outcome::Approved, None);
        assert_eq!(correction(&e), None, "no request id");
        e.request_id = Some("r1".to_owned());
        assert_eq!(correction(&e), Some(("r1".to_owned(), false)), "it should have denied");
        e.end(Outcome::Denied, None);
        assert_eq!(correction(&e), Some(("r1".to_owned(), true)));
        e.autopilot = Some(Attribution {
            by: DecidedBy::Lockdown,
            confidence: None,
            reasons: vec![],
            correctable: false,
        });
        assert_eq!(correction(&e), None, "nothing to teach Lockdown");
        assert_eq!(clock(-4), "0:00");
    }
}
