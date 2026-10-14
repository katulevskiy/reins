//! The daemon's leases of Reins Secrets: environments the phone released, held in memory only ([`Zeroizing`]) until
//! they expire, are dropped (`reins secrets revoke`, the desktop app) or revoked from the phone (Revoke on all
//! computers, picked up by [`poll_revocations`]).
//!
//! A lease is reused only for what the phone bound it to: the same runner, the same project folder, the same project
//! and environment, variables among those released (every variable only after a release of every one), and a command
//! the released command scope covers (exactly the approved command, or the prefix the user chose on the phone).
//! Anything else asks the phone again.
//!
//! Control routes (with the control token): `POST /_reins/secrets/lease` ([`LeaseRequest`] → [`LeaseAnswer`]),
//! `GET /_reins/secrets/leases` ([`LeaseInfo`]s, never a value), `POST /_reins/secrets/revoke` ([`RevokeRequest`] →
//! `{"dropped": n}`).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hyper::{Method, Response, StatusCode};
use reins_proto::desktop::{CommandScope, LeaseRevocation, REVOCATION_TTL_SECS, TO_DESKTOP};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use crate::auth::Refusal;
use crate::journal::{Entry, Kind};
use crate::phone::{PhoneLink, awaiting};
use crate::proxy::{Body, full, text};
use crate::secrets_client::{LeaseRequest, Released, release_env};

pub const PREFIX: &str = "/_reins/secrets/";
/// The largest request body of a control route.
pub const MAX_BODY: usize = 256 * 1024;
/// How often the daemon asks the server for revocations while it holds leases.
const POLL_EVERY: Duration = Duration::from_secs(30);

/// One environment held for some commands.
pub struct Lease {
    pub id: String,
    pub project: String,
    pub env: String,
    pub runner: String,
    pub root: String,
    pub command: CommandScope,
    /// Every variable of the environment was released (else only those in `values`).
    pub all: bool,
    pub values: Vec<(String, Zeroizing<String>)>,
    pub revision: String,
    pub created_at: i64,
    pub expires_at: i64,
}

impl Lease {
    /// Whether this lease may answer `req` now (see the module docs).
    #[must_use]
    pub fn covers(&self, req: &LeaseRequest, now: i64) -> bool {
        now < self.expires_at
            && !req.print
            && self.runner == req.runner
            && self.root == req.root
            && self.project == req.project
            && self.env == req.env
            && self.command.matches(&req.program, &req.args)
            && match &req.fields {
                None => self.all,
                Some(asked) => asked.iter().all(|f| self.values.iter().any(|(n, _)| n == f)),
            }
    }

    fn info(&self) -> LeaseInfo {
        LeaseInfo {
            id: self.id.clone(),
            project: self.project.clone(),
            env: self.env.clone(),
            runner: self.runner.clone(),
            root: self.root.clone(),
            command: self.command.shown(),
            variables: self.values.iter().map(|(n, _)| n.clone()).collect(),
            all: self.all,
            revision: self.revision.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}

/// A lease as listed: never a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub id: String,
    pub project: String,
    pub env: String,
    pub runner: String,
    pub root: String,
    pub command: String,
    pub variables: Vec<String>,
    pub all: bool,
    pub revision: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// What `POST /_reins/secrets/lease` answers: the values for the command, from a lease held or just released.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseAnswer {
    pub id: String,
    pub expires_at: i64,
    pub revision: String,
    /// The variables asked for (every one, for a whole environment), sorted by name.
    pub values: Vec<(String, String)>,
    /// From a lease held: the phone was not asked.
    pub cached: bool,
}

impl Drop for LeaseAnswer {
    fn drop(&mut self) {
        for (_, v) in &mut self.values {
            v.zeroize();
        }
    }
}

impl std::fmt::Debug for LeaseAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.values.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("LeaseAnswer").field("id", &self.id).field("names", &names).finish_non_exhaustive()
    }
}

/// `POST /_reins/secrets/revoke`: one lease by id, the leases of a project (and environment), or every lease.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub all: bool,
}

/// The leases held, and the revocations already applied.
#[derive(Default)]
pub struct LeaseCache {
    leases: Mutex<Vec<Lease>>,
    applied: Mutex<BTreeSet<String>>,
}

impl LeaseCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Lease>> {
        self.leases.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The values a held lease gives `req`, if one covers it.
    #[must_use]
    pub fn find(&self, req: &LeaseRequest, now: i64) -> Option<LeaseAnswer> {
        let mut all = self.lock();
        all.retain(|l| l.expires_at > now);
        let lease = all.iter().find(|l| l.covers(req, now))?;
        Some(answer(lease, req, true))
    }

    pub fn insert(&self, lease: Lease) {
        self.lock().push(lease);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    #[must_use]
    pub fn list(&self, now: i64) -> Vec<LeaseInfo> {
        let mut all = self.lock();
        all.retain(|l| l.expires_at > now);
        all.iter().map(Lease::info).collect()
    }

    /// Drops what `r` names; how many.
    pub fn revoke(&self, r: &RevokeRequest) -> usize {
        let mut all = self.lock();
        let before = all.len();
        all.retain(|l| {
            let named = r.all
                || r.id.as_deref() == Some(l.id.as_str())
                || (r.id.is_none()
                    && r.project.as_deref() == Some(l.project.as_str())
                    && r.env.as_deref().is_none_or(|e| e == l.env));
            !named
        });
        before - all.len()
    }

    /// Applies a revocation from the phone once; how many leases it dropped.
    pub fn apply(&self, r: &LeaseRevocation, now: i64) -> usize {
        if r.dir != TO_DESKTOP || r.issued_at < now - REVOCATION_TTL_SECS {
            return 0;
        }
        if !self.applied.lock().unwrap_or_else(PoisonError::into_inner).insert(r.id.clone()) {
            return 0;
        }
        self.revoke(&RevokeRequest {
            id: None,
            all: r.project.is_none(),
            project: r.project.clone(),
            env: r.env.clone(),
        })
    }
}

fn answer(lease: &Lease, req: &LeaseRequest, cached: bool) -> LeaseAnswer {
    let values = lease
        .values
        .iter()
        .filter(|(n, _)| req.fields.as_ref().is_none_or(|f| f.contains(n)))
        .map(|(n, v)| (n.clone(), v.to_string()))
        .collect();
    LeaseAnswer {
        id: lease.id.clone(),
        expires_at: lease.expires_at,
        revision: lease.revision.clone(),
        values,
        cached,
    }
}

/// Checks a request before it reaches the phone.
fn check(req: &LeaseRequest) -> Result<(), String> {
    use reins_proto::desktop::{name_ok, runner_ok, var_ok};
    if !name_ok(&req.project) || !name_ok(&req.env) {
        return Err("not a project and an environment".to_owned());
    }
    if !runner_ok(&req.runner) {
        return Err("the runner is person or agent:<harness>".to_owned());
    }
    if req.root.is_empty() || req.fields.as_ref().is_some_and(|f| f.is_empty() || !f.iter().all(|n| var_ok(n))) {
        return Err("a release names the project's folder and variable names".to_owned());
    }
    req.command().validate()
}

/// The leases and the phone: what the control routes do.
pub struct SecretsService {
    cache: Arc<LeaseCache>,
    phone: Arc<PhoneLink>,
    /// One question to the phone at a time per kind of request, so that two runs of the same command ask once.
    asking: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SecretsService {
    #[must_use]
    pub fn new(cache: Arc<LeaseCache>, phone: Arc<PhoneLink>) -> Self {
        Self {
            cache,
            phone,
            asking: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn cache(&self) -> Arc<LeaseCache> {
        Arc::clone(&self.cache)
    }

    /// The values for `req`: from a held lease, else released by the phone (and held for what it bound them to).
    pub async fn lease(&self, req: &LeaseRequest) -> Result<LeaseAnswer, Refusal> {
        check(req).map_err(Refusal::Unavailable)?;
        if req.print {
            return Err(Refusal::Unavailable("A value to print is never held by the daemon.".to_owned()));
        }
        if let Some(hit) = self.cache.find(req, crate::now_unix()) {
            return Ok(hit);
        }
        let key = format!("{}\n{}\n{}\n{}\n{:?}", req.item(), req.runner, req.root, req.program, req.args);
        let gate =
            Arc::clone(self.asking.lock().unwrap_or_else(PoisonError::into_inner).entry(key.clone()).or_default());
        let _turn = gate.lock().await;
        // Another run of the same command may have been approved meanwhile.
        if let Some(hit) = self.cache.find(req, crate::now_unix()) {
            return Ok(hit);
        }
        let phone = self.phone.phone()?;
        let what = format!("{} for `{}`", req.item(), crate::secrets::cut(&req.command().shown(), 80));
        let entry = Entry::new(Kind::Secrets, &what).source(Some("reins run")).detail(Some(&format!(
            "Command: {}\nEnvironment: {}\nVariables: {}\nRun by: {}\nIn: {}",
            req.command().shown(),
            req.item(),
            req.fields.as_ref().map_or_else(|| "all".to_owned(), |f| f.join(", ")),
            req.runner,
            req.root
        )));
        let journal = self.phone.journal().clone();
        let released =
            awaiting(&journal, entry, None, self.phone.timeout(), release_env(&phone, req, Some(&key))).await?;
        let lease = lease_from(req, released, crate::now_unix());
        let out = answer(&lease, req, false);
        self.cache.insert(lease);
        Ok(out)
    }

    /// A control route under [`PREFIX`] (the token was checked).
    pub async fn handle(&self, method: &Method, path: &str, body: &[u8]) -> Response<Body> {
        let json = |value: &dyn erased::Json| match value.to_vec() {
            Ok(bytes) => full(StatusCode::OK, "application/json", bytes),
            Err(e) => text(StatusCode::INTERNAL_SERVER_ERROR, &e),
        };
        match (method, path.strip_prefix(PREFIX).unwrap_or("")) {
            (&Method::POST, "lease") => {
                let Ok(req) = serde_json::from_slice::<LeaseRequest>(body) else {
                    return text(StatusCode::BAD_REQUEST, "A lease request is JSON (LeaseRequest).");
                };
                match self.lease(&req).await {
                    Ok(a) => json(&a),
                    Err(r) => refusal(&r),
                }
            }
            (&Method::GET, "leases") => json(&self.cache.list(crate::now_unix())),
            (&Method::POST, "revoke") => {
                let Ok(r) = serde_json::from_slice::<RevokeRequest>(body) else {
                    return text(StatusCode::BAD_REQUEST, "A revoke request is JSON (RevokeRequest).");
                };
                let dropped = self.cache.revoke(&r);
                log::info!("dropped {dropped} lease(s) of secrets from the command line");
                json(&serde_json::json!({ "dropped": dropped }))
            }
            _ => text(StatusCode::NOT_FOUND, "Unknown control path."),
        }
    }
}

/// How a refusal reaches the command line: 403 denied, 408 no answer, 503 anything else, with the message.
fn refusal(r: &Refusal) -> Response<Body> {
    let status = match r {
        Refusal::Denied(_) => StatusCode::FORBIDDEN,
        Refusal::Waiting(_) => StatusCode::REQUEST_TIMEOUT,
        Refusal::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
    };
    text(status, r.message())
}

/// A refusal again from what [`refusal`] sent.
#[must_use]
pub fn refusal_from(status: u16, message: String) -> Refusal {
    match status {
        403 => Refusal::Denied(message),
        408 => Refusal::Waiting(message),
        _ => Refusal::Unavailable(message),
    }
}

/// The lease a release makes: held until the phone's expiry or the request's own lease, whichever is first.
#[must_use]
pub fn lease_from(req: &LeaseRequest, released: Released, now: i64) -> Lease {
    Lease {
        id: crate::server::random_token(8),
        project: req.project.clone(),
        env: req.env.clone(),
        runner: req.runner.clone(),
        root: req.root.clone(),
        command: released.command,
        all: released.all,
        values: released.values,
        revision: released.revision,
        created_at: now,
        expires_at: released.expires_at.min(now + i64::from(req.lease_secs.max(60))),
    }
}

/// Serialization behind a reference to any `Serialize` (the routes answer several types).
mod erased {
    pub trait Json {
        fn to_vec(&self) -> Result<Vec<u8>, String>;
    }

    impl<T: serde::Serialize> Json for T {
        fn to_vec(&self) -> Result<Vec<u8>, String> {
            serde_json::to_vec(self).map_err(|e| e.to_string())
        }
    }
}

/// While leases are held, asks the server every 30 s for the revocations the phone left for this computer, and applies
/// them. Each is sealed to this app's key; one that does not open or is not a revocation is ignored.
pub async fn poll_revocations(
    cache: Arc<LeaseCache>,
    paths: crate::config::Paths,
    identity: Arc<crate::identity::Identity>,
) {
    loop {
        tokio::time::sleep(POLL_EVERY).await;
        if cache.is_empty() || crate::server::oauth::logged_in_server(&paths).is_none() {
            continue;
        }
        let Ok(client) = crate::server::client::DesktopClient::new(&paths) else {
            continue;
        };
        match client.revocations().await {
            Ok(items) => {
                for sealed in items {
                    let Ok(plain) = identity.unseal(&sealed) else {
                        continue;
                    };
                    if let Ok(r) = serde_json::from_slice::<LeaseRevocation>(&plain) {
                        let dropped = cache.apply(&r, crate::now_unix());
                        log::info!("your phone revoked secrets: dropped {dropped} lease(s)");
                    }
                }
            }
            Err(e) => log::debug!("revocations: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(runner: &str, root: &str, fields: Option<&[&str]>, args: &[&str]) -> LeaseRequest {
        LeaseRequest {
            project: "acme".into(),
            env: "dev".into(),
            fields: fields.map(|f| f.iter().map(|x| (*x).to_owned()).collect()),
            program: "/usr/bin/npm".into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            root: root.into(),
            runner: runner.into(),
            lease_secs: 600,
            purpose: None,
            confirm_always: false,
            print: false,
        }
    }

    fn lease(command: CommandScope, names: &[&str], all: bool) -> Lease {
        Lease {
            id: "l1".into(),
            project: "acme".into(),
            env: "dev".into(),
            runner: "person".into(),
            root: "/home/me/dev/a".into(),
            command,
            all,
            values: names.iter().map(|n| ((*n).to_owned(), Zeroizing::new(format!("v-{n}")))).collect(),
            revision: "r1".into(),
            created_at: 0,
            expires_at: 1_000,
        }
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn a_lease_is_reused_only_for_what_it_was_released_for() {
        let a = "/home/me/dev/a";
        let exact = lease(CommandScope::exact("/usr/bin/npm", &words(&["run", "dev"])), &["A"], false);
        assert!(exact.covers(&req("person", a, Some(&["A"]), &["run", "dev"]), 10));
        assert!(!exact.covers(&req("agent:claude-code", a, Some(&["A"]), &["run", "dev"]), 10), "another runner");
        assert!(!exact.covers(&req("person", "/home/me/dev/b", Some(&["A"]), &["run", "dev"]), 10), "another folder");
        assert!(!exact.covers(&req("person", a, Some(&["A", "B"]), &["run", "dev"]), 10), "more variables");
        assert!(!exact.covers(&req("person", a, None, &["run", "dev"]), 10), "every variable");
        assert!(!exact.covers(&req("person", a, Some(&["A"]), &["run", "test"]), 10), "another command");
        assert!(!exact.covers(&req("person", a, Some(&["A"]), &["run", "dev"]), 1_000), "expired");
        let mut printed = req("person", a, Some(&["A"]), &["run", "dev"]);
        printed.print = true;
        assert!(!exact.covers(&printed, 10), "a value to print is always asked");
        let wide = lease(
            CommandScope {
                program: "/usr/bin/npm".into(),
                args: words(&["run"]),
                prefix: true,
            },
            &["A", "B"],
            true,
        );
        assert!(wide.covers(&req("person", a, None, &["run", "test", "--watch"]), 10));
        assert!(wide.covers(&req("person", a, Some(&["B"]), &["run", "lint"]), 10));
        assert!(!wide.covers(&req("person", a, None, &["runx"]), 10));
    }

    #[test]
    fn leases_are_listed_without_values_and_dropped_by_id_project_or_all() {
        let cache = LeaseCache::default();
        cache.insert(lease(CommandScope::exact("/usr/bin/npm", &words(&["run", "dev"])), &["A", "B"], true));
        let listed = cache.list(10);
        assert_eq!((listed[0].variables.clone(), listed[0].command.as_str()), (words(&["A", "B"]), "npm run dev"));
        assert!(!serde_json::to_string(&listed).unwrap().contains("v-A"));
        let hit = cache.find(&req("person", "/home/me/dev/a", Some(&["B"]), &["run", "dev"]), 10).unwrap();
        assert_eq!((hit.values.clone(), hit.cached), (vec![("B".to_owned(), "v-B".to_owned())], true));
        assert!(!format!("{hit:?}").contains("v-B"));
        assert_eq!(
            cache.revoke(&RevokeRequest {
                project: Some("other".into()),
                ..RevokeRequest::default()
            }),
            0
        );
        assert_eq!(
            cache.revoke(&RevokeRequest {
                project: Some("acme".into()),
                ..RevokeRequest::default()
            }),
            1
        );
        assert!(cache.is_empty());
        cache.insert(lease(CommandScope::exact("/usr/bin/npm", &words(&["x"])), &["A"], false));
        assert_eq!(
            cache.revoke(&RevokeRequest {
                id: Some("l1".into()),
                ..RevokeRequest::default()
            }),
            1
        );
    }

    #[test]
    fn a_revocation_from_the_phone_applies_once_and_only_while_fresh() {
        let cache = LeaseCache::default();
        let r = |id: &str, project: Option<&str>, issued_at: i64| LeaseRevocation {
            v: 1,
            dir: TO_DESKTOP.to_owned(),
            id: id.to_owned(),
            issued_at,
            project: project.map(str::to_owned),
            env: None,
        };
        cache.insert(lease(CommandScope::exact("/usr/bin/npm", &words(&["x"])), &["A"], false));
        assert_eq!(cache.apply(&r("old", None, 10 - REVOCATION_TTL_SECS - 1), 10), 0, "too old");
        assert_eq!(cache.apply(&r("other", Some("shop"), 5), 10), 0, "another project");
        assert_eq!(cache.apply(&r("all", None, 5), 10), 1);
        cache.insert(lease(CommandScope::exact("/usr/bin/npm", &words(&["x"])), &["A"], false));
        assert_eq!(cache.apply(&r("all", None, 5), 10), 0, "applied once");
        let mut wrong = r("w", None, 5);
        wrong.dir = "desktop-to-phone".into();
        assert_eq!(cache.apply(&wrong, 10), 0);
    }

    #[test]
    fn requests_are_checked_before_the_phone_is_asked() {
        assert!(check(&req("person", "/a", None, &["run", "dev"])).is_ok());
        assert!(check(&req("robot", "/a", None, &["run"])).is_err());
        assert!(check(&req("person", "", None, &["run"])).is_err());
        assert!(check(&req("person", "/a", Some(&[]), &["run"])).is_err());
        assert!(check(&req("person", "/a", Some(&["1X"]), &["run"])).is_err());
    }
}
