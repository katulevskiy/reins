//! `reins secrets ls`, `set`, `unset`, `rename`, `cp`, `push`, `pull`, `diff`: names from the phone (never a value),
//! and changes through one batch store each (one approval on the phone, with the check digits shown here first).

use std::path::{Path, PathBuf};

use reins_proto::desktop::{EnvChanges, EnvListing, ProjectListing};
use zeroize::Zeroizing;

use crate::project::{DEFAULT_ENV, ENV_FILE, EnvFile, EnvValue, Project};
use crate::secrets::{Reference, check_name, check_var};
use crate::vault_writer::{StoreError, StoreReceipt, VaultWriter, check_digits};

/// The vault, as these commands use it.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// The names of one project (or of every one).
    async fn names(&self, project: Option<&str>) -> Result<Vec<ProjectListing>, String>;
    /// One store, one approval.
    async fn store(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError>;
}

#[async_trait::async_trait]
impl Backend for crate::secrets_client::PhoneVaultWriter {
    async fn names(&self, project: Option<&str>) -> Result<Vec<ProjectListing>, String> {
        crate::secrets_client::PhoneVaultWriter::names(self, project).await.map_err(|e| e.to_string())
    }

    async fn store(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError> {
        self.store_envs(envs).await
    }
}

/// The project a command is about: `--project`, else the `reins.toml` at or above the working folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Here {
    pub project: String,
    pub default_env: String,
    /// The folder of its `reins.toml`, when there is one here.
    pub root: Option<PathBuf>,
}

impl Here {
    pub fn find(cwd: &Path, project: Option<&str>) -> Result<Self, String> {
        let found = Project::find(cwd)?;
        match (project, found) {
            (Some(name), found) => {
                check_name("project", name)?;
                let found = found.filter(|p| p.file.name == name);
                Ok(Self {
                    project: name.to_owned(),
                    default_env: found.as_ref().map_or_else(|| DEFAULT_ENV.to_owned(), |p| p.file.default_env.clone()),
                    root: found.map(|p| p.root),
                })
            }
            (None, Some(p)) => Ok(Self {
                project: p.file.name,
                default_env: p.file.default_env,
                root: Some(p.root),
            }),
            (None, None) => {
                Err("no reins.toml here or above: run it in a project (reins secrets init), or give --project"
                    .to_owned())
            }
        }
    }

    /// `ENV` (the default environment when `None`), checked.
    pub fn env(&self, env: Option<&str>) -> Result<String, String> {
        let env = env.unwrap_or(&self.default_env);
        check_name("environment", env)?;
        Ok(env.to_owned())
    }

    /// `ENV/NAME`, or `NAME` in the default environment.
    pub fn target(&self, s: &str) -> Result<(String, String), String> {
        let (env, name) = match s.split_once('/') {
            Some((env, name)) => (Some(env), name),
            None => (None, s),
        };
        check_var(name)?;
        Ok((self.env(env)?, name.to_owned()))
    }

    fn root(&self) -> Result<&Path, String> {
        self.root.as_deref().ok_or_else(|| {
            format!("no reins.toml for {} here: run it in the project's folder (reins secrets init)", self.project)
        })
    }
}

/// `ls`: projects, environments, variable names and versions.
pub async fn ls(backend: &dyn Backend, here: Option<&Here>, what: Option<&str>) -> Result<Vec<String>, String> {
    let (project, env) = match what.map(|w| w.split_once('/').unwrap_or((w, ""))) {
        Some((p, e)) => (Some(p.to_owned()), Some(e).filter(|e| !e.is_empty())),
        None => (here.map(|h| h.project.clone()), None),
    };
    if let Some(p) = &project {
        check_name("project", p)?;
    }
    if let Some(e) = env {
        check_name("environment", e)?;
    }
    let listing = backend.names(project.as_deref()).await?;
    let mut lines = Vec::new();
    for p in &listing {
        lines.push(match &p.origin {
            Some(origin) => format!("{}  {origin}", p.name),
            None => p.name.clone(),
        });
        for e in p.envs.iter().filter(|e| env.is_none_or(|w| w == e.name)) {
            lines.push(format!("  {}  {}  {}", e.name, version(&e.revision), e.variables.join(" ")));
        }
    }
    if lines.is_empty() {
        return Ok(vec![match (&project, env) {
            (Some(p), Some(e)) => format!("No {p}/{e} in the vault."),
            (Some(p), None) => format!("No {p} in the vault."),
            _ => "No projects in the vault.".to_owned(),
        }]);
    }
    if let (Some(p), Some(e)) = (&project, env)
        && !listing.iter().any(|l| l.envs.iter().any(|x| x.name == e))
    {
        lines.push(format!("No {p}/{e} in the vault."));
    }
    Ok(lines)
}

/// A version as shown: the revision date to the minute.
fn version(revision: &str) -> String {
    revision.get(..16).unwrap_or(revision).replace('T', " ")
}

/// Shows the check digits, then stores; a line per environment.
async fn store(backend: &dyn Backend, envs: &[EnvChanges]) -> Result<Vec<String>, String> {
    reins_proto::desktop::validate_store(envs)?;
    eprintln!("Check digits {}: approve on your phone if it shows the same.", check_digits(envs));
    let receipt = backend.store(envs).await.map_err(|e| e.to_string())?;
    Ok(receipt
        .envs
        .iter()
        .map(|e| {
            let mut done = Vec::new();
            for (n, word) in [
                (e.added, "added"),
                (e.replaced, "replaced"),
                (e.removed, "removed"),
                (e.renamed, "renamed"),
                (e.copied, "copied"),
            ] {
                if n > 0 {
                    done.push(format!("{n} {word}"));
                }
            }
            let what = if done.is_empty() {
                "nothing changed".to_owned()
            } else {
                done.join(", ")
            };
            format!(
                "{}/{}: {what}{}",
                e.project,
                e.env,
                if e.created {
                    " (new)"
                } else {
                    ""
                }
            )
        })
        .collect())
}

/// `set ENV/NAME`: the value read without echo, or from stdin.
pub async fn set(
    backend: &dyn Backend,
    here: &Here,
    target: &str,
    value: Zeroizing<String>,
) -> Result<Vec<String>, String> {
    let (env, name) = here.target(target)?;
    let mut change = EnvChanges::new(&here.project, &env);
    change.set = vec![(name, value.to_string())];
    change.replace = true;
    store(backend, std::slice::from_ref(&change)).await
}

/// Reads the value of `set` (not shown on a terminal; piped in otherwise).
pub fn read_value(here: &Here, target: &str) -> Result<Zeroizing<String>, String> {
    let (env, name) = here.target(target)?;
    crate::vault_cli::read_value(&format!("{env}/{name}"), "Value", crate::vault_cli::ItemKind::ApiKey)
}

/// `unset ENV/NAME…`: one store for every environment named.
pub async fn unset(backend: &dyn Backend, here: &Here, targets: &[String]) -> Result<Vec<String>, String> {
    let mut changes: Vec<EnvChanges> = Vec::new();
    for t in targets {
        let (env, name) = here.target(t)?;
        if let Some(c) = changes.iter_mut().find(|c| c.env == env) {
            c.unset.push(name);
        } else {
            let mut c = EnvChanges::new(&here.project, &env);
            c.unset.push(name);
            changes.push(c);
        }
    }
    if changes.is_empty() {
        return Err("name what to remove: reins secrets unset ENV/NAME".to_owned());
    }
    store(backend, &changes).await
}

/// `rename ENV/OLD NEW`.
pub async fn rename(backend: &dyn Backend, here: &Here, from: &str, to: &str) -> Result<Vec<String>, String> {
    let (env, old) = here.target(from)?;
    let new = match to.split_once('/') {
        Some((e, n)) if e == env => n,
        Some(_) => return Err("a rename stays in its environment: reins secrets cp copies to another".to_owned()),
        None => to,
    };
    check_var(new)?;
    let mut change = EnvChanges::new(&here.project, &env);
    change.rename = vec![(old, new.to_owned())];
    store(backend, std::slice::from_ref(&change)).await
}

/// `cp ENV ENV2`: every variable of `ENV` into `ENV2`, copied on the phone (the values never come here).
pub async fn cp(backend: &dyn Backend, here: &Here, from: &str, to: &str) -> Result<Vec<String>, String> {
    let (from, to) = (here.env(Some(from))?, here.env(Some(to))?);
    if from == to {
        return Err("copy to another environment".to_owned());
    }
    let mut change = EnvChanges::new(&here.project, &to);
    change.copy_from = Some(from);
    store(backend, std::slice::from_ref(&change)).await
}

/// `push [-e ENV] [FILE]`: a dotenv file's values into an environment (existing ones replaced, kept in history).
/// Returns the lines and the file read, which the caller may offer to move to the trash.
pub async fn push(
    backend: &dyn Backend,
    here: &Here,
    env: Option<&str>,
    file: Option<&Path>,
    cwd: &Path,
) -> Result<(Vec<String>, PathBuf), String> {
    let env = here.env(env)?;
    let path = match file {
        Some(f) => cwd.join(f),
        None => here.root.as_deref().unwrap_or(cwd).join(".env"),
    };
    if path.file_name().is_some_and(|n| n == ENV_FILE) {
        return Err(format!("{ENV_FILE} holds references, not values: push the file that has the values"));
    }
    let text = Zeroizing::new(std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?);
    let vars = crate::project::parse_dotenv(&text).map_err(|e| {
        let lines: Vec<String> = e.errors.iter().map(|(n, m)| format!("{}:{n}: {m}", path.display())).collect();
        lines.join("\n")
    })?;
    drop(text);
    let vars: Vec<_> = vars.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    if vars.is_empty() {
        return Err(format!("{}: no values", path.display()));
    }
    let mut change = EnvChanges::new(&here.project, &env);
    change.set = vars.iter().map(|(n, v)| (n.clone(), v.to_string())).collect();
    change.replace = true;
    drop(vars);
    let lines = store(backend, std::slice::from_ref(&change)).await?;
    Ok((lines, path))
}

/// `pull [-e ENV]`: `.env.reins` gets a reference for every variable of the environment it does not have yet.
pub async fn pull(backend: &dyn Backend, here: &Here, env: Option<&str>) -> Result<Vec<String>, String> {
    let env = here.env(env)?;
    let root = here.root()?.to_path_buf();
    let names = env_names(backend, here, &env).await?;
    let path = root.join(ENV_FILE);
    let mut file = if path.is_file() {
        EnvFile::load(&path)?
    } else {
        EnvFile::new()
    };
    let (mut added, mut kept) = (0, Vec::new());
    for name in &names {
        match file.get(name) {
            None => {
                file.set(
                    name,
                    EnvValue::Reference(Reference::ProjectVar {
                        env: env.clone(),
                        name: name.clone(),
                    }),
                )?;
                added += 1;
            }
            Some(v) if !matches!(&v.value, EnvValue::Reference(Reference::ProjectVar { env: e, name: n }) if *e == env && n == name) =>
            {
                kept.push(name.clone());
            }
            Some(_) => {}
        }
    }
    if added > 0 {
        file.write(&path)?;
    }
    let mut lines = vec![format!("{ENV_FILE}: {added} added from {}/{env}", here.project)];
    if !kept.is_empty() {
        lines.push(format!("Kept as they are: {}", kept.join(", ")));
    }
    Ok(lines)
}

/// The variable names of `project/env`; an error when it is not in the vault.
async fn env_names(backend: &dyn Backend, here: &Here, env: &str) -> Result<Vec<String>, String> {
    find_env(&backend.names(Some(&here.project)).await?, &here.project, env)
        .map(|e| e.variables.clone())
        .ok_or_else(|| format!("No {}/{env} in the vault.", here.project))
}

pub(crate) fn find_env<'a>(listing: &'a [ProjectListing], project: &str, env: &str) -> Option<&'a EnvListing> {
    listing.iter().filter(|p| p.name == project).flat_map(|p| &p.envs).find(|e| e.name == env)
}

/// `diff ENV ENV2`: the names in one and not the other.
pub async fn diff(backend: &dyn Backend, here: &Here, a: &str, b: &str) -> Result<Vec<String>, String> {
    let (a, b) = (here.env(Some(a))?, here.env(Some(b))?);
    let listing = backend.names(Some(&here.project)).await?;
    let names = |env: &str| {
        find_env(&listing, &here.project, env)
            .map(|e| e.variables.clone())
            .ok_or_else(|| format!("No {}/{env} in the vault.", here.project))
    };
    let (left, right) = (names(&a)?, names(&b)?);
    let only = |x: &[String], y: &[String]| x.iter().filter(|n| !y.contains(n)).cloned().collect::<Vec<_>>();
    let (only_a, only_b) = (only(&left, &right), only(&right, &left));
    if only_a.is_empty() && only_b.is_empty() {
        return Ok(vec![format!("{a} and {b} have the same {} names.", left.len())]);
    }
    let mut lines = Vec::new();
    for (env, list) in [(&a, only_a), (&b, only_b)] {
        if !list.is_empty() {
            lines.push(format!("Only in {env}: {}", list.join(", ")));
        }
    }
    Ok(lines)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A vault in memory: names, and what was stored.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub listing: Vec<ProjectListing>,
        pub stored: Mutex<Vec<EnvChanges>>,
    }

    #[async_trait::async_trait]
    impl Backend for Fake {
        async fn names(&self, project: Option<&str>) -> Result<Vec<ProjectListing>, String> {
            Ok(self.listing.iter().filter(|p| project.is_none_or(|n| n == p.name)).cloned().collect())
        }

        async fn store(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError> {
            self.stored.lock().unwrap().extend(envs.iter().cloned());
            crate::vault_writer::MemoryWriter::default().store_envs(envs).await
        }
    }

    pub(crate) fn fake() -> Fake {
        let env = |name: &str, vars: &[&str]| EnvListing {
            name: name.into(),
            revision: "2026-10-14T09:30:12.123Z".into(),
            variables: vars.iter().map(|v| (*v).to_owned()).collect(),
            origin: None,
        };
        Fake {
            listing: vec![
                ProjectListing {
                    name: "acme".into(),
                    envs: vec![env("development", &["API_KEY", "DB_URL"]), env("production", &["API_KEY", "SENTRY"])],
                    origin: Some("github.com/acme/api".into()),
                },
                ProjectListing {
                    name: "blog".into(),
                    envs: vec![env("development", &["TOKEN"])],
                    origin: None,
                },
            ],
            stored: Mutex::default(),
        }
    }

    fn here(root: Option<&Path>) -> Here {
        Here {
            project: "acme".into(),
            default_env: "development".into(),
            root: root.map(Path::to_path_buf),
        }
    }

    #[test]
    fn targets_name_an_environment_and_a_variable() {
        let h = here(None);
        assert_eq!(h.target("production/API_KEY").unwrap(), ("production".into(), "API_KEY".into()));
        assert_eq!(h.target("API_KEY").unwrap(), ("development".into(), "API_KEY".into()));
        for bad in ["Prod/API_KEY", "dev/1A", "dev/A/B", "a/b/c", ""] {
            assert!(h.target(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_project_comes_from_reins_toml_or_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Here::find(dir.path(), None).is_err());
        std::fs::write(dir.path().join("reins.toml"), "[project]\nname = \"acme\"\ndefault_env = \"dev\"\n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let h = Here::find(&dir.path().join("src"), None).unwrap();
        assert_eq!((h.project.as_str(), h.default_env.as_str(), h.root.as_deref()), ("acme", "dev", Some(dir.path())));
        let other = Here::find(dir.path(), Some("blog")).unwrap();
        assert_eq!((other.default_env.as_str(), other.root), ("development", None));
        assert!(Here::find(dir.path(), Some("Blog")).is_err());
    }

    #[tokio::test]
    async fn ls_shows_names_and_versions_never_values() {
        let f = fake();
        let lines = ls(&f, Some(&here(None)), None).await.unwrap();
        assert_eq!(
            lines,
            [
                "acme  github.com/acme/api",
                "  development  2026-10-14 09:30  API_KEY DB_URL",
                "  production  2026-10-14 09:30  API_KEY SENTRY"
            ]
        );
        assert_eq!(ls(&f, None, None).await.unwrap().len(), 5);
        assert_eq!(ls(&f, None, Some("acme/production")).await.unwrap().len(), 2);
        assert_eq!(ls(&f, None, Some("acme/staging")).await.unwrap().last().unwrap(), "No acme/staging in the vault.");
        assert_eq!(ls(&f, None, Some("nope")).await.unwrap(), ["No nope in the vault."]);
    }

    #[tokio::test]
    async fn edits_go_through_one_store_each() {
        let f = fake();
        let h = here(None);
        let out = set(&f, &h, "production/API_KEY", Zeroizing::new("sk-1".into())).await.unwrap();
        assert_eq!(out, ["acme/production: 1 added"]);
        unset(&f, &h, &["A".into(), "production/B".into(), "C".into()]).await.unwrap();
        rename(&f, &h, "OLD", "NEW").await.unwrap();
        assert!(rename(&f, &h, "OLD", "production/NEW").await.is_err());
        cp(&f, &h, "development", "staging").await.unwrap();
        assert!(cp(&f, &h, "development", "development").await.is_err());
        let stored = f.stored.lock().unwrap();
        assert_eq!(stored.len(), 5);
        assert!(stored[0].replace && stored[0].set == [("API_KEY".to_owned(), "sk-1".to_owned())]);
        assert_eq!((stored[1].env.as_str(), stored[1].unset.clone()), ("development", vec!["A".into(), "C".into()]));
        assert_eq!((stored[2].env.as_str(), stored[2].unset.clone()), ("production", vec!["B".into()]));
        assert_eq!(stored[3].rename, [("OLD".to_owned(), "NEW".to_owned())]);
        assert_eq!((stored[4].env.as_str(), stored[4].copy_from.as_deref()), ("staging", Some("development")));
    }

    #[tokio::test]
    async fn push_sends_a_dotenv_files_values_and_pull_writes_references() {
        let dir = tempfile::tempdir().unwrap();
        let f = fake();
        let h = here(Some(dir.path()));
        std::fs::write(dir.path().join(".env"), "# keys\nAPI_KEY=sk-live-1\nexport DB_URL=\"postgres://x\"\nEMPTY=\n")
            .unwrap();
        let (lines, path) = push(&f, &h, None, None, dir.path()).await.unwrap();
        assert_eq!((lines[0].as_str(), path), ("acme/development: 2 added", dir.path().join(".env")));
        let stored = f.stored.lock().unwrap().remove(0);
        assert!(stored.replace);
        assert_eq!(stored.set.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["API_KEY", "DB_URL"]);
        std::fs::write(dir.path().join(ENV_FILE), "LOG_LEVEL=debug\nDB_URL=postgres://localhost\n").unwrap();
        assert!(push(&f, &h, None, Some(Path::new(ENV_FILE)), dir.path()).await.is_err());

        let lines = pull(&f, &h, None).await.unwrap();
        assert_eq!(lines, [".env.reins: 1 added from acme/development", "Kept as they are: DB_URL"]);
        let file = EnvFile::load(&dir.path().join(ENV_FILE)).unwrap();
        assert_eq!(
            file.get("API_KEY").unwrap().value,
            EnvValue::Reference(Reference::parse_value("env:development/API_KEY").unwrap())
        );
        assert_eq!(file.get("LOG_LEVEL").unwrap().value, EnvValue::Literal("debug".into()));
        assert_eq!(pull(&f, &h, None).await.unwrap()[0], ".env.reins: 0 added from acme/development");
        assert!(pull(&f, &h, Some("staging")).await.unwrap_err().contains("No acme/staging"));
        assert!(pull(&f, &here(None), None).await.is_err(), "no project folder");
    }

    #[tokio::test]
    async fn diff_names_what_one_environment_lacks() {
        let f = fake();
        let h = here(None);
        assert_eq!(
            diff(&f, &h, "development", "production").await.unwrap(),
            ["Only in development: DB_URL", "Only in production: SENTRY"]
        );
        assert_eq!(
            diff(&f, &h, "development", "development").await.unwrap(),
            ["development and development have the same 2 names."]
        );
        assert!(diff(&f, &h, "development", "staging").await.is_err());
    }
}
