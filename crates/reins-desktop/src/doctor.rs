//! `reins doctor`, and the desktop app's health card: is this computer set up so that agents reach the phone? Each
//! check says what it found and, when something is wrong, the one thing to do about it.
//!
//! Checks: paired with a phone, the session still accepted, the server reachable, this computer's clock (against the
//! server's), the phone seen lately, the background service running (and running this version), git routed through
//! Reins for the enabled hosts, each installed AI tool connected (and up to date), and desktop notifications.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::config::{Config, Mode, Paths};
use crate::control::{Client, ClientError};
use crate::harness::{self, Harness};
use crate::server::LinkError;
use crate::setup::{Git, Scope};

/// How a check went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Ok,
    /// Works, but worth a look.
    Warn,
    /// Agents do not reach the phone (or not everything does) until this is fixed.
    Fail,
    /// Does not apply here (local mode, nothing installed).
    Skip,
}

impl Level {
    #[must_use]
    pub fn mark(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Skip => "–",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Stable id (`service`, `harness:codex`).
    pub id: String,
    /// What is checked ("Background service").
    pub label: String,
    pub level: Level,
    /// What was found.
    pub detail: String,
    /// What to do, when something is wrong.
    pub fix: Option<String>,
}

impl Check {
    fn new(id: &str, label: &str, level: Level, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            level,
            detail: detail.into(),
            fix: fix.map(str::to_owned),
        }
    }
}

/// The worst level among `checks` (skipped ones do not count).
#[must_use]
pub fn overall(checks: &[Check]) -> Level {
    checks.iter().map(|c| c.level).filter(|l| *l != Level::Skip).max().unwrap_or(Level::Ok)
}

/// What the checks look at.
pub struct Doctor {
    pub paths: Paths,
    pub config: Config,
    /// Where the AI tools keep their settings.
    pub home: PathBuf,
    /// The `reins` program the harness entries name (the app's bundled one, or this one).
    pub exe: PathBuf,
    /// How git is run (the app sets `HOME` when `REINS_HOME` stands in for it).
    pub git: Git,
}

/// More than this between this computer's clock and the server's is a warning, ten times more an error.
const SKEW_WARN_SECS: f64 = 30.0;
/// The phone not seen for this long is worth a word.
const PHONE_QUIET_SECS: i64 = 24 * 3600;

impl Doctor {
    /// Every check, in the order a person would fix them.
    pub async fn run(&self) -> Vec<Check> {
        let mut checks = Vec::new();
        let server = crate::server::oauth::logged_in_server(&self.paths);
        if self.config.mode == Mode::Local {
            checks.push(Check::new(
                "pairing",
                "Paired with your phone",
                Level::Skip,
                "Local mode: this computer decides (`mode = \"local\"` in config.toml).",
                None,
            ));
        } else if let Some(server) = &server {
            checks.push(Check::new("pairing", "Paired with your phone", Level::Ok, format!("Through {server}"), None));
            checks.extend(self.server_checks(server).await);
        } else {
            checks.push(Check::new(
                "pairing",
                "Paired with your phone",
                Level::Fail,
                "Not paired: agents cannot reach your phone.",
                Some("Run `reins setup` (or `reins login`) and scan the QR code with the Reins app."),
            ));
        }
        checks.push(self.service().await);
        checks.push(self.git_check());
        checks.extend(self.harness_checks());
        checks.push(self.notifications());
        checks
    }

    async fn server_checks(&self, server: &str) -> Vec<Check> {
        let mut out = Vec::new();
        let http = match crate::http::client(Some(Duration::from_secs(10))) {
            Ok(h) => h,
            Err(e) => return vec![Check::new("server", "Server reachable", Level::Fail, e, None)],
        };
        let started = Instant::now();
        let alive = http.get(format!("{server}/alive")).send().await;
        let took = started.elapsed();
        match alive {
            Ok(resp) if resp.status().is_success() => {
                out.push(Check::new(
                    "server",
                    "Server reachable",
                    Level::Ok,
                    format!("{} answered in {} ms", host_of(server), took.as_millis()),
                    None,
                ));
                let text = resp.text().await.unwrap_or_default();
                if let Some(theirs) = parse_rfc3339(text.trim().trim_matches('"')) {
                    out.push(clock_check(theirs - took.as_secs_f64() / 2.0, now_secs()));
                }
            }
            Ok(resp) => out.push(Check::new(
                "server",
                "Server reachable",
                Level::Fail,
                format!("{} answered {}", host_of(server), resp.status()),
                Some("The server has trouble; try again later, or check its address with `reins status`."),
            )),
            Err(e) => out.push(Check::new(
                "server",
                "Server reachable",
                Level::Fail,
                format!("Cannot reach {}: {}", host_of(server), e.without_url()),
                Some("Check this computer's internet connection (and a VPN or proxy that blocks it)."),
            )),
        }
        let client = match crate::server::client::DesktopClient::new(&self.paths) {
            Ok(c) => c,
            Err(e) => {
                out.push(Check::new("session", "Session accepted", Level::Fail, e, None));
                return out;
            }
        };
        match client.phone().await {
            Ok(seen) => {
                out.push(Check::new("session", "Session accepted", Level::Ok, "The server knows this computer", None));
                out.push(phone_check(seen.last_seen, seen.server_time));
            }
            Err(LinkError::LoggedOut(m)) => out.push(Check::new(
                "session",
                "Session accepted",
                Level::Fail,
                m,
                Some("Pair again: `reins login` (the phone may have removed this computer)."),
            )),
            Err(LinkError::NotFound) => {
                out.push(Check::new("session", "Session accepted", Level::Ok, "The server knows this computer", None));
                out.push(Check::new(
                    "phone",
                    "Phone seen",
                    Level::Skip,
                    "This server does not say when the phone was last seen (it is older).",
                    None,
                ));
            }
            Err(LinkError::Failed(m)) => {
                out.push(Check::new("session", "Session accepted", Level::Warn, m, None));
            }
        }
        out
    }

    async fn service(&self) -> Check {
        let not_running = |detail: &str| {
            Check::new(
                "service",
                "Background service",
                Level::Fail,
                detail,
                Some("Run `reins resume` (or Start in the Reins app)."),
            )
        };
        let client = match Client::new(&self.paths, self.config.listen) {
            Ok(c) => c,
            Err(ClientError::NotRunning) => return not_running("Not running: git and the API proxy do not work."),
            Err(e) => return not_running(&e.to_string()),
        };
        match client.status().await {
            Ok(s) if s.version != env!("CARGO_PKG_VERSION") => Check::new(
                "service",
                "Background service",
                Level::Warn,
                format!("Runs reins {}, this is {}", s.version, env!("CARGO_PKG_VERSION")),
                Some("Restart it so it runs this version: `reins service install`."),
            ),
            Ok(s) => Check::new("service", "Background service", Level::Ok, format!("Running on {}", s.listen), None),
            Err(ClientError::NotRunning) => not_running("Not running: git and the API proxy do not work."),
            Err(e) => not_running(&e.to_string()),
        }
    }

    fn git_check(&self) -> Check {
        let enabled: Vec<String> =
            self.config.enabled_hosts().map(|h| h.into_iter().map(|h| h.host).collect()).unwrap_or_default();
        if enabled.is_empty() {
            return Check::new("git", "git through Reins", Level::Skip, "No git host is enabled.", None);
        }
        let routed = match self.git.hosts_set_up(&Scope::Global, &self.config) {
            Ok(r) => r,
            Err(e) => return Check::new("git", "git through Reins", Level::Warn, e, None),
        };
        let missing: Vec<&String> = enabled.iter().filter(|h| !routed.contains(h)).collect();
        if missing.is_empty() {
            Check::new("git", "git through Reins", Level::Ok, enabled.join(", "), None)
        } else if routed.is_empty() {
            Check::new(
                "git",
                "git through Reins",
                Level::Warn,
                "git talks to the hosts directly (paused, or never turned on): pushes are not checked.",
                Some("Run `reins resume` (or Resume in the Reins app)."),
            )
        } else {
            Check::new(
                "git",
                "git through Reins",
                Level::Warn,
                format!("Not routed: {}", missing.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(", ")),
                Some("Run `reins resume` to route every enabled host."),
            )
        }
    }

    fn harness_checks(&self) -> Vec<Check> {
        let setup = harness::Setup {
            home: self.home.clone(),
            exe: self.exe.clone(),
            hook_timeout_secs: self.config.guard.timeout_secs,
        };
        let mut out = Vec::new();
        for h in Harness::ALL {
            let found = harness::detect::found(h, &self.home);
            let reg = harness::registered(&self.paths, &setup, h).ok();
            let id = format!("harness:{}", h.id());
            let label = h.label();
            let add = format!("Run `reins harness add {}` (or connect it in the Reins app).", h.id());
            let check = match (found, reg) {
                (_, Some(r)) if r.complete() => {
                    Check::new(&id, label, Level::Ok, "Connected: its tools and risky commands reach your phone", None)
                }
                (_, Some(r)) if r.any() => Check::new(
                    &id,
                    label,
                    Level::Warn,
                    "Partly connected, or set up for another copy of reins",
                    Some(&add),
                ),
                (true, _) => Check::new(&id, label, Level::Warn, "Installed but not connected", Some(&add)),
                (false, _) => continue,
            };
            out.push(check);
        }
        if out.is_empty() {
            out.push(Check::new(
                "harness",
                "AI tools",
                Level::Skip,
                "No AI tool found (Claude Code, Codex, Gemini CLI, Cursor).",
                None,
            ));
        }
        out
    }

    fn notifications(&self) -> Check {
        if !self.config.notify.phone {
            return Check::new(
                "notify",
                "\"Check your phone\" notifications",
                Level::Skip,
                "Turned off in settings",
                None,
            );
        }
        if cfg!(all(unix, not(target_os = "macos"))) && !on_path("notify-send") {
            return Check::new(
                "notify",
                "\"Check your phone\" notifications",
                Level::Warn,
                "`notify-send` is not installed: this computer cannot say when your phone needs you.",
                Some("Install libnotify (`notify-send`), e.g. `sudo apt install libnotify-bin`."),
            );
        }
        Check::new("notify", "\"Check your phone\" notifications", Level::Ok, "On", None)
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn host_of(server: &str) -> &str {
    server.split("://").nth(1).unwrap_or(server).trim_end_matches('/')
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The clock check: `server` and `local` in Unix seconds.
#[must_use]
pub fn clock_check(server: f64, local: f64) -> Check {
    let skew = local - server;
    let off = skew.abs();
    let detail = |what: &str| {
        let dir = if skew > 0.0 {
            "ahead of"
        } else {
            "behind"
        };
        format!("{what}: {off:.0} s {dir} the server")
    };
    let fix = Some("Turn on automatic date and time on this computer.");
    if off <= SKEW_WARN_SECS {
        Check::new("clock", "Clock", Level::Ok, format!("Within {:.0} s of the server", off.max(1.0)), None)
    } else if off <= SKEW_WARN_SECS * 10.0 {
        Check::new("clock", "Clock", Level::Warn, detail("Off"), fix)
    } else {
        Check::new("clock", "Clock", Level::Fail, detail("Far off (sign-ins and grant times depend on it)"), fix)
    }
}

/// The phone check: when it last polled, by the server's clock.
#[must_use]
pub fn phone_check(last_seen: Option<i64>, server_time: i64) -> Check {
    let open = Some("Open Reins on your phone once (and allow its notifications), then check again.");
    match last_seen {
        None => Check::new(
            "phone",
            "Phone seen",
            Level::Warn,
            "Your phone has not checked in since the server started.",
            open,
        ),
        Some(at) => {
            let ago = (server_time - at).max(0);
            let text = format!("Last checked in {} ago", crate::ago(ago));
            if ago > PHONE_QUIET_SECS {
                Check::new("phone", "Phone seen", Level::Warn, text, open)
            } else {
                Check::new("phone", "Phone seen", Level::Ok, text, None)
            }
        }
    }
}

/// Unix seconds (with fractions) from RFC 3339 UTC text (`2026-10-09T12:35:31.451918Z`).
#[must_use]
#[allow(clippy::many_single_char_names, reason = "the calendar formula's own names")]
pub fn parse_rfc3339(s: &str) -> Option<f64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec: f64 = t.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || !(0.0..61.0).contains(&sec) {
        return None;
    }
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 {
        y - 1
    } else {
        y
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    #[allow(clippy::cast_precision_loss, reason = "seconds since 1970 fit an f64 exactly")]
    Some((days * 86_400 + h * 3_600 + min * 60) as f64 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_time_parses() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_rfc3339("2026-10-09T12:35:31.5Z"), Some(1_791_549_331.5));
        assert_eq!(parse_rfc3339("2000-02-29T23:59:59Z"), Some(951_868_799.0));
        for bad in ["2026-10-09 12:35:31Z", "2026-13-01T00:00:00Z", "2026-10-09T12:35:31", "x"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_clock_is_judged_by_how_far_off_it_is() {
        assert_eq!(clock_check(1_000.0, 1_005.0).level, Level::Ok);
        let ahead = clock_check(1_000.0, 1_100.0);
        assert_eq!(ahead.level, Level::Warn);
        assert!(ahead.detail.contains("100 s ahead"), "{}", ahead.detail);
        assert_eq!(clock_check(1_000.0, 0.0).level, Level::Fail);
        assert!(clock_check(1_000.0, 0.0).fix.is_some());
    }

    #[test]
    fn the_phone_is_judged_by_when_it_last_checked_in() {
        assert_eq!(phone_check(Some(990), 1_000).level, Level::Ok);
        assert!(phone_check(Some(990), 1_000).detail.contains("10 s"));
        assert_eq!(phone_check(Some(0), 200_000).level, Level::Warn);
        assert_eq!(phone_check(None, 1_000).level, Level::Warn);
    }

    #[test]
    fn the_overall_level_is_the_worst_that_applies() {
        let c = |level| Check::new("x", "x", level, "", None);
        assert_eq!(overall(&[c(Level::Ok), c(Level::Skip)]), Level::Ok);
        assert_eq!(overall(&[c(Level::Ok), c(Level::Warn), c(Level::Skip)]), Level::Warn);
        assert_eq!(overall(&[c(Level::Fail), c(Level::Warn)]), Level::Fail);
        assert_eq!(overall(&[]), Level::Ok);
    }

    #[tokio::test]
    async fn an_unpaired_computer_is_told_to_set_up_and_start_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        // Nothing listens here.
        let config = Config {
            listen: "127.0.0.1:9".parse().unwrap(),
            ..Config::default()
        };
        let doctor = Doctor {
            paths,
            config,
            exe: home.join("reins"),
            git: Git::default().env("HOME", &home).env("XDG_CONFIG_HOME", home.join(".config")),
            home,
        };
        let checks = doctor.run().await;
        let by = |id: &str| checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id}: {checks:?}"));
        assert_eq!(by("pairing").level, Level::Fail);
        assert!(by("pairing").fix.as_deref().unwrap().contains("reins setup"));
        assert_eq!(by("service").level, Level::Fail);
        assert!(by("service").fix.as_deref().unwrap().contains("reins resume"));
        assert_eq!(by("git").level, Level::Warn);
        assert_eq!(by("harness:codex").level, Level::Warn);
        assert!(by("harness:codex").fix.as_deref().unwrap().contains("reins harness add codex"));
        assert_eq!(overall(&checks), Level::Fail);
    }
}
