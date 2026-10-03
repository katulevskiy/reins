//! The running daemon: config, identity, the chosen authorizer, the proxy and control API on one loopback listener.
//!
//! Browsers are kept out: the `Host` header must name the loopback listener (DNS rebinding), and `OPTIONS` or any
//! request with an `Origin` header is refused. Git sends neither.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::rt::{TokioIo, TokioTimer};
use reins_proto::desktop::PushSummary;
use tokio::net::TcpListener;

use crate::auth::local::LocalAuthorizer;
use crate::auth::prompt::{DesktopPrompter, Pending, Prompter};
use crate::auth::reins::ReinsAuthorizer;
use crate::auth::{Authorizer, Credential, Refusal, Repo};
use crate::config::{Config, Mode, Paths};
use crate::control::{self, Control, Status};
use crate::identity::Identity;
use crate::proxy::{Body, Proxy, text};

/// What the embedding code may replace (tests): the authorizer, the desktop prompt, the hardening.
pub struct Options {
    pub authorizer: Option<Arc<dyn Authorizer>>,
    pub prompter: Arc<dyn Prompter>,
    pub harden: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            authorizer: None,
            prompter: Arc::new(DesktopPrompter),
            harden: true,
        }
    }
}

/// Picks who decides on every request, so logging in or out takes effect without a restart.
struct ModeAuthorizer {
    mode: Mode,
    paths: Paths,
    identity: Arc<Identity>,
    config: Config,
    local: Arc<LocalAuthorizer>,
    phone: Mutex<Option<(String, Arc<ReinsAuthorizer>)>>,
}

impl ModeAuthorizer {
    fn pick(&self) -> Result<Arc<dyn Authorizer>, Refusal> {
        if self.mode == Mode::Local {
            return Ok(Arc::<LocalAuthorizer>::clone(&self.local));
        }
        let Some(server) = crate::server::oauth::logged_in_server(&self.paths) else {
            return match self.mode {
                Mode::Auto => Ok(Arc::<LocalAuthorizer>::clone(&self.local)),
                _ => Err(Refusal::Unavailable("Not logged in to a Reins server: run `reins login`.".to_owned())),
            };
        };
        let mut phone = self.phone.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((s, a)) = phone.as_ref()
            && *s == server
        {
            return Ok(Arc::<ReinsAuthorizer>::clone(a));
        }
        let a = Arc::new(
            ReinsAuthorizer::new(&self.paths, Arc::clone(&self.identity), &self.config)
                .map_err(Refusal::Unavailable)?,
        );
        *phone = Some((server, Arc::clone(&a)));
        Ok(a)
    }
}

#[async_trait::async_trait]
impl Authorizer for ModeAuthorizer {
    async fn read(&self, repo: &Repo) -> Result<Credential, Refusal> {
        self.pick()?.read(repo).await
    }

    async fn push(&self, repo: &Repo, summary: &PushSummary, digest: &str) -> Result<Credential, Refusal> {
        self.pick()?.push(repo, summary, digest).await
    }

    fn describe(&self) -> String {
        match self.pick() {
            Ok(a) => a.describe(),
            Err(r) => format!("nobody: {}", r.message()),
        }
    }

    fn waiting_hint(&self) -> String {
        self.pick().map_or_else(|_| "waiting for approval".to_owned(), |a| a.waiting_hint())
    }
}

struct State {
    port: u16,
    proxy: Proxy,
    control: Control,
    api: crate::api_proxy::ApiProxy,
}

impl State {
    fn host_ok(&self, req: &Request<hyper::body::Incoming>) -> bool {
        let Some(host) = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) else {
            return false;
        };
        let port = self.port;
        [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
            .iter()
            .any(|ok| ok.eq_ignore_ascii_case(host))
    }

    async fn handle(&self, req: Request<hyper::body::Incoming>) -> Response<Body> {
        if !self.host_ok(&req) {
            log::warn!("refused a request with a foreign Host header");
            return text(StatusCode::FORBIDDEN, "Refused: this proxy only answers requests to its loopback address.");
        }
        if req.method() == Method::OPTIONS || req.headers().contains_key(header::ORIGIN) {
            log::warn!("refused a browser request");
            return text(StatusCode::FORBIDDEN, "Refused: browsers may not use this proxy.");
        }
        let path = req.uri().path().to_owned();
        if path.starts_with(control::PREFIX) {
            return self.control.handle(req.method(), &path, req.headers());
        }
        if path.starts_with(crate::api_proxy::PREFIX) {
            return self.api.handle(req).await;
        }
        match self.proxy.route(req.method(), &path, req.uri().query()) {
            Some(r) => self.proxy.handle(r, req).await,
            None => text(StatusCode::NOT_FOUND, "Not a git path this proxy serves."),
        }
    }
}

/// A bound daemon, not yet serving.
pub struct Daemon {
    listener: TcpListener,
    state: Arc<State>,
    token_file: std::path::PathBuf,
    ssh: Option<crate::ssh_agent::SshAgent>,
}

/// A daemon serving in the background (tests).
pub struct Running {
    pub addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Daemon {
    /// Loads what it needs, picks the authorizer, writes the control token, hardens the process and binds
    /// `config.listen`.
    pub async fn bind(paths: &Paths, config: Config, options: Options) -> Result<Self, String> {
        config.validate()?;
        paths.ensure().map_err(|e| e.to_string())?;
        let identity = Arc::new(Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?);
        let pending = Arc::new(Pending::default());
        let authorizer: Arc<dyn Authorizer> = match options.authorizer {
            Some(a) => a,
            None => Arc::new(ModeAuthorizer {
                mode: config.mode,
                paths: paths.clone(),
                identity: Arc::clone(&identity),
                config: config.clone(),
                local: Arc::new(LocalAuthorizer::new(&config, Arc::clone(&pending), options.prompter)?),
                phone: Mutex::new(None),
            }),
        };
        if options.harden {
            crate::harden::harden()?;
        }
        let listener =
            TcpListener::bind(config.listen).await.map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let token = control::new_token();
        let token_file = paths.control_token_file();
        crate::config::write_private(&token_file, token.as_bytes())
            .map_err(|e| format!("{}: {e}", token_file.display()))?;
        let proxy = Proxy::new(&config, Arc::clone(&authorizer))?;
        let phone = Arc::new(crate::phone::PhoneLink::new(
            config.mode,
            paths.clone(),
            Arc::clone(&identity),
            Duration::from_secs(config.approval_timeout_secs),
        ));
        let api = crate::api_proxy::ApiProxy::new(&config.api, Arc::clone(&phone))?;
        let ssh = crate::ssh_agent::start(paths, &config.ssh, phone);
        let status = {
            let pending = Arc::clone(&pending);
            let paths = paths.clone();
            let mode = config.mode;
            let mut listen_config = config.clone();
            listen_config.listen = addr;
            let fingerprint = identity.fingerprint();
            Box::new(move || Status {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                mode: format!("{mode:?}").to_ascii_lowercase(),
                decides: authorizer.describe(),
                listen: addr.to_string(),
                proxy_base: listen_config.proxy_base(),
                server: crate::server::oauth::logged_in_server(&paths),
                fingerprint: fingerprint.clone(),
                pending: pending.list().len(),
            })
        };
        Ok(Self {
            listener,
            state: Arc::new(State {
                port: addr.port(),
                proxy,
                control: Control::new(token, pending, status),
                api,
            }),
            token_file,
            ssh,
        })
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], self.state.port)))
    }

    /// Where the SSH agent listens, when it runs.
    #[must_use]
    pub fn ssh_socket(&self) -> Option<&std::path::Path> {
        self.ssh.as_ref().map(crate::ssh_agent::SshAgent::path)
    }

    /// Who decides (for the start-up line).
    #[must_use]
    pub fn describe(&self) -> String {
        self.state.proxy.authorizer().describe()
    }

    async fn accept_loop(listener: TcpListener, state: Arc<State>) {
        loop {
            let (stream, client) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("accept: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let state = Arc::clone(&state);
            let peer = stream.local_addr().ok().map(|server| crate::notice::Peer {
                client,
                server,
            });
            tokio::spawn(async move {
                let service = service_fn(move |mut req: Request<hyper::body::Incoming>| {
                    let state = Arc::clone(&state);
                    if let Some(peer) = peer {
                        req.extensions_mut().insert(peer);
                    }
                    async move { Ok::<_, Infallible>(state.handle(req).await) }
                });
                let conn = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(30))
                    .serve_connection(TokioIo::new(stream), service);
                if let Err(e) = conn.await {
                    log::debug!("connection: {e}");
                }
            });
        }
    }

    /// Serves until Ctrl-C, SIGTERM (on Windows also the console closing, logoff and shutdown) or `POST shutdown` on the
    /// control API, then removes the control token.
    pub async fn run(self) -> Result<(), String> {
        let token_file = self.token_file.clone();
        let stop = self.state.control.stop_handle();
        let serve = Self::accept_loop(self.listener, self.state);
        tokio::select! {
            () = serve => {}
            () = crate::ssh_agent::serve(self.ssh) => {}
            () = shutdown_signal() => log::info!("stopping"),
            () = stop.notified() => {
                log::info!("stopping");
                // Lets the answer to the shutdown request reach the command line.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        if let Err(e) = std::fs::remove_file(&token_file) {
            log::warn!("{}: {e}", token_file.display());
        }
        Ok(())
    }

    /// Serves in the background until the returned handle is dropped.
    #[must_use]
    pub fn spawn(self) -> Running {
        let addr = self.local_addr();
        Running {
            addr,
            task: tokio::spawn(async move {
                tokio::join!(Self::accept_loop(self.listener, self.state), crate::ssh_agent::serve(self.ssh));
            }),
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) else {
            tokio::signal::ctrl_c().await.ok();
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    // Ctrl-C and Ctrl-Break in its console; the console window closing, the user logging off and Windows shutting down.
    // The background service has no console: then only the control API stops it (a handler that cannot be set up
    // waits forever instead of stopping the daemon at once).
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        macro_rules! event {
            ($handler:expr) => {
                async {
                    match $handler {
                        Ok(mut s) => {
                            s.recv().await;
                        }
                        Err(_) => std::future::pending::<()>().await,
                    }
                }
            };
        }
        tokio::select! {
            () = event!(windows::ctrl_c()) => {}
            () = event!(windows::ctrl_break()) => {}
            () = event!(windows::ctrl_close()) => {}
            () = event!(windows::ctrl_logoff()) => {}
            () = event!(windows::ctrl_shutdown()) => {}
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}

/// Above this size the log file is moved to `<file>.old` when the daemon starts.
const MAX_LOG: u64 = 4 << 20;

/// Logs to stderr (the service manager keeps it), and with `file` (the Windows background service, which has no
/// stderr) appended to that file as well. `REINS_LOG`: `error`, `warn`, `info` (default), `debug`.
pub fn init_logging(file: Option<&std::path::Path>) {
    struct Logger {
        level: log::LevelFilter,
        file: Option<Mutex<std::fs::File>>,
    }
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            // This crate's modules, and the desktop app's (`reins_app`).
            m.level() <= self.level && m.target().starts_with("reins")
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!("{} {}", r.level(), r.args());
                if let Some(f) = &self.file {
                    let mut f = f.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let line = format!("{} {} {}\n", crate::now_unix(), r.level(), r.args());
                    std::io::Write::write_all(&mut *f, line.as_bytes()).ok();
                }
            }
        }
        fn flush(&self) {}
    }
    let level = std::env::var("REINS_LOG").ok().and_then(|l| l.parse().ok()).unwrap_or(log::LevelFilter::Info);
    let file = file.and_then(|path| {
        if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG) {
            let mut old = path.as_os_str().to_owned();
            old.push(".old");
            std::fs::rename(path, old).ok();
        }
        match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => Some(Mutex::new(f)),
            Err(e) => {
                eprintln!("reins: {}: {e}", path.display());
                None
            }
        }
    });
    if log::set_logger(Box::leak(Box::new(Logger {
        level,
        file,
    })))
    .is_ok()
    {
        log::set_max_level(level);
    }
}
