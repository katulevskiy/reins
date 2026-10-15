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
    CommandScope, ENV_RELEASE_TOOL, EnvChanges, HostToken, MAX_ENV_VARS, PHONE_KEY_CHANGED, SECRETS_LIST_TOOL,
    SECRETS_STORE_TOOL, SecretGrant, SecretsListing, SecretsStoreAck, SecretsToStore, SshKeyToStore, TO_DESKTOP,
    TO_PHONE, TOKEN_STORE_TOOL, TokenStoreAck, TokenToStore, var_ok,
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
    /// The program as typed (`npm`, `./npm`) and each link on the way to it: the phone weighs every name.
    #[serde(default)]
    pub typed: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Whether this computer's own `PATH` finds the same program for the name typed ([`reins_proto::desktop::PATH_SAME`]
    /// or [`reins_proto::desktop::PATH_OTHER`]); `None`: it finds none, or a path was typed.
    #[serde(default)]
    pub path_check: Option<String>,
    /// A digest of the caller's variables that change what a program runs ([`exec_env_digest`]): a lease is reused
    /// only for the same.
    #[serde(default)]
    pub exec_env: String,
}

/// Variables that change what a program runs or loads, besides its arguments: under a remembered pattern they are not
/// passed to the command ([`strip_exec_env`]), and a lease is reused only when they (and `PATH`) are as they were.
pub const EXEC_ENV: &[&str] = &[
    "NODE_OPTIONS",
    "NODE_PATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "RUBYLIB",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC",
    "RUSTFLAGS",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_HOME",
    "GIT_EXEC_PATH",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_ASKPASS",
    "GIT_EXTERNAL_DIFF",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "SHELL",
    "PROMPT_COMMAND",
    "npm_config_script_shell",
    "npm_config_node_options",
    "npm_config_prefix",
    "MAKEFLAGS",
    "MAKEFILES",
    "GOFLAGS",
    "PYTEST_ADDOPTS",
    "PYTEST_PLUGINS",
];

/// Whether `name` is one of [`EXEC_ENV`], or a family of them (`DYLD_*`, `CARGO_TARGET_*_RUNNER`, `GIT_CONFIG_*`).
#[must_use]
pub fn exec_env(name: &str) -> bool {
    EXEC_ENV.iter().any(|e| e.eq_ignore_ascii_case(name))
        || name.starts_with("DYLD_")
        || name.starts_with("LD_")
        || (name.starts_with("CARGO_TARGET_") && name.ends_with("_RUNNER"))
        || (name.starts_with("CARGO_") && name.ends_with("_RUSTC_WRAPPER"))
        || name.starts_with("GIT_CONFIG_")
        || name.starts_with("NPM_CONFIG_")
        || name.starts_with("npm_config_")
}

/// A digest of `PATH` and the [`exec_env`] variables in `env`, in name order.
#[must_use]
pub fn exec_env_digest(env: &[(String, String)]) -> String {
    use sha2::Digest as _;
    let mut picked: Vec<&(String, String)> = env.iter().filter(|(k, _)| k == "PATH" || exec_env(k)).collect();
    picked.sort();
    let mut h = sha2::Sha256::new();
    for (k, v) in picked {
        h.update((k.len() as u64).to_be_bytes());
        h.update(k.as_bytes());
        h.update((v.len() as u64).to_be_bytes());
        h.update(v.as_bytes());
    }
    data_encoding::HEXLOWER.encode(&h.finalize()[..16])
}

/// `env` without the [`exec_env`] variables (`PATH` stays: the program is resolved already, and a lease is bound to
/// it); the names left out.
#[must_use]
pub fn strip_exec_env(env: &mut Vec<(String, String)>) -> Vec<String> {
    let mut gone = Vec::new();
    env.retain(|(k, _)| {
        let keep = !exec_env(k);
        if !keep {
            gone.push(k.clone());
        }
        keep
    });
    gone
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
    /// For this run only: never kept for another.
    pub one_shot: bool,
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
        if !reins_proto::desktop::release_gate(&command, &req.program, &req.args) {
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
            one_shot: grant.one_shot || req.print,
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
                if let Some(t) = &sorted.typed {
                    args["typed"] = json!(t);
                }
                if !sorted.aliases.is_empty() {
                    args["aliases"] = json!(sorted.aliases);
                }
                if let Some(c) = &sorted.path_check {
                    args["path_check"] = json!(c);
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

    /// Saves environments, SSH keys and git hosts' tokens with one approval on the phone (`vault_secrets_store`): the
    /// phone's answer, one result per part, in the order sent. Show [`reins_proto::desktop::batch_check`] first.
    pub async fn store_batch(
        &self,
        envs: &[EnvChanges],
        ssh: &[SshKeyToStore],
        tokens: &[HostToken],
    ) -> Result<SecretsStoreAck, StoreError> {
        reins_proto::desktop::validate_batch(envs, ssh, tokens).map_err(StoreError::Failed)?;
        let (phone, phone_key) = self.phone().await?;
        let names: Vec<String> = envs.iter().map(EnvChanges::item).collect();
        let keys: Vec<String> = ssh.iter().map(|k| k.name.trim().to_owned()).collect();
        let hosts: Vec<String> = tokens.iter().map(|t| t.host.clone()).collect();
        let what = format!(
            "save {} in the vault",
            names.iter().chain(&keys).chain(&hosts).cloned().collect::<Vec<_>>().join(", ")
        );
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
                        ssh: ssh.to_vec(),
                        tokens: tokens.to_vec(),
                    },
                );
                json!({"envs": names, "ssh": keys, "hosts": hosts, "sealed": sealed.unwrap_or_default()})
            },
            NO_ANSWER,
        );
        let answer = awaiting(&Journal::new(&self.paths), entry, Some(tell), self.timeout(), asked)
            .await
            .map_err(|r| store_error(&r))?;
        let ack: SecretsStoreAck =
            crate::vault_cli::open_from_phone(&phone, &phone_key, &answer.data).map_err(StoreError::Failed)?;
        if ack.dir != TO_DESKTOP
            || ack.nonce != answer.nonce
            || ack.envs.len() != envs.len()
            || ack.ssh.len() != ssh.len()
            || ack.tokens.len() != tokens.len()
        {
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
        Ok(ack)
    }
}

/// An import's batch (`secrets_import`) in the store's own terms: environments (existing values replaced, kept in
/// history), SSH keys (always into `Reins/ssh`) and git hosts' tokens.
fn import_parts(batch: &crate::secrets_import::Batch) -> (Vec<EnvChanges>, Vec<SshKeyToStore>, Vec<HostToken>) {
    let envs = batch
        .envs
        .iter()
        .map(|e| {
            let mut c = EnvChanges::new(&e.project, &e.env);
            c.set = e.vars.iter().map(|(n, v)| (n.clone(), v.to_string())).collect();
            c.replace = true;
            c.origin.clone_from(&e.origin);
            c
        })
        .collect();
    let ssh = batch
        .ssh
        .iter()
        .map(|k| SshKeyToStore {
            name: k.name.clone(),
            private_key: k.private_key.to_string(),
        })
        .collect();
    let tokens = batch
        .tokens
        .iter()
        .map(|t| HostToken {
            host: t.host.clone(),
            user: t.user.clone(),
            scopes: Vec::new(),
            token: t.token.to_string(),
        })
        .collect();
    (envs, ssh, tokens)
}

/// The check digits of an import's batch, as the phone computes them ([`reins_proto::desktop::batch_check`]).
#[must_use]
pub fn import_check(batch: &crate::secrets_import::Batch) -> String {
    let (envs, ssh, tokens) = import_parts(batch);
    reins_proto::desktop::batch_check(&envs, &ssh, &tokens)
}

impl PhoneVaultWriter {
    /// The Reins projects in the vault and where each came from (`reins.origin`): names only, never a value. One
    /// approval on the phone (which can be remembered there).
    pub async fn projects(&self) -> Result<Vec<reins_proto::desktop::ProjectListing>, StoreError> {
        let (phone, phone_key) = self.phone().await?;
        let listing = list(&phone, &phone_key, &Journal::new(&self.paths), self.timeout(), None)
            .await
            .map_err(StoreError::Failed)?;
        Ok(listing.projects)
    }

    /// [`PhoneVaultWriter::projects`] as an import asks for them.
    pub async fn import_projects(&self) -> Result<Vec<crate::secrets_import::VaultProject>, String> {
        Ok(self
            .projects()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| crate::secrets_import::VaultProject {
                name: p.name,
                origin: p.origin,
            })
            .collect())
    }

    /// Stores an import's batch with one approval: a result per environment, SSH key and token, in the batch's order.
    pub async fn store_import(&self, batch: &crate::secrets_import::Batch) -> crate::secrets_import::Stored {
        let (envs, ssh, tokens) = import_parts(batch);
        match self.store_batch(&envs, &ssh, &tokens).await {
            Ok(ack) => {
                let result =
                    |e: &Option<String>| e.as_ref().map_or(Ok(()), |why| Err(reins_proto::desktop::printable(why)));
                crate::secrets_import::Stored {
                    envs: ack.envs.iter().map(|e| result(&e.error)).collect(),
                    ssh: ack.ssh.iter().map(|k| result(&k.error)).collect(),
                    tokens: ack.tokens.iter().map(|t| result(&t.error)).collect(),
                }
            }
            Err(e) => {
                let why = e.to_string();
                crate::secrets_import::Stored {
                    envs: envs.iter().map(|_| Err(why.clone())).collect(),
                    ssh: ssh.iter().map(|_| Err(why.clone())).collect(),
                    tokens: tokens.iter().map(|_| Err(why.clone())).collect(),
                }
            }
        }
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
        let ack = self.store_batch(envs, &[], &[]).await?;
        if let Some((e, why)) = ack.envs.iter().find_map(|e| e.error.as_ref().map(|w| (e, w))) {
            return Err(StoreError::Failed(format!(
                "{}/{}: {}",
                e.project,
                e.env,
                reins_proto::desktop::printable(why)
            )));
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
    found_program(program, cwd, path_var)
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map_or_else(|| program.to_owned(), |p| p.to_string_lossy().into_owned())
}

/// A program and every name it goes by, for the phone to weigh: the path it resolves to, the name typed, and each link
/// on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramNames {
    /// The absolute, canonical path (as typed when it could not be found).
    pub program: String,
    pub typed: String,
    /// The file name of each link followed from the name typed to the program, in order.
    pub aliases: Vec<String>,
    /// [`reins_proto::desktop::PATH_SAME`] / [`reins_proto::desktop::PATH_OTHER`]: whether `own_path` (the daemon's
    /// own) finds the same program for a bare name typed; `None` when a path was typed or it finds none.
    pub path_check: Option<String>,
}

impl ProgramNames {
    /// Whether the program was found (an absolute path): only then is it run.
    #[must_use]
    pub fn found(&self) -> bool {
        std::path::Path::new(&self.program).is_absolute()
    }
}

/// The most links followed.
const MAX_HOPS: usize = 40;

/// Resolves `typed` as [`resolve_program_with`] does with the caller's `path_var`, following each link to record its
/// name, and compares with `own_path`.
#[must_use]
pub fn program_names(
    typed: &str,
    cwd: &std::path::Path,
    path_var: Option<&str>,
    own_path: Option<&str>,
) -> ProgramNames {
    let found = found_program(typed, cwd, path_var);
    let mut aliases: Vec<String> = Vec::new();
    if let Some(start) = &found {
        let mut at = start.clone();
        for _ in 0..MAX_HOPS {
            if let Some(name) = at.file_name() {
                let name = name.to_string_lossy().into_owned();
                if !aliases.contains(&name) {
                    aliases.push(name);
                }
            }
            match std::fs::read_link(&at) {
                Ok(next) => at = at.parent().map_or(next.clone(), |dir| dir.join(&next)),
                Err(_) => break,
            }
        }
    }
    let program = found
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map_or_else(|| typed.to_owned(), |p| p.to_string_lossy().into_owned());
    let bare = std::path::Path::new(typed).components().count() == 1 && !std::path::Path::new(typed).is_absolute();
    let path_check =
        bare.then(|| found_program(typed, cwd, own_path)).flatten().and_then(|p| std::fs::canonicalize(p).ok()).map(
            |own| {
                if own.to_string_lossy() == program {
                    reins_proto::desktop::PATH_SAME.to_owned()
                } else {
                    reins_proto::desktop::PATH_OTHER.to_owned()
                }
            },
        );
    ProgramNames {
        program,
        typed: typed.to_owned(),
        aliases,
        path_check,
    }
}

fn found_program(program: &str, cwd: &std::path::Path, path_var: Option<&str>) -> Option<PathBuf> {
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
            typed: None,
            aliases: Vec::new(),
            path_check: None,
            exec_env: String::new(),
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
            one_shot: false,
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
            args: vec!["run".into(), "dev".into()],
            prefix: true,
        };
        assert!(check_env_grant(grant(&[("A", "x")], prefix), "n", &r, 100).is_ok(), "a wider scope the user chose");
        let refused = CommandScope {
            program: "/usr/bin/npm".into(),
            args: vec!["run".into()],
            prefix: true,
        };
        assert!(check_env_grant(grant(&[("A", "x")], refused), "n", &r, 100).is_err(), "not on the allow-list");
        let mut once = grant(&[("A", "x")], r.command());
        once.one_shot = true;
        assert!(check_env_grant(once, "n", &r, 100).unwrap().one_shot, "for this run only");
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
    fn an_imports_batch_maps_onto_one_store_and_its_check_digits() {
        use crate::secrets_import::{Batch, EnvWrite, SshWrite, TokenWrite};
        let batch = Batch {
            computer: "laptop".into(),
            envs: vec![EnvWrite {
                project: "acme".into(),
                env: "dev".into(),
                vars: vec![("A".into(), Zeroizing::new("1".into()))],
                origin: Some("github.com/acme/api".into()),
            }],
            ssh: vec![SshWrite {
                name: "key".into(),
                private_key: Zeroizing::new("PEM".into()),
            }],
            tokens: vec![TokenWrite {
                host: "github.com".into(),
                user: Some("octo".into()),
                token: Zeroizing::new("ghp".into()),
            }],
        };
        let (envs, ssh, tokens) = import_parts(&batch);
        assert!(envs[0].replace && envs[0].set == [("A".to_owned(), "1".to_owned())]);
        assert_eq!(envs[0].origin.as_deref(), Some("github.com/acme/api"));
        assert_eq!((ssh[0].name.as_str(), tokens[0].user.as_deref()), ("key", Some("octo")));
        reins_proto::desktop::validate_batch(&envs, &ssh, &tokens).unwrap();
        assert_eq!(import_check(&batch), reins_proto::desktop::batch_check(&envs, &ssh, &tokens));
    }

    #[test]
    #[cfg(unix)]
    fn every_name_on_the_way_to_a_program_is_recorded_and_the_phone_refuses_a_launcher_among_them() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let dir = tempfile::tempdir().unwrap();
        let d = std::fs::canonicalize(dir.path()).unwrap();
        // npm's own layout: bin/npm -> ../lib/node_modules/npm/bin/npm-cli.js
        let cli = d.join("lib/node_modules/npm/bin/npm-cli.js");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, "#!/usr/bin/env node").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(d.join("bin")).unwrap();
        symlink("../lib/node_modules/npm/bin/npm-cli.js", d.join("bin/npm")).unwrap();
        // A chain through a launcher's name: tools/deploy -> env -> npm-cli.js
        std::fs::create_dir_all(d.join("tools")).unwrap();
        symlink(&cli, d.join("tools/env")).unwrap();
        symlink(d.join("tools/env"), d.join("tools/deploy")).unwrap();
        let bin = d.join("bin").to_string_lossy().into_owned();
        let names = program_names("npm", &d, Some(&bin), Some(&bin));
        assert_eq!(names.program, cli.to_string_lossy());
        assert_eq!(names.aliases, ["npm", "npm-cli.js"]);
        assert_eq!(names.path_check.as_deref(), Some(reins_proto::desktop::PATH_SAME));
        let run = CommandScope::exact(&names.program, &["exec".to_owned(), "vite".to_owned()]);
        let all: Vec<&str> =
            std::iter::once(names.typed.as_str()).chain(names.aliases.iter().map(String::as_str)).collect();
        assert_eq!(run.offers_as(&all).len(), 1, "npm exec: exact only");
        let dev = CommandScope::exact(&names.program, &["run".to_owned(), "dev".to_owned()]);
        assert_eq!(dev.offers_as(&all).len(), 2, "npm run dev *");
        let tools = d.join("tools").to_string_lossy().into_owned();
        let linked = program_names("deploy", &d, Some(&tools), Some(&bin));
        assert_eq!(linked.aliases, ["deploy", "env", "npm-cli.js"]);
        assert_eq!(linked.path_check, None, "this computer's own PATH finds no deploy");
        let all: Vec<&str> =
            std::iter::once(linked.typed.as_str()).chain(linked.aliases.iter().map(String::as_str)).collect();
        assert_eq!(dev.offers_as(&all).len(), 1, "a launcher's name on the way: exact only");
        // Found elsewhere than this computer's own PATH finds it.
        let other = program_names("npm", &d, Some(&tools), Some(&bin));
        assert!(!other.found(), "the caller's PATH has no npm");
        symlink(&cli, d.join("tools/npm")).unwrap();
        std::fs::write(d.join("npm-other"), "x").unwrap();
        let shadow = d.join("shadow");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(shadow.join("npm"), "#!/bin/sh").unwrap();
        std::fs::set_permissions(shadow.join("npm"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let shadowed = program_names("npm", &d, Some(&shadow.to_string_lossy()), Some(&bin));
        assert_eq!(shadowed.path_check.as_deref(), Some(reins_proto::desktop::PATH_OTHER));
    }

    #[test]
    fn what_changes_the_program_is_named_left_out_and_bound() {
        let env =
            |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<Vec<_>>();
        let base = env(&[("PATH", "/usr/bin"), ("HOME", "/home/me")]);
        assert_eq!(exec_env_digest(&base), exec_env_digest(&env(&[("HOME", "/elsewhere"), ("PATH", "/usr/bin")])));
        for (k, v) in [
            ("PATH", "/tmp/x:/usr/bin"),
            ("NODE_OPTIONS", "--require=/tmp/x.js"),
            ("LD_PRELOAD", "/tmp/x.so"),
            ("DYLD_INSERT_LIBRARIES", "/tmp/x.dylib"),
            ("PYTHONPATH", "/tmp"),
            ("RUSTC_WRAPPER", "/tmp/x"),
            ("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER", "/tmp/x"),
            ("GIT_SSH_COMMAND", "/tmp/x"),
            ("BASH_ENV", "/tmp/x"),
            ("ENV", "/tmp/x"),
            ("npm_config_script_shell", "/tmp/sh"),
        ] {
            let mut changed = base.clone();
            changed.retain(|(n, _)| n != k);
            changed.push((k.to_owned(), v.to_owned()));
            assert_ne!(exec_env_digest(&base), exec_env_digest(&changed), "{k}");
            if k != "PATH" {
                let mut stripped = changed.clone();
                assert_eq!(strip_exec_env(&mut stripped), [k.to_owned()]);
                assert!(stripped.iter().any(|(n, _)| n == "PATH"), "PATH stays (it is bound instead)");
            }
        }
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
