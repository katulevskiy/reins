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

pub mod real;
pub mod receipt;
mod rewrite;

use std::collections::{BTreeMap, BTreeSet};
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

/// The longest project or environment name ([`reins_proto::desktop::name_ok`]).
const MAX_NAME: usize = 64;

/// `base` then `suffix`, `base` cut so that the whole is a name ([`reins_proto::desktop::name_ok`]: at most
/// [`MAX_NAME`] lowercase letters, digits and `-`, not ending in `-`).
fn named(base: &str, suffix: &str) -> String {
    let room = MAX_NAME.saturating_sub(suffix.len());
    let cut: String = base.chars().take(room).collect();
    format!("{}{suffix}", cut.trim_end_matches('-'))
}

/// Four hex characters of `what`'s SHA-256: tells apart names that would otherwise be the same.
fn short_hash(what: &str) -> String {
    use sha2::Digest as _;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(what.as_bytes())[..2])
}

/// The project of this computer's own credentials: `machine-<computer>-<4 hex of this computer's key fingerprint>`,
/// the name lowercased to letters, digits and `-` (project and environment names never hold a `/`), the hash so that
/// two computers with the same name stay apart: "Dana's MacBook" → `machine-danas-macbook-3f9a`. The window shows
/// the friendly name.
#[must_use]
pub fn machine_project(computer: &str, key_fingerprint: &str) -> String {
    let name = slug(&computer.replace(['\'', '’'], ""));
    let name = if name.is_empty() {
        "computer".to_owned()
    } else {
        name
    };
    named(&format!("machine-{name}"), &format!("-{}", short_hash(key_fingerprint)))
}

/// This computer's machine project, from its name and its key (made the first time, as pairing does).
pub fn this_machine(paths: &crate::config::Paths, computer: &str) -> Result<String, String> {
    let identity = crate::identity::Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string())?;
    Ok(machine_project(computer, &identity.fingerprint()))
}

/// The project a repository's files go to before any clash is settled: `owner-repo` from its git origin
/// (`github.com/acme/api` → `acme-api`), else its folder's name.
#[must_use]
pub fn base_project(f: &Finding) -> String {
    let from_origin = f.origin.as_deref().map(|o| {
        let parts: Vec<&str> = o.split('/').skip(1).collect();
        slug(&parts[parts.len().saturating_sub(2)..].join("-"))
    });
    let name = from_origin.filter(|n| !n.is_empty()).unwrap_or_else(|| slug(f.project.as_deref().unwrap_or_default()));
    name_of(&name, "project")
}

/// `s` as a project or environment name ([`named`]), `fallback` when nothing of it is left.
fn name_of(s: &str, fallback: &str) -> String {
    let n = named(&slug(s), "");
    if n.is_empty() {
        fallback.to_owned()
    } else {
        n
    }
}

/// How specific a dotenv file is, for the same environment: the more specific file's value wins (`.env` <
/// `.env.<env>` < `.env.local` < `.env.<env>.local`), as dotenv tools read them.
#[must_use]
pub fn dotenv_rank(file_name: &str) -> u8 {
    let rest = file_name.trim_start_matches(".env").trim_start_matches('.').to_ascii_lowercase();
    let local = rest.rsplit_once('.').is_some_and(|(_, last)| last == "local");
    match (rest.is_empty(), rest == "local", local) {
        (true, ..) => 0,
        (_, true, _) => 2,
        (_, _, true) => 3,
        _ => 1,
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
        other => name_of(other, DEFAULT_ENV),
    }
}

/// The machine environment and variable a credential goes to (`aws`, profile `prod`, `aws_secret_access_key` →
/// `aws-prod`, `AWS_SECRET_ACCESS_KEY`).
#[must_use]
pub fn machine_var(source: Source, group: Option<&str>, name: &str) -> (String, String) {
    let g = group.unwrap_or_default();
    match source {
        Source::Aws => (name_of(&format!("aws-{g}"), "aws"), var_name(name)),
        Source::Npm => {
            let env = if g == "registry.npmjs.org" {
                "npm".to_owned()
            } else {
                name_of(&format!("npm-{g}"), "npm")
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
                name_of(&format!("pypi-{g}"), "pypi")
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
    /// Earlier settings of the same name in the same file: this one's value is what goes, and they leave the file
    /// with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
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
    /// The project's git origin (`github.com/acme/api`), kept with the environment (`reins.origin`) and in
    /// `reins.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl EnvTarget {
    /// The environment as Reins Secrets names it (project and env names are always valid here: [`named`]).
    fn env_ref(&self) -> Option<crate::secrets::EnvRef> {
        crate::secrets::EnvRef::new(&self.project, &self.env).ok()
    }

    /// The vault folder: `Reins/<project>`.
    #[must_use]
    pub fn folder(&self) -> String {
        self.env_ref().map_or_else(|| format!("Reins/{}", self.project), |e| e.folder())
    }

    /// The vault item: `<project>/<env>`.
    #[must_use]
    pub fn item(&self) -> String {
        format!("{}/{}", self.project, self.env)
    }

    /// `vault:<project>/<env>/<var>`.
    #[must_use]
    pub fn reference(&self, var: &str) -> String {
        self.env_ref().map_or_else(|| format!("vault:{}/{}/{var}", self.project, self.env), |e| e.var(var).to_string())
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

/// A project already in the vault, and the origin it was imported from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultProject {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// What the vault already has, as far as this computer can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Existing {
    Known(Vec<VaultProject>),
    /// The vault's projects could not be listed (why): nothing is merged unless chosen, and new projects get a hash
    /// of their origin to stay apart.
    Unknown(String),
}

/// A project of this import whose name the vault already has, from another folder (another origin, or none).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clash {
    /// The name it would have had.
    pub name: String,
    /// The name it gets: a new one (`api-2`, `personal-api`), or `name` when merging was chosen.
    pub chosen: String,
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The vault's project's origin; `None` with `known` false: the vault could not be asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_origin: Option<String>,
    pub known: bool,
}

impl Clash {
    #[must_use]
    pub fn merged(&self) -> bool {
        self.chosen == self.name
    }

    /// What the review says about it.
    #[must_use]
    pub fn line(&self) -> String {
        match (self.known, self.merged()) {
            (true, true) => format!("{} already exists from another folder: merging into it, as chosen", self.name),
            (true, false) => {
                format!("{} already exists from another folder: this one becomes {}", self.name, self.chosen)
            }
            (false, true) => {
                format!("{}: the vault's projects could not be checked; merging into it, as chosen", self.name)
            }
            (false, false) => format!(
                "{}: the vault's projects could not be checked, so this one becomes {} to stay apart",
                self.name, self.chosen
            ),
        }
    }
}

/// A variable two selected files give the same environment: the more specific file's goes ([`dotenv_rank`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// `<project>/<env>`.
    pub item: String,
    pub name: String,
    /// The finding whose value goes, and the one whose value does not.
    pub kept: String,
    pub dropped: String,
    /// Both are in one file (`kept` and `dropped` say which lines).
    #[serde(default)]
    pub same_file: bool,
}

impl Conflict {
    #[must_use]
    pub fn line(&self) -> String {
        if self.same_file {
            format!(
                "{} is set twice ({}, {}): the last one goes to {}, and both leave the file",
                self.name, self.dropped, self.kept, self.item
            )
        } else {
            format!(
                "{} is in both {} and {}: {}'s goes to {} (the more specific file wins)",
                self.name, self.dropped, self.kept, self.kept, self.item
            )
        }
    }
}

/// Where each selected secret goes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub computer: String,
    pub envs: Vec<EnvTarget>,
    pub ssh: Vec<SshTarget>,
    pub tokens: Vec<TokenTarget>,
    /// Projects whose name the vault already has from elsewhere.
    #[serde(default)]
    pub clashes: Vec<Clash>,
    /// Variables two files give the same environment.
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
}

/// What a plan is made with besides the scan: this computer's names, what the vault has, and the projects the person
/// chose to merge into anyway.
#[derive(Clone, Debug)]
pub struct Names<'a> {
    /// The friendly name ("Dana's MacBook").
    pub computer: &'a str,
    /// Its machine project ([`machine_project`]).
    pub machine: &'a str,
    pub existing: &'a Existing,
    /// Project names to merge into although the vault's project came from elsewhere.
    pub merge: &'a BTreeSet<String>,
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

/// A name for `root`'s project that `taken` does not have: its parent folder's name in front (`personal-api`), else
/// a number (`api-2`).
fn another_name(name: &str, root: &Path, taken: &dyn Fn(&str) -> bool) -> String {
    let parent = root.parent().and_then(Path::file_name).map(|p| slug(&p.to_string_lossy())).unwrap_or_default();
    let with_parent = named(&format!("{parent}-{name}"), "");
    if !parent.is_empty() && !taken(&with_parent) {
        return with_parent;
    }
    (2..10_000).map(|n| named(name, &format!("-{n}"))).find(|n| !taken(n)).unwrap_or_else(|| named(name, "-x"))
}

/// Each repository's project (by its folder), and where it came from.
type Projects = BTreeMap<PathBuf, (String, Option<String>)>;

/// The project of each repository in the selection (by its folder), and the clashes with the vault: names from the
/// git origin (else the folder), apart from each other, and never merged into a vault project from elsewhere unless
/// the person chose to.
fn project_names(findings: &[&Finding], names: &Names<'_>) -> (Projects, Vec<Clash>) {
    let mut roots: Vec<(PathBuf, String, Option<String>)> = findings
        .iter()
        .filter(|f| f.source == Source::Dotenv)
        .filter_map(|f| Some((f.project_root.clone()?, base_project(f), f.origin.clone())))
        .collect();
    roots.sort();
    roots.dedup_by(|a, b| a.0 == b.0);
    let mut out: Projects = BTreeMap::new();
    let mut clashes = Vec::new();
    for (root, base, origin) in roots {
        // Apart from the other repositories of this import (unless they share an origin: the same project).
        let same_here =
            |n: &str, out: &Projects| out.values().any(|(m, o)| m == n && (origin.is_none() || *o != origin));
        let mut name = base.clone();
        if same_here(&name, &out) {
            name = another_name(&base, &root, &|n| same_here(n, &out));
        }
        // Apart from the vault's projects from elsewhere.
        let in_vault = |n: &str| match names.existing {
            Existing::Known(list) => list.iter().any(|p| p.name == n),
            Existing::Unknown(_) => false,
        };
        let clash = match names.existing {
            Existing::Known(list) => list
                .iter()
                .find(|p| p.name == name)
                .filter(|p| origin.is_none() || p.origin != origin)
                .map(|p| (true, p.origin.clone())),
            Existing::Unknown(_) => Some((false, None)),
        };
        if let Some((known, existing_origin)) = clash {
            let chosen = if names.merge.contains(&name) {
                name.clone()
            } else if known {
                another_name(&name, &root, &|n| in_vault(n) || same_here(n, &out))
            } else {
                let what = origin.clone().unwrap_or_else(|| root.to_string_lossy().into_owned());
                named(&name, &format!("-{}", short_hash(&what)))
            };
            clashes.push(Clash {
                name: name.clone(),
                chosen: chosen.clone(),
                root: root.clone(),
                origin: origin.clone(),
                existing_origin,
                known,
            });
            name = chosen;
        }
        out.insert(root, (name, origin));
    }
    (out, clashes)
}

/// Groups the selected items of `report` by where they go.
#[must_use]
pub fn plan(report: &ScanReport, selected: &BTreeSet<String>, names: &Names<'_>) -> Plan {
    let computer = names.computer;
    let mut plan = Plan {
        computer: computer.to_owned(),
        ..Plan::default()
    };
    let chosen: Vec<&Finding> =
        report.findings.iter().filter(|f| f.items.iter().any(|i| selected.contains(&i.id))).collect();
    let (projects, clashes) = project_names(&chosen, names);
    plan.clashes = clashes;
    let machine = names.machine.to_owned();
    // The less specific file first: the more specific one's value then wins ([`dotenv_rank`]).
    let mut ordered = chosen.clone();
    ordered.sort_by_key(|f| {
        let dotenv_order = if f.source == Source::Dotenv {
            f.abs_path.file_name().map_or(0, |n| dotenv_rank(&n.to_string_lossy()))
        } else {
            0
        };
        (f.project_root.clone(), dotenv_order, f.abs_path.clone())
    });
    let title_of = |id: &str| report.finding(id).map_or_else(|| id.to_owned(), |f| f.title.clone());
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
                    let (project, env, var, root, origin) = if source == Source::Dotenv {
                        let root = f.project_root.clone().or_else(|| f.abs_path.parent().map(Path::to_path_buf));
                        let (project, origin) = root
                            .as_ref()
                            .and_then(|r| projects.get(r).cloned())
                            .unwrap_or_else(|| (base_project(f), f.origin.clone()));
                        let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        (project, env_for(&file), var_name(&i.name), root, origin)
                    } else {
                        let (env, var) = machine_var(source, i.group.as_deref(), &i.name);
                        (machine.clone(), env, var, None, None)
                    };
                    let at = plan.envs.iter().position(|e| e.project == project && e.env == env).unwrap_or_else(|| {
                        plan.envs.push(EnvTarget {
                            project,
                            env,
                            vars: Vec::new(),
                            project_root: root,
                            origin,
                        });
                        plan.envs.len() - 1
                    });
                    let target = &mut plan.envs[at];
                    let target_item = target.item();
                    let var = PlannedVar {
                        name: var,
                        item: i.id.clone(),
                        finding: f.id.clone(),
                        covers: Vec::new(),
                    };
                    match target.vars.iter_mut().find(|v| v.name == var.name) {
                        Some(v) => {
                            // Never silently: the review says which value goes.
                            let mut var = var;
                            if v.finding == var.finding {
                                // Set twice in one file: the last one is what dotenv readers use, and the first
                                // leaves the file with it.
                                let line_of = |id: &str| f.items.iter().find(|i| i.id == id).map_or(0, |i| i.line);
                                plan.conflicts.push(Conflict {
                                    item: target_item,
                                    name: var.name.clone(),
                                    kept: format!("{} line {}", f.title, i.line),
                                    dropped: format!("{} line {}", f.title, line_of(&v.item)),
                                    same_file: true,
                                });
                                var.covers = std::mem::take(&mut v.covers);
                                var.covers.push(v.item.clone());
                            } else {
                                plan.conflicts.push(Conflict {
                                    item: target_item,
                                    name: var.name.clone(),
                                    kept: title_of(&var.finding),
                                    dropped: title_of(&v.finding),
                                    same_file: false,
                                });
                            }
                            *v = var;
                        }
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
    /// The project's git origin, kept on the environment's item as the hidden field `reins.origin`, so a later
    /// import from another folder is not merged into it unasked.
    pub origin: Option<String>,
}

/// An SSH private key, to store as an SSH key item (always in the vault folder `Reins/ssh`).
pub struct SshWrite {
    pub name: String,
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
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SshWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshWrite").field("name", &self.name).finish_non_exhaustive()
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

    /// The four check digits of the whole batch, as the phone computes them for the store
    /// ([`reins_proto::desktop::batch_check`]): the same on both screens, or something changed the batch on the way.
    #[must_use]
    pub fn check(&self) -> String {
        crate::secrets_client::import_check(self)
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
    /// The Reins projects in the vault, with the origin each was imported from (`reins.origin`): names only, never a
    /// value. An error when the vault cannot be listed.
    async fn projects(&self) -> Result<Vec<VaultProject>, String>;
}

/// What the vault has, for [`plan`]: its projects, or why they could not be listed.
pub async fn existing(writer: &dyn VaultWriter) -> Existing {
    match writer.projects().await {
        Ok(list) => Existing::Known(list),
        Err(why) => Existing::Unknown(why),
    }
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
    /// This computer's friendly name, and its machine project ([`this_machine`]).
    pub computer: String,
    pub machine: String,
    pub caller: Caller,
    /// Projects the person chose to merge into although the vault's came from elsewhere (see [`Clash`]).
    pub merge: BTreeSet<String>,
}

impl Context {
    /// The plan of `selected` against what the vault has now.
    pub async fn plan(&self, report: &ScanReport, selected: &BTreeSet<String>, writer: &dyn VaultWriter) -> Plan {
        let existing = existing(writer).await;
        plan(report, selected, &self.names(&existing))
    }

    #[must_use]
    pub fn names<'a>(&'a self, existing: &'a Existing) -> Names<'a> {
        Names {
            computer: &self.computer,
            machine: &self.machine,
            existing,
            merge: &self.merge,
        }
    }
}

/// The text of each selected finding's file (by finding id), read once: the values sent come from it, and the rewrite
/// edits exactly it (and only while the file still holds it).
pub(crate) type Texts = BTreeMap<String, Zeroizing<String>>;

/// Reads the selected values again from their files (the scan kept none), and keeps each file's text. A file that
/// changed since the scan is an error: scan again.
fn read_values(
    report: &ScanReport,
    selected: &BTreeSet<String>,
    machine: &dyn Machine,
) -> Result<(BTreeMap<String, Zeroizing<String>>, Texts), String> {
    let mut out = BTreeMap::new();
    let mut texts = Texts::new();
    for f in report.findings.iter().filter(|f| f.items.iter().any(|i| selected.contains(&i.id))) {
        let text = secrets_scan::read_text(&f.abs_path)?;
        let entries = secrets_scan::parse_entries(f.source, &text);
        texts.insert(f.id.clone(), text);
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
                    // A key file and a `.env` value go as written (quotes may hold spaces on purpose).
                    let v = if matches!(f.source, Source::SshKey | Source::Dotenv) {
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
    Ok((out, texts))
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
                origin: e.origin.clone(),
            })
            .collect(),
        ssh: plan
            .ssh
            .iter()
            .map(|s| SshWrite {
                name: s.name.clone(),
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
    let plan = ctx.plan(report, selected, writer).await;
    if plan.count() == 0 {
        return Err("nothing selected to move".to_owned());
    }
    let (mut values, texts) = read_values(report, selected, machine)?;
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
    receipt.notes.extend(plan.clashes.iter().map(Clash::line));
    receipt.notes.extend(plan.conflicts.iter().map(Conflict::line));
    // Which items arrived.
    let mut arrived: BTreeSet<String> = BTreeSet::new();
    for (target, result) in plan.envs.iter().zip(&stored.envs) {
        match result {
            Ok(()) => {
                for v in &target.vars {
                    for item in std::iter::once(&v.item).chain(&v.covers) {
                        arrived.insert(item.clone());
                        receipt.moved.push(Moved {
                            item: item.clone(),
                            finding: v.finding.clone(),
                            to: target.reference(&v.name),
                            on_disk: true,
                            in_trash: false,
                        });
                    }
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
                    in_trash: false,
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
                    in_trash: false,
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
        rewrite::Gone::default()
    } else {
        rewrite::all(report, selected, &arrived, &plan, &texts, machine, &mut receipt).await
    };
    drop(texts);
    // What the vault has but this computer still holds too: where it was, or in the Trash until shredded.
    for m in &mut receipt.moved {
        m.in_trash = off_disk.trashed.contains(&m.item);
        m.on_disk = !off_disk.off.contains(&m.item) || m.in_trash;
    }
    if receipt.moved.iter().any(|m| m.in_trash) {
        receipt.notes.push(
            "The original files are in the Trash, still in plain text: empty the Trash, or Shred them once all works."
                .to_owned(),
        );
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
