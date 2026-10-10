//! Work sessions: before focused work, the user approves on the phone, once, a time-boxed bundle of permissions for the
//! AI tools on this computer (reading chosen integrations, pushing with git to named branches) instead of answering
//! each request. The phone turns it into ordinary grants that end together (see `reins_core::work_session`); what is
//! asked every time (force pushes, deletions, the vault, purchases) stays asked. `reins session start|end`, `reins
//! allow`, and the desktop app's "Start a work session" use this module. The active session is kept in
//! `work-session.json` in the state directory so the app and the tray can show its time left.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::Refusal;
use crate::config::{Config, Paths};
use crate::journal::{Entry, Journal, Kind};
use crate::phone::{PhoneLink, Tell, awaiting};

pub const START_TOOL: &str = "desktop_session";
pub const END_TOOL: &str = "desktop_session_end";
/// The shortest and longest session (the phone checks the same).
pub const MIN_SECS: u64 = 15 * 60;
pub const MAX_SECS: u64 = 12 * 3_600;

/// What to ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub secs: u64,
    pub reason: String,
    /// Integrations to read (`gmail`, `gcalendar`, `github`, ...).
    pub read: Vec<String>,
    /// Branches to push to.
    pub push: Vec<Branch>,
}

/// A branch of a repository on a git host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// `github`, `gitlab`, `codeberg`, `bitbucket`.
    pub service: String,
    pub host: String,
    pub repo: String,
    pub branch: String,
}

impl Branch {
    /// How the phone's tool names it: `github:me/app@feature/x`.
    #[must_use]
    pub fn wire(&self) -> String {
        format!("{}:{}@{}", self.service, self.repo, self.branch)
    }

    /// `main`, `master`, and the like: never pushed to by a preset.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self.branch.as_str(), "main" | "master" | "trunk" | "develop" | "production" | "release")
    }
}

/// The session running on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub reason: String,
    pub started_at: i64,
    pub expires_at: i64,
    /// The phone's grant ids (to end them early).
    pub grants: Vec<String>,
    /// What each grant allows, as the phone says it.
    pub allows: Vec<String>,
    /// Integrations asked for that the phone has no account for.
    #[serde(default)]
    pub skipped: Vec<String>,
}

impl Session {
    /// Seconds left (0 once it ended).
    #[must_use]
    pub fn left(&self, now: i64) -> i64 {
        (self.expires_at - now).max(0)
    }
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.state_dir.join("work-session.json")
}

/// The running session, if one has not ended yet.
#[must_use]
pub fn current(paths: &Paths) -> Option<Session> {
    let s: Session = serde_json::from_slice(&std::fs::read(file(paths)).ok()?).ok()?;
    (s.left(crate::now_unix()) > 0).then_some(s)
}

fn save(paths: &Paths, s: &Session) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    crate::config::write_private(&file(paths), &bytes).map_err(|e| format!("{}: {e}", file(paths).display()))
}

/// `2h`, `90m`, `1h30m`, `45 min`, `1.5h` → seconds.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    let bad = || format!("`{text}` is not a duration like 2h, 90m or 1h30m");
    let mut secs = 0.0_f64;
    let mut number = String::new();
    let mut rest = t.as_str();
    while !rest.is_empty() {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if digits.is_empty() {
            return Err(bad());
        }
        rest = &rest[digits.len()..];
        let unit: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
        rest = &rest[unit.len()..];
        let per = match unit.as_str() {
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600.0,
            "m" | "min" | "mins" | "minute" | "minutes" | "" => 60.0,
            _ => return Err(bad()),
        };
        let n: f64 = digits.parse().map_err(|_| bad())?;
        secs += n * per;
        number.clear();
    }
    if secs <= 0.0 {
        return Err(bad());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "checked positive, at most hours")]
    let secs = secs.round() as u64;
    if !(MIN_SECS..=MAX_SECS).contains(&secs) {
        return Err("a work session lasts 15 minutes to 12 hours".to_owned());
    }
    Ok(secs)
}

/// The names people use for integrations (`mail`, `calendar`) → the phone's ids.
#[must_use]
pub fn service_id(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "mail" | "email" | "gmail" => "gmail".to_owned(),
        "calendar" | "gcal" | "gcalendar" | "google-calendar" => "gcalendar".to_owned(),
        "contacts" | "gcontacts" => "gcontacts".to_owned(),
        other => other.to_owned(),
    }
}

/// The repository and branch checked out in `dir`, when its `origin` is on one of `config`'s git hosts.
#[must_use]
pub fn current_branch(dir: &Path, config: &Config) -> Option<Branch> {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
    };
    // Also on a branch without commits yet; nothing when HEAD is detached.
    let branch = git(&["symbolic-ref", "--short", "HEAD"])?;
    let url = git(&["remote", "get-url", "origin"])?;
    let (host, path) = remote_parts(&url)?;
    let h = config.git_hosts().ok()?.into_iter().find(|h| h.host.eq_ignore_ascii_case(&host))?;
    Some(Branch {
        service: h.service,
        host: h.host,
        repo: path,
        branch,
    })
}

/// `(host, owner/repo)` of a remote URL: `https://github.com/me/app.git`, `git@github.com:me/app`,
/// `ssh://git@github.com/me/app`, or the Reins proxy's `http://127.0.0.1:7457/github.com/me/app`.
#[must_use]
pub fn remote_parts(url: &str) -> Option<(String, String)> {
    let clean = |p: &str| p.trim_matches('/').trim_end_matches(".git").to_owned();
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some((host.to_ascii_lowercase(), clean(path)));
    }
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = clean(parsed.path());
    if matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
        // The proxy: the real host is the first part of the path.
        let (real, repo) = path.split_once('/')?;
        return Some((real.to_ascii_lowercase(), repo.to_owned()));
    }
    (!path.is_empty() && path.contains('/')).then_some((host, path))
}

/// "2 h", "45 min", "1 h 30 min".
#[must_use]
pub fn span(secs: u64) -> String {
    let (h, m) = (secs / 3_600, (secs % 3_600) / 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Asks the phone for the session and waits for the answer (up to the approval timeout); on approval saves it as the
/// running session. `tell` gets the waiting and outcome lines (the terminal; `None` in the app).
pub async fn start(paths: &Paths, config: &Config, request: &Request, tell: Option<Tell>) -> Result<Session, String> {
    if !(MIN_SECS..=MAX_SECS).contains(&request.secs) {
        return Err("a work session lasts 15 minutes to 12 hours".to_owned());
    }
    if request.read.is_empty() && request.push.is_empty() {
        return Err("choose something to allow: integrations to read, or a branch to push to".to_owned());
    }
    let link = phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| r.message().to_owned())?;
    let what = format!("work session for {}: {}", span(request.secs), request.reason);
    let mut detail: Vec<String> = Vec::new();
    if !request.read.is_empty() {
        detail.push(format!("Read: {}", request.read.join(", ")));
    }
    for b in &request.push {
        detail.push(format!("Push: {} {}, branch {}", b.host, b.repo, b.branch));
    }
    let entry = Entry::new(Kind::Ask, &what).source(Some("work session")).detail(Some(&detail.join("\n")));
    let args = json!({
        "duration_secs": request.secs,
        "reason": request.reason,
        "read": request.read,
        "push": request.push.iter().map(Branch::wire).collect::<Vec<_>>(),
    });
    let answer = awaiting(
        link.journal(),
        entry,
        tell,
        link.timeout(),
        phone.ask(
            START_TOOL,
            None,
            |_| args.clone(),
            "No answer from your phone in time. Approve it there, then try again.",
        ),
    )
    .await
    .map_err(|r| refusal_text(&r))?;
    let session = session_from(&answer.data, &request.reason)?;
    save(paths, &session)?;
    Ok(session)
}

fn refusal_text(r: &Refusal) -> String {
    match r {
        Refusal::Denied(m) => format!("denied on your phone: {m}"),
        Refusal::Waiting(m) | Refusal::Unavailable(m) => m.clone(),
    }
}

/// The session the phone's answer describes.
pub fn session_from(data: &Value, reason: &str) -> Result<Session, String> {
    let expires_at = data.get("expires_at").and_then(Value::as_i64).ok_or("the phone's answer has no end time")?;
    let grants = data.get("grants").and_then(Value::as_array).ok_or("the phone's answer lists no permissions")?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(Session {
        reason: reason.to_owned(),
        started_at: crate::now_unix(),
        expires_at,
        grants: grants.iter().map(|g| text(g, "id")).filter(|id| !id.is_empty()).collect(),
        allows: grants.iter().map(|g| text(g, "summary")).filter(|s| !s.is_empty()).collect(),
        skipped: data
            .get("skipped")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default(),
    })
}

/// Ends the running session now: its permissions end on the phone (no approval needed). Returns how many ended;
/// `Ok(0)` when no session runs here.
pub async fn end(paths: &Paths, config: &Config) -> Result<usize, String> {
    let Ok(bytes) = std::fs::read(file(paths)) else {
        return Ok(0);
    };
    let session: Session = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let link = phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| r.message().to_owned())?;
    let grants = session.grants.clone();
    let answer = tokio::time::timeout(
        Duration::from_secs(60),
        phone.ask(END_TOOL, None, |_| json!({"grants": grants}), "The phone did not answer in time."),
    )
    .await
    .map_err(|_| "the phone did not answer in time; end the permissions under Grants on the phone".to_owned())?
    .map_err(|r| refusal_text(&r))?;
    std::fs::remove_file(file(paths)).ok();
    let ended = answer.data.get("ended").and_then(Value::as_u64).unwrap_or(0);
    let entry =
        Entry::new(Kind::Ask, &format!("ended the work session: {}", session.reason)).source(Some("work session"));
    let mut entry = entry;
    entry.end(crate::journal::Outcome::Approved, None);
    link.journal().record(&entry);
    Ok(usize::try_from(ended).unwrap_or(usize::MAX))
}

fn phone_link(paths: &Paths, config: &Config) -> Result<Arc<PhoneLink>, String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let identity =
        Arc::new(crate::identity::Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?);
    Ok(Arc::new(PhoneLink::new(
        config.mode,
        paths.clone(),
        identity,
        Duration::from_secs(config.approval_timeout_secs),
    )))
}

/// The journal of `paths` (for callers that log around a session).
#[must_use]
pub fn journal(paths: &Paths) -> Journal {
    Journal::new(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_people_write_them() {
        assert_eq!(parse_duration("2h"), Ok(7_200));
        assert_eq!(parse_duration("90m"), Ok(5_400));
        assert_eq!(parse_duration("1h30m"), Ok(5_400));
        assert_eq!(parse_duration("45 min"), Ok(2_700));
        assert_eq!(parse_duration("1.5h"), Ok(5_400));
        assert_eq!(parse_duration("30"), Ok(1_800), "minutes by default");
        assert!(parse_duration("5m").is_err(), "too short");
        assert!(parse_duration("13h").is_err(), "too long");
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn remotes_name_their_host_and_repository() {
        let want = Some(("github.com".to_owned(), "me/app".to_owned()));
        assert_eq!(remote_parts("https://github.com/me/app.git"), want);
        assert_eq!(remote_parts("git@github.com:me/app.git"), want);
        assert_eq!(remote_parts("ssh://git@github.com/me/app"), want);
        assert_eq!(remote_parts("http://127.0.0.1:7457/github.com/me/app"), want);
        assert_eq!(
            remote_parts("https://gitlab.com/g/sub/app"),
            Some(("gitlab.com".to_owned(), "g/sub/app".to_owned()))
        );
        assert_eq!(remote_parts("/tmp/local-repo"), None);
    }

    #[test]
    fn the_checked_out_branch_of_a_repository_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(dir.path()).args(args).status().unwrap().success());
        };
        run(&["init", "-q", "-b", "feature/login"]);
        run(&["remote", "add", "origin", "git@github.com:me/app.git"]);
        let b = current_branch(dir.path(), &Config::default()).unwrap();
        assert_eq!((b.service.as_str(), b.repo.as_str(), b.branch.as_str()), ("github", "me/app", "feature/login"));
        assert_eq!(b.wire(), "github:me/app@feature/login");
        assert!(!b.is_default());
        assert!(
            Branch {
                branch: "main".to_owned(),
                ..b
            }
            .is_default()
        );
    }

    #[test]
    fn a_session_is_saved_and_runs_out() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        paths.ensure().unwrap();
        let data = json!({"expires_at": crate::now_unix() + 600, "skipped": ["Google Calendar"],
            "grants": [{"id": "g1", "summary": "write me/app, branch dev"}, {"id": "g2", "summary": "read me/app"}]});
        let s = session_from(&data, "Fix the bug").unwrap();
        assert_eq!(s.grants, ["g1", "g2"]);
        assert_eq!(s.skipped, ["Google Calendar"]);
        save(&paths, &s).unwrap();
        assert_eq!(current(&paths), Some(s.clone()));
        let old = Session {
            expires_at: crate::now_unix() - 1,
            ..s
        };
        save(&paths, &old).unwrap();
        assert_eq!(current(&paths), None);
        assert!(session_from(&json!({"grants": []}), "x").is_err());
        assert_eq!(service_id("Mail"), "gmail");
        assert_eq!(span(5_400), "1 h 30 min");
    }
}
