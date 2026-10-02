//! Everything the app does to the computer, through the `rewarden_desktop` library: the session, the harnesses'
//! settings, the background service, git's routing, the daemon's control API and the update feed.
//!
//! The service runs the `rewarden` program that ships with the app (next to it in the bundle or install directory).
//! Where no service manager is available, or with `REINS_DAEMON=in-app`, the daemon runs inside the app instead, for
//! as long as the app runs.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rewarden_desktop::config::{Config, Paths};
use rewarden_desktop::control::{Client, ClientError};
use rewarden_desktop::daemon::{Daemon, Options};
use rewarden_desktop::harness::{self, Harness, detect};
use rewarden_desktop::identity::Identity;
use rewarden_desktop::server::oauth;
use rewarden_desktop::setup::{Git, Scope};
use rewarden_desktop::update::{self, Check};
use rewarden_desktop::service;

use crate::state::Saved;

/// The `rewarden` program's file name.
const CLI_NAME: &str = if cfg!(windows) {
    "rewarden.exe"
} else {
    "rewarden"
};

/// How long starting the service may take before the app says it did not start.
const SERVICE_START: Duration = Duration::from_secs(15);

/// One harness as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessRow {
    pub harness: Harness,
    /// Installed on this computer.
    pub found: bool,
    /// Reins is in its settings.
    pub added: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonState {
    Running {
        /// Local approvals waiting (local mode only).
        pending: usize,
    },
    Stopped,
    /// The control API answered something unexpected.
    Unknown(String),
}

/// What the status window and the tray show, read fresh every few seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The server this computer is paired with.
    pub server: Option<String>,
    pub daemon: DaemonState,
    /// The git hosts sent through Reins.
    pub git_routed: Vec<String>,
    pub harnesses: Vec<HarnessRow>,
    /// This computer's key, as the phone shows it when pairing.
    pub fingerprint: Option<String>,
}

pub struct Backend {
    paths: Paths,
    home: PathBuf,
    /// The daemon, when it runs inside the app.
    in_app: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "cannot find your home directory (HOME is not set)".to_owned())
}

/// A copy of the app that will not be at this path next time: inside a disk image, moved aside by Gatekeeper
/// (App Translocation), or an AppImage's temporary mount.
fn is_transient(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/AppTranslocation/")
        || s.starts_with("/Volumes/")
        || std::env::var_os("APPDIR").is_some_and(|dir| !dir.is_empty() && path.starts_with(dir))
}

impl Backend {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            paths: Paths::from_env().map_err(|e| e.to_string())?,
            home: home_dir()?,
            in_app: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub fn under(root: &Path) -> Self {
        Self {
            paths: Paths::under(root),
            home: root.to_path_buf(),
            in_app: Mutex::new(None),
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    fn config(&self) -> Result<Config, String> {
        Config::load(&self.paths).map_err(|e| e.to_string())
    }

    /// The server to pair with: `REWARDEN_SERVER`, else the one picked in the app, else the default.
    #[must_use]
    pub fn server(saved: &Saved) -> String {
        std::env::var("REWARDEN_SERVER")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| saved.server.clone())
            .unwrap_or_else(|| crate::links::SERVER.to_owned())
    }

    /// The server this computer is paired with.
    #[must_use]
    pub fn paired_server(&self) -> Option<String> {
        oauth::logged_in_server(&self.paths)
    }

    /// This computer's key fingerprint (made on first use).
    pub fn fingerprint(&self) -> Result<String, String> {
        self.paths.ensure().map_err(|e| e.to_string())?;
        Identity::load_or_create(&self.paths.identity_file()).map(|id| id.fingerprint()).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn identity(&self) -> Result<Identity, String> {
        self.paths.ensure().map_err(|e| e.to_string())?;
        Identity::load_or_create(&self.paths.identity_file()).map_err(|e| e.to_string())
    }

    /// Where the app may not stay: the setup needs a copy of Reins that is still there after a restart.
    #[must_use]
    pub fn location_problem() -> Option<&'static str> {
        let exe = std::env::current_exe().ok()?;
        if std::env::var_os("APPIMAGE").is_some() {
            return None;
        }
        is_transient(&exe).then_some(if cfg!(target_os = "macos") {
            "Move Reins to your Applications folder first, then open it from there."
        } else {
            "Install Reins first, then open the installed copy."
        })
    }

    /// The `rewarden` program the harnesses and the service run: `REINS_CLI`, the one next to the app, the installed
    /// command line tool, or one on the `PATH`.
    pub fn cli(&self) -> Result<PathBuf, String> {
        if let Some(cli) = std::env::var_os("REINS_CLI").filter(|c| !c.is_empty()) {
            return Ok(PathBuf::from(cli));
        }
        if let Some(bundled) = Self::bundled_cli()
            && !is_transient(&bundled)
        {
            return Ok(bundled);
        }
        let installed = self.home.join(".local/bin").join(CLI_NAME);
        if installed.is_file() {
            return Ok(installed);
        }
        std::env::var_os("PATH")
            .and_then(|p| std::env::split_paths(&p).map(|d| d.join(CLI_NAME)).find(|c| c.is_file()))
            .ok_or_else(|| format!("cannot find the `{CLI_NAME}` program that comes with Reins; reinstall Reins"))
    }

    /// The `rewarden` program shipped next to this app.
    fn bundled_cli() -> Option<PathBuf> {
        let exe = update::current_executable().ok()?;
        let bundled = exe.parent()?.join(CLI_NAME);
        bundled.is_file().then_some(bundled)
    }

    /// "Install command line tool": `~/.local/bin/rewarden`, a link to the program in the app (a copy where the
    /// app's own path changes, as with an AppImage). Returns where it is.
    pub fn install_cli(&self) -> Result<PathBuf, String> {
        let bundled = Self::bundled_cli().ok_or("this copy of Reins has no command line tool next to it")?;
        let dir = self.home.join(".local/bin");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let target = dir.join(CLI_NAME);
        if target == bundled {
            return Ok(target);
        }
        let tmp = dir.join(format!(".{CLI_NAME}.reins-new"));
        let _old = std::fs::remove_file(&tmp);
        #[cfg(unix)]
        let made = if is_transient(&bundled) {
            std::fs::copy(&bundled, &tmp).map(drop)
        } else {
            std::os::unix::fs::symlink(&bundled, &tmp)
        };
        #[cfg(not(unix))]
        let made = std::fs::copy(&bundled, &tmp).map(drop);
        made.map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        Ok(target)
    }

    fn harness_setup(&self, config: &Config) -> Result<harness::Setup, String> {
        Ok(harness::Setup {
            home: self.home.clone(),
            exe: self.cli()?,
            hook_timeout_secs: config.guard.timeout_secs,
        })
    }

    /// Every harness Reins knows, whether it is installed and whether Reins is in it.
    #[must_use]
    pub fn harnesses(&self) -> Vec<HarnessRow> {
        let setup = self.config().and_then(|c| self.harness_setup(&c)).ok();
        Harness::ALL
            .into_iter()
            .map(|h| HarnessRow {
                harness: h,
                found: detect::found(h, &self.home),
                added: setup
                    .as_ref()
                    .and_then(|s| harness::registered(&self.paths, s, h).ok())
                    .is_some_and(|r| r.any()),
            })
            .collect()
    }

    /// Adds Reins to `h` (its MCP server and hook), or takes it out again.
    pub fn set_harness(&self, h: Harness, on: bool) -> Result<(), String> {
        let config = self.config()?;
        let setup = self.harness_setup(&config)?;
        let lines = if on {
            harness::add(&self.paths, &setup, h)?
        } else {
            harness::remove(&self.paths, &setup, h)?
        };
        for line in lines {
            log::info!("{}: {line}", h.label());
        }
        Ok(())
    }

    async fn daemon_status(&self, config: &Config) -> DaemonState {
        let client = match Client::new(&self.paths, config.listen) {
            Ok(c) => c,
            Err(ClientError::NotRunning) => return DaemonState::Stopped,
            Err(e) => return DaemonState::Unknown(e.to_string()),
        };
        match client.status().await {
            Ok(s) => DaemonState::Running {
                pending: s.pending,
            },
            Err(ClientError::NotRunning) => DaemonState::Stopped,
            Err(e) => DaemonState::Unknown(e.to_string()),
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let config = self.config();
        let daemon = match &config {
            Ok(c) => self.daemon_status(c).await,
            Err(e) => DaemonState::Unknown(e.clone()),
        };
        let git_routed = config.as_ref().map_or_else(
            |_| Vec::new(),
            |c| Git::default().hosts_set_up(&Scope::Global, c).unwrap_or_default(),
        );
        Snapshot {
            server: self.paired_server(),
            daemon,
            git_routed,
            harnesses: self.harnesses(),
            fingerprint: self.paths.identity_file().exists().then(|| self.fingerprint().ok()).flatten(),
        }
    }

    /// Whether the daemon should run inside the app rather than as a service.
    fn daemon_in_app() -> bool {
        std::env::var("REINS_DAEMON").is_ok_and(|v| v == "in-app") || service::Manager::current().is_err()
    }

    /// Starts the background service (installing it if needed) and waits until the daemon answers. Says how it runs.
    pub async fn start_service(&self) -> Result<String, String> {
        let config = self.config()?;
        if matches!(self.daemon_status(&config).await, DaemonState::Running { .. }) {
            return Ok("The background service is running.".to_owned());
        }
        let how = if Self::daemon_in_app() {
            self.start_in_app(config.clone()).await?;
            "Reins runs the service while it is open."
        } else {
            let manager = service::Manager::current()?;
            let file = service::install(manager, &self.home, &self.cli()?, true)?;
            log::info!("installed the service at {}", file.display());
            "The background service runs at login."
        };
        let deadline = Instant::now() + SERVICE_START;
        loop {
            match self.daemon_status(&config).await {
                DaemonState::Running { .. } => return Ok(how.to_owned()),
                _ if Instant::now() > deadline => {
                    return Err("the background service did not start (run `rewarden daemon` in a terminal to see why)"
                        .to_owned());
                }
                _ => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }

    async fn start_in_app(&self, config: Config) -> Result<(), String> {
        let mut running = self.in_app.lock().map_err(|_| "the app's daemon lock is poisoned".to_owned())?;
        if running.as_ref().is_some_and(|t| !t.is_finished()) {
            return Ok(());
        }
        let daemon = Daemon::bind(
            &self.paths,
            config,
            Options {
                harden: false,
                ..Options::default()
            },
        )
        .await?;
        log::info!("the daemon runs in the app on http://{} ({} decides)", daemon.local_addr(), daemon.describe());
        *running = Some(tokio::spawn(async move {
            if let Err(e) = daemon.run().await {
                log::warn!("the app's daemon stopped: {e}");
            }
        }));
        Ok(())
    }

    /// Sends the enabled git hosts through Reins (starting the service first if needed).
    pub async fn resume(&self) -> Result<(), String> {
        self.start_service().await?;
        let config = self.config()?;
        Git::default().setup_hosts(&Scope::Global, &config)?;
        Ok(())
    }

    /// git talks to the hosts directly again.
    pub fn pause(&self) -> Result<(), String> {
        let config = self.config()?;
        Git::default().unsetup_hosts(&Scope::Global, &config)?;
        Ok(())
    }

    /// A newer release, if there is one for this computer.
    pub async fn update_available(&self) -> Option<String> {
        let config = self.config().ok()?;
        let updater = update::Updater::for_this_binary(&config.releases).ok()?;
        match updater.check().await {
            Ok(Check::Available(latest, _)) => Some(latest.version),
            Ok(_) => None,
            Err(e) => {
                log::info!("update check: {e}");
                None
            }
        }
    }

    /// Signs in through the browser (`rewarden login`'s way). Returns the server.
    pub async fn sign_in_with_browser(&self, server: &str) -> Result<String, String> {
        let identity = self.identity()?;
        oauth::login(&self.paths, &identity, server, true).await
    }

    /// Forgets the session (the phone keeps the connection until it is removed there).
    pub fn sign_out(&self) -> Result<(), String> {
        oauth::logout(&self.paths).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_locations() {
        assert!(is_transient(Path::new("/Volumes/Reins/Reins.app/Contents/MacOS/Reins")));
        assert!(is_transient(Path::new(
            "/private/var/folders/x/AppTranslocation/1234/d/Reins.app/Contents/MacOS/rewarden"
        )));
        assert!(!is_transient(Path::new("/Applications/Reins.app/Contents/MacOS/rewarden")));
    }

    #[test]
    fn harness_rows_follow_what_is_installed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codex")).unwrap();
        let backend = Backend::under(root.path());
        let rows = backend.harnesses();
        assert_eq!(rows.len(), Harness::ALL.len());
        let codex = rows.iter().find(|r| r.harness == Harness::Codex).unwrap();
        assert!(codex.found && !codex.added);
    }

    #[test]
    fn the_server_defaults_to_the_hosted_one() {
        let saved = Saved::default();
        if std::env::var_os("REWARDEN_SERVER").is_none() {
            assert_eq!(Backend::server(&saved), crate::links::SERVER);
            let own = Saved {
                server: Some("https://reins.example.org".to_owned()),
                ..Saved::default()
            };
            assert_eq!(Backend::server(&own), "https://reins.example.org");
        }
    }
}
