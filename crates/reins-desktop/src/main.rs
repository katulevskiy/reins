//! `reins`: the Reins desktop app's command line and daemon.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use reins_desktop::config::{Config, Paths};
use reins_desktop::control::{Client, ClientError};
use reins_desktop::daemon::{Daemon, Options, init_logging};
use reins_desktop::identity::Identity;
use reins_desktop::setup::{Git, Scope};
use reins_desktop::update::{self, Check};
use reins_desktop::{server, service};

/// `println!` that ends the program quietly when stdout is gone (`reins status | head -3`) instead of panicking.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if writeln!(std::io::stdout(), $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

#[derive(Parser)]
#[command(
    name = "reins",
    version = update::LONG_VERSION,
    about = "Reins: your phone approves what AI agents do on this computer (commands, git, MCP tools, secrets, SSH)",
    after_help = "New here? `reins setup` pairs your phone and connects your AI tools, `reins test` shows the whole \
                  loop, `reins doctor` says what to fix.\nMore: https://github.com/katulevskiy/reins/blob/main/docs/quick-start.md"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(display_order = 1)]
    /// Set everything up: pair with your phone, start the service, route git, connect your AI tools.
    ///
    /// Pairs only when this computer is not paired yet; connects every AI tool found (Claude Code, Codex, Gemini CLI,
    /// Cursor) that is not connected already.
    Setup {
        /// The server your phone signed in to; a self-hosted one needs its address here.
        #[arg(default_value = reins_proto::DEFAULT_SERVER)]
        server_url: String,
        /// Sign in in the browser instead of scanning a QR code.
        #[arg(long)]
        browser: bool,
    },
    #[command(display_order = 3)]
    /// Check that everything works, and say how to fix what does not.
    ///
    /// Exit code 1 when something needs fixing.
    Doctor,
    #[command(display_order = 2)]
    /// Send a test request to your phone, to see the whole loop work.
    Test,
    /// Run the daemon in the foreground (what the service runs).
    #[command(hide = true)]
    Daemon {
        /// Also append the log to this file (the Windows background service has no other place for it).
        #[arg(long, value_name = "FILE")]
        log_file: Option<PathBuf>,
    },
    #[command(display_order = 4)]
    /// Whether the daemon runs, who decides, the listen address, the server and this app's key fingerprint.
    Status,
    #[command(display_order = 21)]
    /// Route the enabled git hosts' remotes (`[[git.hosts]]`) through the proxy, or stop doing so.
    Git {
        #[command(subcommand)]
        action: GitCmd,
    },
    #[command(display_order = 23)]
    /// Local approvals waiting for an answer.
    Pending,
    #[command(display_order = 24)]
    /// Allow a pending local approval.
    Approve {
        id: String,
    },
    #[command(display_order = 25)]
    /// Refuse a pending local approval.
    Deny {
        id: String,
    },
    #[command(display_order = 5)]
    /// Pair with your phone: scan the QR code shown here with the Reins app. Your phone decides from then on.
    Login {
        /// The server your phone signed in to; a self-hosted one needs its address here.
        #[arg(default_value = reins_proto::DEFAULT_SERVER)]
        server_url: String,
        /// Sign in in the browser (enter your email there) instead of scanning a QR code.
        #[arg(long)]
        browser: bool,
        /// With --browser: print the sign-in link instead of opening the browser.
        #[arg(long)]
        no_browser: bool,
    },
    #[command(display_order = 26)]
    /// Sign this computer out: ends its connection on the server and forgets the session here.
    Logout,
    #[command(display_order = 22)]
    /// Run the daemon at login (systemd user unit, launchd agent, or on Windows the user's Run key).
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    #[command(display_order = 10)]
    /// Stop sending git through Reins: git talks to the git hosts directly again (undo with `reins resume`).
    #[command(alias = "disable")]
    Pause,
    #[command(display_order = 9)]
    /// Send git through Reins: starts the background service if needed, then routes the enabled hosts' remotes
    /// through it.
    #[command(alias = "enable")]
    Resume,
    #[command(display_order = 12)]
    /// Install the latest release (signed by the Reins release key) and restart the service.
    Update {
        /// Only say whether a newer release exists.
        #[arg(long)]
        check: bool,
    },
    /// Undo everything Reins did on this computer and end its connection to your phone.
    ///
    /// Takes Reins out of every AI tool, sends git directly again, removes the SSH agent block from ~/.ssh/config,
    /// stops and removes the background service, and signs this computer out (the phone's list no longer shows
    /// it). `--purge` also deletes this computer's key, activity log and settings. The `reins` program itself stays.
    #[command(display_order = 30)]
    Uninstall {
        /// Also delete this computer's key, the activity log and config.toml.
        #[arg(long)]
        purge: bool,
    },
    /// Approve a bundle of permissions on your phone once, for a while: the AI tools here read and push to your branch
    /// without asking each time.
    ///
    /// `reins allow 2h` pushes to the branch checked out here (never main) and reads what `--read` names (`mail`,
    /// `calendar`, `github`). Force pushes, deleting, the vault and purchases are still asked every time.
    #[command(display_order = 7)]
    Allow(SessionArgs),
    /// The running work session; `reins session start` / `reins session end`.
    #[command(display_order = 8)]
    Session {
        #[command(subcommand)]
        action: Option<SessionCmd>,
    },
    #[command(flatten)]
    Agents(reins_desktop::agents_cli::Command),
    #[command(display_order = 13)]
    /// Run a command with secrets from the vault on your phone as environment variables.
    Run(reins_desktop::run::RunArgs),
    #[command(display_order = 14)]
    /// Save a secret in the vault on your phone (typed here, never shown or kept), or list the items' names.
    Vault {
        #[command(subcommand)]
        action: reins_desktop::vault_cli::VaultCmd,
    },
    #[command(display_order = 20)]
    /// The SSH agent whose keys stay on your phone.
    Ssh {
        #[command(subcommand)]
        action: reins_desktop::ssh_agent::setup::SshCmd,
    },
}

#[derive(Subcommand)]
enum GitCmd {
    /// Add `url.<proxy>.insteadOf` rules for every enabled git host (global git config, or one repository).
    Setup {
        #[arg(long, value_name = "DIR")]
        repo: Option<PathBuf>,
    },
    /// Remove exactly the rules `setup` added.
    Unsetup {
        #[arg(long, value_name = "DIR")]
        repo: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionCmd {
    /// Start a work session (the same as `reins allow`).
    Start(SessionArgs),
    /// End the running work session now: its permissions end on your phone.
    End,
}

#[derive(clap::Args)]
struct SessionArgs {
    /// How long: 2h, 90m, 1h30m (15 minutes to 12 hours).
    #[arg(default_value = "2h")]
    duration: String,
    /// What it is for, shown on your phone.
    #[arg(long)]
    reason: Option<String>,
    /// Integrations to read, comma-separated: mail, calendar, github, ...
    #[arg(long, value_delimiter = ',')]
    read: Vec<String>,
    /// The repository whose checked-out branch may be pushed to (default: this directory).
    #[arg(long, value_name = "DIR")]
    repo: Option<PathBuf>,
    /// Push to this branch instead of the checked-out one.
    #[arg(long)]
    branch: Option<String>,
    /// Do not include pushing.
    #[arg(long)]
    no_push: bool,
}

#[derive(Subcommand)]
enum ServiceCmd {
    Install,
    Uninstall,
}

/// What `reins login` shows while the phone has to scan: the QR code, the code to type instead, the number to tap
/// and the key to compare.
fn show_pairing(pairing: &server::device::DevicePairing) {
    use std::io::IsTerminal as _;
    out!("Scan this QR code with the Reins app on your phone (or with the phone's camera):\n");
    match server::device::terminal_qr(&pairing.qr_url, std::io::stdout().is_terminal()) {
        Ok(qr) => out!("{qr}"),
        Err(e) => out!("({e}; open {} on your phone instead)\n", pairing.qr_url),
    }
    out!("No camera? In the Reins app: Settings, Connect a computer, and enter {}.", pairing.user_code);
    if let Some(number) = pairing.confirm_code {
        out!("Then tap {number} on your phone.");
    }
    out!("Your phone shows this computer's key {}. Approve only if it matches.\n", pairing.key_fingerprint);
    let minutes = (pairing.expires_at - reins_desktop::now_unix()).max(60) / 60;
    out!("Waiting for your phone (the code works for {minutes} minutes)...");
}

fn home() -> Result<PathBuf, String> {
    reins_desktop::config::home_dir()
}

/// Installs the background service for `exe` and starts it; what was installed, for the message.
// Only Windows waits (for the daemon's control API).
#[cfg_attr(not(windows), allow(clippy::unused_async))]
async fn install_service(paths: &Paths, config: &Config, exe: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        service::windows::install(paths, config, exe, true).await
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, config);
        let manager = service::Manager::current()?;
        Ok(service::install(manager, &home()?, exe, true)?.display().to_string())
    }
}

/// `reins status`'s service line (Windows only: elsewhere the service manager shows it).
fn service_status(paths: &Paths) -> Option<String> {
    #[cfg(windows)]
    {
        Some(match service::windows::installed() {
            Ok(value) => service::windows::describe(value.as_deref(), paths),
            Err(e) => format!("unknown ({e})"),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        None
    }
}

async fn status(paths: &Paths, config: &Config) -> Result<(), String> {
    match Client::new(paths, config.listen) {
        Ok(client) => match client.status().await {
            Ok(s) => {
                out!("Version:       {}", update::LONG_VERSION);
                out!("Daemon:        running on {}", s.listen);
                out!("Mode:          {}", s.mode);
                out!("Who decides:   {}", s.decides);
                out!("Server:        {}", s.server.as_deref().unwrap_or("not logged in"));
                out!("Key:           {}", s.fingerprint);
                out!("Pending:       {}", s.pending);
                out!("Git:           {}", git_mode(config));
                out!("AI tools:      {}", harnesses_line(paths, config));
                if let Some(line) = service_status(paths) {
                    out!("Service:       {line}");
                }
                return Ok(());
            }
            Err(ClientError::NotRunning) => {}
            Err(e) => return Err(e.to_string()),
        },
        Err(ClientError::NotRunning) => {}
        Err(e) => return Err(e.to_string()),
    }
    out!("Version:       {}", update::LONG_VERSION);
    out!("Daemon:        not running; `reins resume` starts it");
    out!("Mode:          {}", format!("{:?}", config.mode).to_ascii_lowercase());
    out!(
        "Server:        {}",
        server::oauth::logged_in_server(paths).as_deref().unwrap_or("not logged in; `reins login` pairs your phone")
    );
    if paths.identity_file().exists() {
        let id = Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
        out!("Key:           {}", id.fingerprint());
    }
    out!("Listen:        {}", config.listen);
    out!("Git:           {}", git_mode(config));
    out!("AI tools:      {}", harnesses_line(paths, config));
    if let Some(line) = service_status(paths) {
        out!("Service:       {line}");
    }
    Ok(())
}

fn git_mode(config: &Config) -> String {
    match Git::default().hosts_set_up(&Scope::Global, config) {
        Ok(hosts) if !hosts.is_empty() => {
            format!("{} through Reins (http://{}/); `reins pause` to go direct", hosts.join(", "), config.listen)
        }
        Ok(_) => "direct to the git hosts; `reins resume` to go through Reins".to_owned(),
        Err(e) => format!("unknown ({e})"),
    }
}

/// `Claude Code, Codex`: the harnesses Reins is added to, or how to add it.
fn harnesses_line(paths: &Paths, config: &Config) -> String {
    use reins_desktop::harness::{self, Harness};
    let setup = match reins_desktop::agents_cli::setup(config) {
        Ok(setup) => setup,
        Err(e) => return format!("unknown ({e})"),
    };
    let added: Vec<&str> = Harness::ALL
        .into_iter()
        .filter(|h| harness::registered(paths, &setup, *h).is_ok_and(|r| r.any()))
        .map(Harness::label)
        .collect();
    if added.is_empty() {
        "none yet; `reins harness add --all` adds Reins to every one on this computer".to_owned()
    } else {
        format!("{} (`reins harness list` for details)", added.join(", "))
    }
}

/// `github.com, gitlab.com`: the hosts git is sent through Reins for.
fn enabled_hosts(config: &Config) -> Result<String, String> {
    Ok(config.enabled_hosts()?.into_iter().map(|h| h.host).collect::<Vec<_>>().join(", "))
}

async fn answer(paths: &Paths, config: &Config, id: &str, approve: bool) -> Result<(), String> {
    let client = Client::new(paths, config.listen).map_err(|e| e.to_string())?;
    client.answer(id, approve).await.map_err(|e| e.to_string())?;
    out!(
        "{} {id}",
        if approve {
            "Approved"
        } else {
            "Denied"
        }
    );
    Ok(())
}

async fn run(cmd: Cmd) -> Result<(), String> {
    let paths = Paths::from_env().map_err(|e| e.to_string())?;
    let config = Config::load(&paths).map_err(|e| e.to_string())?;
    match cmd {
        Cmd::Daemon {
            log_file,
        } => {
            init_logging(log_file.as_deref());
            #[cfg(windows)]
            update::remove_set_aside(&std::env::current_exe().unwrap_or_default());
            let daemon = Daemon::bind(&paths, config, Options::default()).await?;
            log::info!("listening on http://{} ({} decides)", daemon.local_addr(), daemon.describe());
            daemon.run().await
        }
        Cmd::Status => status(&paths, &config).await,
        Cmd::Git {
            action,
        } => {
            let git = Git::default();
            match action {
                GitCmd::Setup {
                    repo,
                } => {
                    let scope = repo.map_or(Scope::Global, Scope::Repo);
                    let (added, removed) = git.setup_hosts(&scope, &config)?;
                    let bases: Vec<String> =
                        config.enabled_hosts()?.iter().map(|h| config.proxy_base_for(&h.host)).collect();
                    if added.is_empty() {
                        out!("git already uses the proxy at {}", bases.join(", "));
                    } else {
                        out!("git now sends {} through {}", added.join(", "), bases.join(", "));
                    }
                    if removed > 0 {
                        out!("removed {removed} rule(s) of disabled hosts");
                    }
                    Ok(())
                }
                GitCmd::Unsetup {
                    repo,
                } => {
                    let scope = repo.map_or(Scope::Global, Scope::Repo);
                    let removed = git.unsetup_hosts(&scope, &config)?;
                    out!("removed {removed} rule(s)");
                    Ok(())
                }
            }
        }
        Cmd::Pending => {
            let items = Client::new(&paths, config.listen)
                .map_err(|e| e.to_string())?
                .pending()
                .await
                .map_err(|e| e.to_string())?;
            if items.is_empty() {
                out!("Nothing is waiting.");
            }
            for item in items {
                out!("{}  {}", item.id, item.what);
                for line in item.lines {
                    out!("    {line}");
                }
            }
            Ok(())
        }
        Cmd::Approve {
            id,
        } => answer(&paths, &config, &id, true).await,
        Cmd::Deny {
            id,
        } => answer(&paths, &config, &id, false).await,
        Cmd::Login {
            server_url,
            browser,
            no_browser,
        } => {
            let server = login(&paths, &server_url, browser, no_browser).await?;
            out!("Logged in to {server}. Your phone decides from now on.");
            Ok(())
        }
        Cmd::Setup {
            server_url,
            browser,
        } => setup(&paths, &config, &server_url, browser).await,
        Cmd::Doctor => {
            let ok = doctor(&paths, &config).await?;
            if !ok {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Test => test(&paths, &config).await,
        Cmd::Logout => {
            match server::oauth::sign_out(&paths).await? {
                server::oauth::SignedOut::NotLoggedIn => out!("Not logged in."),
                server::oauth::SignedOut::Revoked => {
                    out!("Logged out. Your phone no longer lists this computer.");
                }
                server::oauth::SignedOut::LocalOnly(why) => out!(
                    "Logged out here; the server was not told ({why}). Remove this computer in the Reins app on your \
                     phone (Settings, AI connections)."
                ),
            }
            Ok(())
        }
        Cmd::Uninstall {
            purge,
        } => uninstall(&paths, &config, purge).await,
        Cmd::Allow(args)
        | Cmd::Session {
            action: Some(SessionCmd::Start(args)),
        } => session_start(&paths, &config, &args).await,
        Cmd::Session {
            action: Some(SessionCmd::End),
        } => {
            let ended = reins_desktop::work_session::end(&paths, &config).await?;
            if ended == 0 {
                out!("No work session runs here.");
            } else {
                out!("Work session ended: {ended} permission(s) ended on your phone.");
            }
            Ok(())
        }
        Cmd::Session {
            action: None,
        } => {
            if let Some(s) = reins_desktop::work_session::current(&paths) {
                let left = u64::try_from(s.left(reins_desktop::now_unix())).unwrap_or(0);
                out!("Work session: {} ({} left)", s.reason, reins_desktop::work_session::span(left));
                for a in &s.allows {
                    out!("  allows: {a}");
                }
                out!("`reins session end` ends it now.");
            } else {
                out!("No work session runs here. `reins allow 2h` starts one.");
            }
            Ok(())
        }
        Cmd::Service {
            action,
        } => {
            match action {
                ServiceCmd::Install => {
                    #[cfg(windows)]
                    let exe = update::current_executable()?;
                    #[cfg(not(windows))]
                    let exe = std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?;
                    let installed = install_service(&paths, &config, &exe).await?;
                    out!("Installed and started {installed}");
                }
                ServiceCmd::Uninstall => {
                    #[cfg(windows)]
                    let removed = service::windows::uninstall(&paths, &config, true).await?;
                    #[cfg(not(windows))]
                    let removed = service::uninstall(service::Manager::current()?, &home()?, true)?;
                    if removed {
                        out!("Stopped and removed the service.");
                    } else {
                        out!("The service was not installed.");
                    }
                }
            }
            Ok(())
        }
        Cmd::Update {
            check,
        } => self_update(&paths, &config, check).await,
        Cmd::Pause => pause(&config),
        Cmd::Resume => resume(&paths, &config).await,
        Cmd::Agents(_) => unreachable!("handled in main"),
        Cmd::Run(args) => {
            let code = reins_desktop::run::main(&paths, &config, &args).await;
            std::process::exit(i32::from(code));
        }
        Cmd::Vault {
            action,
        } => {
            for line in reins_desktop::vault_cli::run(&action, &paths, &config).await? {
                out!("{line}");
            }
            Ok(())
        }
        Cmd::Ssh {
            action,
        } => {
            for line in reins_desktop::ssh_agent::setup::run(&action, &paths, &config, &home()?)? {
                out!("{line}");
            }
            Ok(())
        }
    }
}

/// Pairs with the phone: the QR code (device flow), or the browser sign-in. Returns the server.
async fn login(paths: &Paths, server_url: &str, browser: bool, no_browser: bool) -> Result<String, String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let identity = Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
    if browser || no_browser {
        return server::oauth::login(paths, &identity, server_url, !no_browser).await;
    }
    match server::device::DevicePairing::start(&identity, server_url).await {
        Ok(mut pairing) => {
            show_pairing(&pairing);
            pairing.wait(paths).await
        }
        Err(server::device::StartError::Unsupported) => {
            out!("This server cannot pair by QR code; signing in through the browser instead.\n");
            server::oauth::login(paths, &identity, server_url, true).await
        }
        Err(server::device::StartError::Failed(e)) => Err(e),
    }
}

/// `reins setup`: pair (if needed), start the service and route git, connect every AI tool found; then a summary.
async fn setup(paths: &Paths, config: &Config, server_url: &str, browser: bool) -> Result<(), String> {
    let mut summary: Vec<String> = Vec::new();
    match server::oauth::logged_in_server(paths) {
        Some(server) => summary.push(format!("✓ Paired with your phone through {server}")),
        None if config.mode == reins_desktop::config::Mode::Local => {
            summary.push("– Local mode: this computer decides (no phone)".to_owned());
        }
        None => {
            out!("Step 1 of 3: pair this computer with your phone.\n");
            let server = login(paths, server_url, browser, false).await?;
            summary.push(format!("✓ Paired with your phone through {server}"));
        }
    }
    out!("\nStep 2 of 3: the background service and git.");
    if !daemon_running(paths, config).await {
        let exe = update::current_executable()?;
        install_service(paths, config, &exe).await?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !daemon_running(paths, config).await {
            if std::time::Instant::now() > deadline {
                return Err("the background service did not start; `reins daemon` shows why".to_owned());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    summary.push(format!("✓ Background service running on {}", config.listen));
    let (routed, direct) = reins_desktop::setup::routable(paths, config).await;
    Git::default().setup_hosts(&Scope::Global, &routed)?;
    let hosts = enabled_hosts(&routed)?;
    if !hosts.is_empty() {
        summary.push(format!("✓ git for {hosts} goes through Reins"));
    }
    let mut next: Vec<String> = direct.iter().map(reins_desktop::setup::unserved_note).collect();
    out!("\nStep 3 of 3: your AI tools.");
    let s = reins_desktop::harness::Setup {
        home: home()?,
        exe: update::current_executable()?,
        hook_timeout_secs: config.guard.timeout_secs,
    };
    let found = reins_desktop::harness::detect::all_found(&s.home);
    if found.is_empty() {
        summary.push(
            "– No AI tool found (Claude Code, Codex, Gemini CLI, Cursor); `reins harness add <name>` later".to_owned(),
        );
    }
    for h in found {
        let before = reins_desktop::harness::registered(paths, &s, h).is_ok_and(|r| r.complete());
        if before {
            summary.push(format!("✓ {} was already connected", h.label()));
            continue;
        }
        match reins_desktop::harness::add(paths, &s, h) {
            Ok(_) => {
                summary.push(format!("✓ {} connected", h.label()));
                next.push(format!("{}: {}", h.label(), reins_desktop::harness::after_add_note(h)));
            }
            Err(e) => summary.push(format!("✗ {}: {e}", h.label())),
        }
    }
    out!("\nDone:");
    for line in &summary {
        out!("  {line}");
    }
    if !next.is_empty() {
        out!("\nStill to do:");
        for line in &next {
            out!("  → {line}");
        }
    }
    out!("\nTry it: `reins test` sends a test to your phone. `reins doctor` checks everything any time.");
    Ok(())
}

/// `reins allow` / `reins session start`.
async fn session_start(paths: &Paths, config: &Config, args: &SessionArgs) -> Result<(), String> {
    use reins_desktop::work_session::{self, Branch, Request};
    let secs = work_session::parse_duration(&args.duration)?;
    let read: Vec<String> =
        args.read.iter().filter(|r| !r.trim().is_empty()).map(|r| work_session::service_id(r)).collect();
    let mut push: Vec<Branch> = Vec::new();
    if !args.no_push {
        let dir = match &args.repo {
            Some(d) => d.clone(),
            None => std::env::current_dir().map_err(|e| e.to_string())?,
        };
        match work_session::current_branch(&dir, config) {
            Some(mut b) => {
                if let Some(other) = &args.branch {
                    b.branch.clone_from(other);
                }
                if b.is_default() && args.branch.is_none() {
                    out!(
                        "Not including pushes: {} is on {}; pushes there still ask. `--branch NAME` names one.",
                        b.repo,
                        b.branch
                    );
                } else {
                    push.push(b);
                }
            }
            None if args.repo.is_some() || args.branch.is_some() => {
                return Err(format!("{} is not a git repository with an origin on a known git host", dir.display()));
            }
            None => {}
        }
    }
    if read.is_empty() && push.is_empty() {
        return Err("nothing to allow: run it in a repository on a feature branch, or name integrations with --read mail,calendar".to_owned());
    }
    let reason = args.reason.clone().unwrap_or_else(|| match push.first() {
        Some(b) => format!("Work on {} ({})", b.repo, b.branch),
        None => "Work session".to_owned(),
    });
    let request = Request {
        secs,
        reason,
        read,
        push,
    };
    out!("Asking your phone for a {} work session (requested from this terminal)…", work_session::span(secs));
    let tell = reins_desktop::phone::teller(|line: &str| eprintln!("{line}"));
    let s = work_session::start(paths, config, &request, Some(tell)).await?;
    out!("Work session started: {} (until {} on this computer's clock).", s.reason, clock(s.expires_at));
    for a in &s.allows {
        out!("  allows: {a}");
    }
    if !s.skipped.is_empty() {
        out!("  not included (not connected on your phone): {}", s.skipped.join(", "));
    }
    out!("Still asked every time: force pushes, deleting, the vault, purchases. `reins session end` ends it early.");
    Ok(())
}

/// `HH:MM` local time of `unix` (UTC when the local offset is unknown).
fn clock(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    let utc = format!("{:02}:{:02} UTC", secs / 3_600, (secs % 3_600) / 60);
    std::process::Command::new("date")
        .args(["-d", &format!("@{unix}"), "+%H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or(utc)
}

/// `reins uninstall`: every undo step, each reported; failures do not stop the others.
async fn uninstall(paths: &Paths, config: &Config, purge: bool) -> Result<(), String> {
    let mut failed = false;
    let mut step = |what: &str, r: Result<String, String>| match r {
        Ok(done) => out!("✓ {what}: {done}"),
        Err(e) => {
            failed = true;
            out!("✗ {what}: {e}");
        }
    };
    let home = home()?;
    let s = reins_desktop::harness::Setup {
        home: home.clone(),
        exe: update::current_executable()?,
        hook_timeout_secs: config.guard.timeout_secs,
    };
    for h in reins_desktop::harness::Harness::ALL {
        if !reins_desktop::harness::registered(paths, &s, h).is_ok_and(|r| r.any()) {
            continue;
        }
        step(
            &format!("Take Reins out of {}", h.label()),
            reins_desktop::harness::remove(paths, &s, h).map(|lines| format!("{} change(s)", lines.len())),
        );
    }
    step(
        "Send git directly again",
        Git::default().unsetup_hosts(&Scope::Global, config).map(|n| format!("removed {n} rule(s)")),
    );
    step(
        "SSH agent",
        reins_desktop::ssh_agent::setup::unsetup(&reins_desktop::ssh_agent::setup::config_file(&home)).map(|removed| {
            if removed {
                "removed its block from ~/.ssh/config".to_owned()
            } else {
                "nothing to remove".to_owned()
            }
        }),
    );
    #[cfg(windows)]
    let service = service::windows::uninstall(paths, config, true).await;
    #[cfg(not(windows))]
    let service = service::Manager::current().and_then(|m| service::uninstall(m, &home, true));
    step(
        "Background service",
        service.map(|removed| {
            if removed {
                "stopped and removed"
            } else {
                "was not installed"
            }
            .to_owned()
        }),
    );
    step(
        "Sign out",
        server::oauth::sign_out(paths).await.map(|r| match r {
            server::oauth::SignedOut::NotLoggedIn => "was not paired".to_owned(),
            server::oauth::SignedOut::Revoked => "your phone no longer lists this computer".to_owned(),
            server::oauth::SignedOut::LocalOnly(why) => {
                format!("signed out here; remove this computer in the Reins app on your phone ({why})")
            }
        }),
    );
    if purge {
        for dir in [&paths.state_dir, &paths.config_dir] {
            let r = match std::fs::remove_dir_all(dir) {
                Ok(()) => Ok("deleted".to_owned()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("nothing there".to_owned()),
                Err(e) => Err(e.to_string()),
            };
            step(&dir.display().to_string(), r);
        }
    }
    let exe = update::current_executable().map_or_else(|_| "reins".to_owned(), |e| e.display().to_string());
    out!("\nThe program itself is still at {exe}; delete it to finish (the Reins app: uninstall it like any app).");
    if failed {
        return Err("some steps failed (✗ above)".to_owned());
    }
    Ok(())
}

/// `reins doctor`: every check with its fix. `Ok(false)` when something fails.
async fn doctor(paths: &Paths, config: &Config) -> Result<bool, String> {
    let d = reins_desktop::doctor::Doctor {
        paths: paths.clone(),
        config: config.clone(),
        home: home()?,
        exe: update::current_executable()?,
        git: Git::default(),
    };
    let checks = d.run().await;
    let width = checks.iter().map(|c| c.label.chars().count()).max().unwrap_or(0);
    for c in &checks {
        out!("{} {:width$}  {}", c.level.mark(), c.label, c.detail);
        if let Some(fix) = &c.fix {
            out!("  {:width$}  → {fix}", "");
        }
    }
    let worst = reins_desktop::doctor::overall(&checks);
    out!(
        "\n{}",
        match worst {
            reins_desktop::doctor::Level::Fail => "Something needs fixing (✗ above).",
            reins_desktop::doctor::Level::Warn => "Working; a few things are worth a look (! above).",
            _ => "Everything works.",
        }
    );
    Ok(worst != reins_desktop::doctor::Level::Fail)
}

/// `reins test`: a test question to the phone; prints how it ended.
async fn test(paths: &Paths, config: &Config) -> Result<(), String> {
    out!("Sent a test to your phone. Open Reins there and approve or deny it…");
    let (answer, timed_out) = reins_desktop::ask::send_test(paths, config, &reins_desktop::ask::DesktopAsk).await;
    match answer {
        reins_desktop::ask::Answer::Yes => {
            out!("✓ Approved on your phone. Reins works end to end.");
            Ok(())
        }
        reins_desktop::ask::Answer::No(_) => {
            out!("✓ Denied on your phone. Reins works end to end (a denial stops the agent the same way).");
            Ok(())
        }
        reins_desktop::ask::Answer::Unanswered(why) if timed_out => Err(format!(
            "no answer within {} s ({why}); `reins doctor` checks the phone and the server",
            reins_desktop::ask::TEST_TIMEOUT.as_secs()
        )),
        reins_desktop::ask::Answer::Unanswered(why) => Err(format!("{why}; `reins doctor` says what to fix")),
    }
}

async fn daemon_running(paths: &Paths, config: &Config) -> bool {
    match Client::new(paths, config.listen) {
        Ok(client) => client.status().await.is_ok(),
        Err(_) => false,
    }
}

fn pause(config: &Config) -> Result<(), String> {
    let removed = Git::default().unsetup_hosts(&Scope::Global, config)?;
    if removed == 0 {
        out!("git already talks to the git hosts directly.");
    } else {
        out!("Paused: git talks to the git hosts directly again. `reins resume` switches back.");
    }
    Ok(())
}

async fn resume(paths: &Paths, config: &Config) -> Result<(), String> {
    if !daemon_running(paths, config).await {
        let exe = update::current_executable()?;
        let installed = install_service(paths, config, &exe).await?;
        out!("Started the background service ({installed}).");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !daemon_running(paths, config).await {
            if std::time::Instant::now() > deadline {
                return Err("the background service did not start; see `reins daemon` for why. git was left as it was"
                    .to_owned());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let (routed, direct) = reins_desktop::setup::routable(paths, config).await;
    Git::default().setup_hosts(&Scope::Global, &routed)?;
    let hosts = enabled_hosts(&routed)?;
    if !hosts.is_empty() {
        out!("git sends {hosts} through Reins (http://{}/).", config.listen);
    }
    for h in &direct {
        out!("{}", reins_desktop::setup::unserved_note(h));
    }
    if server::oauth::logged_in_server(paths).is_none() {
        out!("Not logged in: the local policy decides. `reins login` to decide on your phone.");
    }
    out!("`reins pause` switches back to talking to the git hosts directly.");
    Ok(())
}

async fn self_update(paths: &Paths, config: &Config, check_only: bool) -> Result<(), String> {
    #[cfg(not(windows))]
    let _ = paths;
    let updater = update::Updater::for_this_binary(&config.releases)?;
    match updater.check().await? {
        Check::UpToDate(latest) => {
            out!("reins {} is up to date (latest release: {}).", update::LONG_VERSION, latest.build);
        }
        Check::NoBuild(latest) => {
            out!("Release {} has no build for this computer yet.", latest.build);
        }
        Check::Available(latest, _) if check_only => {
            out!("Update available: {} → {}. Run `reins update`.", update::BUILD, latest.build);
        }
        Check::Available(latest, asset) => {
            let exe = update::current_executable()?;
            if update::installed_by_package_manager(&exe) {
                out!(
                    "Update available: {} → {}. This reins came with a system package: update it with the package \
                     manager (apt, dnf, or your AUR helper).",
                    update::BUILD,
                    latest.build
                );
                return Ok(());
            }
            if update::installed_with_app(&exe) {
                out!(
                    "Update available: {} → {}. This reins came with the Reins app: update the app ({}).",
                    update::BUILD,
                    latest.build,
                    update::APP_DOWNLOADS
                );
                return Ok(());
            }
            let bytes = updater.download(&asset).await?;
            update::replace_executable(&exe, &bytes)?;
            out!("Updated {} to {} ({}).", exe.display(), latest.version, latest.build);
            #[cfg(windows)]
            if let Some(done) = service::windows::restart_if_installed(paths, config, &exe).await? {
                out!("{done}");
            }
            #[cfg(not(windows))]
            if let Ok(manager) = service::Manager::current()
                && let Some(done) = service::restart_if_installed(manager, &home()?)?
            {
                out!("{done}");
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Cmd::Agents(cmd) = cli.command {
        return reins_desktop::agents_cli::run(cmd).await;
    }
    match run(cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reins: {e}");
            ExitCode::FAILURE
        }
    }
}
