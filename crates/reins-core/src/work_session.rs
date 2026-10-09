//! Work sessions: before a stretch of focused work, the user approves a bundle of permissions from the desktop app once
//! (`desktop_session`), instead of tapping through each request. On approval each part becomes an ordinary grant of the
//! desktop app's connection (listed under Grants, revocable, logged, origin `session`) that ends with the session:
//! reading the chosen integrations, and pushing with git to named branches (with git fetches of those repositories).
//! The git grants name their operation (`git_push`, `git_fetch`): pushing to a branch does not also allow changing its
//! files, merging into it or reading the repository's issues through the host's API.
//!
//! A session never covers the hard floor: changes asked for every time (force pushes, deleted branches, moved tags,
//! deleting repositories) never look at grants; the vault, purchases, the desktop app's own questions and secrets, and
//! uploaded files are never part of one; nothing that looks like a code or a password is released by any grant. The
//! session request itself is asked every time. Ending a session (`desktop_session_end`) only takes access away, so it
//! is done at once.

use reins_policy::{Grant, ReadScope, Scope, ServiceScope};
use reins_proto::connector::{ConnectorCall, MAX_SESSION_SECS, MIN_SESSION_SECS};
use reins_proto::desktop::{GIT_FETCH_OP, GIT_PUSH_OP};
use reins_proto::ids::{ConnectionId, GrantId};
use serde_json::{Value, json};

use crate::CoreError;
use crate::connector::Preview;
use crate::engine::Engine;
use crate::store::{AuditInfo, AuditRecord};
use crate::views;

/// The grants' origin, as `GrantView.origin` shows it.
pub const ORIGIN: &str = "session";
/// What a session never covers, whatever it asks for.
const NEVER: [&str; 4] = ["vault", reins_proto::connector::DESKTOP, crate::blob::SERVICE_FILES, "payments"];
/// The git hosts a session may push to.
const GIT_SERVICES: [&str; 4] = ["github", "gitlab", "codeberg", "bitbucket"];

/// A branch git may push to during the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Push {
    pub service: String,
    pub repo: String,
    pub branch: String,
}

/// What a session asks for, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub secs: i64,
    pub reason: String,
    pub reads: Vec<String>,
    pub pushes: Vec<Push>,
}

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

fn branch_ok(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 200
        && !b.starts_with(['-', '/'])
        && !b.ends_with(['/', '.'])
        && !b.contains("..")
        && !b.contains("//")
        && !b.contains("@{")
        && b.bytes().all(|c| c.is_ascii_graphic() && !matches!(c, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'))
}

/// The session a `desktop_session` call asks for.
pub fn plan(call: &ConnectorCall) -> Result<Plan, CoreError> {
    let secs = call.args.get("duration_secs").and_then(Value::as_i64).unwrap_or(0);
    if !(MIN_SESSION_SECS..=MAX_SESSION_SECS).contains(&secs) {
        return Err(bad("A work session lasts 15 minutes to 12 hours."));
    }
    let reason = crate::text::one_line(call.str_arg("reason").unwrap_or_default());
    if reason.is_empty() {
        return Err(bad("Say what the session is for (`reason`)."));
    }
    let list = |name: &str| -> Vec<String> {
        call.args
            .get(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let mut reads: Vec<String> = Vec::new();
    for service in list("read") {
        if NEVER.contains(&service.as_str()) {
            return Err(bad(format!("A work session never covers {}.", views::service_name(&service))));
        }
        let known = service == views::SERVICE_GMAIL
            || reins_proto::connector::specs().iter().any(|s| s.service == service && !s.desktop_only);
        if !known {
            return Err(bad(format!("`{service}` is not an integration.")));
        }
        if !reads.contains(&service) {
            reads.push(service);
        }
    }
    let mut pushes: Vec<Push> = Vec::new();
    for item in list("push") {
        let parsed = item.split_once(':').and_then(|(service, rest)| {
            let (repo, branch) = rest.split_once('@')?;
            Some(Push {
                service: service.to_owned(),
                repo: repo.to_owned(),
                branch: branch.to_owned(),
            })
        });
        let Some(p) = parsed else {
            return Err(bad(format!("`{item}` is not `<service>:<repo>@<branch>`.")));
        };
        if !GIT_SERVICES.contains(&p.service.as_str()) {
            return Err(bad(format!("`{}` is not a git host.", p.service)));
        }
        if !crate::connector::git::repo_ok(&p.repo, p.service == "gitlab") || !branch_ok(&p.branch) {
            return Err(bad(format!("`{item}` does not name a repository and a branch.")));
        }
        if !pushes.contains(&p) {
            pushes.push(p);
        }
    }
    if reads.is_empty() && pushes.is_empty() {
        return Err(bad("A work session needs something to allow: integrations to read, or branches to push to."));
    }
    Ok(Plan {
        secs,
        reason,
        reads,
        pushes,
    })
}

/// "2 h", "45 min", "1 h 30 min".
fn span(secs: i64) -> String {
    let (h, m) = (secs / 3_600, (secs % 3_600) / 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// What the phone shows for the session: one line for each thing it allows, and what stays asked.
#[must_use]
pub fn preview(plan: &Plan) -> Preview {
    let mut lines = vec![format!("Work session for {}: {}", span(plan.secs), plan.reason)];
    if !plan.reads.is_empty() {
        let names: Vec<&str> = plan.reads.iter().map(|s| views::service_name(s)).collect();
        lines.push(format!("Read (search, list, read): {}", names.join(", ")));
    }
    for p in &plan.pushes {
        lines.push(format!("Push with git: {} {}, branch {}", views::service_name(&p.service), p.repo, p.branch));
    }
    lines.push(
        "Still asked every time: force pushes and deleted branches, deleting anything, the vault, purchases".to_owned(),
    );
    lines.push("Not part of the session (asked as usual): sending messages, and anything not listed above".to_owned());
    lines.push("For the AI tools on this computer; ends by itself, or earlier from the desktop app.".to_owned());
    Preview {
        resource: ORIGIN.to_owned(),
        resource_label: "Work session".to_owned(),
        lines,
        once_only: true,
        ..Preview::default()
    }
}

fn session_grant(connection: &ConnectionId, scope: Scope, now: i64, expires: i64) -> Result<Grant, CoreError> {
    Grant::new(GrantId(uuid::Uuid::new_v4().to_string()), connection.clone(), scope, now, Some(expires), None)
        .map_err(|e| CoreError::invalid(e.to_string()))
}

impl Engine {
    /// Creates the session's grants for `connection` (the desktop app's), ending `plan.secs` from `now`. Reads of an
    /// integration that is not connected are left out. Returns what the desktop app is told: the grants (id and what
    /// each allows) and when they end.
    pub(crate) fn start_work_session(
        &self,
        connection: &ConnectionId,
        label: &str,
        plan: &Plan,
        now: i64,
    ) -> Result<Value, CoreError> {
        let expires = now + plan.secs;
        let connected = self.integrations();
        let mut grants: Vec<Grant> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for service in &plan.reads {
            if !connected.contains(service) {
                skipped.push(views::service_name(service).to_owned());
                continue;
            }
            let scope = if service == views::SERVICE_GMAIL {
                Scope::Read(ReadScope {
                    any: true,
                    ..ReadScope::default()
                })
            } else {
                Scope::Service(ServiceScope {
                    service: service.clone(),
                    access: "read".to_owned(),
                    resources: Vec::new(),
                    labels: Vec::new(),
                    any: true,
                    classes: Vec::new(),
                    ops: Vec::new(),
                })
            };
            grants.push(session_grant(connection, scope, now, expires)?);
        }
        for p in &plan.pushes {
            let branch = format!("{}@{}", p.repo, p.branch);
            grants.push(session_grant(
                connection,
                Scope::Service(ServiceScope {
                    service: p.service.clone(),
                    access: "write".to_owned(),
                    resources: vec![branch],
                    labels: vec![format!("{}, branch {}", p.repo, p.branch)],
                    any: false,
                    classes: vec!["code".to_owned()],
                    // git pushes only: not also changing the branch's files through the host's API.
                    ops: vec![GIT_PUSH_OP.to_owned()],
                }),
                now,
                expires,
            )?);
            // A push reads the repository first (its branches).
            if !plan.reads.contains(&p.service) {
                grants.push(session_grant(
                    connection,
                    Scope::Service(ServiceScope {
                        service: p.service.clone(),
                        access: "read".to_owned(),
                        resources: vec![p.repo.clone()],
                        labels: vec![p.repo.clone()],
                        any: false,
                        classes: Vec::new(),
                        // git fetches only: not also the repository's issues or files through the host's API.
                        ops: vec![GIT_FETCH_OP.to_owned()],
                    }),
                    now,
                    expires,
                )?);
            }
        }
        for g in &grants {
            self.store.insert_grant_from(g, label, ORIGIN)?;
        }
        let summary = format!("a work session for {}: {}", span(plan.secs), plan.reason);
        self.store.append_audit(&AuditRecord {
            seq: 0,
            at: now,
            connection_id: connection.0.clone(),
            connection_label: label.to_owned(),
            action: "grant".to_owned(),
            outcome: "granted".to_owned(),
            detail: format!("allowed: {summary}"),
            grant_id: grants.first().map(|g| g.id.0.clone()),
            service: String::new(),
            account: None,
            count: u32::try_from(grants.len()).unwrap_or(u32::MAX),
            op: String::new(),
            info: AuditInfo {
                grant_summary: Some(summary),
                note: (!skipped.is_empty()).then(|| format!("Not connected, so not included: {}.", skipped.join(", "))),
                ..AuditInfo::default()
            },
        })?;
        Ok(json!({
            "expires_at": expires,
            "grants": grants.iter().map(|g| json!({"id": g.id.0, "summary": views::grant_summary(&g.scope)})).collect::<Vec<_>>(),
            "skipped": skipped,
        }))
    }

    /// Revokes the listed grants of `connection` that a work session made; others are left alone. Returns how many
    /// ended.
    pub(crate) fn end_work_session(&self, connection: &ConnectionId, ids: &[String]) -> Result<usize, CoreError> {
        let mut ended = 0;
        for stored in self.store.grants()? {
            let g = &stored.grant;
            if stored.origin == ORIGIN && g.connection_id == *connection && ids.contains(&g.id.0) {
                ended += usize::from(self.store.revoke_grant(&g.id)?);
            }
        }
        Ok(ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: reins_proto::connector::DESKTOP.to_owned(),
            op: reins_proto::connector::SESSION_OP.to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn a_session_is_read_with_its_limits() {
        let p = plan(&call(&json!({"duration_secs": 7200, "reason": "Fix the login bug",
            "read": ["gmail", "github", "gmail"], "push": ["github:me/app@feature/login"]})))
        .unwrap();
        assert_eq!(p.secs, 7200);
        assert_eq!(p.reads, ["gmail", "github"]);
        assert_eq!(p.pushes[0].branch, "feature/login");
        let shown = preview(&p);
        assert!(shown.once_only, "the session itself is asked every time");
        assert!(shown.lines[0].starts_with("Work session for 2 h: Fix the login bug"));
        assert!(shown.lines.iter().any(|l| l.contains("me/app, branch feature/login")));
        assert!(shown.lines.iter().any(|l| l.starts_with("Still asked every time")));
    }

    #[test]
    fn the_hard_floor_and_nonsense_are_refused() {
        let base = |extra: Value| {
            let mut v = json!({"duration_secs": 3600, "reason": "work"});
            for (k, val) in extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            plan(&call(&v))
        };
        for never in ["vault", "desktop", "payments"] {
            assert!(base(json!({"read": [never]})).is_err(), "{never}");
        }
        assert!(base(json!({"read": ["nope"]})).is_err());
        assert!(base(json!({})).is_err(), "nothing to allow");
        assert!(base(json!({"push": ["telegram:me/app@x"]})).is_err(), "not a git host");
        for bad in ["github:me/app", "github:me@x", "github:../x@main", "github:me/app@", "github:me/app@a..b"] {
            assert!(base(json!({"push": [bad]})).is_err(), "{bad}");
        }
        assert!(plan(&call(&json!({"duration_secs": 60, "reason": "x", "read": ["gmail"]}))).is_err());
        assert!(plan(&call(&json!({"duration_secs": 13 * 3600, "reason": "x", "read": ["gmail"]}))).is_err());
        assert!(plan(&call(&json!({"duration_secs": 3600, "reason": " ", "read": ["gmail"]}))).is_err());
    }

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span(900), "15 min");
        assert_eq!(span(7200), "2 h");
        assert_eq!(span(5400), "1 h 30 min");
    }
}
