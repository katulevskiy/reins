//! Moving what a scan found into the vault on the phone (design `docs/secrets.md`, 5.3):
//!
//! 1. **Group** the selection ([`plan`]): a project's `.env` files into its environments (the project is the
//!    repository's folder name; `.env` and `.env.local` → `development`, `.env.production` → `production`), the
//!    computer's own credentials into `machine-<computer>` environments (`aws-default`, `npm`, `pypi`, …), SSH keys
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
pub mod receipt;
mod rewrite;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::secrets_scan::{self, Caller, Confidence, Finding, ScanReport, Source};

pub use self::receipt::{Change, Done, Moved, PendingKey, RECEIPT_VERSION, Receipt};
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

/// The project of this computer's own credentials: `machine-<computer>`, the name lowercased to letters, digits and
/// `-` (project and environment names never hold a `/`): "Dana's MacBook" → `machine-danas-macbook`.
#[must_use]
pub fn machine_project(computer: &str) -> String {
    let name = slug(&computer.replace(['\'', '’'], ""));
    if name.is_empty() {
        "machine-computer".to_owned()
    } else {
        format!("machine-{name}")
    }
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
    /// `acme-web`, or `machine-<computer>`.
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
    let machine = machine_project(computer);
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
    /// Moves a file to the trash; the trash's own id for it, where the trash keeps one.
    fn trash(&self, path: &Path) -> Result<Option<String>, String>;
    /// Puts back the trash's item `trash_id`, whose original place was `path` (never over a file there).
    fn restore(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String>;
    /// Deletes the trash's item `trash_id` (from `path`) for good.
    fn purge(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String>;
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
                        on_disk: true,
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
                    on_disk: true,
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
                    on_disk: true,
                });
            }
            Err(e) => receipt.notes.push(format!("the token for {} was not stored: {e}", target.host)),
        }
    }
    let off_disk = if keep_local {
        if !arrived.is_empty() {
            receipt
                .notes
                .push("Local copies were kept: the vault has a backup of each, and they are still on disk.".to_owned());
        }
        BTreeSet::new()
    } else {
        rewrite::all(report, selected, &arrived, &plan, machine, &mut receipt).await
    };
    // What the vault has but this computer still holds too.
    for m in &mut receipt.moved {
        m.on_disk = !off_disk.contains(&m.item);
    }
    let path = if receipt.moved.is_empty() && receipt.changes.is_empty() {
        None
    } else {
        Some(receipt::save(&ctx.state_dir, &receipt)?)
    };
    Ok((receipt, path))
}

#[cfg(test)]
mod tests;
