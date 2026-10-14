//! Step 3 of import: each source whose selected secrets all reached the vault stops holding them (design
//! `docs/secrets.md`, 5.3). A file is never changed in place: the original goes to the trash first, then the new
//! one is written, and the receipt says so ([`Change::Replaced`]), so undo puts the original back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::formats::{self, EnvLine, EnvSettings};
use super::{Change, DEFAULT_ENV, Done, Machine, PendingKey, Plan, Receipt, env_for, var_name};
use crate::secrets_scan::{self, Confidence, Finding, ScanReport, Source, sources};

/// Most variables in one run profile (`reins run`'s limit for one release).
const PROFILE_MAX: usize = crate::secrets::MAX_SECRETS;

/// Writes a new file (it must not exist).
fn create(path: &Path, text: &str, private: bool, receipt: &mut Receipt) -> Result<(), String> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if private {
            0o600
        } else {
            0o644
        });
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    receipt.changes.push(Change::Created {
        path: path.to_path_buf(),
    });
    Ok(())
}

/// Sends `path` to the trash and writes `text` in its place, with the original's permissions.
fn replace(path: &Path, text: &str, machine: &dyn Machine, receipt: &mut Receipt) -> Result<(), String> {
    let private = private_file(path);
    machine.trash(path)?;
    receipt.changes.push(Change::Trashed {
        path: path.to_path_buf(),
    });
    create(path, text, private, receipt)?;
    // One change for both: undo sends the new file to the trash and brings the original back.
    receipt.changes.truncate(receipt.changes.len() - 2);
    receipt.changes.push(Change::Replaced {
        path: path.to_path_buf(),
    });
    Ok(())
}

/// Whether others cannot read the file (so its replacement must not be readable either).
fn private_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).map_or(true, |m| m.permissions().mode() & 0o044 == 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn trash(path: &Path, machine: &dyn Machine, receipt: &mut Receipt) -> Result<(), String> {
    machine.trash(path)?;
    receipt.changes.push(Change::Trashed {
        path: path.to_path_buf(),
    });
    Ok(())
}

/// `text` with lines changed: by 1-based number, `None` drops the line, `Some` replaces it.
fn edit_lines(text: &str, edits: &BTreeMap<u32, Option<String>>) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.lines().enumerate() {
        match edits.get(&u32::try_from(n + 1).unwrap_or(u32::MAX)) {
            Some(None) => {}
            Some(Some(new)) => {
                out.push_str(new);
                out.push('\n');
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if !text.ends_with('\n') && out.ends_with('\n') && !text.is_empty() {
        out.pop();
    }
    out
}

fn note(receipt: &mut Receipt, r: Result<(), String>) {
    if let Err(e) = r {
        receipt.notes.push(e);
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Runs the step for every finding whose selected items all arrived.
pub(super) async fn all(
    report: &ScanReport,
    selected: &BTreeSet<String>,
    arrived: &BTreeSet<String>,
    plan: &Plan,
    machine: &dyn Machine,
    receipt: &mut Receipt,
) {
    let mut ready: Vec<&Finding> = Vec::new();
    for f in &report.findings {
        let chosen: Vec<&str> = f.items.iter().filter(|i| selected.contains(&i.id)).map(|i| i.id.as_str()).collect();
        if chosen.is_empty() || !chosen.iter().any(|i| arrived.contains(*i)) {
            continue;
        }
        if chosen.iter().all(|i| arrived.contains(*i)) {
            ready.push(f);
        } else {
            receipt.notes.push(format!("{} was left as it is: not all of its secrets reached your vault.", f.path));
        }
    }
    dotenv(&ready, selected, plan, machine, receipt);
    ssh(&ready, machine, receipt).await;
    git(&ready, selected, plan, machine, receipt);
    machine_files(&ready, selected, plan, machine, receipt);
}

/// `.env` files: per project, `reins.toml` (when missing) and `.env.reins` (the development variables' references
/// and its plain settings); each file goes to the trash, or keeps only what was not moved and may be a secret.
fn dotenv(ready: &[&Finding], selected: &BTreeSet<String>, plan: &Plan, machine: &dyn Machine, receipt: &mut Receipt) {
    let mut by_root: BTreeMap<PathBuf, Vec<&Finding>> = BTreeMap::new();
    for f in ready.iter().filter(|f| f.source == Source::Dotenv) {
        let root = f.project_root.clone().or_else(|| f.abs_path.parent().map(Path::to_path_buf)).unwrap_or_default();
        by_root.entry(root).or_default().push(f);
    }
    let mut files = 0;
    let mut in_history = Vec::new();
    for (root, mut found) in by_root {
        // `.env` before `.env.local`: the later file's line wins in `.env.reins`.
        found.sort_by_key(|f| (f.abs_path.file_name().map(std::ffi::OsStr::len), f.abs_path.clone()));
        let envs: Vec<&super::EnvTarget> =
            plan.envs.iter().filter(|e| e.project_root.as_deref() == Some(root.as_path())).collect();
        let Some(project) = envs.first().map(|e| e.project.clone()) else {
            continue;
        };
        let toml = root.join("reins.toml");
        if !toml.exists() {
            let mut names: Vec<&str> = envs.iter().map(|e| e.env.as_str()).collect();
            names.sort_by_key(|n| (*n != DEFAULT_ENV, *n));
            names.dedup();
            let settings: Vec<EnvSettings> = names.iter().map(|n| EnvSettings::for_env(n)).collect();
            let r = create(&toml, &formats::reins_toml(&project, DEFAULT_ENV, &settings), false, receipt);
            note(receipt, r);
        }
        let mut lines: Vec<EnvLine> = Vec::new();
        for f in found {
            let file_name = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let env = env_for(&file_name);
            let entries = match secrets_scan::read_entries(Source::Dotenv, &f.abs_path) {
                Ok(e) => e,
                Err(e) => {
                    receipt.notes.push(e);
                    continue;
                }
            };
            let ids = secrets_scan::item_ids(&f.id, &entries);
            let mut kept: Vec<EnvLine> = Vec::new();
            for (e, id) in entries.iter().zip(&ids) {
                let item = f.items.iter().find(|i| &i.id == id);
                let moved = selected.contains(id);
                let secretish = item.is_some_and(|i| i.confidence > Confidence::None);
                let literal = EnvLine::Literal {
                    name: e.name.clone(),
                    value: e.value.clone(),
                };
                let line = if moved {
                    Some(EnvLine::Reference {
                        name: e.name.clone(),
                        reference: formats::reference(&project, &env, &var_name(&e.name)),
                    })
                } else if secretish || env != DEFAULT_ENV {
                    kept.push(literal);
                    None
                } else {
                    Some(literal)
                };
                if env == DEFAULT_ENV
                    && let Some(line) = line
                {
                    lines.retain(|l| l.name() != line.name());
                    lines.push(line);
                }
            }
            let r = if kept.is_empty() {
                trash(&f.abs_path, machine, receipt)
            } else {
                receipt.notes.push(format!(
                    "{}: {} not moved stay in it.",
                    f.path,
                    plural(kept.len(), "variable", "variables")
                ));
                replace(&f.abs_path, &formats::env_reins(&kept), machine, receipt)
            };
            files += usize::from(r.is_ok());
            note(receipt, r);
            if f.git_commit.is_some() {
                in_history.push(f.title.clone());
            }
        }
        if !lines.is_empty() {
            let path = root.join(".env.reins");
            let r = match std::fs::read_to_string(&path) {
                Ok(existing) => replace(&path, &formats::merge_env_reins(&existing, &lines), machine, receipt),
                Err(_) => create(&path, &formats::env_reins(&lines), false, receipt),
            };
            note(receipt, r);
        }
    }
    if files > 0 {
        receipt.done.push(Done {
            source: Source::Dotenv,
            what: format!("{} → .env.reins", plural(files, ".env file", ".env files")),
            how: "reins run -- <your command>".to_owned(),
            ok: true,
        });
    }
    for title in in_history {
        receipt.notes.push(format!(
            "{title} is in git history: moving it does not unpublish it. Rotate its keys (make new ones at each service)."
        ));
    }
}

/// SSH keys: ssh asks the Reins agent; once the agent signed with the moved key, the private key file goes to the
/// trash (the `.pub` stays).
async fn ssh(ready: &[&Finding], machine: &dyn Machine, receipt: &mut Receipt) {
    let keys: Vec<&&Finding> = ready.iter().filter(|f| f.source == Source::SshKey).collect();
    if keys.is_empty() {
        return;
    }
    match machine.ssh_setup() {
        Ok(true) => receipt.changes.push(Change::SshSetup),
        Ok(false) => {}
        Err(e) => receipt.notes.push(format!("ssh was not pointed at the Reins agent: {e}")),
    }
    for f in keys {
        let public =
            std::fs::read_to_string(&f.abs_path).ok().and_then(|t| sources::ssh_key(&t)).and_then(|k| k.public);
        let Some(public) = public else {
            receipt.notes.push(format!(
                "{} stays: its public key is not in the file, so the agent's signature cannot be checked.",
                f.path
            ));
            continue;
        };
        let (ok, how) = match machine.agent_signs_with(&public).await {
            Ok(()) => match trash(&f.abs_path, machine, receipt) {
                Ok(()) => (true, "signs on your phone; the file is in the trash".to_owned()),
                Err(e) => (false, e),
            },
            Err(why) => {
                receipt.pending_keys.push(PendingKey {
                    path: f.abs_path.clone(),
                    public_key: data_encoding::BASE64.encode(&public),
                });
                (false, format!("kept until the agent signs with it ({why}): reins secrets import --finish"))
            }
        };
        receipt.done.push(Done {
            source: Source::SshKey,
            what: format!("{} → the Reins SSH agent", f.title),
            how,
            ok,
        });
    }
}

/// GitHub CLI and git credentials: git for the host goes through Reins, and the token leaves the file (or the
/// keychain: `gh auth logout`).
fn git(ready: &[&Finding], selected: &BTreeSet<String>, plan: &Plan, machine: &dyn Machine, receipt: &mut Receipt) {
    let found: Vec<&&Finding> =
        ready.iter().filter(|f| matches!(f.source, Source::GhCli | Source::GitCredentials)).collect();
    if found.is_empty() {
        return;
    }
    let hosts: BTreeSet<&str> = plan
        .tokens
        .iter()
        .filter(|t| found.iter().any(|f| f.id == t.finding || t.covers.iter().any(|c| c.starts_with(&f.id))))
        .map(|t| t.host.as_str())
        .collect();
    for host in &hosts {
        match machine.git_through_reins(host) {
            Ok(()) => receipt.changes.push(Change::GitHost {
                host: (*host).to_owned(),
            }),
            Err(e) => receipt.notes.push(format!("git for {host} does not go through Reins yet: {e}")),
        }
    }
    let mut ok = true;
    for f in &found {
        let mut edits: BTreeMap<u32, Option<String>> = BTreeMap::new();
        for item in f.items.iter().filter(|i| selected.contains(&i.id)) {
            if item.id.ends_with("/keychain") {
                let host = item.group.clone().unwrap_or_default();
                match machine.gh_logout(&host) {
                    Ok(()) => receipt.changes.push(Change::GhLogout {
                        host,
                    }),
                    Err(e) => {
                        ok = false;
                        receipt.notes.push(format!("the GitHub CLI is still signed in to {host}: {e}"));
                    }
                }
            } else {
                edits.insert(item.line, None);
            }
        }
        if !edits.is_empty() {
            let r = std::fs::read_to_string(&f.abs_path)
                .map_err(|e| format!("{}: {e}", f.path))
                .and_then(|text| replace(&f.abs_path, &edit_lines(&text, &edits), machine, receipt));
            ok &= r.is_ok();
            note(receipt, r);
        }
    }
    receipt.done.push(Done {
        source: Source::GhCli,
        what: format!("gh and git → git through Reins ({})", hosts.into_iter().collect::<Vec<_>>().join(", ")),
        how: "pushes ask your phone; no token on disk".to_owned(),
        ok,
    });
}

/// The computer's own credentials: run profiles for the tools that read variables (AWS, npm, PyPI, cargo, shell
/// exports) and the token out of their files; the tools that read only their own file keep it.
fn machine_files(
    ready: &[&Finding],
    selected: &BTreeSet<String>,
    plan: &Plan,
    machine: &dyn Machine,
    receipt: &mut Receipt,
) {
    for f in ready {
        let profiles = match f.source {
            Source::Aws | Source::Npm | Source::PyPi | Source::Cargo | Source::Shell => {
                add_profiles(f, plan, machine, receipt)
            }
            Source::Docker | Source::Gcp | Source::Kube | Source::Netrc => {
                receipt.done.push(Done {
                    source: f.source,
                    what: format!("{} → your vault", f.title),
                    how: format!("{} kept: the tool reads only that file; your vault has a copy", f.path),
                    ok: false,
                });
                continue;
            }
            Source::Dotenv | Source::SshKey | Source::GhCli | Source::GitCredentials => continue,
        };
        let Some(profiles) = profiles else {
            continue;
        };
        let text = match std::fs::read_to_string(&f.abs_path) {
            Ok(t) => t,
            Err(e) => {
                receipt.notes.push(format!("{}: {e}", f.path));
                continue;
            }
        };
        let mut edits: BTreeMap<u32, Option<String>> = BTreeMap::new();
        let lines: Vec<&str> = text.lines().collect();
        for item in f.items.iter().filter(|i| selected.contains(&i.id)) {
            let edit = match f.source {
                Source::Npm => {
                    let (_, var) = super::machine_var(f.source, item.group.as_deref(), &item.name);
                    lines
                        .get(item.line as usize - 1)
                        .and_then(|l| l.split_once('='))
                        .map(|(key, _)| format!("{}=${{{var}}}", key.trim_end()))
                }
                Source::Shell => Some(format!(
                    "# {} is in your vault now (Reins): reins run --profile {} -- <command>",
                    item.name,
                    profiles.first().map_or("shell", String::as_str)
                )),
                _ => None,
            };
            edits.insert(item.line, edit);
        }
        let r = replace(&f.abs_path, &edit_lines(&text, &edits), machine, receipt);
        let ok = r.is_ok();
        note(receipt, r);
        let how = match f.source {
            Source::Npm => format!("reins run --profile {} -- npm publish", profiles[0]),
            Source::PyPi => format!("reins run --profile {} -- twine upload dist/*", profiles[0]),
            Source::Cargo => format!("reins run --profile {} -- cargo publish", profiles[0]),
            _ => format!("reins run --profile {} -- <command>", profiles.last().map_or("", String::as_str)),
        };
        let names: Vec<&str> = profiles.iter().map(String::as_str).collect();
        receipt.done.push(Done {
            source: f.source,
            what: format!(
                "{} → run {} {}",
                f.source.label().split_whitespace().next().unwrap_or_default(),
                if names.len() == 1 {
                    "profile"
                } else {
                    "profiles"
                },
                names.join(", ")
            ),
            how,
            ok,
        });
    }
}

/// `[run.profiles.<env>]` for the machine environments of `f`'s moved variables; their names.
fn add_profiles(f: &Finding, plan: &Plan, machine: &dyn Machine, receipt: &mut Receipt) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for env in plan.envs.iter().filter(|e| e.project_root.is_none()) {
        let vars: Vec<&super::PlannedVar> = env.vars.iter().filter(|v| v.finding == f.id).collect();
        for (n, chunk) in vars.chunks(PROFILE_MAX).enumerate() {
            let name = if n == 0 {
                env.env.clone()
            } else {
                format!("{}-{}", env.env, n + 1)
            };
            let map: BTreeMap<String, String> =
                chunk.iter().map(|v| (v.name.clone(), env.reference(&v.name))).collect();
            match machine.add_profile(&name, &map, &format!("{} from {}", env.env, f.path)) {
                Ok(()) => {
                    receipt.changes.push(Change::Profile {
                        name: name.clone(),
                    });
                    names.push(name);
                }
                Err(e) => {
                    receipt
                        .notes
                        .push(format!("{} stays as it is: the run profile {name} was not added ({e})", f.path));
                    return None;
                }
            }
        }
    }
    (!names.is_empty()).then_some(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_dropped_or_replaced_by_number() {
        let edits: BTreeMap<u32, Option<String>> = [(2, None), (3, Some("x=${T}".to_owned()))].into_iter().collect();
        assert_eq!(edit_lines("a\nb\nc\nd\n", &edits), "a\nx=${T}\nd\n");
        assert_eq!(edit_lines("a\nb", &BTreeMap::from([(1, None)])), "b");
    }
}
