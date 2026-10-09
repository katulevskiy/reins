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
#[command(name = "reins", version = update::LONG_VERSION, about = "Reins desktop app: git for AI agents, allowed from your phone")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon in the foreground (what the service runs).
    Daemon {
        /// Also append the log to this file (the Windows background service has no other place for it).
        #[arg(long, value_name = "FILE")]
        log_file: Option<PathBuf>,
    },
    /// Whether the daemon runs, who decides, the listen address, the server and this app's key fingerprint.
    Status,
    /// Route the enabled git hosts' remotes (`[[git.hosts]]`) through the proxy, or stop doing so.
    Git {
        #[command(subcommand)]
        action: GitCmd,
    },
    /// Local approvals waiting for an answer.
    Pending,
    /// Allow a pending local approval.
    Approve {
        id: String,
    },
    /// Refuse a pending local approval.
    Deny {
        id: String,
    },
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
    /// Forget the Reins server session.
    Logout,
    /// Run the daemon at login (systemd user unit, launchd agent, or on Windows the user's Run key).
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Stop sending git through Reins: git talks to the git hosts directly again (undo with `reins resume`).
    #[command(alias = "disable")]
    Pause,
    /// Send git through Reins: starts the background service if needed, then routes the enabled hosts' remotes
    /// through it.
    #[command(alias = "enable")]
    Resume,
    /// Install the latest release (signed by the Reins release key) and restart the service.
    Update {
        /// Only say whether a newer release exists.
        #[arg(long)]
        check: bool,
    },
    #[command(flatten)]
    Agents(reins_desktop::agents_cli::Command),
    /// Run a command with secrets from the vault on your phone as environment variables.
    Run(reins_desktop::run::RunArgs),
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
            paths.ensure().map_err(|e| e.to_string())?;
            let identity = Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
            let server = if browser || no_browser {
                server::oauth::login(&paths, &identity, &server_url, !no_browser).await?
            } else {
                match server::device::DevicePairing::start(&identity, &server_url).await {
                    Ok(mut pairing) => {
                        show_pairing(&pairing);
                        pairing.wait(&paths).await?
                    }
                    Err(server::device::StartError::Unsupported) => {
                        out!("This server cannot pair by QR code; signing in through the browser instead.\n");
                        server::oauth::login(&paths, &identity, &server_url, true).await?
                    }
                    Err(server::device::StartError::Failed(e)) => return Err(e),
                }
            };
            out!("Logged in to {server}. Your phone decides from now on.");
            Ok(())
        }
        Cmd::Logout => {
            if server::oauth::logout(&paths)? {
                out!("Logged out.");
            } else {
                out!("Not logged in.");
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
    Git::default().setup_hosts(&Scope::Global, config)?;
    out!("git sends {} through Reins (http://{}/).", enabled_hosts(config)?, config.listen);
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
