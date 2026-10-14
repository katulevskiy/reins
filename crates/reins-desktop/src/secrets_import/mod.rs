//! Moving what a scan found into the vault on the phone (design `docs/secrets.md`, 5.3):
//!
//! 1. **Group** the selection ([`plan`]): a project's `.env` files into its environments (the project is the
//!    repository's folder name; `.env` and `.env.local` → `development`, `.env.production` → `production`), the
//!    computer's own credentials into `machine/<computer>` environments (`aws-default`, `npm`, `pypi`, …), SSH keys
//!    into `Reins/ssh`, GitHub and git tokens into the phone's git integration.
//! 2. **Store** them as one batch ([`VaultWriter`]): one approval on the phone, one set of check digits.
//! 3. **Rewrite** each source whose secrets all arrived, unless the person keeps local copies ([`rewrite`]): a `.env`
//!    becomes `reins.toml` and `.env.reins` (references only) and goes to the trash; an SSH key goes to the trash once
//!    the Reins agent signed with it; tokens leave their files for run profiles; shell exports are commented out.
//! 4. **Record** every change in a receipt (`<state>/imports/<time>.json`), which [`undo`] follows backwards: files
//!    come back from the trash. Nothing is ever deleted.
//!
//! Values are read again from the files (the scan kept none), held in [`Zeroizing`] buffers, and never printed:
//! the types that hold them have a [`Debug`] that leaves them out and are never serialized.

pub mod formats;
pub mod real;
mod rewrite;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::secrets_scan::{self, Caller, Confidence, Finding, ScanReport, Source};

/// The receipt's format.
pub const RECEIPT_VERSION: u32 = 1;
/// The environment a project's plain `.env` goes to.
pub const DEFAULT_ENV: &str = "development";

/// What the review's ticks select: item ids. A finding's id stands for its pre-ticked items.
pub fn select(report: &ScanReport, ids: &[String], all_suggested: bool) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    if all_suggested {
        out.extend(report.findings.iter().flat_map(|f| f.suggested_items().map(|i| i.id.clone())));
    }
    for id in ids.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
        if let Some(f) = report.finding(id) {
            let picked: Vec<String> = f.suggested_items().map(|i| i.id.clone()).collect();
            if picked.is_empty() {
                out.extend(f.items.iter().filter(|i| i.confidence > Confidence::None).map(|i| i.id.clone()));
            } else {
                out.extend(picked);
            }
        } else if report.findings.iter().any(|f| f.items.iter().any(|i| i.id == id)) {
            out.insert(id.to_owned());
        } else {
            return Err(format!("{id} is not in the scan (ids are shown by `reins secrets scan`)"));
        }
    }
    Ok(out)
}

pub use crate::secrets_scan::computer_name;

/// An environment variable's name made from anything (`aws_access_key_id` → `AWS_ACCESS_KEY_ID`, `ghcr.io` →
/// `GHCR_IO`).
#[must_use]
pub fn var_name(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_owned();
    if out.starts_with(|c: char| c.is_ascii_digit()) || out.is_empty() {
        format!("V_{out}")
    } else {
        out
    }
}

/// A part of a name made from anything (`registry.npmjs.org` → `registry-npmjs-org`).
fn slug(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    out.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// The environment a dotenv file's variables go to, from its name.
#[must_use]
pub fn env_for(file_name: &str) -> String {
    let rest = file_name.trim_start_matches(".env").trim_start_matches('.');
    let rest = rest.strip_suffix(".local").unwrap_or(rest);
    match rest {
        "" | "local" | "development" | "dev" => DEFAULT_ENV.to_owned(),
        "production" | "prod" => "production".to_owned(),
        "test" | "testing" => "test".to_owned(),
        other => slug(other),
    }
}

/// The machine environment and variable a credential goes to (`aws`, profile `prod`, `aws_secret_access_key` →
/// `aws-prod`, `AWS_SECRET_ACCESS_KEY`).
#[must_use]
pub fn machine_var(source: Source, group: Option<&str>, name: &str) -> (String, String) {
    let g = group.unwrap_or_default();
    match source {
        Source::Aws => (format!("aws-{}", slug(g)), var_name(name)),
        Source::Npm => {
            let env = if g == "registry.npmjs.org" {
                "npm".to_owned()
            } else {
                format!("npm-{}", slug(g))
            };
            let var = match name {
                "_auth" => "NPM_AUTH",
                "_password" => "NPM_PASSWORD",
                _ => "NPM_TOKEN",
            };
            (env, var.to_owned())
        }
        Source::PyPi => (
            if g == "pypi" {
                "pypi".to_owned()
            } else {
                format!("pypi-{}", slug(g))
            },
            "TWINE_PASSWORD".to_owned(),
        ),
        Source::Cargo => (
            "cargo".to_owned(),
            if g == "crates.io" {
                "CARGO_REGISTRY_TOKEN".to_owned()
            } else {
                format!("CARGO_REGISTRIES_{}_TOKEN", var_name(g))
            },
        ),
        Source::Docker => ("docker".to_owned(), format!("DOCKER_{}_{}", var_name(g), var_name(name))),
        Source::Gcp => ("gcloud".to_owned(), format!("GCLOUD_{}", var_name(name))),
        Source::Kube => ("kube".to_owned(), format!("KUBE_{}_{}", var_name(g), var_name(name))),
        Source::Netrc => ("netrc".to_owned(), format!("NETRC_{}", var_name(g))),
        Source::Shell => ("shell".to_owned(), var_name(name)),
        Source::Dotenv | Source::SshKey | Source::GhCli | Source::GitCredentials => (slug(source.id()), var_name(name)),
    }
}

/// One variable of an environment: where it comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedVar {
    /// Its name in the vault (the environment variable's name).
    pub name: String,
    pub item: String,
    pub finding: String,
}

/// An environment the batch stores (`vault_secrets_store`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvTarget {
    /// `acme-web`, or `machine/<computer>`.
    pub project: String,
    pub env: String,
    pub vars: Vec<PlannedVar>,
    /// A project's environment (else the computer's own).
    pub project_root: Option<PathBuf>,
}

impl EnvTarget {
    /// The vault folder: `Reins/<project>`.
    #[must_use]
    pub fn folder(&self) -> String {
        format!("Reins/{}", self.project)
    }

    /// The vault item: `<project>/<env>`.
    #[must_use]
    pub fn item(&self) -> String {
        format!("{}/{}", self.project, self.env)
    }

    #[must_use]
    pub fn reference(&self, var: &str) -> String {
        formats::reference(&self.project, &self.env, var)
    }
}

/// An SSH key the batch stores (`vault_secret_store`, kind ssh, folder `Reins/ssh`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    /// The item's name: the file's name and this computer's.
    pub name: String,
    pub item: String,
    pub finding: String,
    pub path: PathBuf,
}

/// A git host's token the batch hands to the phone's git integration (`integration_token_store`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTarget {
    pub host: String,
    pub user: Option<String>,
    pub item: String,
    pub finding: String,
    /// The other selected items with a token for the same host (the same token saved twice, another file): they
    /// leave their files when this one arrives.
    pub covers: Vec<String>,
}

/// Where each selected secret goes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub computer: String,
    pub envs: Vec<EnvTarget>,
    pub ssh: Vec<SshTarget>,
    pub tokens: Vec<TokenTarget>,
}

impl Plan {
    /// How many secrets move.
    #[must_use]
    pub fn count(&self) -> usize {
        self.envs.iter().map(|e| e.vars.len()).sum::<usize>() + self.ssh.len() + self.tokens.len()
    }

    /// One line per target, for the terminal and the approval screen ("acme-web/development: 4 variables").
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .envs
            .iter()
            .map(|e| {
                let n = e.vars.len();
                format!(
                    "{}: {n} variable{}",
                    e.item(),
                    if n == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            })
            .collect();
        out.extend(self.ssh.iter().map(|s| format!("Reins/ssh: {}", s.name)));
        out.extend(self.tokens.iter().map(|t| format!("git through Reins: {}", t.host)));
        out
    }
}

/// The project a dotenv finding belongs to, unique per folder: two repositories named alike get their parent's name
/// too.
fn project_names(findings: &[&Finding]) -> BTreeMap<PathBuf, String> {
    let mut out: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut roots: Vec<(&PathBuf, &str)> =
        findings.iter().filter_map(|f| Some((f.project_root.as_ref()?, f.project.as_deref()?))).collect();
    roots.sort();
    roots.dedup();
    for (root, name) in roots {
        let mut chosen = slug(name);
        if chosen.is_empty() {
            "project".clone_into(&mut chosen);
        }
        if out.values().any(|n| *n == chosen) {
            let parent =
                root.parent().and_then(Path::file_name).map(|p| slug(&p.to_string_lossy())).unwrap_or_default();
            chosen = format!("{parent}-{chosen}");
            let mut n = 2;
            while out.values().any(|v| *v == chosen) {
                chosen = format!("{}-{n}", slug(name));
                n += 1;
            }
        }
        out.insert(root.clone(), chosen);
    }
    out
}

/// Groups the selected items of `report` by where they go.
#[must_use]
pub fn plan(report: &ScanReport, selected: &BTreeSet<String>, computer: &str) -> Plan {
    let mut plan = Plan {
        computer: computer.to_owned(),
        ..Plan::default()
    };
    let chosen: Vec<&Finding> =
        report.findings.iter().filter(|f| f.items.iter().any(|i| selected.contains(&i.id))).collect();
    let projects = project_names(&chosen);
    let machine = format!("machine/{computer}");
    // `.env` before `.env.local`: the later file's value wins, as dotenv tools read them.
    let mut ordered = chosen.clone();
    ordered.sort_by_key(|f| {
        let dotenv_order = if f.source == Source::Dotenv {
            f.abs_path.file_name().map_or(0, std::ffi::OsStr::len)
        } else {
            0
        };
        (f.project_root.clone(), dotenv_order, f.abs_path.clone())
    });
    for f in ordered {
        let items = f.items.iter().filter(|i| selected.contains(&i.id));
        match f.source {
            Source::SshKey => {
                let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                for i in items {
                    plan.ssh.push(SshTarget {
                        name: format!("{file} ({computer})"),
                        item: i.id.clone(),
                        finding: f.id.clone(),
                        path: f.abs_path.clone(),
                    });
                }
            }
            Source::GhCli | Source::GitCredentials => {
                for i in items {
                    let host = i.group.clone().unwrap_or_else(|| "github.com".to_owned());
                    // One token per host: the first selected.
                    if let Some(t) = plan.tokens.iter_mut().find(|t| t.host == host) {
                        t.covers.push(i.id.clone());
                        continue;
                    }
                    plan.tokens.push(TokenTarget {
                        host,
                        user: (f.source == Source::GitCredentials).then(|| i.name.clone()),
                        item: i.id.clone(),
                        finding: f.id.clone(),
                        covers: Vec::new(),
                    });
                }
            }
            source => {
                for i in items {
                    let (project, env, var, root) = if source == Source::Dotenv {
                        let root = f.project_root.clone().or_else(|| f.abs_path.parent().map(Path::to_path_buf));
                        let project = root
                            .as_ref()
                            .and_then(|r| projects.get(r).cloned())
                            .unwrap_or_else(|| "project".to_owned());
                        let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        (project, env_for(&file), var_name(&i.name), root)
                    } else {
                        let (env, var) = machine_var(source, i.group.as_deref(), &i.name);
                        (machine.clone(), env, var, None)
                    };
                    let at = plan.envs.iter().position(|e| e.project == project && e.env == env).unwrap_or_else(|| {
                        plan.envs.push(EnvTarget {
                            project,
                            env,
                            vars: Vec::new(),
                            project_root: root,
                        });
                        plan.envs.len() - 1
                    });
                    let target = &mut plan.envs[at];
                    let var = PlannedVar {
                        name: var,
                        item: i.id.clone(),
                        finding: f.id.clone(),
                    };
                    match target.vars.iter_mut().find(|v| v.name == var.name) {
                        Some(v) => *v = var,
                        None => target.vars.push(var),
                    }
                }
            }
        }
    }
    plan
}

/// The variables of one environment, to store as one sealed box.
pub struct EnvWrite {
    pub project: String,
    pub env: String,
    pub vars: Vec<(String, Zeroizing<String>)>,
}

/// An SSH private key, to store as an SSH key item.
pub struct SshWrite {
    pub name: String,
    pub folder: String,
    pub private_key: Zeroizing<String>,
}

/// A git host's token, for the phone's git integration.
pub struct TokenWrite {
    pub host: String,
    pub user: Option<String>,
    pub token: Zeroizing<String>,
}

/// Everything to store, for one approval on the phone.
pub struct Batch {
    pub computer: String,
    pub envs: Vec<EnvWrite>,
    pub ssh: Vec<SshWrite>,
    pub tokens: Vec<TokenWrite>,
}

impl std::fmt::Debug for EnvWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.vars.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("EnvWrite")
            .field("project", &self.project)
            .field("env", &self.env)
            .field("vars", &names)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SshWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshWrite").field("name", &self.name).field("folder", &self.folder).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for TokenWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenWrite").field("host", &self.host).field("user", &self.user).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Batch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Batch")
            .field("computer", &self.computer)
            .field("envs", &self.envs)
            .field("ssh", &self.ssh)
            .field("tokens", &self.tokens)
            .finish()
    }
}

impl Batch {
    #[must_use]
    pub fn count(&self) -> usize {
        self.envs.iter().map(|e| e.vars.len()).sum::<usize>() + self.ssh.len() + self.tokens.len()
    }

    /// The four check digits of the whole batch, shown here and on the phone: over every target and value in order.
    /// (The batch store tool of Reins Secrets core defines the exact form the phone computes; this follows it.)
    #[must_use]
    pub fn check(&self) -> String {
        let mut all = Zeroizing::new(String::new());
        for e in &self.envs {
            for (name, value) in &e.vars {
                let _ = writeln!(all, "env {}/{}/{name}", e.project, e.env);
                all.push_str(value);
                all.push('\n');
            }
        }
        for s in &self.ssh {
            let _ = writeln!(all, "ssh {}", s.name);
            all.push_str(&s.private_key);
            all.push('\n');
        }
        for t in &self.tokens {
            let _ = writeln!(all, "token {}", t.host);
            all.push_str(&t.token);
            all.push('\n');
        }
        reins_proto::desktop::value_check(&all)
    }
}

/// What the phone did with each part of a batch, in the batch's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    pub envs: Vec<Result<(), String>>,
    pub ssh: Vec<Result<(), String>>,
    pub tokens: Vec<Result<(), String>>,
}

/// Stores a batch in the vault on the phone, as Reins Secrets core's tools do it: `vault_secrets_store` per
/// environment, `vault_secret_store` (kind ssh) per SSH key, `integration_token_store` per git host, shown on the
/// phone as one approval.
#[async_trait::async_trait]
pub trait VaultWriter: Send + Sync {
    async fn store(&self, batch: &Batch) -> Stored;
}

/// What import does to this computer besides its files: the trash, the SSH agent, git, gh and the run profiles.
#[async_trait::async_trait]
pub trait Machine: Send + Sync {
    /// Moves a file to the trash.
    fn trash(&self, path: &Path) -> Result<(), String>;
    /// Puts the file last trashed from `path` back.
    fn restore(&self, path: &Path) -> Result<(), String>;
    /// The GitHub CLI's token for `host`, when it keeps it in the keychain.
    fn gh_token(&self, host: &str) -> Option<Zeroizing<String>>;
    /// Whether the Reins SSH agent signs with this key (asks the phone); why not, else.
    async fn agent_signs_with(&self, public_key: &[u8]) -> Result<(), String>;
    /// Points ssh at the Reins agent; whether that changed anything.
    fn ssh_setup(&self) -> Result<bool, String>;
    fn ssh_unsetup(&self) -> Result<(), String>;
    /// Sends git for `host` through Reins.
    fn git_through_reins(&self, host: &str) -> Result<(), String>;
    /// Signs the GitHub CLI out of `host` (its token leaves the keychain).
    fn gh_logout(&self, host: &str) -> Result<(), String>;
    /// Adds `[run.profiles.<name>]` to config.toml (replacing one of that name).
    fn add_profile(&self, name: &str, env: &BTreeMap<String, String>, purpose: &str) -> Result<(), String>;
    fn remove_profile(&self, name: &str) -> Result<(), String>;
}

/// One change to this computer, as the receipt keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// A new file (undo: it goes to the trash).
    Created {
        path: PathBuf,
    },
    /// The file went to the trash and a new one is there now (undo: the new one to the trash, the old one back).
    Replaced {
        path: PathBuf,
    },
    /// The file went to the trash (undo: it comes back).
    Trashed {
        path: PathBuf,
    },
    /// `[run.profiles.<name>]` was added.
    Profile {
        name: String,
    },
    /// ssh was pointed at the Reins agent.
    SshSetup,
    /// git for the host goes through Reins.
    GitHost {
        host: String,
    },
    /// The GitHub CLI was signed out of the host.
    GhLogout {
        host: String,
    },
}

/// A secret that moved, and where to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub item: String,
    pub finding: String,
    /// `vault:acme-web/development/OPENAI_API_KEY`, `Reins/ssh/id_ed25519 (Dana's MacBook)`, `git: github.com`.
    pub to: String,
}

/// What the Done screen lists: one line per kind of thing that changed, and how to use it now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    pub source: Source,
    /// `2 .env files → .env.reins`.
    pub what: String,
    /// `reins run -- <your command>`.
    pub how: String,
    /// It went as planned (else `how` says what is left to do).
    pub ok: bool,
}

/// An SSH key still waiting for the agent's signature before it goes to the trash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingKey {
    pub path: PathBuf,
    /// The public key blob, base64.
    pub public_key: String,
}

/// What an import did: kept as `<state>/imports/<time>.json`, read by `--undo` and `--finish`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub at: i64,
    pub computer: String,
    pub keep_local: bool,
    pub moved: Vec<Moved>,
    pub changes: Vec<Change>,
    pub done: Vec<Done>,
    /// What did not go, and why; what to do about it.
    pub notes: Vec<String>,
    #[serde(default)]
    pub pending_keys: Vec<PendingKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<i64>,
}

/// Where the receipts are.
#[must_use]
pub fn receipts_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("imports")
}

/// Writes `receipt` as a new file in the receipts folder; its path.
pub fn save_receipt(state_dir: &Path, receipt: &Receipt) -> Result<PathBuf, String> {
    let dir = receipts_dir(state_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut path = dir.join(format!("{}.json", receipt.at));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{}-{n}.json", receipt.at));
        n += 1;
    }
    write_receipt(&path, receipt)?;
    Ok(path)
}

fn write_receipt(path: &Path, receipt: &Receipt) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|e| e.to_string())?;
    crate::config::write_private(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A receipt: `last` (the newest), or a file.
pub fn load_receipt(state_dir: &Path, which: &str) -> Result<(PathBuf, Receipt), String> {
    let path = if which == "last" {
        let dir = receipts_dir(state_dir);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|_| "no import to undo".to_owned())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        files.pop().ok_or("no import to undo")?
    } else {
        PathBuf::from(which)
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let receipt: Receipt =
        serde_json::from_str(&text).map_err(|e| format!("{}: not an import receipt ({e})", path.display()))?;
    Ok((path, receipt))
}

/// Where import runs.
#[derive(Clone, Debug)]
pub struct Context {
    pub home: PathBuf,
    pub state_dir: PathBuf,
    pub computer: String,
    pub caller: Caller,
}

/// Reads the selected values again from their files (the scan kept none). A file that changed since the scan is an
/// error: scan again.
fn read_values(
    report: &ScanReport,
    selected: &BTreeSet<String>,
    machine: &dyn Machine,
) -> Result<BTreeMap<String, Zeroizing<String>>, String> {
    let mut out = BTreeMap::new();
    for f in report.findings.iter().filter(|f| f.items.iter().any(|i| selected.contains(&i.id))) {
        let entries = secrets_scan::read_entries(f.source, &f.abs_path)?;
        let ids = secrets_scan::item_ids(&f.id, &entries);
        let mut found: BTreeMap<String, Zeroizing<String>> =
            ids.into_iter().zip(entries).map(|(id, e)| (id, e.value)).collect();
        for item in f.items.iter().filter(|i| selected.contains(&i.id)) {
            let value = if item.id.ends_with("/keychain") {
                machine.gh_token(item.group.as_deref().unwrap_or_default())
            } else {
                found.remove(&item.id)
            };
            match value.filter(|v| !v.trim().is_empty()) {
                Some(v) => {
                    let v = if f.source == Source::SshKey {
                        v
                    } else {
                        Zeroizing::new(v.trim().to_owned())
                    };
                    out.insert(item.id.clone(), v);
                }
                None => {
                    return Err(format!(
                        "{} changed since the scan ({} is not there any more): scan again",
                        f.path, item.name
                    ));
                }
            }
        }
    }
    Ok(out)
}

/// Builds the batch for `plan` from the values read.
fn batch(plan: &Plan, values: &mut BTreeMap<String, Zeroizing<String>>) -> Batch {
    let mut take = |item: &str| values.remove(item).unwrap_or_default();
    Batch {
        computer: plan.computer.clone(),
        envs: plan
            .envs
            .iter()
            .map(|e| EnvWrite {
                project: e.project.clone(),
                env: e.env.clone(),
                vars: e.vars.iter().map(|v| (v.name.clone(), take(&v.item))).collect(),
            })
            .collect(),
        ssh: plan
            .ssh
            .iter()
            .map(|s| SshWrite {
                name: s.name.clone(),
                folder: "Reins/ssh".to_owned(),
                private_key: take(&s.item),
            })
            .collect(),
        tokens: plan
            .tokens
            .iter()
            .map(|t| TokenWrite {
                host: t.host.clone(),
                user: t.user.clone(),
                token: take(&t.item),
            })
            .collect(),
    }
}

/// Moves the `selected` items of `report` into the vault (one batch), then, unless `keep_local`, rewrites their
/// sources; writes the receipt. `confirmed` is called with the plan and the batch's check digits before anything is
/// sent (to show them; an `Err` stops the import).
pub async fn import(
    ctx: &Context,
    report: &ScanReport,
    selected: &BTreeSet<String>,
    keep_local: bool,
    writer: &dyn VaultWriter,
    machine: &dyn Machine,
    confirmed: &mut (dyn FnMut(&Plan, &str) -> Result<(), String> + Send),
) -> Result<(Receipt, Option<PathBuf>), String> {
    ctx.caller.check("Importing secrets")?;
    let plan = plan(report, selected, &ctx.computer);
    if plan.count() == 0 {
        return Err("nothing selected to move".to_owned());
    }
    let mut values = read_values(report, selected, machine)?;
    let batch = batch(&plan, &mut values);
    drop(values);
    confirmed(&plan, &batch.check())?;
    let stored = writer.store(&batch).await;
    drop(batch);
    let mut receipt = Receipt {
        version: RECEIPT_VERSION,
        at: crate::now_unix(),
        computer: ctx.computer.clone(),
        keep_local,
        ..Receipt::default()
    };
    // Which items arrived.
    let mut arrived: BTreeSet<String> = BTreeSet::new();
    for (target, result) in plan.envs.iter().zip(&stored.envs) {
        match result {
            Ok(()) => {
                for v in &target.vars {
                    arrived.insert(v.item.clone());
                    receipt.moved.push(Moved {
                        item: v.item.clone(),
                        finding: v.finding.clone(),
                        to: target.reference(&v.name),
                    });
                }
            }
            Err(e) => receipt.notes.push(format!("{} was not stored: {e}", target.item())),
        }
    }
    for (target, result) in plan.ssh.iter().zip(&stored.ssh) {
        match result {
            Ok(()) => {
                arrived.insert(target.item.clone());
                receipt.moved.push(Moved {
                    item: target.item.clone(),
                    finding: target.finding.clone(),
                    to: format!("Reins/ssh/{}", target.name),
                });
            }
            Err(e) => receipt.notes.push(format!("{} was not stored: {e}", target.name)),
        }
    }
    for (target, result) in plan.tokens.iter().zip(&stored.tokens) {
        match result {
            Ok(()) => {
                arrived.insert(target.item.clone());
                arrived.extend(target.covers.iter().cloned());
                receipt.moved.push(Moved {
                    item: target.item.clone(),
                    finding: target.finding.clone(),
                    to: format!("git: {}", target.host),
                });
            }
            Err(e) => receipt.notes.push(format!("the token for {} was not stored: {e}", target.host)),
        }
    }
    if keep_local {
        if !arrived.is_empty() {
            receipt.notes.push("Local copies were kept: the vault has a backup of each.".to_owned());
        }
    } else {
        rewrite::all(report, selected, &arrived, &plan, machine, &mut receipt).await;
    }
    let path = if receipt.moved.is_empty() && receipt.changes.is_empty() {
        None
    } else {
        Some(save_receipt(&ctx.state_dir, &receipt)?)
    };
    Ok((receipt, path))
}

/// Retries the SSH keys an import kept until the Reins agent signs with them; the receipt is updated.
pub async fn finish(path: &Path, receipt: &mut Receipt, machine: &dyn Machine) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut left = Vec::new();
    for key in std::mem::take(&mut receipt.pending_keys) {
        let blob = data_encoding::BASE64.decode(key.public_key.as_bytes()).map_err(|e| e.to_string())?;
        match machine.agent_signs_with(&blob).await {
            Ok(()) => match machine.trash(&key.path) {
                Ok(()) => {
                    lines.push(format!(
                        "{}: the Reins agent signs with it; the file is in the trash.",
                        key.path.display()
                    ));
                    receipt.changes.push(Change::Trashed {
                        path: key.path.clone(),
                    });
                }
                Err(e) => {
                    lines.push(format!("{}: {e}", key.path.display()));
                    left.push(key);
                }
            },
            Err(why) => {
                lines.push(format!("{}: still kept ({why})", key.path.display()));
                left.push(key);
            }
        }
    }
    receipt.pending_keys = left;
    write_receipt(path, receipt)?;
    if lines.is_empty() {
        lines.push("No SSH key is waiting.".to_owned());
    }
    Ok(lines)
}

/// Brings back the file import replaced at `path`: the new one steps aside, the original comes back from the trash,
/// then the new one goes to the trash (in that order, so the trash gives back the original, not the new one).
fn put_back(path: &Path, machine: &dyn Machine) -> Result<(), String> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".reins-undo");
    let aside = PathBuf::from(aside);
    std::fs::rename(path, &aside).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Err(e) = machine.restore(path) {
        std::fs::rename(&aside, path).ok();
        return Err(e);
    }
    machine.trash(&aside)
}

/// Undoes what the receipt at `path` recorded, last change first: files come back from the trash, files import made
/// go to the trash, profiles go. The secrets stay in the vault.
pub fn undo(path: &Path, receipt: &mut Receipt, machine: &dyn Machine) -> Result<Vec<String>, String> {
    if receipt.undone_at.is_some() {
        return Err("this import was undone already".to_owned());
    }
    let mut lines = Vec::new();
    let mut failed = false;
    for change in receipt.changes.iter().rev() {
        let r = match change {
            Change::Created {
                path,
            } => machine.trash(path).map(|()| format!("{} is in the trash", path.display())),
            Change::Replaced {
                path,
            } => put_back(path, machine).map(|()| format!("{} is back as it was", path.display())),
            Change::Trashed {
                path,
            } => machine.restore(path).map(|()| format!("{} is back", path.display())),
            Change::Profile {
                name,
            } => machine.remove_profile(name).map(|()| format!("run profile {name} removed")),
            Change::SshSetup => machine.ssh_unsetup().map(|()| "ssh uses its own keys again".to_owned()),
            Change::GitHost {
                host,
            } => Ok(format!("git for {host} still goes through Reins (`reins pause` sends it directly)")),
            Change::GhLogout {
                host,
            } => Ok(format!("the GitHub CLI is still signed out of {host}: gh auth login -h {host}")),
        };
        match r {
            Ok(line) => lines.push(line),
            Err(e) => {
                failed = true;
                lines.push(format!("not undone: {e}"));
            }
        }
    }
    if !failed {
        receipt.undone_at = Some(crate::now_unix());
        write_receipt(path, receipt)?;
    }
    lines.push("The secrets stay in your vault; delete them on your phone if you do not want them there.".to_owned());
    Ok(lines)
}

#[cfg(test)]
mod tests;
