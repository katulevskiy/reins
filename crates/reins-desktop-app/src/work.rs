//! Work sessions as the window and the tray show them (`reins_desktop::work_session` does the work): the start form
//! (how long, what for, which integrations to read, which branches to push to), the request it makes, the
//! repositories this computer recently used with git, how much time a running session has left, and the tray's first
//! line while one runs.

use std::time::Instant;

use reins_desktop::config::Config;
use reins_desktop::journal::{Entry, Kind};
use reins_desktop::stats::Connection;
use reins_desktop::work_session::{Branch, Request, Session};

use crate::format;
use crate::tray::Look;

/// What the session is for when the field is left empty.
pub const DEFAULT_REASON: &str = "Focused work";
/// The most recent repositories the form offers.
const MAX_REPOS: usize = 6;

/// The integrations the form offers to read: the phone's id and the name shown.
pub const READS: [(&str, &str); 3] = [("gmail", "Mail"), ("gcalendar", "Calendar"), ("github", "GitHub")];

/// The lengths the form offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Length {
    HalfHour,
    Hour,
    #[default]
    TwoHours,
    FourHours,
    EightHours,
}

impl Length {
    pub const ALL: [Self; 5] = [Self::HalfHour, Self::Hour, Self::TwoHours, Self::FourHours, Self::EightHours];

    #[must_use]
    pub fn secs(self) -> u64 {
        match self {
            Self::HalfHour => 30 * 60,
            Self::Hour => 3_600,
            Self::TwoHours => 2 * 3_600,
            Self::FourHours => 4 * 3_600,
            Self::EightHours => 8 * 3_600,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::HalfHour => "30 min",
            Self::Hour => "1 hour",
            Self::TwoHours => "2 hours",
            Self::FourHours => "4 hours",
            Self::EightHours => "8 hours",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::HalfHour => "30m",
            Self::Hour => "1h",
            Self::TwoHours => "2h",
            Self::FourHours => "4h",
            Self::EightHours => "8h",
        }
    }
}

/// A repository this computer recently used with git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    /// `github`, `gitlab`, ... (the config's name for the host).
    pub service: String,
    pub host: String,
    /// `owner/name`.
    pub repo: String,
}

impl Repo {
    /// `github.com/acme/web`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}/{}", self.host, self.repo)
    }
}

/// The repositories git reached lately (the daemon's connections, then the activity log's git requests), newest
/// first, on the hosts `config` knows (whose service names the phone's tool).
#[must_use]
pub fn recent_repos(connections: &[Connection], activity: &[Entry], config: &Config) -> Vec<Repo> {
    let hosts = config.git_hosts().unwrap_or_default();
    let mut seen: Vec<(i64, &str)> = connections
        .iter()
        .filter(|c| c.kind == "git")
        .map(|c| (c.last_at, c.target.as_str()))
        .chain(activity.iter().filter(|e| e.kind == Kind::Git).filter_map(|e| Some((e.at, e.service.as_deref()?))))
        .collect();
    seen.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let mut repos: Vec<Repo> = Vec::new();
    for (_, target) in seen {
        let Some((host, repo)) = target.trim().trim_matches('/').split_once('/') else {
            continue;
        };
        let repo = repo.trim_end_matches(".git");
        if !repo.contains('/') || repo.split('/').any(str::is_empty) {
            continue;
        }
        let Some(h) = hosts.iter().find(|h| h.host.eq_ignore_ascii_case(host)) else {
            continue;
        };
        let found = Repo {
            service: h.service.clone(),
            host: h.host.clone(),
            repo: repo.to_owned(),
        };
        if !repos.iter().any(|r| r.host == found.host && r.repo.eq_ignore_ascii_case(&found.repo)) {
            repos.push(found);
        }
        if repos.len() == MAX_REPOS {
            break;
        }
    }
    repos
}

/// A push row of the form, as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushRow {
    /// No branch: not included.
    Empty,
    Push(Branch),
    /// Why the branch is refused.
    Refused(String),
}

/// The start form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub length: Length,
    pub reason: String,
    /// The integrations to read ([`READS`] ids).
    pub read: Vec<&'static str>,
    /// The repositories offered (as they were when the form opened, so a row keeps its place while typing).
    pub repos: Vec<Repo>,
    /// The branch typed for each of `repos`.
    pub branches: Vec<String>,
}

impl Form {
    #[must_use]
    pub fn new(repos: Vec<Repo>) -> Self {
        Self {
            branches: vec![String::new(); repos.len()],
            repos,
            ..Self::default()
        }
    }

    pub fn toggle_read(&mut self, id: &'static str) {
        if let Some(i) = self.read.iter().position(|r| *r == id) {
            self.read.remove(i);
        } else {
            self.read.push(id);
        }
    }

    /// The push row for repository `i`.
    #[must_use]
    pub fn push_row(&self, i: usize) -> PushRow {
        let (Some(repo), Some(typed)) = (self.repos.get(i), self.branches.get(i)) else {
            return PushRow::Empty;
        };
        let name = typed.trim();
        if name.is_empty() {
            return PushRow::Empty;
        }
        let looks_like_a_branch = !name.starts_with(['-', '/', '.'])
            && !name.ends_with(['/', '.'])
            && !name.contains("..")
            && !name.contains("//")
            && !name.contains("@{")
            && name.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'));
        if !looks_like_a_branch {
            return PushRow::Refused("not a branch name".to_owned());
        }
        let branch = Branch {
            service: repo.service.clone(),
            host: repo.host.clone(),
            repo: repo.repo.clone(),
            branch: name.to_owned(),
        };
        if branch.is_default() {
            return PushRow::Refused(format!("pushes to {name} still ask; name a feature branch"));
        }
        PushRow::Push(branch)
    }

    /// The request to send: `None` while nothing is chosen, or a branch is refused.
    #[must_use]
    pub fn request(&self) -> Option<Request> {
        let mut push = Vec::new();
        for i in 0..self.repos.len() {
            match self.push_row(i) {
                PushRow::Empty => {}
                PushRow::Push(b) => push.push(b),
                PushRow::Refused(_) => return None,
            }
        }
        // In the order the form shows them.
        let read: Vec<String> =
            READS.iter().filter(|(id, _)| self.read.contains(id)).map(|(id, _)| (*id).to_owned()).collect();
        if read.is_empty() && push.is_empty() {
            return None;
        }
        let reason = self.reason.split_whitespace().collect::<Vec<_>>().join(" ");
        Some(Request {
            secs: self.length.secs(),
            reason: if reason.is_empty() {
                DEFAULT_REASON.to_owned()
            } else {
                reason
            },
            read,
            push,
        })
    }
}

/// What a request asks for, in a line: "2 h · read Mail, Calendar · push acme/web (feature/login)".
#[must_use]
pub fn summary(request: &Request) -> String {
    let mut parts = vec![reins_desktop::work_session::span(request.secs)];
    if !request.read.is_empty() {
        let names: Vec<&str> = request
            .read
            .iter()
            .map(|id| READS.iter().find(|(r, _)| r == id).map_or(id.as_str(), |(_, n)| *n))
            .collect();
        parts.push(format!("read {}", names.join(", ")));
    }
    for b in &request.push {
        parts.push(format!("push {} ({})", b.repo, b.branch));
    }
    parts.join(" · ")
}

/// Asking the phone for a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asking {
    pub since: Instant,
    /// How long the phone has to answer (the config's approval wait).
    pub timeout_secs: u64,
    pub request: Request,
}

impl Asking {
    /// The seconds left of the wait, rounded up.
    #[must_use]
    pub fn left(&self, now: Instant) -> u64 {
        let elapsed = now.saturating_duration_since(self.since);
        let left = std::time::Duration::from_secs(self.timeout_secs).saturating_sub(elapsed);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }
}

/// The overview card's state besides the running session itself (which the snapshot has).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// The start form is open.
    pub form: Option<Form>,
    /// Waiting for the phone.
    pub asking: Option<Asking>,
    /// Why the last request did not start a session.
    pub failed: Option<String>,
    /// "End now" asks first.
    pub confirm_end: bool,
    pub ending: bool,
    /// Why ending did not work.
    pub end_error: Option<String>,
    /// `--demo`: the pretend session (never written anywhere).
    pub demo: Option<Session>,
    /// `--demo`: the form to open with the first snapshot (`REINS_DEMO_SESSION=form` or `waiting`).
    pub demo_start: Option<DemoStart>,
}

/// `REINS_DEMO_SESSION`: how the demo's work session card starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoStart {
    /// `1`: a session runs.
    Running,
    /// `form`: the form is open and filled in.
    Form,
    /// `waiting`: and sent; the pretend phone takes its time.
    Waiting,
}

impl DemoStart {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "0" => None,
            "form" => Some(Self::Form),
            "waiting" | "asking" => Some(Self::Waiting),
            _ => Some(Self::Running),
        }
    }
}

/// The demo's filled-in form: Mail to read, and the first repository's `feature/login`.
#[must_use]
pub fn demo_form(repos: Vec<Repo>) -> Form {
    let mut form = Form {
        reason: "Fix the login bug".to_owned(),
        ..Form::new(repos)
    };
    form.toggle_read("gmail");
    if let Some(branch) = form.branches.first_mut() {
        "feature/login".clone_into(branch);
    }
    form
}

/// A session still running at `now`.
#[must_use]
pub fn running(session: Option<&Session>, now: i64) -> Option<&Session> {
    session.filter(|s| s.left(now) > 0)
}

/// What is left of a session, rounded up to the minute: "1 h 12 min".
#[must_use]
pub fn left(session: &Session, now: i64) -> String {
    format::left(session.left(now))
}

/// The tray's first line (and tooltip): what needs the user, or waits on the phone, comes first; then a running
/// session ("Work session: 1 h 12 min left"); else the state.
#[must_use]
pub fn tray_line(look: Look, status: String, session: Option<&Session>, now: i64) -> String {
    match (look, running(session, now)) {
        (Look::Attention | Look::Waiting, _) | (_, None) => status,
        (_, Some(s)) => format!("Work session: {} left", left(s, now)),
    }
}

/// The phone's name for an integration.
#[must_use]
pub fn service_name(id: &str) -> &str {
    match id {
        "gmail" => "Gmail",
        "gcalendar" => "Google Calendar",
        "github" => "GitHub",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(expires_at: i64) -> Session {
        Session {
            reason: "Fix the login bug".to_owned(),
            started_at: expires_at - 7_200,
            expires_at,
            grants: vec!["g1".to_owned()],
            allows: vec!["read Gmail".to_owned()],
            skipped: Vec::new(),
        }
    }

    fn connection(target: &str, last_at: i64) -> Connection {
        Connection {
            kind: "git".to_owned(),
            target: target.to_owned(),
            count: 1,
            last_at,
            last: "read".to_owned(),
        }
    }

    fn form() -> Form {
        Form::new(vec![
            Repo {
                service: "github".to_owned(),
                host: "github.com".to_owned(),
                repo: "acme/web".to_owned(),
            },
            Repo {
                service: "gitlab".to_owned(),
                host: "gitlab.com".to_owned(),
                repo: "acme/infra".to_owned(),
            },
        ])
    }

    #[test]
    fn lengths_are_what_they_say() {
        let secs: Vec<u64> = Length::ALL.iter().map(|l| l.secs()).collect();
        assert_eq!(secs, [1_800, 3_600, 7_200, 14_400, 28_800]);
        assert_eq!(Length::default(), Length::TwoHours);
        let range = reins_desktop::work_session::MIN_SECS..=reins_desktop::work_session::MAX_SECS;
        assert!(secs.iter().all(|s| range.contains(s)), "the phone accepts every length");
        let ids: std::collections::HashSet<_> = Length::ALL.iter().map(|l| l.id()).collect();
        assert_eq!(ids.len(), Length::ALL.len());
        assert_eq!(Length::Hour.label(), "1 hour");
    }

    #[test]
    fn nothing_chosen_asks_nothing() {
        let f = form();
        assert_eq!(f.request(), None);
        assert_eq!(f.push_row(0), PushRow::Empty);
        assert_eq!(f.push_row(9), PushRow::Empty);
    }

    #[test]
    fn requests_carry_reads_pushes_and_a_reason() {
        let mut f = form();
        f.toggle_read("gcalendar");
        f.toggle_read("gmail");
        f.toggle_read("github");
        f.toggle_read("github");
        f.branches[0] = " feature/login ".to_owned();
        f.length = Length::FourHours;
        let r = f.request().unwrap();
        assert_eq!(r.secs, 14_400);
        assert_eq!(r.reason, DEFAULT_REASON, "empty: the default reason");
        assert_eq!(r.read, ["gmail", "gcalendar"], "in the form's order");
        assert_eq!(r.push.len(), 1, "a row without a branch is left out");
        assert_eq!(r.push[0].wire(), "github:acme/web@feature/login");
        assert_eq!(summary(&r), "4 h · read Mail, Calendar · push acme/web (feature/login)");

        f.reason = "  Fix\n the  bug ".to_owned();
        f.branches[1] = "infra-fix".to_owned();
        let r = f.request().unwrap();
        assert_eq!(r.reason, "Fix the bug");
        assert_eq!(r.push[1].service, "gitlab", "the host's own service");
        assert_eq!(r.push[1].host, "gitlab.com");
    }

    #[test]
    fn default_branches_and_bad_names_are_refused() {
        let mut f = form();
        f.toggle_read("gmail");
        f.branches[0] = "main".to_owned();
        assert_eq!(f.push_row(0), PushRow::Refused("pushes to main still ask; name a feature branch".to_owned()));
        assert_eq!(f.request(), None, "a refused row stops the request");
        for bad in ["feature login", "-x", "a..b", "x:y", "topic/"] {
            f.branches[0] = bad.to_owned();
            assert!(matches!(f.push_row(0), PushRow::Refused(_)), "{bad}");
        }
        f.branches[0].clear();
        assert!(f.request().is_some(), "reads alone are enough");
    }

    #[test]
    fn recent_repositories_come_from_git_on_known_hosts() {
        let config = Config::default();
        let mut push = Entry::new(Kind::Git, "push to acme/api (main)").service(Some("github.com/acme/api"));
        push.at = 500;
        let host_only = Entry::new(Kind::Git, "push").service(Some("github.com"));
        let mut other = Entry::new(Kind::Command, "x").service(Some("github.com/acme/nope"));
        other.at = 900;
        let connections = vec![
            connection("github.com/acme/web", 100),
            connection("GitHub.com/acme/web", 50),
            connection("unknown.example/acme/x", 800),
            Connection {
                kind: "api".to_owned(),
                ..connection("github.com/acme/api-key", 700)
            },
        ];
        let repos = recent_repos(&connections, &[push, host_only, other], &config);
        let labels: Vec<String> = repos.iter().map(Repo::label).collect();
        assert_eq!(labels, ["github.com/acme/api", "github.com/acme/web"], "newest first, once each");
        assert!(repos.iter().all(|r| r.service == "github"));

        // The host names its service (as `work_session::current_branch` maps it), nested groups included.
        let repos = recent_repos(&[connection("gitlab.com/g/sub/app.git", 1)], &[], &config);
        assert_eq!((repos[0].service.as_str(), repos[0].repo.as_str()), ("gitlab", "g/sub/app"));
    }

    #[test]
    fn time_left_rounds_up_to_the_minute() {
        let now = 1_000_000;
        let s = session(now + 72 * 60);
        assert_eq!(left(&s, now), "1 h 12 min");
        assert_eq!(left(&s, now + 72 * 60 - 20), "1 min");
        assert_eq!(running(Some(&s), now + 72 * 60), None, "ended");
        assert!(running(Some(&s), now).is_some());
        let asking = Asking {
            since: Instant::now(),
            timeout_secs: 120,
            request: Request::default(),
        };
        assert_eq!(asking.left(asking.since), 120);
        assert_eq!(asking.left(asking.since + std::time::Duration::from_millis(1_500)), 119);
        assert_eq!(asking.left(asking.since + std::time::Duration::from_secs(500)), 0);
    }

    #[test]
    fn the_demo_starts_as_asked() {
        assert_eq!(DemoStart::parse("1"), Some(DemoStart::Running));
        assert_eq!(DemoStart::parse(" Form "), Some(DemoStart::Form));
        assert_eq!(DemoStart::parse("waiting"), Some(DemoStart::Waiting));
        assert_eq!(DemoStart::parse("0"), None);
        let f = demo_form(form().repos);
        let r = f.request().unwrap();
        assert_eq!((r.reason.as_str(), r.read.len(), r.push.len()), ("Fix the login bug", 1, 1));
        assert!(demo_form(Vec::new()).request().is_some(), "Mail alone");
    }

    #[test]
    fn the_tray_puts_what_needs_you_first() {
        let now = 1_000_000;
        let s = session(now + 72 * 60);
        let line = |look, session| tray_line(look, "status".to_owned(), session, now);
        assert_eq!(line(Look::On, Some(&s)), "Work session: 1 h 12 min left");
        assert_eq!(line(Look::Paused, Some(&s)), "Work session: 1 h 12 min left");
        assert_eq!(line(Look::Waiting, Some(&s)), "status");
        assert_eq!(line(Look::Attention, Some(&s)), "status");
        assert_eq!(line(Look::On, None), "status");
        assert_eq!(line(Look::On, Some(&session(now))), "status", "an ended session says nothing");
    }
}
