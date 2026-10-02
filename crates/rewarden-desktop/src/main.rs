//! `rewarden`: the Rewarden desktop app's command line and daemon.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rewarden_desktop::config::{Config, Paths};
use rewarden_desktop::control::{Client, ClientError};
use rewarden_desktop::daemon::{Daemon, Options, init_logging};
use rewarden_desktop::identity::Identity;
use rewarden_desktop::setup::{Git, Scope};
use rewarden_desktop::update::{self, Check};
use rewarden_desktop::{server, service};

/// `println!` that ends the program quietly when stdout is gone (`rewarden status | head -3`) instead of panicking.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if writeln!(std::io::stdout(), $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

#[derive(Parser)]
#[command(name = "rewarden", version = update::LONG_VERSION, about = "Rewarden desktop app: git for AI agents, allowed from your phone")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon in the foreground (what the service runs).
    Daemon,
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
        #[arg(default_value = rewarden_proto::DEFAULT_SERVER)]
        server_url: String,
        /// Sign in in the browser (enter your email there) instead of scanning a QR code.
        #[arg(long)]
        browser: bool,
        /// With --browser: print the sign-in link instead of opening the browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget the Rewarden server session.
    Logout,
    /// Run the daemon at login (systemd user unit or launchd agent).
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Stop sending git through Rewarden: git talks to the git hosts directly again (undo with `rewarden resume`).
    #[command(alias = "disable")]
    Pause,
    /// Send git through Rewarden: starts the background service if needed, then routes the enabled hosts' remotes
    /// through it.
    #[command(alias = "enable")]
    Resume,
    /// Install the latest release (signed by the Rewarden release key) and restart the service.
    Update {
        /// Only say whether a newer release exists.
        #[arg(long)]
        check: bool,
    },
    #[command(flatten)]
    Agents(rewarden_desktop::agents_cli::Command),
    /// Run a command with secrets from the vault on your phone as environment variables.
    Run(rewarden_desktop::run::RunArgs),
    /// The SSH agent whose keys stay on your phone.
    Ssh {
        #[command(subcommand)]
        action: rewarden_desktop::ssh_agent::setup::SshCmd,
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

/// What `rewarden login` shows while the phone has to scan: the QR code, the code to type instead, the number to tap
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
    let minutes = (pairing.expires_at - rewarden_desktop::now_unix()).max(60) / 60;
    out!("Waiting for your phone (the code works for {minutes} minutes)...");
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).ok_or_else(|| "HOME is not set".to_owned())
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
                return Ok(());
            }
            Err(ClientError::NotRunning) => {}
            Err(e) => return Err(e.to_string()),
        },
        Err(ClientError::NotRunning) => {}
        Err(e) => return Err(e.to_string()),
    }
    out!("Version:       {}", update::LONG_VERSION);
    out!("Daemon:        not running (rewarden daemon, or rewarden service install)");
    out!("Mode:          {}", format!("{:?}", config.mode).to_ascii_lowercase());
    out!("Server:        {}", server::oauth::logged_in_server(paths).as_deref().unwrap_or("not logged in"));
    if paths.identity_file().exists() {
        let id = Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
        out!("Key:           {}", id.fingerprint());
    }
    out!("Listen:        {}", config.listen);
    out!("Git:           {}", git_mode(config));
    Ok(())
}

fn git_mode(config: &Config) -> String {
    match Git::default().hosts_set_up(&Scope::Global, config) {
        Ok(hosts) if !hosts.is_empty() => {
            format!("{} through Rewarden (http://{}/); `rewarden pause` to go direct", hosts.join(", "), config.listen)
        }
        Ok(_) => "direct to the git hosts; `rewarden resume` to go through Rewarden".to_owned(),
        Err(e) => format!("unknown ({e})"),
    }
}

/// `github.com, gitlab.com`: the hosts git is sent through Rewarden for.
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
        Cmd::Daemon => {
            init_logging();
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
            let manager = service::Manager::current()?;
            let home = home()?;
            match action {
                ServiceCmd::Install => {
                    let exe = std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?;
                    let file = service::install(manager, &home, &exe, true)?;
                    out!("Installed and started {}", file.display());
                }
                ServiceCmd::Uninstall => {
                    if service::uninstall(manager, &home, true)? {
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
        } => self_update(&config, check).await,
        Cmd::Pause => pause(&config),
        Cmd::Resume => resume(&paths, &config).await,
        Cmd::Agents(_) => unreachable!("handled in main"),
        Cmd::Run(args) => {
            let code = rewarden_desktop::run::main(&paths, &config, &args).await;
            std::process::exit(i32::from(code));
        }
        Cmd::Ssh {
            action,
        } => {
            for line in rewarden_desktop::ssh_agent::setup::run(&action, &paths, &config, &home()?)? {
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
        out!("Paused: git talks to the git hosts directly again. `rewarden resume` switches back.");
    }
    Ok(())
}

async fn resume(paths: &Paths, config: &Config) -> Result<(), String> {
    if !daemon_running(paths, config).await {
        let manager = service::Manager::current()?;
        let exe = update::current_executable()?;
        let file = service::install(manager, &home()?, &exe, true)?;
        out!("Started the background service ({}).", file.display());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !daemon_running(paths, config).await {
            if std::time::Instant::now() > deadline {
                return Err(
                    "the background service did not start; see `rewarden daemon` for why. git was left as it was"
                        .to_owned(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    Git::default().setup_hosts(&Scope::Global, config)?;
    out!("git sends {} through Rewarden (http://{}/).", enabled_hosts(config)?, config.listen);
    if server::oauth::logged_in_server(paths).is_none() {
        out!("Not logged in: the local policy decides. `rewarden login` to decide on your phone.");
    }
    out!("`rewarden pause` switches back to talking to the git hosts directly.");
    Ok(())
}

async fn self_update(config: &Config, check_only: bool) -> Result<(), String> {
    let updater = update::Updater::for_this_binary(&config.releases)?;
    match updater.check().await? {
        Check::UpToDate(latest) => {
            out!("rewarden {} is up to date (latest release: {}).", update::LONG_VERSION, latest.build);
        }
        Check::NoBuild(latest) => {
            out!("Release {} has no build for this computer yet.", latest.build);
        }
        Check::Available(latest, _) if check_only => {
            out!("Update available: {} → {}. Run `rewarden update`.", update::BUILD, latest.build);
        }
        Check::Available(latest, asset) => {
            let bytes = updater.download(&asset).await?;
            let exe = update::current_executable()?;
            update::replace_executable(&exe, &bytes)?;
            out!("Updated {} to {} ({}).", exe.display(), latest.version, latest.build);
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
        return rewarden_desktop::agents_cli::run(cmd).await;
    }
    match run(cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rewarden: {e}");
            ExitCode::FAILURE
        }
    }
}
