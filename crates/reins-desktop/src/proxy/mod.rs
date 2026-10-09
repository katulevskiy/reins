//! The git smart-HTTP proxy. Git sends plain HTTP on loopback; the proxy forwards to the git host (GitHub, GitLab,
//! Codeberg, Bitbucket, as configured), adding a credential only after the [`Authorizer`] allowed the read or that
//! exact push. Packs stream through in both directions, except a push, which is held whole: it is described and
//! digested before anything reaches the host.

pub mod body;
pub mod report;
pub mod route;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use reins_proto::desktop::push_digest;

use self::body::{BodyError, MAX_PUSH, Spool, parse_push, read_body};
use self::route::{Kind, Route, Service};
use crate::auth::{Authorizer, Credential, Refusal, Repo};
use crate::config::{Config, GitHost};
use crate::git::{GitHubRemote, NoRemote, Remote, analyze_push};
use crate::journal::{Entry, Journal, Kind as JournalKind};
use crate::notice::{self, Peer};
use crate::stats::Stats;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
/// What the daemon answers with.
pub type Body = UnsyncBoxBody<Bytes, BoxError>;

/// A fetch's request (wants and haves) is held for a possible retry with a credential; bounded.
const MAX_FETCH_REQUEST: u64 = 256 << 20;
const MAX_REDIRECTS: usize = 3;
/// Repositories remembered as private or moved (bounded: the names come from the agent).
const MAX_REMEMBERED: usize = 10_000;

/// Headers git sends that GitHub needs.
const REQUEST_HEADERS: [HeaderName; 6] = [
    HeaderName::from_static("git-protocol"),
    header::CONTENT_TYPE,
    header::CONTENT_ENCODING,
    header::ACCEPT,
    header::ACCEPT_ENCODING,
    header::USER_AGENT,
];
/// Headers of GitHub's answer git needs; never `WWW-Authenticate` (git would ask for a password) nor `Location`.
const RESPONSE_HEADERS: [HeaderName; 5] =
    [header::CONTENT_TYPE, header::CONTENT_ENCODING, header::CACHE_CONTROL, header::EXPIRES, header::PRAGMA];

#[must_use]
pub fn full(status: StatusCode, content_type: &'static str, bytes: impl Into<Bytes>) -> Response<Body> {
    let mut r = Response::new(Full::new(bytes.into()).map_err(|never| match never {}).boxed_unsync());
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    r
}

/// A message git shows as `remote: <line>`.
#[must_use]
pub fn text(status: StatusCode, message: &str) -> Response<Body> {
    let mut body = message.trim_end().to_owned();
    body.push('\n');
    full(status, "text/plain; charset=utf-8", body)
}

/// "push to github.com/me/app: main (3 commits), v1.2", for the waiting line.
fn push_what(repo: &Repo, summary: &reins_proto::desktop::PushSummary) -> String {
    let refs: Vec<String> = summary
        .updates
        .iter()
        .map(|u| {
            let name = u.branch().or_else(|| u.tag()).unwrap_or(&u.name);
            match (u.change, u.commit_count) {
                (reins_proto::desktop::RefChange::Delete, _) => format!("delete {name}"),
                (_, 0) => name.to_owned(),
                (_, 1) => format!("{name} (1 commit)"),
                (_, n) => format!("{name} ({n} commits)"),
            }
        })
        .collect();
    format!("push to {}: {}", repo.label(), refs.join(", "))
}

fn refused(r: &Refusal) -> Response<Body> {
    text(StatusCode::FORBIDDEN, &with_hint(r.message()))
}

/// The phone's refusal, plus what to do when the host is not connected there (the phone words it for an AI).
fn with_hint(message: &str) -> String {
    if message.contains("is not connected") {
        format!(
            "{message}\nConnect it in the Reins app on your phone (Integrations), then try again; `reins pause` lets \
             git talk to the host directly meanwhile."
        )
    } else {
        message.to_owned()
    }
}

/// One word for a refusal, for logs and the connection list.
#[must_use]
pub fn refusal_kind(r: &Refusal) -> &'static str {
    match r {
        Refusal::Denied(_) => "denied",
        Refusal::Waiting(_) => "waiting",
        Refusal::Unavailable(_) => "unavailable",
    }
}

/// What goes upstream as the request body.
enum Out<'a> {
    Empty,
    Held(&'a Spool),
    /// Git's own stream: can be sent once (no redirect, no retry).
    Stream(Incoming),
}

/// An answer for git that ends the request early.
type Failure = Response<Body>;

/// One enabled git host and where its git traffic goes.
struct Upstream {
    host: GitHost,
    git_base: url::Url,
}

impl Upstream {
    fn base_path(&self) -> &str {
        self.git_base.path().trim_end_matches('/')
    }
}

pub struct Proxy {
    /// The enabled hosts, as [`route::parse`] takes them.
    hosts: Vec<GitHost>,
    upstreams: HashMap<String, Upstream>,
    authorizer: Arc<dyn Authorizer>,
    http: reqwest::Client,
    api_http: reqwest::Client,
    /// Lowercase `host/path` of repositories the anonymous request could not read.
    private: Mutex<HashSet<String>>,
    /// Lowercase `host/path` → upstream path (`/owner/new.git`) after the host redirected (a renamed repository).
    moved: Mutex<HashMap<String, String>>,
    journal: Journal,
    stats: Arc<Stats>,
    timeout: Duration,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// "GitLab", for messages.
fn label(repo: &Repo) -> &'static str {
    crate::config::service_label(&repo.service)
}

impl Proxy {
    /// Serves every enabled host of `config`.
    pub fn new(config: &Config, authorizer: Arc<dyn Authorizer>) -> Result<Self, String> {
        let hosts = config.enabled_hosts()?;
        let mut upstreams = HashMap::new();
        for h in &hosts {
            let git_base = url::Url::parse(h.git_base.trim_end_matches('/'))
                .map_err(|e| format!("`git_base` of {}: {e}", h.host))?;
            upstreams.insert(
                h.host.clone(),
                Upstream {
                    host: h.clone(),
                    git_base,
                },
            );
        }
        Ok(Self {
            hosts,
            upstreams,
            authorizer,
            http: crate::http::client(None)?,
            api_http: crate::http::client(Some(Duration::from_secs(60)))?,
            private: Mutex::default(),
            moved: Mutex::default(),
            journal: Journal::off(),
            stats: Arc::default(),
            timeout: Duration::from_secs(config.approval_timeout_secs),
        })
    }

    /// Logs decisions to `journal` and counts connections in `stats`.
    #[must_use]
    pub fn with_activity(mut self, journal: Journal, stats: Arc<Stats>) -> Self {
        self.journal = journal;
        self.stats = stats;
        self
    }

    /// The git hosts served (the enabled ones).
    #[must_use]
    pub fn hosts(&self) -> &[GitHost] {
        &self.hosts
    }

    /// Which proxy request this is, if any (see [`route::parse`]).
    #[must_use]
    pub fn route(&self, method: &Method, path: &str, query: Option<&str>) -> Option<Route> {
        route::parse(&self.hosts, method, path, query)
    }

    #[must_use]
    pub fn authorizer(&self) -> &Arc<dyn Authorizer> {
        &self.authorizer
    }

    fn key(repo: &Repo) -> String {
        repo.label().to_ascii_lowercase()
    }

    /// The upstream of a routed repository (routes only name served hosts).
    fn upstream_of(&self, repo: &Repo) -> Result<&Upstream, Failure> {
        self.upstreams.get(&repo.host).ok_or_else(|| text(StatusCode::NOT_FOUND, "Not a git host this proxy serves."))
    }

    fn upstream(&self, repo: &Repo, suffix: &str) -> Result<url::Url, Failure> {
        let up = self.upstream_of(repo)?;
        let repo_path =
            lock(&self.moved).get(&Self::key(repo)).cloned().unwrap_or_else(|| format!("/{}.git", repo.path));
        let mut url = up.git_base.clone();
        let (path, query) = suffix.split_once('?').map_or((suffix, None), |(p, q)| (p, Some(q)));
        url.set_path(&format!("{}{repo_path}{path}", up.base_path()));
        url.set_query(query);
        Ok(url)
    }

    /// Where a redirect for `repo` points, when it stays on the host and still names a repository: remembered, so
    /// the next requests go there directly.
    fn follow(&self, repo: &Repo, from: &url::Url, location: &str, suffix_path: &str) -> Result<url::Url, String> {
        let host = label(repo);
        let to = from.join(location).map_err(|_| format!("{host} answered with a bad redirect"))?;
        let up = self.upstreams.get(&repo.host).ok_or_else(|| "Not a git host this proxy serves.".to_owned())?;
        let same_origin = to.scheme() == up.git_base.scheme()
            && to.host_str() == up.git_base.host_str()
            && to.port_or_known_default() == up.git_base.port_or_known_default();
        if !same_origin {
            return Err(format!("{host} redirected this request to another host; the proxy does not follow it"));
        }
        let repo_path = to
            .path()
            .strip_prefix(up.base_path())
            .and_then(|p| p.strip_suffix(suffix_path))
            .filter(|p| route::repo_path(&repo.service, p.trim_start_matches('/')).is_some())
            .ok_or_else(|| format!("{host} redirected this repository somewhere the proxy does not follow"))?;
        let mut moved = lock(&self.moved);
        if moved.len() >= MAX_REMEMBERED {
            moved.clear();
        }
        moved.insert(Self::key(repo), repo_path.to_owned());
        log::info!("{} moved on {host} to {repo_path}", repo.label());
        Ok(to)
    }

    /// Sends one request upstream, following same-host redirects (never passed to git).
    async fn send(
        &self,
        method: Method,
        repo: &Repo,
        suffix: &str,
        headers: &HeaderMap,
        mut out: Out<'_>,
        credential: Option<&Credential>,
    ) -> Result<reqwest::Response, Failure> {
        let suffix_path = suffix.split_once('?').map_or(suffix, |(p, _)| p);
        let host = label(repo);
        let mut url = self.upstream(repo, suffix)?;
        for _ in 0..=MAX_REDIRECTS {
            let mut req = self.http.request(method.clone(), url.clone());
            for name in &REQUEST_HEADERS {
                for value in headers.get_all(name) {
                    req = req.header(name, value);
                }
            }
            if let Some(c) = credential {
                let mut value = HeaderValue::from_str(&c.authorization())
                    .map_err(|_| text(StatusCode::BAD_GATEWAY, "The credential is malformed."))?;
                value.set_sensitive(true);
                req = req.header(header::AUTHORIZATION, value);
            }
            let mut sent_stream = false;
            req = match std::mem::replace(&mut out, Out::Empty) {
                Out::Empty => req,
                Out::Held(spool) => {
                    out = Out::Held(spool);
                    let body = spool.body().map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
                    req.header(header::CONTENT_LENGTH, spool.len()).body(body)
                }
                Out::Stream(incoming) => {
                    sent_stream = true;
                    req.body(reqwest::Body::wrap_stream(incoming.into_data_stream()))
                }
            };
            let resp = req.send().await.map_err(|e| {
                let e = e.without_url();
                log::warn!("{} upstream: {e}", repo.label());
                text(StatusCode::BAD_GATEWAY, &format!("Cannot reach {host}: {e}"))
            })?;
            let location = resp.headers().get(header::LOCATION).and_then(|l| l.to_str().ok()).map(str::to_owned);
            match (resp.status().is_redirection(), location) {
                (true, Some(location)) => {
                    url = self
                        .follow(repo, &url, &location, suffix_path)
                        .map_err(|e| text(StatusCode::BAD_GATEWAY, &e))?;
                    if sent_stream {
                        return Err(text(
                            StatusCode::SERVICE_UNAVAILABLE,
                            &format!("The repository moved on {host}; run git again."),
                        ));
                    }
                }
                _ => return Ok(resp),
            }
        }
        Err(text(StatusCode::BAD_GATEWAY, &format!("Too many redirects from {host}.")))
    }

    /// The host's answer for git, streamed.
    fn forward(resp: reqwest::Response) -> Response<Body> {
        let status = resp.status();
        let mut headers = HeaderMap::new();
        for name in &RESPONSE_HEADERS {
            for value in resp.headers().get_all(name) {
                headers.append(name, value.clone());
            }
        }
        let body = reqwest::Body::from(resp).map_err(BoxError::from).boxed_unsync();
        let mut r = Response::new(body);
        *r.status_mut() = status;
        *r.headers_mut() = headers;
        r
    }

    /// An upstream refusal after the credential was added: never a 401 to git (it would ask for a password).
    fn upstream_refused(repo: &Repo, status: StatusCode) -> Response<Body> {
        let host = label(repo);
        if status == StatusCode::NOT_FOUND {
            text(
                StatusCode::NOT_FOUND,
                &format!("{} was not found on {host}, or the token cannot see it.", repo.full_name()),
            )
        } else {
            text(StatusCode::FORBIDDEN, &format!("{host} refused access to {} ({status}).", repo.full_name()))
        }
    }

    fn denied_by_upstream(status: StatusCode) -> bool {
        matches!(status.as_u16(), 401 | 403 | 404)
    }

    /// Waits for a decision through [`crate::phone::awaiting_logged`]: the activity log, "check your phone" and, since
    /// git itself shows nothing while it waits, lines on the git process's stderr saying whose turn it is and how it
    /// ended. Reads are logged when they had to ask; pushes always.
    async fn awaiting<T>(
        &self,
        peer: Option<Peer>,
        repo: &Repo,
        what: &str,
        push: bool,
        decision: impl Future<Output = Result<T, Refusal>>,
    ) -> Result<T, Refusal> {
        let entry = Entry::new(JournalKind::Git, what)
            .source(Some("git"))
            .service(Some(&repo.label()))
            .decider(self.authorizer.decider());
        let tell = peer.map(|p| crate::phone::teller(move |line: &str| notice::tell(p, line)));
        let r = crate::phone::awaiting_logged(&self.journal, entry, tell, self.timeout, push, decision).await;
        let how = match &r {
            Ok(_) if push => "push approved",
            Ok(_) => "read",
            Err(e) => refusal_kind(e),
        };
        self.stats.record("git", &repo.label(), how);
        r
    }

    pub async fn handle(&self, route: Route, req: Request<Incoming>) -> Response<Body> {
        let result = match route.kind.clone() {
            Kind::InfoRefs(Service::UploadPack) | Kind::Rpc(Service::UploadPack) => self.upload_pack(&route, req).await,
            Kind::InfoRefs(Service::ReceivePack) => self.with_read(&route, req, false).await,
            Kind::Lfs {
                ..
            } => self.with_read(&route, req, true).await,
            Kind::Rpc(Service::ReceivePack) => self.receive_pack(&route, req).await,
        };
        match result {
            Ok(r) | Err(r) => r,
        }
    }

    async fn upload_pack(&self, route: &Route, req: Request<Incoming>) -> Result<Response<Body>, Failure> {
        let repo = &route.repo;
        let suffix = route.suffix();
        let method = req.method().clone();
        let peer = req.extensions().get::<Peer>().copied();
        let (parts, incoming) = req.into_parts();
        let known_private = lock(&self.private).contains(&Self::key(repo));
        let mut held = None;
        let mut stream = Some(incoming);
        if !known_private {
            if method == Method::POST {
                let spool = read_body(
                    stream.take().ok_or_else(|| text(StatusCode::INTERNAL_SERVER_ERROR, "no body"))?,
                    MAX_FETCH_REQUEST,
                )
                .await
                .map_err(|e| body_error(&e))?;
                held = Some(spool);
            }
            let out = held.as_ref().map_or(Out::Empty, Out::Held);
            let resp = self.send(method.clone(), repo, &suffix, &parts.headers, out, None).await?;
            if !Self::denied_by_upstream(resp.status()) {
                log::info!("{method} {} {suffix}: public, {}", repo.label(), resp.status().as_u16());
                return Ok(Self::forward(resp));
            }
            log::info!("{method} {} {suffix}: anonymous {}, asking", repo.label(), resp.status().as_u16());
        }
        let credential =
            match self.awaiting(peer, repo, &format!("read {}", repo.label()), false, self.authorizer.read(repo)).await
            {
                Ok(c) => c,
                Err(r) => {
                    log::info!("{method} {} {suffix}: read {}", repo.label(), refusal_kind(&r));
                    return Ok(refused(&r));
                }
            };
        let out = match (&held, stream) {
            (Some(spool), _) => Out::Held(spool),
            (None, Some(s)) if method == Method::POST => Out::Stream(s),
            _ => Out::Empty,
        };
        let resp = self.send(method.clone(), repo, &suffix, &parts.headers, out, Some(&credential)).await?;
        log::info!("{method} {} {suffix}: read allowed, {}", repo.label(), resp.status().as_u16());
        if Self::denied_by_upstream(resp.status()) {
            return Ok(Self::upstream_refused(repo, resp.status()));
        }
        let mut private = lock(&self.private);
        if private.len() >= MAX_REMEMBERED {
            private.clear();
        }
        private.insert(Self::key(repo));
        drop(private);
        Ok(Self::forward(resp))
    }

    /// The ref list before a push, and LFS: always with the read credential.
    async fn with_read(&self, route: &Route, req: Request<Incoming>, lfs: bool) -> Result<Response<Body>, Failure> {
        let repo = &route.repo;
        let suffix = route.suffix();
        let method = req.method().clone();
        let peer = req.extensions().get::<Peer>().copied();
        let (parts, incoming) = req.into_parts();
        let credential =
            match self.awaiting(peer, repo, &format!("read {}", repo.label()), false, self.authorizer.read(repo)).await
            {
                Ok(c) => c,
                Err(r) => {
                    log::info!("{method} {} {suffix}: read {}", repo.label(), refusal_kind(&r));
                    return Ok(refused(&r));
                }
            };
        let out = if method == Method::POST {
            Out::Stream(incoming)
        } else {
            Out::Empty
        };
        let resp = self.send(method.clone(), repo, &suffix, &parts.headers, out, Some(&credential)).await?;
        log::info!("{method} {} {suffix}: read allowed, {}", repo.label(), resp.status().as_u16());
        if !lfs && Self::denied_by_upstream(resp.status()) {
            return Ok(Self::upstream_refused(repo, resp.status()));
        }
        if resp.status() == StatusCode::UNAUTHORIZED {
            return Ok(Self::upstream_refused(repo, resp.status()));
        }
        Ok(Self::forward(resp))
    }

    async fn receive_pack(&self, route: &Route, req: Request<Incoming>) -> Result<Response<Body>, Failure> {
        let repo = &route.repo;
        let name = repo.full_name();
        let who = repo.label();
        let suffix = route.suffix();
        let peer = req.extensions().get::<Peer>().copied();
        let (parts, incoming) = req.into_parts();
        let gzip = match parts.headers.get(header::CONTENT_ENCODING).map(HeaderValue::as_bytes) {
            None | Some(b"identity") => false,
            Some(b"gzip" | b"x-gzip") => true,
            Some(_) => return Ok(text(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported Content-Encoding for a push.")),
        };
        let raw = read_body(incoming, MAX_PUSH).await.map_err(|e| body_error(&e))?;
        let raw = Arc::new(raw);
        let parsing = Arc::clone(&raw);
        let push = tokio::task::spawn_blocking(move || parse_push(&parsing, gzip))
            .await
            .map_err(|_| text(StatusCode::INTERNAL_SERVER_ERROR, "Reading the push failed."))?
            .map_err(|e| body_error(&e))?;
        let read =
            self.awaiting(peer, repo, &format!("read {}", repo.label()), false, self.authorizer.read(repo)).await;
        if push.head.commands.is_empty() {
            // Git's probe before a large chunked push: nothing to approve yet.
            let credential = match read {
                Ok(c) => c,
                Err(r) => return Ok(refused(&r)),
            };
            let resp =
                self.send(Method::POST, repo, &suffix, &parts.headers, Out::Held(&raw), Some(&credential)).await?;
            log::info!("POST {who} {suffix}: probe, {}", resp.status().as_u16());
            return Ok(Self::forward(resp));
        }
        let reject = |reason: &str, message: &str| -> Response<Body> {
            if push.head.has_capability("report-status") || push.head.has_capability("report-status-v2") {
                full(StatusCode::OK, report::CONTENT_TYPE, report::rejection(&push.head, reason, message))
            } else {
                text(StatusCode::FORBIDDEN, message)
            }
        };
        let read = match read {
            Ok(c) => c,
            Err(r) => {
                log::info!("POST {who} {suffix}: read {}", refusal_kind(&r));
                return Ok(reject(r.message(), &with_hint(r.message())));
            }
        };
        let commands: Vec<(String, String, String)> =
            push.head.commands.iter().map(|c| (c.old.clone(), c.new.clone(), c.name.clone())).collect();
        let digest = push_digest(&name, &commands, &push.pack_sha256, &push.head.push_options);
        // Only GitHub's API is known: elsewhere the push is described from the pack alone (history unknown, so asked
        // every time).
        let github = (repo.service == "github")
            .then(|| self.upstream_of(repo).ok())
            .flatten()
            .map(|up| GitHubRemote::new(self.api_http.clone(), &up.host.api_base, &name, Some(&read)));
        let remote: &dyn Remote = match &github {
            Some(r) => r,
            None => &NoRemote,
        };
        let mut summary = analyze_push(
            &name,
            &push.head.commands,
            &push.head.push_options,
            push.pack.as_ref().map(tempfile::NamedTempFile::path),
            remote,
        )
        .await;
        drop(push.pack);
        let fits = summary.fit(reins_proto::desktop::MAX_SUMMARY_BYTES);
        if let Err(e) = summary.validate().and(if fits {
            Ok(())
        } else {
            Err("it is too large to show".to_owned())
        }) {
            log::info!("POST {who} {suffix}: push not describable: {e}");
            return Ok(reject(&e, &format!("This push cannot be approved: {e}.")));
        }
        log::info!(
            "POST {name} {suffix}: push of {} ref(s), {} bytes, digest {}",
            commands.len(),
            raw.len(),
            digest.get(..12).unwrap_or(&digest)
        );
        let credential = match self
            .awaiting(peer, repo, &push_what(repo, &summary), true, self.authorizer.push(repo, &summary, &digest))
            .await
        {
            Ok(c) => c,
            Err(r) => {
                log::info!("POST {who} {suffix}: push {}", refusal_kind(&r));
                return Ok(reject(r.message(), &with_hint(r.message())));
            }
        };
        let resp = self.send(Method::POST, repo, &suffix, &parts.headers, Out::Held(&raw), Some(&credential)).await?;
        log::info!("POST {who} {suffix}: push allowed, {}", resp.status().as_u16());
        if Self::denied_by_upstream(resp.status()) {
            return Ok(Self::upstream_refused(repo, resp.status()));
        }
        Ok(Self::forward(resp))
    }
}

fn body_error(e: &BodyError) -> Response<Body> {
    match e {
        BodyError::TooLarge(_) => text(StatusCode::PAYLOAD_TOO_LARGE, &format!("Refused: {e}.")),
        BodyError::Read(_) | BodyError::Invalid(_) => text(StatusCode::BAD_REQUEST, &format!("Refused: {e}.")),
        BodyError::Io(_) => text(StatusCode::INTERNAL_SERVER_ERROR, &format!("The proxy failed: {e}.")),
    }
}
