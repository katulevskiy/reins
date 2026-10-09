//! Everything the app does to the computer, through the `reins_desktop` library: the session, the harnesses'
//! settings, the background service, git's routing, the daemon's control API (status and overview), this computer's
//! activity log, the settings in `config.toml`, the update feed and installing what it offers.
//!
//! The service runs the `reins` program that ships with the app (next to it in the bundle or install directory).
//! Where no service manager is available, or with `REINS_DAEMON=in-app`, the daemon runs inside the app instead, for
//! as long as the app runs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use reins_desktop::ask::{self, Answer};
use reins_desktop::config::{Config, Paths};
use reins_desktop::control::{self, Client, ClientError, Overview};
use reins_desktop::daemon::{Daemon, Options};
use reins_desktop::doctor::Doctor;
use reins_desktop::guard::OnNoAnswer;
use reins_desktop::harness::{self, Harness, detect};
use reins_desktop::identity::Identity;
use reins_desktop::journal::{self, Entry};
use reins_desktop::server::oauth;
use reins_desktop::service;
use reins_desktop::settings::{self, Change, GuardList};
use reins_desktop::setup::{Git, Scope};
use reins_desktop::update::{self, Check};

use crate::state::Saved;
use crate::upgrade::{self, Target};

/// The `reins` program's file name.
const CLI_NAME: &str = if cfg!(windows) {
    "reins.exe"
} else {
    "reins"
};

/// How long starting the service may take before the app says it did not start.
const SERVICE_START: Duration = Duration::from_secs(15);
/// The most activity entries read.
const ACTIVITY_LIMIT: usize = 500;
/// A request still "waiting" after this long was cut off (its process ended without writing the end).
const STALE_AFTER: i64 = 3_600;

/// One harness as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessRow {
    pub harness: Harness,
    /// Installed on this computer.
    pub found: bool,
    /// Reins is in its settings.
    pub added: bool,
    /// Reins is fully in its settings (its MCP server and its hook, for this copy of `reins`).
    pub complete: bool,
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
    /// `config.toml` as read, or why it could not be.
    pub config: Result<Arc<Config>, String>,
    /// The daemon's own status, while it runs.
    pub status: Option<control::Status>,
    /// What the daemon did and holds since it started (empty while it is not running).
    pub overview: Arc<Overview>,
    /// This computer's activity log, newest first.
    pub activity: Arc<Vec<Entry>>,
}

impl Snapshot {
    /// The daemon's address (`127.0.0.1:7457`): as it says, else as the settings say.
    #[must_use]
    pub fn listen(&self) -> String {
        self.status.as_ref().map_or_else(
            || self.config.as_ref().map_or_else(|_| "127.0.0.1:7457".to_owned(), |c| c.listen.to_string()),
            |s| s.listen.clone(),
        )
    }
}

/// One setting the window changes in `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    /// A group of built-in hook rules asks the phone (`true`) or goes through.
    GuardGroup(String, bool),
    GuardList(GuardList, Vec<String>),
    OnNoAnswer(OnNoAnswer),
    GuardTimeout(u64),
    ApprovalTimeout(u64),
    Notify {
        phone: bool,
        sound: bool,
    },
    Ssh(bool),
    /// git for this host goes through Reins.
    Host(String, bool),
}

/// The activity log as last read, and what its files looked like then.
type JournalCache = Option<(JournalKey, Arc<Vec<Entry>>)>;
type JournalKey = (Option<(u64, SystemTime)>, Option<(u64, SystemTime)>, i64);

/// A newer Reins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    /// Downloaded and checked: "Restart to update" installs it. `None`: the download page has it (this copy cannot
    /// install it, or the release has no installer for it).
    pub ready: Option<Ready>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub installer: PathBuf,
    /// The signed entry the installer was checked against; checked again just before installing.
    pub asset: update::Asset,
    pub target: Target,
}

pub struct Backend {
    paths: Paths,
    home: PathBuf,
    /// How git is run (with `HOME` set when `REINS_HOME` stands in for it).
    git: Git,
    /// The daemon, when it runs inside the app.
    in_app: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    journal: Mutex<JournalCache>,
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
    /// From the environment. `REINS_HOME` stands in for the home directory (the harnesses' settings, git's global
    /// config, the login items, and Reins's own settings and state under it), to try the app without touching the
    /// real ones.
    pub fn from_env() -> Result<Self, String> {
        if let Some(home) = std::env::var_os("REINS_HOME").filter(|h| !h.is_empty()).map(PathBuf::from) {
            return Ok(Self {
                paths: Paths {
                    config_dir: home.join(".config").join("reins"),
                    state_dir: home.join(".local").join("state").join("reins"),
                },
                git: Git::default().env("HOME", &home).env("XDG_CONFIG_HOME", home.join(".config")),
                home,
                in_app: tokio::sync::Mutex::new(None),
                journal: Mutex::new(None),
            });
        }
        Ok(Self {
            paths: Paths::from_env().map_err(|e| e.to_string())?,
            home: reins_desktop::config::home_dir()?,
            git: Git::default(),
            in_app: tokio::sync::Mutex::new(None),
            journal: Mutex::new(None),
        })
    }

    /// The home directory the harnesses' settings are under.
    pub fn home(&self) -> &Path {
        &self.home
    }

    #[cfg(test)]
    pub fn under(root: &Path) -> Self {
        Self {
            paths: Paths::under(root),
            home: root.to_path_buf(),
            git: Git::default().env("HOME", root),
            in_app: tokio::sync::Mutex::new(None),
            journal: Mutex::new(None),
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    fn config(&self) -> Result<Config, String> {
        Config::load(&self.paths).map_err(|e| e.to_string())
    }

    /// The server to pair with: `REINS_SERVER`, else the one picked in the app, else the default.
    #[must_use]
    pub fn server(saved: &Saved) -> String {
        std::env::var("REINS_SERVER")
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

    /// The `reins` program the harnesses and the service run: `REINS_CLI`, the one next to the app, the installed
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

    /// The `reins` program shipped next to this app.
    fn bundled_cli() -> Option<PathBuf> {
        let exe = update::current_executable().ok()?;
        let bundled = exe.parent()?.join(CLI_NAME);
        bundled.is_file().then_some(bundled)
    }

    /// "Install command line tool": `~/.local/bin/reins`, a link to the program in the app (a copy where the
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
            .map(|h| {
                let registered = setup.as_ref().and_then(|s| harness::registered(&self.paths, s, h).ok());
                HarnessRow {
                    harness: h,
                    found: detect::found(h, &self.home),
                    added: registered.as_ref().is_some_and(harness::Registered::any),
                    complete: registered.as_ref().is_some_and(harness::Registered::complete),
                }
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
        self.daemon(config, false).await.0
    }

    /// The daemon's state and, while it runs (and `overview`), its status and overview.
    async fn daemon(&self, config: &Config, overview: bool) -> (DaemonState, Option<control::Status>, Overview) {
        let client = match Client::new(&self.paths, config.listen) {
            Ok(c) => c,
            Err(ClientError::NotRunning) => return (DaemonState::Stopped, None, Overview::default()),
            Err(e) => return (DaemonState::Unknown(e.to_string()), None, Overview::default()),
        };
        match client.status().await {
            Ok(s) => {
                let seen = if overview {
                    client.overview().await.unwrap_or_else(|e| {
                        log::info!("overview: {e}");
                        Overview::default()
                    })
                } else {
                    Overview::default()
                };
                (
                    DaemonState::Running {
                        pending: s.pending,
                    },
                    Some(s),
                    seen,
                )
            }
            Err(ClientError::NotRunning) => (DaemonState::Stopped, None, Overview::default()),
            Err(e) => (DaemonState::Unknown(e.to_string()), None, Overview::default()),
        }
    }

    /// This computer's activity log, newest first (read again only when its files changed, or a minute passed for
    /// requests that were cut off).
    pub fn activity(&self) -> Arc<Vec<Entry>> {
        let file = journal::Journal::file(&self.paths);
        let mut old = file.as_os_str().to_owned();
        old.push(".old");
        let look =
            |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        let key = (look(&file), look(Path::new(&old)), reins_desktop::now_unix() / 60);
        let mut cache = self.journal.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((seen, entries)) = cache.as_ref()
            && *seen == key
        {
            return Arc::clone(entries);
        }
        let entries = Arc::new(journal::read(&self.paths, ACTIVITY_LIMIT, STALE_AFTER));
        *cache = Some((key, Arc::clone(&entries)));
        entries
    }

    pub async fn snapshot(&self) -> Snapshot {
        let config = self.config();
        let (daemon, status, overview) = match &config {
            Ok(c) => self.daemon(c, true).await,
            Err(e) => (DaemonState::Unknown(e.clone()), None, Overview::default()),
        };
        let git_routed = config
            .as_ref()
            .map_or_else(|_| Vec::new(), |c| self.git.hosts_set_up(&Scope::Global, c).unwrap_or_default());
        Snapshot {
            server: self.paired_server(),
            daemon,
            git_routed,
            harnesses: self.harnesses(),
            fingerprint: self.paths.identity_file().exists().then(|| self.fingerprint().ok()).flatten(),
            config: config.map(Arc::new),
            status,
            overview: Arc::new(overview),
            activity: self.activity(),
        }
    }

    /// Whether the daemon should run inside the app rather than as a service.
    fn daemon_in_app() -> bool {
        std::env::var("REINS_DAEMON").is_ok_and(|v| v == "in-app")
            || (!cfg!(windows) && service::Manager::current().is_err())
    }

    /// Installs the background service for the bundled `reins` and starts it (the `Run` key's windowless copy on
    /// Windows, launchd or systemd elsewhere).
    // Only Windows waits (for the daemon's control API).
    #[cfg_attr(not(windows), allow(clippy::unused_async, clippy::unused_async_trait_impl))]
    async fn install_service(&self, config: &Config) -> Result<(), String> {
        let cli = self.cli()?;
        #[cfg(windows)]
        {
            let what = service::windows::install(&self.paths, config, &cli, true).await?;
            log::info!("installed the service: {what}");
        }
        #[cfg(not(windows))]
        {
            let _ = config;
            let manager = service::Manager::current()?;
            let file = service::install(manager, &self.home, &cli, true)?;
            log::info!("installed the service at {}", file.display());
        }
        Ok(())
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
            self.install_service(&config).await?;
            "The background service runs at login."
        };
        let deadline = Instant::now() + SERVICE_START;
        loop {
            match self.daemon_status(&config).await {
                DaemonState::Running {
                    ..
                } => return Ok(how.to_owned()),
                _ if Instant::now() > deadline => {
                    return Err(
                        "the background service did not start (run `reins daemon` in a terminal to see why)".to_owned()
                    );
                }
                _ => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }

    async fn start_in_app(&self, config: Config) -> Result<(), String> {
        let mut running = self.in_app.lock().await;
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
        self.git.setup_hosts(&Scope::Global, &config)?;
        Ok(())
    }

    /// git talks to the hosts directly again.
    pub fn pause(&self) -> Result<(), String> {
        let config = self.config()?;
        self.git.unsetup_hosts(&Scope::Global, &config)?;
        Ok(())
    }

    /// Writes one setting to `config.toml`. When the daemon only sees it after a restart (and `restart`), restarts the
    /// background service (or the daemon inside the app) and, for a git host, routes git again (unless `paused`).
    /// Says anything the user should know (`None`: nothing to say).
    pub async fn change(&self, setting: Setting, paused: bool, restart: bool) -> Result<Option<String>, String> {
        let host = matches!(setting, Setting::Host(..));
        let change = match &setting {
            Setting::GuardGroup(id, on) => settings::set_guard_group(&self.paths, id, *on),
            Setting::GuardList(list, items) => settings::set_guard_list(&self.paths, *list, items),
            Setting::OnNoAnswer(n) => settings::set_guard_on_no_answer(&self.paths, *n),
            Setting::GuardTimeout(secs) => settings::set_guard_timeout(&self.paths, *secs),
            Setting::ApprovalTimeout(secs) => settings::set_approval_timeout(&self.paths, *secs),
            Setting::Notify {
                phone,
                sound,
            } => settings::set_notify(&self.paths, *phone, *sound),
            Setting::Ssh(on) => settings::set_ssh_enabled(&self.paths, *on),
            Setting::Host(h, on) => settings::set_host_enabled(&self.paths, h, *on),
        }?;
        if change == Change::Live || !restart {
            return Ok(None);
        }
        let note = self.restart_service().await?;
        if host && !paused {
            let config = self.config()?;
            self.git.setup_hosts(&Scope::Global, &config)?;
        }
        Ok(note)
    }

    /// Restarts the daemon so it reads `config.toml` again: the one inside the app, or the installed service.
    pub async fn restart_service(&self) -> Result<Option<String>, String> {
        let config = self.config()?;
        {
            let mut running = self.in_app.lock().await;
            if let Some(task) = running.take().filter(|t| !t.is_finished()) {
                if let Ok(client) = Client::new(&self.paths, config.listen) {
                    client.shutdown(Duration::from_secs(5)).await.map_err(|e| e.to_string())?;
                }
                if tokio::time::timeout(Duration::from_secs(5), task).await.is_err() {
                    log::warn!("the app's daemon did not stop in time");
                }
            }
        }
        if Self::daemon_in_app() {
            if matches!(self.daemon_status(&config).await, DaemonState::Running { .. }) {
                return Ok(Some("Restart `reins daemon` so it uses the new settings.".to_owned()));
            }
            self.start_in_app(config).await?;
            return Ok(None);
        }
        #[cfg(windows)]
        let restarted = service::windows::restart_if_installed(&self.paths, &config, &self.cli()?).await?;
        #[cfg(not(windows))]
        let restarted = service::Manager::current().and_then(|m| service::restart_if_installed(m, &self.home))?;
        match restarted {
            Some(done) => log::info!("{done}"),
            // Started some other way (`reins daemon` in a terminal).
            None if matches!(self.daemon_status(&config).await, DaemonState::Running { .. }) => {
                return Ok(Some("Restart `reins daemon` so it uses the new settings.".to_owned()));
            }
            None => {}
        }
        Ok(None)
    }

    /// Where installers are downloaded to.
    fn update_cache(&self) -> PathBuf {
        self.paths.state_dir.join("updates")
    }

    /// Left by "Restart to update" for the new app's first start ([`Self::after_update`]).
    fn update_marker(&self) -> PathBuf {
        self.paths.state_dir.join("app-updated")
    }

    /// A newer release, if there is one for this computer: downloaded and checked when this copy can install it
    /// (`app.json`), else for the download page (`latest.json`, as when the feed has no installer for it). An error
    /// means the feed could not be read, or the download failed; the next check tries again.
    pub async fn check_update(&self) -> Result<Option<Update>, String> {
        // A build from source has no release time: every release would look newer.
        if update::BUILD == "dev" {
            return Ok(None);
        }
        let config = self.config()?;
        let target = upgrade::target().filter(|t| !is_transient(t.path()));
        if let Some(target) = target
            && let Ok(updater) = update::Updater::for_this_app(&config.releases)
        {
            match updater.check_app().await? {
                Some(Check::UpToDate(_)) => {
                    update::remove_downloads(&self.update_cache(), None);
                    return Ok(None);
                }
                Some(Check::Available(latest, asset)) => {
                    let installer = updater.fetch_app(&asset, &self.update_cache()).await?;
                    return Ok(Some(Update {
                        version: latest.version,
                        ready: Some(Ready {
                            installer,
                            asset,
                            target,
                        }),
                    }));
                }
                // An older feed, or no installer for this computer in it.
                Some(Check::NoBuild(_)) | None => {}
            }
        }
        let updater = update::Updater::for_this_binary(&config.releases)?;
        Ok(match updater.check().await? {
            Check::Available(latest, _) => Some(Update {
                version: latest.version,
                ready: None,
            }),
            _ => None,
        })
    }

    /// Installs a downloaded update (see [`upgrade`]); when the new app is in place, its first start restarts the
    /// background service ([`Self::after_update`]). Blocks.
    pub fn install_update(&self, ready: &Ready) -> Result<upgrade::Installed, String> {
        // The cached download may have changed since it was checked; install only the signed file.
        if !update::is_signed_file(&ready.installer, &ready.asset) {
            update::remove_downloads(&self.update_cache(), None);
            return Err(
                "the downloaded update does not match the signed release; it will be downloaded again".to_owned()
            );
        }
        let installed = upgrade::install(&ready.target, &ready.installer)?;
        if let Err(e) = std::fs::write(self.update_marker(), update::BUILD) {
            log::warn!("{}: {e}", self.update_marker().display());
        }
        Ok(installed)
    }

    /// At start: removes what an update left behind and, on the first start after "Restart to update", makes the
    /// background service run the new `reins` (the one it ran is the old app's). Says what it did.
    pub async fn after_update(&self, setup_done: bool) -> Result<Option<String>, String> {
        if let Some(target) = upgrade::target() {
            upgrade::remove_leftovers(&target);
        }
        if std::fs::remove_file(self.update_marker()).is_err() {
            return Ok(None);
        }
        // An AppImage's command line tool is a copy (see `install_cli`): the new one replaces it.
        if std::env::var_os("APPIMAGE").is_some()
            && self.home.join(".local/bin").join(CLI_NAME).is_file()
            && let Err(e) = self.install_cli()
        {
            log::warn!("after the update: {e}");
        }
        if !Self::daemon_in_app() {
            #[cfg(windows)]
            let restarted = match (self.config(), self.cli()) {
                (Ok(config), Ok(cli)) => service::windows::restart_if_installed(&self.paths, &config, &cli).await,
                (Err(e), _) | (_, Err(e)) => Err(e),
            };
            #[cfg(not(windows))]
            let restarted = service::Manager::current().and_then(|m| service::restart_if_installed(m, &self.home));
            match restarted {
                Ok(Some(done)) => log::info!("after the update: {done}"),
                Ok(None) => {}
                Err(e) => log::warn!("after the update: {e}"),
            }
        }
        // Not running (a daemon that ran inside the old app, or one the restart did not start): started again.
        if setup_done {
            self.start_service().await?;
        }
        Ok(Some(format!("Updated to {}.", update::LONG_VERSION)))
    }

    /// `reins doctor`'s checks for this computer, as the app sets it up (its `reins`, its home and git).
    pub fn doctor(&self) -> Result<Doctor, String> {
        Ok(Doctor {
            paths: self.paths.clone(),
            config: self.config()?,
            home: self.home.clone(),
            // Without the program, the AI tools' entries cannot match it: the checks say so.
            exe: self.cli().unwrap_or_else(|_| PathBuf::from(CLI_NAME)),
            git: self.git.clone(),
        })
    }

    /// "Send a test to my phone" (`reins test`): the answer, and whether the wait ran out. Logged in the activity log.
    pub async fn send_test(&self) -> Result<(Answer, bool), String> {
        let config = self.config()?;
        Ok(ask::send_test(&self.paths, &config, &ask::DesktopAsk).await)
    }

    /// Signs in through the browser (`reins login`'s way). Returns the server.
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
        assert!(is_transient(Path::new("/Volumes/Reins/Reins.app/Contents/MacOS/reins-app")));
        assert!(is_transient(Path::new(
            "/private/var/folders/x/AppTranslocation/1234/d/Reins.app/Contents/MacOS/reins"
        )));
        assert!(!is_transient(Path::new("/Applications/Reins.app/Contents/MacOS/reins")));
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

    // On Windows the service is the user's `Run` key, which a test must not restart.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn only_the_first_start_after_an_update_handles_the_service() {
        let root = tempfile::tempdir().unwrap();
        let backend = Backend::under(root.path());
        backend.paths().ensure().unwrap();
        assert_eq!(backend.after_update(false).await.unwrap(), None);
        std::fs::write(backend.update_marker(), b"").unwrap();
        // No service installed under this home: nothing to restart.
        let done = backend.after_update(false).await.unwrap().unwrap();
        assert!(done.starts_with("Updated to "), "{done}");
        assert!(!backend.update_marker().exists());
        assert_eq!(backend.after_update(false).await.unwrap(), None);
    }

    #[tokio::test]
    async fn live_settings_are_written_without_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let backend = Backend::under(root.path());
        let note = backend
            .change(
                Setting::Notify {
                    phone: false,
                    sound: false,
                },
                false,
                true,
            )
            .await
            .unwrap();
        assert_eq!(note, None);
        backend.change(Setting::GuardGroup("publishing".to_owned(), false), false, true).await.unwrap();
        backend
            .change(Setting::GuardList(GuardList::AllowCommands, vec!["make test".to_owned()]), false, true)
            .await
            .unwrap();
        // A change for after a restart, without restarting (as in the demo).
        backend.change(Setting::Host("gitlab.com".to_owned(), true), true, false).await.unwrap();
        let config = backend.config().unwrap();
        assert!(!config.notify.phone && config.guard.group_off("publishing"));
        assert_eq!(config.guard.allow_commands, vec!["make test"]);
        assert!(config.git_hosts().unwrap().iter().any(|h| h.host == "gitlab.com" && h.enabled));
        assert!(backend.change(Setting::GuardTimeout(1), false, true).await.is_err(), "out of range");
    }

    #[test]
    fn the_activity_log_is_read_again_when_it_changes() {
        let root = tempfile::tempdir().unwrap();
        let backend = Backend::under(root.path());
        backend.paths().ensure().unwrap();
        assert!(backend.activity().is_empty());
        journal::Journal::new(backend.paths()).record(&Entry::new(journal::Kind::Ask, "Deploy now?"));
        let read = backend.activity();
        assert_eq!(read.len(), 1);
        assert!(Arc::ptr_eq(&read, &backend.activity()), "unchanged: the same list");
    }

    #[test]
    fn the_server_defaults_to_the_hosted_one() {
        let saved = Saved::default();
        if std::env::var_os("REINS_SERVER").is_none() {
            assert_eq!(Backend::server(&saved), crate::links::SERVER);
            let own = Saved {
                server: Some("https://reins.example.org".to_owned()),
                ..Saved::default()
            };
            assert_eq!(Backend::server(&own), "https://reins.example.org");
        }
    }
}
