//! Shared by the proxy and daemon tests: an upstream made of `git http-backend` behind a small hyper server (some
//! repositories private behind a basic-auth token, some moved), a scripted authorizer, a log capture, and real git
//! run in a temporary `HOME`.

#![allow(dead_code, reason = "each test binary uses a different part")]

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use bytes::Bytes;
use data_encoding::BASE64;
use http_body_util::{BodyExt as _, Full};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reins_desktop::auth::{Authorizer, Credential, Refusal, Repo};
use reins_desktop::config::{Config, Mode, Paths};
use reins_desktop::daemon::{Daemon, Options, Running};
use reins_proto::desktop::PushSummary;
use tokio::io::AsyncWriteExt as _;

/// The upstream's token for private repositories.
pub const TOKEN: &str = "ghp_Upstream5ecretToken0123456789";

#[must_use]
pub fn token_basic() -> String {
    BASE64.encode(format!("x-access-token:{TOKEN}").as_bytes())
}

/// One request the upstream saw.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: String,
    pub authorization: Option<String>,
    pub git_protocol: Option<String>,
    pub content_encoding: Option<String>,
    pub body_len: usize,
    pub status: u16,
}

#[derive(Default)]
struct UpstreamState {
    root: PathBuf,
    private: HashSet<String>,
    /// `old/name` → `new/name`
    moved: HashMap<String, String>,
    seen: Vec<Seen>,
}

pub struct Upstream {
    pub addr: SocketAddr,
    pub dir: tempfile::TempDir,
    state: Arc<Mutex<UpstreamState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn reply(status: StatusCode, headers: &[(&str, String)], body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(body.into()));
    *r.status_mut() = status;
    for (k, v) in headers {
        r.headers_mut().append(hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
    }
    r
}

async fn cgi(
    root: &Path,
    method: &str,
    path: &str,
    query: &str,
    headers: &hyper::HeaderMap,
    body: Bytes,
    user: bool,
) -> Response<Full<Bytes>> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", header("content-type"))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("REMOTE_ADDR", "127.0.0.1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Windows programs need these to start at all (the system directory, temporary files).
    #[cfg(windows)]
    for var in ["SystemRoot", "SystemDrive", "TEMP", "TMP", "windir", "ComSpec", "PATHEXT"] {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    for (name, var) in [("content-encoding", "HTTP_CONTENT_ENCODING"), ("git-protocol", "HTTP_GIT_PROTOCOL")] {
        if headers.contains_key(name) {
            cmd.env(var, header(name));
        }
    }
    if user {
        cmd.env("REMOTE_USER", "x-access-token");
    }
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    tokio::spawn(async move {
        stdin.write_all(&body).await.ok();
        drop(stdin);
    });
    let out = child.wait_with_output().await.unwrap();
    let raw = out.stdout;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2)));
    let Some((end, sep)) = split else {
        return reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            &[],
            format!("bad CGI output: {}", String::from_utf8_lossy(&out.stderr)),
        );
    };
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut status = StatusCode::OK;
    let mut hs = Vec::new();
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().to_owned();
        if k.eq_ignore_ascii_case("status") {
            status = StatusCode::from_u16(v[..3].parse().unwrap()).unwrap();
        } else {
            hs.push((k.to_owned(), v));
        }
    }
    let refs: Vec<(&str, String)> = hs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    reply(status, &refs, Bytes::copy_from_slice(&raw[end + sep..]))
}

async fn serve(state: Arc<Mutex<UpstreamState>>, req: Request<hyper::body::Incoming>) -> Response<Full<Bytes>> {
    let method = req.method().to_string();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let headers = req.headers().clone();
    let body = req.into_body().collect().await.map(http_body_util::Collected::to_bytes).unwrap_or_default();
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let authorization = header("authorization");
    let trimmed = path.trim_start_matches('/');
    let mut parts = trimmed.splitn(3, '/');
    let (owner, name) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").trim_end_matches(".git"));
    // Nested repositories (GitLab groups) end at the `.git` the proxy always adds.
    let repo = trimmed.find(".git/").map_or_else(|| format!("{owner}/{name}"), |i| trimmed[..i].to_owned());
    let (root, private, moved) = {
        let s = state.lock().unwrap();
        (s.root.clone(), s.private.contains(&repo), s.moved.get(&repo).cloned())
    };
    let authed = authorization.as_deref() == Some(&format!("Basic {}", token_basic()));
    let resp = if owner == "api" {
        reply(StatusCode::NOT_FOUND, &[("content-type", "application/json".to_owned())], "{\"message\":\"Not Found\"}")
    } else if let Some(to) = moved {
        let rest = path.splitn(4, '/').nth(3).unwrap_or("");
        let q = if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        };
        reply(StatusCode::MOVED_PERMANENTLY, &[("location", format!("/{to}.git/{rest}{q}"))], "")
    } else if private && !authed {
        reply(StatusCode::UNAUTHORIZED, &[("www-authenticate", "Basic realm=\"GitHub\"".to_owned())], "")
    } else if path.ends_with("/info/lfs/objects/batch") {
        reply(
            StatusCode::OK,
            &[("content-type", "application/vnd.git-lfs+json".to_owned())],
            format!("{{\"authed\":{authed}}}"),
        )
    } else {
        cgi(&root, &method, &path, &query, &headers, body.clone(), authed).await
    };
    state.lock().unwrap().seen.push(Seen {
        method,
        path,
        query,
        authorization,
        git_protocol: header("git-protocol"),
        content_encoding: header("content-encoding"),
        body_len: body.len(),
        status: resp.status().as_u16(),
    });
    resp
}

impl Upstream {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(UpstreamState {
            root: dir.path().join("repos"),
            ..UpstreamState::default()
        }));
        std::fs::create_dir_all(dir.path().join("repos")).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let s = Arc::clone(&state);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let s = Arc::clone(&s);
                tokio::spawn(async move {
                    let svc = service_fn(move |req| {
                        let s = Arc::clone(&s);
                        async move { Ok::<_, Infallible>(serve(s, req).await) }
                    });
                    hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await.ok();
                });
            }
        });
        Self {
            addr,
            dir,
            state,
            task,
        }
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[must_use]
    pub fn bare(&self, repo: &str) -> PathBuf {
        self.dir.path().join("repos").join(format!("{repo}.git"))
    }

    /// A bare repository with one commit on `main`, accepting pushes.
    pub fn create(&self, repo: &str, private: bool) {
        let work = self.dir.path().join("seed").join(repo);
        std::fs::create_dir_all(&work).unwrap();
        run_git(&work, &[], &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("README.md"), format!("# {repo}\n")).unwrap();
        run_git(&work, &[], &["add", "."]);
        run_git(
            &work,
            &[],
            &["-c", "user.name=Seed", "-c", "user.email=seed@example.com", "commit", "-q", "-m", "First"],
        );
        let bare = self.bare(repo);
        run_git(self.dir.path(), &[], &["clone", "-q", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
        run_git(&bare, &[], &["config", "http.receivepack", "true"]);
        if private {
            self.state.lock().unwrap().private.insert(repo.to_owned());
        }
    }

    pub fn move_repo(&self, from: &str, to: &str) {
        self.state.lock().unwrap().moved.insert(from.to_owned(), to.to_owned());
    }

    #[must_use]
    pub fn seen(&self) -> Vec<Seen> {
        self.state.lock().unwrap().seen.clone()
    }

    /// The commit `refs/heads/<branch>` points at upstream, if any.
    #[must_use]
    pub fn head(&self, repo: &str, branch: &str) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(self.bare(repo))
            .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}

/// Runs git (blocking) in `dir`, panicking on failure; returns stdout.
pub fn run_git(dir: &Path, env: &[(&str, &Path)], args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir).args(args).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_TERMINAL_PROMPT", "0");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A temporary `HOME` whose git config sends `https://github.com/` through the proxy at `proxy`.
pub struct Home {
    pub dir: tempfile::TempDir,
}

/// What a git run printed.
#[derive(Debug)]
pub struct GitRun {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl GitRun {
    #[must_use]
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

impl Home {
    #[must_use]
    pub fn new(proxy: SocketAddr) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = format!(
            "[user]\n\tname = Agent\n\temail = agent@example.com\n[init]\n\tdefaultBranch = main\n[url \"http://{proxy}/github.com/\"]\n\tinsteadOf = https://github.com/\n[advice]\n\tdetachedHead = false\n"
        );
        std::fs::write(dir.path().join(".gitconfig"), config).unwrap();
        Self {
            dir,
        }
    }

    #[must_use]
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Runs git in `HOME/<rel>` (async, so the in-process daemon keeps serving).
    pub async fn git(&self, rel: &str, args: &[&str]) -> GitRun {
        let cwd = self.path(rel);
        std::fs::create_dir_all(&cwd).unwrap();
        let out = tokio::process::Command::new("git")
            .current_dir(&cwd)
            .args(args)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path(".config"))
            .env("GIT_CONFIG_GLOBAL", self.path(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "/bin/false")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        GitRun {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Like [`Home::git`], panicking on failure.
    pub async fn git_ok(&self, rel: &str, args: &[&str]) -> GitRun {
        let run = self.git(rel, args).await;
        assert!(run.ok, "git {args:?} failed:\n{}", run.all());
        run
    }

    /// Commits a file in the clone `rel`; returns the new commit.
    pub async fn commit(&self, rel: &str, file: &str, contents: &[u8], message: &str) -> String {
        std::fs::write(self.path(rel).join(file), contents).unwrap();
        self.git_ok(rel, &["add", file]).await;
        self.git_ok(rel, &["commit", "-q", "-m", message]).await;
        self.git_ok(rel, &["rev-parse", "HEAD"]).await.stdout.trim().to_owned()
    }
}

/// How the scripted authorizer answers.
#[derive(Clone, Debug)]
pub enum Answer {
    Allow,
    Deny(&'static str),
    Wait(&'static str),
}

/// A scripted [`Authorizer`] that records what it was asked.
pub struct Scripted {
    pub read: Mutex<Answer>,
    pub push: Mutex<Answer>,
    /// How long each answer takes (a phone being asked).
    pub delay: Mutex<std::time::Duration>,
    pub reads: Mutex<Vec<String>>,
    pub pushes: Mutex<Vec<(String, PushSummary, String)>>,
}

impl Scripted {
    #[must_use]
    pub fn new(read: Answer, push: Answer) -> Arc<Self> {
        Arc::new(Self {
            read: Mutex::new(read),
            push: Mutex::new(push),
            delay: Mutex::default(),
            reads: Mutex::default(),
            pushes: Mutex::default(),
        })
    }

    fn answer(a: &Answer) -> Result<Credential, Refusal> {
        match a {
            Answer::Allow => Ok(Credential {
                username: "x-access-token".to_owned(),
                token: zeroize::Zeroizing::new(TOKEN.to_owned()),
                expires_at: reins_desktop::now_unix() + 600,
            }),
            Answer::Deny(m) => Err(Refusal::Denied((*m).to_owned())),
            Answer::Wait(m) => Err(Refusal::Waiting((*m).to_owned())),
        }
    }
}

#[async_trait::async_trait]
impl Authorizer for Scripted {
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal> {
        self.reads.lock().unwrap().push(repo.full_name());
        let delay = *self.delay.lock().unwrap();
        tokio::time::sleep(delay).await;
        Self::answer(&self.read.lock().unwrap())
    }

    async fn push(&self, repo: &Repo, summary: &PushSummary, digest: &str) -> Result<Credential, Refusal> {
        self.pushes.lock().unwrap().push((repo.full_name(), summary.clone(), digest.to_owned()));
        let delay = *self.delay.lock().unwrap();
        tokio::time::sleep(delay).await;
        Self::answer(&self.push.lock().unwrap())
    }

    fn describe(&self) -> String {
        "a script".to_owned()
    }
}

/// Everything the daemon and the proxy logged in this test binary.
static LOGS: Mutex<String> = Mutex::new(String::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _m: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, r: &log::Record<'_>) {
        use std::fmt::Write as _;
        let mut logs = LOGS.lock().unwrap();
        writeln!(logs, "{} {} {}", r.level(), r.target(), r.args()).unwrap();
    }
    fn flush(&self) {}
}

pub fn capture_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

#[must_use]
pub fn logs() -> String {
    LOGS.lock().unwrap().clone()
}

/// A config pointing the proxy at `upstream`, listening on a free loopback port.
#[must_use]
pub fn config(upstream: &Upstream) -> Config {
    let mut c = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        mode: Mode::Local,
        ..Config::default()
    };
    c.github.git_base = upstream.url();
    c.github.api_base = format!("{}/api", upstream.url());
    c
}

/// A daemon in this process.
pub struct Proxy {
    pub running: Running,
    pub paths: Paths,
    pub state: tempfile::TempDir,
}

impl Proxy {
    pub async fn start(config: Config, options: Options) -> Self {
        capture_logs();
        let state = tempfile::tempdir().unwrap();
        let paths = Paths::under(state.path());
        let daemon = Daemon::bind(&paths, config, options).await.unwrap();
        Self {
            running: daemon.spawn(),
            paths,
            state,
        }
    }

    pub async fn scripted(upstream: &Upstream, auth: &Arc<Scripted>) -> Self {
        Self::start(
            config(upstream),
            Options {
                authorizer: Some(Arc::<Scripted>::clone(auth)),
                prompter: Arc::new(reins_desktop::auth::prompt::NoPrompter),
                harden: false,
            },
        )
        .await
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.running.addr
    }

    #[must_use]
    pub fn control_token(&self) -> String {
        std::fs::read_to_string(self.paths.control_token_file()).unwrap()
    }
}

/// Sends raw HTTP/1.1 to `addr` and returns the whole answer.
pub async fn raw_http(addr: SocketAddr, request: &str) -> String {
    use tokio::io::AsyncReadExt as _;
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}
