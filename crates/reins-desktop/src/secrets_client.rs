//! Reins Secrets through the phone: releasing an environment for one command (`vault_env_release`), listing names
//! (`vault_secrets_list`), and storing (`vault_secrets_store`, `integration_token_store`: [`PhoneVaultWriter`]).
//!
//! A release answers with a [`SecretGrant`] sealed to this app's key. It is used only when it answers this very request:
//! the nonce, not expired, the project and environment asked for, the variables asked for (or valid names when every one
//! was), and the binding the phone decided: the runner and the project's folder asked for, and a command scope that
//! covers the command asked for. The daemon keeps it for that scope only ([`crate::leases`]).
//!
//! Stores and the names travel boxed between this app's key and the phone's own key (as `reins vault add`), so only
//! the pinned phone can open the values or have written the answer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reins_proto::desktop::{
    CommandScope, ENV_RELEASE_TOOL, EnvChanges, MAX_ENV_VARS, PHONE_KEY_CHANGED, SECRETS_LIST_TOOL, SECRETS_STORE_TOOL,
    SecretGrant, SecretsListing, SecretsStoreAck, SecretsToStore, TO_DESKTOP, TO_PHONE, TOKEN_STORE_TOOL,
    TokenStoreAck, TokenToStore, var_ok,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::{Zeroize as _, Zeroizing};

use crate::auth::Refusal;
use crate::config::{Config, Paths};
use crate::journal::{Entry, Journal, Kind};
use crate::phone::{Phone, awaiting, teller, unavailable};
use crate::vault_writer::{IntegrationToken, StoreError, StoreReceipt, TokenReceipt, VaultWriter};

/// How long a release is kept when `reins.toml` does not say.
pub const DEFAULT_LEASE_SECS: u32 = 3_600;
const NO_ANSWER: &str = "No answer from your phone in time. Approve it, then run again.";

/// What `reins run` asks for: one environment, for one command, run by someone, in a project's folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRequest {
    pub project: String,
    pub env: String,
    /// `None`: every variable of the environment.
    pub fields: Option<Vec<String>>,
    /// The program by its absolute path when it could be resolved.
    pub program: String,
    pub args: Vec<String>,
    /// The project's folder (canonical path).
    pub root: String,
    /// `person` or `agent:<harness>`.
    pub runner: String,
    pub lease_secs: u32,
    pub purpose: Option<String>,
    /// `confirm = "always"` in reins.toml.
    pub confirm_always: bool,
    /// `reins secrets get`: the value is printed; never cached, always asked.
    #[serde(default)]
    pub print: bool,
}

impl LeaseRequest {
    /// The exact command asked for.
    #[must_use]
    pub fn command(&self) -> CommandScope {
        CommandScope::exact(&self.program, &self.args)
    }

    /// `project/env`.
    #[must_use]
    pub fn item(&self) -> String {
        format!("{}/{}", self.project, self.env)
    }
}

/// An environment released by the phone, as it bound it.
pub struct Released {
    /// Variable names and values, sorted by name.
    pub values: Vec<(String, Zeroizing<String>)>,
    pub expires_at: i64,
    pub revision: String,
    /// The commands the values may be handed to without asking again.
    pub command: CommandScope,
    /// Every variable of the environment (else only these names).
    pub all: bool,
}

impl std::fmt::Debug for Released {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.values.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("Released").field("names", &names).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// The grant must answer this request: the nonce, not expired, the environment, the variables, and the binding the
/// phone decided (runner, folder, and a command scope that covers this command).
pub fn check_env_grant(mut grant: SecretGrant, nonce: &str, req: &LeaseRequest, now: i64) -> Result<Released, Refusal> {
    let result = (|| {
        let refused = |what: &str| Err(unavailable(&format!("The phone's answer {what}; refused.")));
        if grant.nonce != nonce {
            return refused("is for another request (the nonce differs)");
        }
        if grant.expires_at <= now {
            return refused("has expired");
        }
        if grant.project.as_deref() != Some(req.project.as_str()) || grant.env.as_deref() != Some(req.env.as_str()) {
            return refused("is for another environment");
        }
        if grant.runner.as_deref() != Some(req.runner.as_str()) || grant.root.as_deref() != Some(req.root.as_str()) {
            return refused("is bound to someone else or another folder");
        }
        let Some(command) = grant.command.clone() else {
            return refused("names no command");
        };
        if command.validate().is_err() || !req.command().within(&command) {
            return refused("is for another command");
        }
        let names: Vec<&str> = grant.secrets.iter().map(|(n, _)| n.as_str()).collect();
        let named_ok = match &req.fields {
            Some(wanted) => names == wanted.iter().map(String::as_str).collect::<Vec<_>>(),
            None => names.len() <= MAX_ENV_VARS && names.iter().all(|n| var_ok(n)) && names.is_sorted(),
        };
        if !named_ok {
            return refused("does not hold the variables asked for");
        }
        Ok(Released {
            values: grant.secrets.iter_mut().map(|(n, v)| (n.clone(), Zeroizing::new(std::mem::take(v)))).collect(),
            expires_at: grant.expires_at,
            revision: grant.revision.clone().unwrap_or_default(),
            command,
            all: req.fields.is_none(),
        })
    })();
    for (_, v) in &mut grant.secrets {
        v.zeroize();
    }
    result
}

/// Asks the phone to release `req`. `reuse`: a key under which an unanswered earlier question is polled again.
pub async fn release_env(phone: &Phone, req: &LeaseRequest, reuse: Option<&str>) -> Result<Released, Refusal> {
    let mut sorted = req.clone();
    if let Some(f) = &mut sorted.fields {
        f.sort();
        f.dedup();
    }
    let answer = phone
        .ask(
            ENV_RELEASE_TOOL,
            reuse,
            |_| {
                let mut args = json!({"project": sorted.project, "env": sorted.env, "program": sorted.program,
                    "args": sorted.args, "root": sorted.root, "runner": sorted.runner,
                    "lease_secs": sorted.lease_secs.clamp(60, 86_400)});
                if let Some(f) = &sorted.fields {
                    args["fields"] = json!(f);
                }
                if let Some(p) = &sorted.purpose {
                    args["purpose"] = json!(crate::secrets::cut(p, 300));
                }
                if sorted.confirm_always {
                    args["confirm"] = json!("always");
                }
                if sorted.print {
                    args["print"] = json!(true);
                }
                args
            },
            NO_ANSWER,
        )
        .await?;
    let grant: SecretGrant = phone.open(&answer.data)?;
    check_env_grant(grant, &answer.nonce, &sorted, crate::now_unix())
}

/// The names in the vault (of one project), boxed from the pinned phone.
pub async fn list(
    phone: &Phone,
    phone_key: &str,
    journal: &Journal,
    timeout: Duration,
    project: Option<&str>,
) -> Result<SecretsListing, String> {
    let entry = Entry::new(Kind::Secrets, &format!("the secret names of {}", project.unwrap_or("every project")))
        .source(Some("reins secrets"));
    let tell = teller(|line: &str| eprintln!("{line}"));
    let asked =
        phone.ask(SECRETS_LIST_TOOL, None, |_| project.map_or_else(|| json!({}), |p| json!({"project": p})), NO_ANSWER);
    let answer =
        awaiting(journal, entry, Some(tell), timeout, asked).await.map_err(|r| crate::vault_cli::relayed(&r))?;
    let listing: SecretsListing = crate::vault_cli::open_from_phone(phone, phone_key, &answer.data)?;
    if listing.dir != TO_DESKTOP || listing.nonce != answer.nonce {
        return Err("The phone's answer is for another request; refused.".to_owned());
    }
    Ok(listing)
}

fn store_error(r: &Refusal) -> StoreError {
    match r {
        Refusal::Waiting(m) => StoreError::Waiting(m.clone()),
        Refusal::Unavailable(m) if m.starts_with(PHONE_KEY_CHANGED) => StoreError::PhoneChanged,
        Refusal::Denied(_) => StoreError::Denied(crate::vault_cli::relayed(r)),
        Refusal::Unavailable(_) => StoreError::Failed(crate::vault_cli::relayed(r)),
    }
}

/// [`VaultWriter`] through the phone this computer is paired with.
pub struct PhoneVaultWriter {
    paths: Paths,
    config: Config,
    /// The twelve digits of the phone's key, for a process that cannot ask at a terminal (the first time only).
    phone_key_digits: Option<String>,
}

impl PhoneVaultWriter {
    #[must_use]
    pub fn new(paths: &Paths, config: &Config, phone_key_digits: Option<String>) -> Self {
        Self {
            paths: paths.clone(),
            config: config.clone(),
            phone_key_digits,
        }
    }

    /// The phone and its key, checking the key with the user the first time.
    async fn phone(&self) -> Result<(Phone, String), StoreError> {
        let phone = crate::vault_cli::logged_in(&self.paths, &self.config).map_err(StoreError::Failed)?;
        let key = match (crate::vault_cli::pinned(&self.paths, phone.server()), &self.phone_key_digits) {
            (Some(key), _) => key,
            // Digits the user read on the phone and typed into this app's own screen.
            (None, Some(digits)) => crate::vault_cli::check_phone_key(&phone, &self.paths, Some(digits))
                .await
                .map_err(StoreError::Failed)?,
            (None, None) if crate::vault_cli::has_terminal() => {
                crate::vault_cli::phone_key_for(&phone, &self.paths, None, false).await.map_err(StoreError::Failed)?
            }
            (None, None) => return Err(StoreError::PhoneKeyNeeded),
        };
        Ok((phone, key))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(self.config.approval_timeout_secs)
    }
}

/// `value` as JSON in a box from this app's key to the phone's, in a buffer that is wiped.
fn boxed<T: Serialize>(phone: &Phone, phone_key: &str, value: &T) -> Option<String> {
    let mut plain = Zeroizing::new(Vec::with_capacity(64 * 1024));
    serde_json::to_writer(&mut *plain, value).ok()?;
    phone.identity().box_to(phone_key, &plain)
}

#[async_trait::async_trait]
impl VaultWriter for PhoneVaultWriter {
    async fn store_envs(&self, envs: &[EnvChanges]) -> Result<StoreReceipt, StoreError> {
        reins_proto::desktop::validate_store(envs).map_err(StoreError::Failed)?;
        let (phone, phone_key) = self.phone().await?;
        let names: Vec<String> = envs.iter().map(EnvChanges::item).collect();
        let what = format!("save secrets in {}", names.join(", "));
        let entry = Entry::new(Kind::Secrets, &what).source(Some("reins secrets"));
        let tell = teller(|line: &str| eprintln!("{line}"));
        let asked = phone.ask(
            SECRETS_STORE_TOOL,
            None,
            |nonce| {
                let sealed = boxed(
                    &phone,
                    &phone_key,
                    &SecretsToStore {
                        v: 1,
                        dir: TO_PHONE.to_owned(),
                        nonce: nonce.to_owned(),
                        created_at: crate::now_unix(),
                        envs: envs.to_vec(),
                        ssh: Vec::new(),
                        tokens: Vec::new(),
                    },
                );
                json!({"envs": names, "sealed": sealed.unwrap_or_default()})
            },
            NO_ANSWER,
        );
        let answer = awaiting(&Journal::new(&self.paths), entry, Some(tell), self.timeout(), asked)
            .await
            .map_err(|r| store_error(&r))?;
        let ack: SecretsStoreAck =
            crate::vault_cli::open_from_phone(&phone, &phone_key, &answer.data).map_err(StoreError::Failed)?;
        if ack.dir != TO_DESKTOP || ack.nonce != answer.nonce {
            return Err(StoreError::Failed("The phone's answer is for another request; refused.".to_owned()));
        }
        // The daemon drops the values it holds of these environments now (the phone tells it too, in a while).
        if let Ok(control) = crate::control::Client::new(&self.paths, self.config.listen) {
            for e in envs {
                let drop = crate::leases::RevokeRequest {
                    project: Some(e.project.clone()),
                    env: Some(e.env.clone()),
                    ..crate::leases::RevokeRequest::default()
                };
                control.revoke(&drop).await.ok();
            }
        }
        Ok(StoreReceipt {
            envs: ack.envs,
        })
    }

    async fn store_integration_token(&self, token: &IntegrationToken) -> Result<TokenReceipt, StoreError> {
        let (phone, phone_key) = self.phone().await?;
        let entry =
            Entry::new(Kind::Secrets, &format!("add a {} account", token.service)).source(Some("reins secrets"));
        let tell = teller(|line: &str| eprintln!("{line}"));
        let asked = phone.ask(
            TOKEN_STORE_TOOL,
            None,
            |nonce| {
                let sealed = boxed(
                    &phone,
                    &phone_key,
                    &TokenToStore {
                        v: 1,
                        dir: TO_PHONE.to_owned(),
                        nonce: nonce.to_owned(),
                        created_at: crate::now_unix(),
                        service: token.service.clone(),
                        host: token.host.clone(),
                        scopes: token.scopes.clone(),
                        token: token.token.to_string(),
                    },
                );
                json!({"service": token.service, "host": token.host, "scopes": token.scopes,
                    "sealed": sealed.unwrap_or_default()})
            },
            NO_ANSWER,
        );
        let answer = awaiting(&Journal::new(&self.paths), entry, Some(tell), self.timeout(), asked)
            .await
            .map_err(|r| store_error(&r))?;
        let ack: TokenStoreAck =
            crate::vault_cli::open_from_phone(&phone, &phone_key, &answer.data).map_err(StoreError::Failed)?;
        if ack.dir != TO_DESKTOP || ack.nonce != answer.nonce || ack.service != token.service {
            return Err(StoreError::Failed("The phone's answer is for another request; refused.".to_owned()));
        }
        Ok(TokenReceipt {
            service: ack.service,
            account: ack.account,
        })
    }
}

/// The program as the phone and a permission name it: its absolute path when it can be found (on PATH, or relative to
/// the working directory), else as typed.
#[must_use]
pub fn resolve_program(program: &str, cwd: &std::path::Path) -> String {
    resolve_program_with(program, cwd, std::env::var("PATH").ok().as_deref())
}

/// [`resolve_program`] with the `PATH` of the process that asks.
#[must_use]
pub fn resolve_program_with(program: &str, cwd: &std::path::Path, path_var: Option<&str>) -> String {
    let path = std::path::Path::new(program);
    let found: Option<PathBuf> = if path.components().count() > 1 || path.is_absolute() {
        Some(cwd.join(path))
    } else {
        let dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
        #[cfg(windows)]
        {
            let pathext = std::env::var("PATHEXT").ok();
            crate::win::resolve_program(
                std::ffi::OsStr::new(program),
                &dirs,
                pathext.as_deref(),
                std::path::Path::is_file,
            )
        }
        #[cfg(not(windows))]
        {
            dirs.iter().map(|d| d.join(program)).find(|p| is_executable(p))
        }
    };
    found
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map_or_else(|| program.to_owned(), |p| p.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Builds the writer for a terminal command.
#[must_use]
pub fn writer(paths: &Paths, config: &Config) -> Arc<dyn VaultWriter> {
    Arc::new(PhoneVaultWriter::new(paths, config, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(fields: Option<&[&str]>) -> LeaseRequest {
        LeaseRequest {
            project: "acme".into(),
            env: "dev".into(),
            fields: fields.map(|f| f.iter().map(|x| (*x).to_owned()).collect()),
            program: "/usr/bin/npm".into(),
            args: vec!["run".into(), "dev".into()],
            root: "/home/me/acme".into(),
            runner: "person".into(),
            lease_secs: 600,
            purpose: None,
            confirm_always: false,
            print: false,
        }
    }

    fn grant(secrets: &[(&str, &str)], command: CommandScope) -> SecretGrant {
        SecretGrant {
            v: 1,
            nonce: "n".into(),
            secrets: secrets.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect(),
            expires_at: 200,
            project: Some("acme".into()),
            env: Some("dev".into()),
            revision: Some("r1".into()),
            runner: Some("person".into()),
            root: Some("/home/me/acme".into()),
            command: Some(command),
        }
    }

    #[test]
    fn a_release_must_answer_exactly_this_request() {
        let r = req(Some(&["A"]));
        let ok = check_env_grant(grant(&[("A", "x")], r.command()), "n", &r, 100).unwrap();
        assert_eq!((ok.values[0].0.as_str(), ok.values[0].1.as_str(), ok.revision.as_str()), ("A", "x", "r1"));
        assert!(!format!("{ok:?}").contains("\"x\""));
        let prefix = CommandScope {
            program: "/usr/bin/npm".into(),
            args: vec!["run".into()],
            prefix: true,
        };
        assert!(check_env_grant(grant(&[("A", "x")], prefix), "n", &r, 100).is_ok(), "a wider scope the user chose");
        let all = req(None);
        assert!(check_env_grant(grant(&[("A", "x"), ("B", "y")], all.command()), "n", &all, 100).is_ok());
        let edit = |f: &dyn Fn(&mut SecretGrant)| {
            let mut g = grant(&[("A", "x")], r.command());
            f(&mut g);
            check_env_grant(g, "n", &r, 100).is_err()
        };
        assert!(edit(&|g| g.nonce = "m".into()), "nonce");
        assert!(edit(&|g| g.expires_at = 100), "expired");
        assert!(edit(&|g| g.env = Some("prod".into())), "env");
        assert!(edit(&|g| g.runner = Some("agent:codex".into())), "runner");
        assert!(edit(&|g| g.root = Some("/tmp".into())), "root");
        assert!(edit(&|g| g.command = None), "no command");
        assert!(edit(&|g| g.command = Some(CommandScope::exact("/usr/bin/npm", &["run".into(), "test".into()]))));
        assert!(
            edit(&|g| {
                g.command = Some(CommandScope {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into()],
                    prefix: true,
                });
            }),
            "a refused prefix"
        );
        assert!(edit(&|g| g.secrets.push(("B".into(), "y".into()))), "more than asked");
        let mut unsorted = grant(&[("B", "y"), ("A", "x")], all.command());
        unsorted.secrets.push(("1X".into(), "z".into()));
        assert!(check_env_grant(unsorted, "n", &all, 100).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn programs_are_named_by_their_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_program("no-such-program-xyz", dir.path()), "no-such-program-xyz");
        let sh = resolve_program("sh", dir.path());
        assert!(sh.starts_with('/'), "{sh}");
        std::fs::write(dir.path().join("tool"), "x").unwrap();
        let local = resolve_program("./tool", dir.path());
        assert_eq!(local, std::fs::canonicalize(dir.path().join("tool")).unwrap().to_string_lossy());
    }
}
