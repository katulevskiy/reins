//! Finds the secrets lying around this computer: `.env` files in projects, SSH private keys, the GitHub CLI's token,
//! git's stored credentials, npm/PyPI/crates.io tokens, Docker logins, AWS/Google Cloud/Kubernetes credentials, shell
//! exports and `~/.netrc` (design: `docs/secrets.md`, 5.1). Local only and read-only: nothing is sent, nothing is
//! written. The report holds names, places, what each value is ([`detect`]), a masked preview (at most the first 4 and
//! last 4 characters, never more than a quarter of the value) and what makes it risky; values are read into
//! [`zeroize::Zeroizing`] buffers only to classify and mask them, and are dropped before [`scan`] returns. Import
//! reads the file again.
//!
//! **Only a person starts a scan.** [`scan`] refuses when the process runs for an AI agent: `REINS_RUNNER=agent:*`
//! (what the harness hooks export for an agent's commands, `docs/secrets.md` 6.1), or the markers the harnesses set
//! in the environment of the commands they run (`CLAUDECODE`, `GEMINI_CLI`, `CODEX_SANDBOX`, `CURSOR_AGENT`,
//! `AI_AGENT`). See [`Caller`].
//!
//! Walk rules: the project folders (`~/dev`, `~/code`, `~/src`, `~/projects`, `~/work`, and git repositories found
//! up to [`DISCOVER_DEPTH`] folders under the home folder), [`PROJECT_DEPTH`] folders deep; never `node_modules`,
//! `.git`, `target`, `vendor`, `.venv`, caches and the like ([`SKIP_DIRS`]); never a symbolic link out of the home
//! folder (or the folder given); never another user's file; never a file over [`MAX_FILE`] or a binary one. The walk
//! stops at a time and a count budget ([`Budget`]) and the report says so.

pub mod detect;
pub mod sources;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub use self::detect::{Confidence, Detector};
use self::sources::RawEntry;
pub use self::sources::Source;
use crate::guard::GuardConfig;

/// The report's format; a field added keeps it, anything else changes it.
pub const SCHEMA_VERSION: u32 = 1;
/// Larger files are skipped.
pub const MAX_FILE: u64 = 1024 * 1024;
/// How deep under the home folder git repositories are looked for.
pub const DISCOVER_DEPTH: usize = 4;
/// How deep a project is walked.
pub const PROJECT_DEPTH: usize = 8;
/// Where projects usually live, under the home folder.
pub const PROJECT_ROOTS: &[&str] = &["dev", "code", "src", "projects", "work", "Developer", "repos", "git"];
/// Folders never walked into.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "Caches",
    ".next",
    ".nuxt",
    ".turbo",
    ".tox",
    ".gradle",
    ".m2",
    "Pods",
    "DerivedData",
    "bower_components",
    ".Trash",
    "Trash",
    "Library",
    ".npm",
    ".pnpm-store",
    ".yarn",
    ".rustup",
    ".cargo",
    "go",
    ".terraform",
    "site-packages",
];
/// `.env` files that hold no secrets by convention.
const DOTENV_TEMPLATES: &[&str] =
    &[".env.example", ".env.sample", ".env.template", ".env.reins", ".env.dist", ".env.defaults"];
/// How long `git` and `gh` may take for one question.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
/// At most this many skipped places are listed (all are counted).
const MAX_SKIPPED_LISTED: usize = 200;

/// Who started the scan, from the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    Person,
    /// An AI agent: the variable that says so.
    Agent(String),
}

impl Caller {
    /// From this process's environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// From `var` (the environment's variables).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(runner) = var("REINS_RUNNER") {
            if runner.trim().starts_with("agent") {
                return Self::Agent(format!("REINS_RUNNER={}", runner.trim()));
            }
            if runner.trim() == "person" {
                return Self::Person;
            }
        }
        for name in ["CLAUDECODE", "GEMINI_CLI", "CODEX_SANDBOX", "CURSOR_AGENT", "AI_AGENT"] {
            if var(name).is_some_and(|v| !v.trim().is_empty() && v.trim() != "0") {
                return Self::Agent(name.to_owned());
            }
        }
        Self::Person
    }

    /// Refuses an agent, saying why.
    pub fn check(&self, what: &str) -> Result<(), String> {
        match self {
            Self::Person => Ok(()),
            Self::Agent(marker) => Err(format!(
                "{what} runs only when you start it yourself, in your own terminal or from the Reins window: this \
                 one was started by an AI agent ({marker} is set). Agents work with secrets through `reins run`, \
                 never by reading them."
            )),
        }
    }
}

/// What makes a finding risky.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// An AI agent on this computer can read the file: a harness is installed and the guard does not ask before
    /// it reads this path (or a harness runs without Reins' hooks).
    ReadableByAgents,
    /// The file is in a git repository's history (`git log --all -- PATH`) or tracked.
    InGitHistory,
    /// An SSH private key without a passphrase.
    NoPassphrase,
    /// Other users of this computer can read the file.
    WorldReadable,
    /// The secret is in the file as it is.
    PlainText,
    /// The secret is in the system keychain (the file only names it): less exposed.
    InKeychain,
}

impl Risk {
    /// What the review's chip says.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadableByAgents => "AI agents can read it",
            Self::InGitHistory => "In git history",
            Self::NoPassphrase => "No passphrase",
            Self::WorldReadable => "Others can read it",
            Self::PlainText => "Plain text",
            Self::InKeychain => "In the keychain",
        }
    }

    /// Whether it is a danger (red) rather than a warning.
    #[must_use]
    pub fn severe(self) -> bool {
        matches!(self, Self::ReadableByAgents | Self::InGitHistory | Self::NoPassphrase | Self::WorldReadable)
    }
}

/// The scan's steps, as the window shows its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Projects,
    Ssh,
    Github,
    Cloud,
    Registries,
    Shell,
}

impl Stage {
    pub const ALL: [Self; 6] = [Self::Projects, Self::Ssh, Self::Github, Self::Cloud, Self::Registries, Self::Shell];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Projects => "Projects (.env files)",
            Self::Ssh => "SSH keys",
            Self::Github => "GitHub CLI and git",
            Self::Cloud => "Cloud",
            Self::Registries => "Package registries",
            Self::Shell => "Shell startup files",
        }
    }

    /// Where it looks.
    #[must_use]
    pub fn places(self) -> &'static str {
        match self {
            Self::Projects => "~/dev, ~/code, ~/src, git repositories",
            Self::Ssh => "~/.ssh",
            Self::Github => "gh, ~/.git-credentials, ~/.netrc",
            Self::Cloud => "AWS, Google Cloud, Kubernetes, Docker",
            Self::Registries => "npm, PyPI, crates.io",
            Self::Shell => "~/.zshrc, ~/.bashrc, ~/.profile",
        }
    }

    /// The stage that finds `source`.
    #[must_use]
    pub fn of(source: Source) -> Self {
        match source {
            Source::Dotenv => Self::Projects,
            Source::SshKey => Self::Ssh,
            Source::GhCli | Source::GitCredentials | Source::Netrc => Self::Github,
            Source::Docker | Source::Aws | Source::Gcp | Source::Kube => Self::Cloud,
            Source::Npm | Source::PyPi | Source::Cargo => Self::Registries,
            Source::Shell => Self::Shell,
        }
    }
}

/// The review's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Projects,
    KeysAndTokens,
    CloudAndShell,
}

impl Group {
    pub const ALL: [Self; 3] = [Self::Projects, Self::KeysAndTokens, Self::CloudAndShell];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Projects => "Project .env files",
            Self::KeysAndTokens => "Keys and tokens",
            Self::CloudAndShell => "Cloud and shell",
        }
    }

    #[must_use]
    pub fn of(source: Source) -> Self {
        match source {
            Source::Dotenv => Self::Projects,
            Source::SshKey
            | Source::GhCli
            | Source::GitCredentials
            | Source::Npm
            | Source::PyPi
            | Source::Cargo
            | Source::Netrc => Self::KeysAndTokens,
            Source::Docker | Source::Aws | Source::Gcp | Source::Kube | Source::Shell => Self::CloudAndShell,
        }
    }
}

/// One value found: a variable, a token field, a login. Never the value itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Stable: `<finding id>/<group>/<name>` (`aws-1a2b3c4d5e6f/prod/aws_secret_access_key`).
    pub id: String,
    pub name: String,
    /// An AWS profile, a registry, a git host, a kube user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector: Option<Detector>,
    /// "OpenAI key", "not a secret".
    pub label: String,
    pub confidence: Confidence,
    /// At most 4 + 4 characters and a quarter of the value ([`detect::mask`]); for an SSH key its type and
    /// fingerprint.
    pub preview: String,
    /// Pre-ticked in the review.
    pub suggested: bool,
}

/// A file with secrets in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable for the same kind and file: `<kind>-<12 hex>`.
    pub id: String,
    pub source: Source,
    pub group: Group,
    /// What the review calls it: `acme-web/.env`, `id_ed25519`, `GitHub CLI token`.
    pub title: String,
    /// The file, as shown (`~/dev/acme-web/.env`).
    pub path: String,
    /// What the review's row shows under the title: the folder for a project file or a key, else the file.
    pub place: String,
    /// The file, absolute.
    pub abs_path: PathBuf,
    /// For a project's file: the project (the repository's folder name) and the folder it is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<PathBuf>,
    /// The latest commit that has the file, when it is in git history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    /// For an SSH key: its type and whether a passphrase protects it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_type: Option<String>,
    /// The review's right column: `6 keys`, `SSH key`, `default, prod`.
    pub summary: String,
    pub items: Vec<Item>,
    /// Most severe first.
    pub risks: Vec<Risk>,
    /// Any item is pre-ticked.
    pub suggested: bool,
}

impl Finding {
    /// The items the review ticks before the user does.
    pub fn suggested_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.suggested)
    }

    /// How many secrets the finding counts as (its pre-ticked items, at least one).
    #[must_use]
    pub fn secrets(&self) -> usize {
        self.items.iter().filter(|i| i.suggested).count().max(usize::from(!self.items.is_empty()))
    }
}

/// Why something was not looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    TooBig,
    Binary,
    OtherUser,
    SymlinkOutside,
    Unreadable,
    /// The secrets are with a credential helper or an exec plugin (the system keychain, a cloud CLI): nothing to move.
    Helper,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub path: String,
    pub reason: SkipReason,
}

/// Why the walk stopped early.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetHit {
    /// `time`, `entries` or `findings`.
    pub limit: String,
    /// Where it was.
    pub at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub version: u32,
    /// Unix seconds.
    pub started_at: i64,
    /// Milliseconds.
    pub took: u64,
    pub home: String,
    pub findings: Vec<Finding>,
    /// The places not looked at (the first [`MAX_SKIPPED_LISTED`]).
    pub skipped: Vec<Skipped>,
    pub skipped_total: usize,
    /// Set when the walk stopped at its budget: some projects were not looked at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetHit>,
    /// Stopped by the user.
    #[serde(default)]
    pub stopped: bool,
}

impl ScanReport {
    /// How many secrets there are (pre-ticked items; a finding counts at least one).
    #[must_use]
    pub fn secrets(&self) -> usize {
        self.findings.iter().map(Finding::secrets).sum()
    }

    /// How many findings have `risk`.
    #[must_use]
    pub fn with_risk(&self, risk: Risk) -> usize {
        self.findings.iter().filter(|f| f.risks.contains(&risk)).count()
    }

    #[must_use]
    pub fn finding(&self, id: &str) -> Option<&Finding> {
        self.findings.iter().find(|f| f.id == id)
    }
}

/// Limits of the walk through the project folders (the known files are always read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub time: Duration,
    /// Folders and files looked at.
    pub entries: usize,
    pub findings: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            time: Duration::from_secs(30),
            entries: 200_000,
            findings: 2_000,
        }
    }
}

/// Who could read a file besides the user: the AI tools installed here, and whether the guard asks before they read
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exposure {
    /// The harnesses installed on this computer (their names).
    pub agents: Vec<String>,
    /// Every installed harness has Reins' hooks (so the guard's file rules apply to it).
    pub hooked: bool,
    pub guard: GuardConfig,
}

impl Exposure {
    /// For this computer: the harnesses found under `home` and whether Reins is added to each.
    #[must_use]
    pub fn detect(paths: &crate::config::Paths, config: &crate::config::Config, home: &Path) -> Self {
        let found = crate::harness::detect::all_found(home);
        let setup = crate::harness::Setup {
            home: home.to_path_buf(),
            exe: crate::update::current_executable().unwrap_or_default(),
            hook_timeout_secs: config.guard.timeout_secs,
        };
        let hooked = found.iter().all(|h| crate::harness::registered(paths, &setup, *h).is_ok_and(|r| r.complete()));
        Self {
            agents: found.iter().map(|h| h.label().to_owned()).collect(),
            hooked,
            guard: config.guard.clone(),
        }
    }

    /// Whether an agent here can read `path`.
    #[must_use]
    pub fn readable(&self, path: &Path) -> bool {
        !self.agents.is_empty() && (!self.hooked || self.guard.check_file(&path.to_string_lossy()).is_none())
    }
}

/// Where a stage of the scan is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StageState {
    #[default]
    Waiting,
    Running,
    /// Done, with this many secrets found.
    Done(usize),
}

/// What the scan is reporting as it goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub stages: BTreeMap<Stage, StageState>,
    pub found: usize,
    /// The place being looked at (`~/dev/acme-web/node_modules/… (skipped)`).
    pub at: String,
    /// About how far along, 0 to 1.
    pub fraction: f32,
}

/// What to scan, and how.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub home: PathBuf,
    /// `reins secrets scan PATH...`: only these files and folders. Empty: the whole computer (5.1's sources).
    pub paths: Vec<PathBuf>,
    pub budget: Budget,
    /// Ask git whether project files are in its history.
    pub git_history: bool,
    /// Ask `gh auth token` for a token the GitHub CLI keeps in the keychain.
    pub gh_cli: bool,
    pub exposure: Exposure,
    pub caller: Caller,
    /// Set to stop the scan.
    pub stop: Option<Arc<AtomicBool>>,
}

impl ScanOptions {
    /// The whole home folder, the caller from the environment.
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            paths: Vec::new(),
            budget: Budget::default(),
            git_history: true,
            gh_cli: true,
            exposure: Exposure::default(),
            caller: Caller::from_env(),
            stop: None,
        }
    }
}

/// `~/…` for a path under `home`.
#[must_use]
pub fn shown(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// A finding's stable id: its kind and a hash of its file.
#[must_use]
pub fn finding_id(source: Source, path: &Path) -> String {
    let mut h = Sha256::new();
    h.update(source.id().as_bytes());
    h.update([0]);
    h.update(path.to_string_lossy().as_bytes());
    let d = h.finalize();
    format!("{}-{}", source.id(), data_encoding::HEXLOWER.encode(&d[..6]))
}

/// Whether `name` is a dotenv file the scanner reads.
#[must_use]
pub fn is_dotenv(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower == ".env" || lower.starts_with(".env."))
        && !DOTENV_TEMPLATES.contains(&lower.as_str())
        && Path::new(&lower).extension().is_none_or(|e| e != "example")
}

/// Runs `cmd` for at most `timeout`; its stdout when it succeeded.
fn run_for(mut cmd: std::process::Command, timeout: Duration) -> Option<Zeroizing<String>> {
    use std::io::Read as _;
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = Zeroizing::new(String::new());
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                child.kill().ok();
                child.wait().ok();
                return None;
            }
        }
    }
}

/// The repository `path` is in (the nearest folder above it with `.git`), up to `stop`.
fn repo_root(path: &Path, stop: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        if dir == stop {
            return None;
        }
        dir = dir.parent()?;
    }
}

/// The latest commit that has `file` (or `tracked` when it is tracked and has none yet).
fn git_history(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let mut log = std::process::Command::new("git");
    log.arg("-C").arg(root).args(["log", "--all", "--max-count=1", "--format=%h", "--"]).arg(rel);
    if let Some(out) = run_for(log, TOOL_TIMEOUT).filter(|o| !o.trim().is_empty()) {
        return Some(out.trim().to_owned());
    }
    let mut ls = std::process::Command::new("git");
    ls.arg("-C").arg(root).args(["ls-files", "--error-unmatch", "--"]).arg(rel);
    run_for(ls, TOOL_TIMEOUT).map(|_| "tracked".to_owned())
}

/// The scan's working state.
struct Scan<'a> {
    opts: &'a ScanOptions,
    home_real: PathBuf,
    started: Instant,
    entries: usize,
    report: ScanReport,
    progress: Progress,
    tell: &'a mut dyn FnMut(&Progress),
    /// Files already read (canonical), so a project found twice is read once.
    seen: BTreeSet<PathBuf>,
    #[cfg(unix)]
    uid: u32,
}

impl Scan<'_> {
    fn stopped(&self) -> bool {
        self.opts.stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed))
    }

    /// Whether the walk must end (and says why, once).
    fn over_budget(&mut self, at: &Path) -> bool {
        if self.report.budget.is_some() || self.stopped() {
            return true;
        }
        let limit = if self.started.elapsed() > self.opts.budget.time {
            "time"
        } else if self.entries > self.opts.budget.entries {
            "entries"
        } else if self.report.findings.len() >= self.opts.budget.findings {
            "findings"
        } else {
            return false;
        };
        self.report.budget = Some(BudgetHit {
            limit: limit.to_owned(),
            at: shown(at, &self.opts.home),
        });
        true
    }

    fn skip(&mut self, path: &Path, reason: SkipReason) {
        self.report.skipped_total += 1;
        if self.report.skipped.len() < MAX_SKIPPED_LISTED {
            self.report.skipped.push(Skipped {
                path: shown(path, &self.opts.home),
                reason,
            });
        }
    }

    fn at(&mut self, path: &Path, note: &str) {
        self.progress.at = format!("{}{note}", shown(path, &self.opts.home));
        (self.tell)(&self.progress);
    }

    fn stage(&mut self, stage: Stage, now: StageState) {
        self.progress.stages.insert(stage, now);
        let done = self.progress.stages.values().filter(|s| matches!(s, StageState::Done(_))).count();
        #[allow(clippy::cast_precision_loss, reason = "six stages")]
        let fraction = done as f32 / Stage::ALL.len() as f32;
        self.progress.fraction = fraction;
        (self.tell)(&self.progress);
    }

    /// Whether `meta` belongs to another user.
    #[cfg(unix)]
    fn other_user(&self, meta: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        meta.uid() != self.uid
    }

    #[cfg(not(unix))]
    #[allow(clippy::unused_self, reason = "the same call on every system")]
    fn other_user(&self, _meta: &std::fs::Metadata) -> bool {
        false
    }

    /// Where a symbolic link may lead: inside the home folder, or inside `root` (a folder given to scan).
    fn inside(&self, target: &Path, root: Option<&Path>) -> bool {
        target.starts_with(&self.home_real) || root.is_some_and(|r| target.starts_with(r))
    }

    /// Reads a file for the parsers, applying the walk rules; `None` (and a note in `skipped`) when it may not.
    fn read(&mut self, path: &Path, root: Option<&Path>) -> Option<(Zeroizing<String>, std::fs::Metadata)> {
        let link = std::fs::symlink_metadata(path).ok()?;
        let real = if link.file_type().is_symlink() {
            match std::fs::canonicalize(path) {
                Ok(t) if self.inside(&t, root) => t,
                Ok(_) => {
                    self.skip(path, SkipReason::SymlinkOutside);
                    return None;
                }
                Err(_) => return None,
            }
        } else {
            path.to_path_buf()
        };
        let meta = std::fs::metadata(&real).ok()?;
        if !meta.is_file() {
            return None;
        }
        if self.other_user(&meta) {
            self.skip(path, SkipReason::OtherUser);
            return None;
        }
        if meta.len() > MAX_FILE {
            self.skip(path, SkipReason::TooBig);
            return None;
        }
        if !self.seen.insert(std::fs::canonicalize(&real).unwrap_or(real.clone())) {
            return None;
        }
        let Ok(bytes) = std::fs::read(&real).map(Zeroizing::new) else {
            self.skip(path, SkipReason::Unreadable);
            return None;
        };
        if bytes.iter().take(8_192).any(|&b| b == 0) {
            self.skip(path, SkipReason::Binary);
            return None;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            self.skip(path, SkipReason::Binary);
            return None;
        };
        Some((Zeroizing::new(text.to_owned()), meta))
    }

    /// Walks `dir` for project files. `project`: folders left to walk inside a project; `None`: still looking for
    /// projects (`depth` folders under the home folder so far).
    fn walk(&mut self, dir: &Path, depth: usize, project: Option<usize>, root: Option<&Path>, out: &mut Vec<PathBuf>) {
        if self.over_budget(dir) {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let is_repo = entries.iter().any(|e| e.file_name() == ".git");
        let project = project.or(is_repo.then_some(PROJECT_DEPTH));
        for e in entries {
            self.entries += 1;
            if self.entries.is_multiple_of(64) {
                self.at(&e.path(), "");
            }
            if self.over_budget(&e.path()) {
                return;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(kind) = e.file_type() else {
                continue;
            };
            let path = e.path();
            if kind.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) {
                    self.at(&path, "/… (skipped)");
                    continue;
                }
                if name.starts_with('.') {
                    continue;
                }
                if let Ok(meta) = e.metadata()
                    && self.other_user(&meta)
                {
                    self.skip(&path, SkipReason::OtherUser);
                    continue;
                }
                match project {
                    Some(left) if left > 1 => self.walk(&path, depth + 1, Some(left - 1), root, out),
                    None if depth < DISCOVER_DEPTH => self.walk(&path, depth + 1, None, root, out),
                    _ => {}
                }
            } else if project.is_some()
                && (kind.is_file() || kind.is_symlink())
                && (is_dotenv(&name) || name == ".npmrc")
            {
                out.push(path);
            } else if kind.is_symlink() && project.is_some() {
                // A link to a folder is not followed (it may lead anywhere, or in a circle).
                if std::fs::canonicalize(&path).is_ok_and(|t| t.is_dir() && !self.inside(&t, root)) {
                    self.skip(&path, SkipReason::SymlinkOutside);
                }
            }
        }
    }

    /// The project files under the usual folders and the git repositories found under the home folder.
    fn project_files(&mut self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if self.opts.paths.is_empty() {
            let home = self.opts.home.clone();
            for name in PROJECT_ROOTS {
                let dir = home.join(name);
                if dir.is_dir() && !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
                    self.walk(&dir, 1, Some(PROJECT_DEPTH), None, &mut out);
                }
            }
            // Repositories elsewhere: not the folders above again (their files are only read once anyway).
            let walked: Vec<PathBuf> = PROJECT_ROOTS.iter().map(|n| home.join(n)).collect();
            if let Ok(read) = std::fs::read_dir(&home) {
                let mut dirs: Vec<PathBuf> = read
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path())
                    .filter(|p| !walked.contains(p))
                    .filter(|p| {
                        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str())
                    })
                    .collect();
                dirs.sort();
                for dir in dirs {
                    self.walk(&dir, 1, None, None, &mut out);
                }
            }
        } else {
            for p in self.opts.paths.clone() {
                let root = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                if root.is_dir() {
                    self.walk(&root, 0, Some(PROJECT_DEPTH), Some(&root), &mut out);
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Turns a file's entries into a finding (none when nothing in it is worth a look).
    fn finding(&mut self, source: Source, path: &Path, entries: Vec<RawEntry>, meta: Option<&std::fs::Metadata>) {
        let id = finding_id(source, path);
        let mut items: Vec<Item> = Vec::new();
        for e in entries {
            let verdict = detect::classify(&e.name, &e.value, e.credential);
            // A dotenv file keeps its settings (shown unticked); elsewhere only what may be a secret.
            if verdict.confidence == Confidence::None && source != Source::Dotenv {
                continue;
            }
            let mut item_id = match &e.group {
                Some(g) => format!("{id}/{g}/{}", e.name),
                None => format!("{id}/{}", e.name),
            };
            if items.iter().any(|i| i.id == item_id) {
                item_id = format!("{item_id}@{}", e.line);
            }
            items.push(Item {
                id: item_id,
                preview: detect::mask(&e.value),
                name: e.name,
                group: e.group,
                line: e.line,
                detector: verdict.detector,
                label: verdict.label().to_owned(),
                confidence: verdict.confidence,
                suggested: verdict.confidence.suggested(),
            });
        }
        if !items.iter().any(|i| i.confidence > Confidence::None) {
            return;
        }
        let mut f = self.blank(source, path, id, meta);
        f.items = items;
        self.finish(f);
    }

    fn blank(&self, source: Source, path: &Path, id: String, meta: Option<&std::fs::Metadata>) -> Finding {
        let home = &self.opts.home;
        let mut risks = Vec::new();
        if self.opts.exposure.readable(path) {
            risks.push(Risk::ReadableByAgents);
        }
        if meta.is_some_and(world_readable) {
            risks.push(Risk::WorldReadable);
        }
        if !matches!(source, Source::SshKey) {
            risks.push(Risk::PlainText);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = path.parent().map(|p| shown(p, home)).unwrap_or_default();
        Finding {
            id,
            source,
            group: Group::of(source),
            title: source.label().to_owned(),
            path: shown(path, home),
            place: if matches!(source, Source::Dotenv | Source::SshKey) {
                folder
            } else {
                shown(path, home)
            },
            abs_path: path.to_path_buf(),
            project: None,
            project_root: None,
            git_commit: None,
            key_type: None,
            summary: String::new(),
            items: Vec::new(),
            risks,
            suggested: false,
        }
        .titled(&name)
    }

    fn finish(&mut self, mut f: Finding) {
        f.risks.sort();
        f.risks.dedup();
        f.suggested = f.items.iter().any(|i| i.suggested);
        f.summary = summary(&f);
        self.progress.found += f.secrets();
        self.report.findings.push(f);
    }

    /// A project's file: its project and whether git has it.
    fn project_file(&mut self, path: &Path) {
        let Some((text, meta)) = self.read(path, None) else {
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (source, entries) = if name == ".npmrc" {
            (Source::Npm, sources::npmrc(&text))
        } else {
            (Source::Dotenv, sources::dotenv(&text))
        };
        drop(text);
        let before = self.report.findings.len();
        self.finding(source, path, entries, Some(&meta));
        if self.report.findings.len() == before {
            return;
        }
        let stop = self.opts.paths.first().map_or_else(|| self.opts.home.clone(), Clone::clone);
        let root = repo_root(path, &stop).or_else(|| path.parent().map(Path::to_path_buf));
        let commit = match &root {
            Some(r) if self.opts.git_history && r.join(".git").exists() => git_history(r, path),
            _ => None,
        };
        let Some(f) = self.report.findings.last_mut() else {
            return;
        };
        if let Some(root) = root {
            let project = root.file_name().map_or_else(|| "project".to_owned(), |n| n.to_string_lossy().into_owned());
            let rel =
                path.strip_prefix(&root).map_or_else(|_| name.clone(), |r| r.to_string_lossy().replace('\\', "/"));
            f.title = format!("{project}/{rel}");
            f.project = Some(project);
            f.project_root = Some(root);
        }
        if let Some(c) = commit {
            f.risks.push(Risk::InGitHistory);
            f.risks.sort();
            f.git_commit = Some(c);
        }
    }

    /// A file at a known place, parsed by `parse`.
    fn known(&mut self, source: Source, path: &Path, parse: fn(&str) -> Vec<RawEntry>) {
        if let Some((text, meta)) = self.read(path, None) {
            let entries = parse(&text);
            drop(text);
            self.finding(source, path, entries, Some(&meta));
        }
    }

    fn ssh_keys(&mut self) {
        let dir = self.opts.home.join(".ssh");
        let Ok(read) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pub"))
                || matches!(
                    name.as_str(),
                    "config" | "known_hosts" | "known_hosts.old" | "authorized_keys" | "environment"
                )
            {
                continue;
            }
            let Some((text, meta)) = self.read(&path, None) else {
                continue;
            };
            let Some(key) = sources::ssh_key(&text) else {
                continue;
            };
            drop(text);
            let id = finding_id(Source::SshKey, &path);
            let mut f = self.blank(Source::SshKey, &path, id.clone(), Some(&meta));
            if !key.encrypted {
                f.risks.push(Risk::NoPassphrase);
            }
            let preview = key.public.as_deref().map_or_else(
                || format!("{} key", key.kind),
                |p| format!("{} {}", short_kind(&key.kind), crate::ssh_agent::keys::fingerprint(p)),
            );
            f.items.push(Item {
                id: format!("{id}/private_key"),
                name: "private key".to_owned(),
                group: None,
                line: 1,
                detector: Some(Detector::PrivateKey),
                label: if key.encrypted {
                    "SSH key with a passphrase".to_owned()
                } else {
                    "SSH key".to_owned()
                },
                confidence: Confidence::High,
                preview,
                suggested: true,
            });
            f.key_type = Some(key.kind);
            self.finish(f);
        }
    }

    fn github(&mut self) {
        let home = self.opts.home.clone();
        let hosts = home.join(".config/gh/hosts.yml");
        if let Some((text, meta)) = self.read(&hosts, None) {
            let (entries, keychain) = sources::gh_hosts(&text);
            drop(text);
            let before = self.report.findings.len();
            self.finding(Source::GhCli, &hosts, entries, Some(&meta));
            for (host, line) in keychain {
                let id = finding_id(Source::GhCli, &hosts);
                // The token is in the keychain: `gh auth token` says what it is (masked), if allowed.
                let token = if self.opts.gh_cli {
                    let mut cmd = std::process::Command::new("gh");
                    cmd.args(["auth", "token", "--hostname", &host]);
                    run_for(cmd, TOOL_TIMEOUT).filter(|t| !t.trim().is_empty())
                } else {
                    None
                };
                let item = Item {
                    id: format!("{id}/{host}/keychain"),
                    name: "oauth_token".to_owned(),
                    group: Some(host.clone()),
                    line: u32::try_from(line).unwrap_or(1),
                    detector: token.as_ref().and_then(|t| detect::classify("token", t.trim(), true).detector),
                    label: "GitHub token".to_owned(),
                    confidence: Confidence::High,
                    preview: token.as_ref().map_or_else(|| "in the keychain".to_owned(), |t| detect::mask(t.trim())),
                    suggested: true,
                };
                drop(token);
                if let Some(f) = self.report.findings.get_mut(before) {
                    f.items.push(item);
                } else {
                    let mut f = self.blank(Source::GhCli, &hosts, id, Some(&meta));
                    f.risks.retain(|r| *r != Risk::PlainText);
                    f.risks.push(Risk::InKeychain);
                    f.items.push(item);
                    self.finish(f);
                }
            }
            if let Some(f) = self.report.findings.get_mut(before) {
                f.summary = summary(f);
            }
        }
        let mut credential_files = vec![home.join(".git-credentials"), home.join(".config/git/credentials")];
        for config in [home.join(".gitconfig"), home.join(".config/git/config")] {
            if let Ok(text) = std::fs::read_to_string(&config) {
                for f in sources::credential_store_files(&text) {
                    credential_files.push(match f.strip_prefix("~/") {
                        Some(rest) => home.join(rest),
                        None => PathBuf::from(f),
                    });
                }
            }
        }
        credential_files.dedup();
        for path in credential_files {
            self.known(Source::GitCredentials, &path, sources::git_credentials);
        }
        self.known(Source::Netrc, &home.join(".netrc"), sources::netrc);
        self.known(Source::Netrc, &home.join("_netrc"), sources::netrc);
    }

    fn cloud(&mut self) {
        let home = self.opts.home.clone();
        self.known(Source::Aws, &home.join(".aws/credentials"), sources::aws_credentials);
        for adc in [
            home.join(".config/gcloud/application_default_credentials.json"),
            home.join("AppData/Roaming/gcloud/application_default_credentials.json"),
        ] {
            self.known(Source::Gcp, &adc, sources::gcp_adc);
        }
        let kube = home.join(".kube/config");
        if let Some((text, meta)) = self.read(&kube, None) {
            let (entries, exec) = sources::kubeconfig(&text);
            drop(text);
            if !exec.is_empty() {
                self.skip(&kube, SkipReason::Helper);
            }
            self.finding(Source::Kube, &kube, entries, Some(&meta));
        }
        let docker = home.join(".docker/config.json");
        if let Some((text, meta)) = self.read(&docker, None) {
            let (entries, helpers) = sources::docker_config(&text);
            drop(text);
            if !helpers.is_empty() {
                self.skip(&docker, SkipReason::Helper);
            }
            self.finding(Source::Docker, &docker, entries, Some(&meta));
        }
    }

    fn registries(&mut self) {
        let home = self.opts.home.clone();
        self.known(Source::Npm, &home.join(".npmrc"), sources::npmrc);
        self.known(Source::PyPi, &home.join(".pypirc"), sources::pypirc);
        self.known(Source::Cargo, &home.join(".cargo/credentials.toml"), sources::cargo_credentials);
        self.known(Source::Cargo, &home.join(".cargo/credentials"), sources::cargo_credentials);
    }

    fn shell(&mut self) {
        let home = self.opts.home.clone();
        for name in
            [".zshrc", ".zshenv", ".zprofile", ".bashrc", ".bash_profile", ".profile", ".config/fish/config.fish"]
        {
            let path = home.join(name);
            let Some((text, meta)) = self.read(&path, None) else {
                continue;
            };
            // Only exports that are secrets: a shell file is full of settings.
            let entries: Vec<RawEntry> = sources::shell(&text)
                .into_iter()
                .filter(|e| detect::classify(&e.name, &e.value, false).confidence >= Confidence::Medium)
                .collect();
            drop(text);
            self.finding(Source::Shell, &path, entries, Some(&meta));
        }
    }

    /// Runs one stage, telling when it starts and how many it found.
    fn run_stage(&mut self, stage: Stage, f: impl FnOnce(&mut Self)) {
        if self.stopped() {
            return;
        }
        self.stage(stage, StageState::Running);
        let before: usize = self.report.findings.iter().map(Finding::secrets).sum();
        f(self);
        let after: usize = self.report.findings.iter().map(Finding::secrets).sum();
        self.stage(stage, StageState::Done(after - before));
    }
}

impl Finding {
    /// The title a key or a known file gets (a project file's is set once its project is known).
    fn titled(mut self, file_name: &str) -> Self {
        if matches!(self.source, Source::SshKey | Source::Dotenv) {
            file_name.clone_into(&mut self.title);
        }
        self
    }
}

#[cfg(unix)]
fn world_readable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o004 != 0
}

#[cfg(not(unix))]
fn world_readable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// `ED25519` from `ssh-ed25519`.
fn short_kind(kind: &str) -> String {
    kind.trim_start_matches("ssh-").trim_start_matches("ecdsa-sha2-").to_ascii_uppercase()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The review's right column for a finding.
fn summary(f: &Finding) -> String {
    let groups: Vec<&str> = {
        let mut g: Vec<&str> = f.items.iter().filter_map(|i| i.group.as_deref()).collect();
        g.dedup();
        g
    };
    let names = |items: &[Item]| -> String {
        let suggested: Vec<&str> = items.iter().filter(|i| i.suggested).map(|i| i.name.as_str()).collect();
        match suggested.as_slice() {
            [] => plural(items.len(), "value", "values"),
            [one] => (*one).to_owned(),
            [first, rest @ ..] => format!("{first}, +{}", rest.len()),
        }
    };
    let list = |unit: &str, units: &str| match groups.as_slice() {
        [] => plural(f.items.len(), unit, units),
        [a] => (*a).to_owned(),
        [a, b] => format!("{a}, {b}"),
        more => plural(more.len(), unit, units),
    };
    match f.source {
        Source::Dotenv => plural(f.items.len(), "key", "keys"),
        Source::SshKey => "SSH key".to_owned(),
        Source::Shell => names(&f.items),
        Source::GitCredentials | Source::Netrc => plural(groups.len().max(1), "host", "hosts"),
        Source::Aws => list("profile", "profiles"),
        Source::Kube => plural(groups.len().max(1), "user", "users"),
        Source::GhCli | Source::Npm | Source::PyPi | Source::Cargo | Source::Docker | Source::Gcp => {
            list("registry", "registries")
        }
    }
}

/// Reads a finding's file again and parses it as the scan did: the entries with their values, for import.
pub fn read_entries(source: Source, path: &Path) -> Result<Vec<RawEntry>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE {
        return Err(format!("{}: larger than 1 MB", path.display()));
    }
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let text = std::str::from_utf8(&bytes).map_err(|_| format!("{}: not text", path.display()))?;
    Ok(match source {
        Source::Dotenv => sources::dotenv(text),
        Source::Shell => sources::shell(text),
        Source::GhCli => sources::gh_hosts(text).0,
        Source::GitCredentials => sources::git_credentials(text),
        Source::Npm => sources::npmrc(text),
        Source::PyPi => sources::pypirc(text),
        Source::Cargo => sources::cargo_credentials(text),
        Source::Docker => sources::docker_config(text).0,
        Source::Aws => sources::aws_credentials(text),
        Source::Gcp => sources::gcp_adc(text),
        Source::Kube => sources::kubeconfig(text).0,
        Source::Netrc => sources::netrc(text),
        Source::SshKey => vec![RawEntry {
            name: "private key".to_owned(),
            group: None,
            line: 1,
            value: Zeroizing::new(text.to_owned()),
            credential: true,
        }],
    })
}

/// Scans as `opts` says, telling `progress` as it goes. Refused when an AI agent started it ([`Caller`]).
pub fn scan(opts: &ScanOptions, progress: &mut dyn FnMut(&Progress)) -> Result<ScanReport, String> {
    opts.caller.check("A secrets scan")?;
    let started = Instant::now();
    let mut s = Scan {
        opts,
        home_real: std::fs::canonicalize(&opts.home).unwrap_or_else(|_| opts.home.clone()),
        started,
        entries: 0,
        report: ScanReport {
            version: SCHEMA_VERSION,
            started_at: crate::now_unix(),
            home: opts.home.to_string_lossy().into_owned(),
            ..ScanReport::default()
        },
        progress: Progress {
            stages: Stage::ALL.iter().map(|s| (*s, StageState::Waiting)).collect(),
            ..Progress::default()
        },
        tell: progress,
        seen: BTreeSet::new(),
        #[cfg(unix)]
        uid: rustix::process::geteuid().as_raw(),
    };
    s.run_stage(Stage::Projects, |s| {
        // What the walk found before it stopped is still read (it is cheap); only a stop or the findings' limit ends it.
        for file in s.project_files() {
            if s.stopped() || (s.report.findings.len() >= opts.budget.findings && s.over_budget(&file)) {
                break;
            }
            s.project_file(&file);
        }
    });
    if opts.paths.is_empty() {
        s.run_stage(Stage::Ssh, Scan::ssh_keys);
        s.run_stage(Stage::Github, Scan::github);
        s.run_stage(Stage::Cloud, Scan::cloud);
        s.run_stage(Stage::Registries, Scan::registries);
        s.run_stage(Stage::Shell, Scan::shell);
    }
    let mut report = s.report;
    report.stopped = opts.stop.as_ref().is_some_and(|x| x.load(Ordering::Relaxed));
    report.took = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

#[cfg(test)]
pub(crate) mod tests;
