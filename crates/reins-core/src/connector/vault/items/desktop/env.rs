//! Reins Secrets on the phone (docs/secrets.md): a project is the folder `Reins/<project>`, an environment the Secure
//! Note `<project>/<env>` in it, marked by the hidden custom field `reins.kind = env/1`, and its variables are the other
//! custom fields.
//!
//! - `env_release`: the variables of one environment for one command on the computer, sealed to the desktop app as a
//!   [`SecretGrant`] that names the environment, its version, and what the phone bound the release to: the runner, the
//!   project's folder on the computer and the command (exactly, or the prefix the user chose; see `flow`).
//! - `secrets_store`: changes to environments in a box only this phone opens (as `secret_store`), one approval, four
//!   check digits over the batch; the folder and the item are made when missing, and replaced or removed values go to
//!   the item's history (`NAME: value`, as the Bitwarden apps keep hidden fields).
//! - `secrets_list`: the projects, environments and variable names, never a value.

use std::collections::BTreeMap;

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::{
    CommandScope, ENV_KIND, ENV_KIND_FIELD, ENV_NAME_FIELD, EnvChanges, EnvListing, EnvStored, MAX_ENV_BYTES,
    MAX_ENV_VARS, PHONE_KEY_CHANGED, ProjectListing, RESERVED_PREFIX, SEALED_FIELD, SECRETS_FOLDER,
    SECRETS_STORE_TTL_SECS, SecretGrant, SecretsListing, SecretsToStore, SshKeyToStore, SshStored, TO_DESKTOP,
    TO_PHONE, batch_check, name_ok, validate_batch, var_ok,
};
use reqwest::Method;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::super::editor::{args_for, public_of_private};
use super::super::model::{Entry, Folder, Kind, Snapshot, State};
use super::super::write::{carry_out, plan_with, ssh_fingerprint};
use super::bad;
use super::store::{CLOCK_SKEW_SECS, Seen, seen_all, seen_save};
use crate::connector::sealed::{box_from_phone, client_key, nonce_arg, open_from, seal};
use crate::connector::vault::Vault;
use crate::connector::{Preview, SecretsPreview};
use crate::store::{Store, unix_now};
use crate::types::{CommandOption, EnvReleaseView, EnvStoreView, SecretsStoreView, SshStoreView, TokenStoreView};
use crate::vault_editor::{VaultFieldInput, VaultItemInput, VaultItemKind};
use crate::{CoreError, text};

/// The resource of a release (`secrets/<project>/<env>`) and of the names listed.
const SECRETS: &str = "secrets";
const NAMES: &str = "secrets_names";
const DEFAULT_LEASE: i64 = 3_600;
const MIN_LEASE: i64 = 60;
const MAX_LEASE: i64 = 86_400;
/// How many earlier values an environment keeps (the Bitwarden apps keep five passwords; an environment holds more).
const MAX_HISTORY: usize = 100;
/// Where the user's own choice of "always ask" per environment is kept (a `meta` key): `project/env` → bool.
const ALWAYS_ASK: &str = "secrets.always-ask";
/// Environments asked every time unless the user says otherwise on the phone.
const PRODUCTION: [&str; 2] = ["production", "prod"];

// ---- environments in the vault --------------------------------------------------------------------------------------

/// `Reins/<project>`.
fn folder_name(project: &str) -> String {
    format!("{SECRETS_FOLDER}/{project}")
}

/// The folder of a project, by its exact name.
fn project_folder<'a>(snap: &'a Snapshot, project: &str) -> Result<Option<&'a str>, CoreError> {
    let name = folder_name(project);
    let mut same = snap.folders.iter().filter(|f| f.name == name);
    match (same.next(), same.next()) {
        (None, _) => Ok(None),
        (Some(f), None) => Ok(Some(f.id.as_str())),
        _ => Err(bad(format!("Several vault folders are named {name}. Rename one in the Reins app."))),
    }
}

/// The environment an item is marked as: a Secure Note with the hidden fields `reins.kind = env/1` and
/// `reins.env = <name>`. Its name in the vault is only shown: an item is never an environment for its name alone.
pub(super) fn env_mark(entry: &Entry) -> Option<String> {
    if entry.kind != Kind::Note {
        return None;
    }
    let fields = entry.custom_fields();
    let kind_ok = fields.iter().any(|c| c.name == ENV_KIND_FIELD && *c.value == *ENV_KIND);
    let env = fields.iter().find(|c| c.name == ENV_NAME_FIELD).map(|c| c.value.to_string())?;
    (kind_ok && name_ok(&env)).then_some(env)
}

/// The project and environment an item is, when it is an environment: marked, in the folder `Reins/<project>`.
pub(super) fn env_identity(snap: &Snapshot, entry: &Entry) -> Option<(String, String)> {
    let env = env_mark(entry)?;
    let folder = snap.folders.iter().find(|f| f.id == entry.folder())?;
    let project = folder.name.strip_prefix(&format!("{SECRETS_FOLDER}/")).filter(|p| name_ok(p))?;
    Some((project.to_owned(), env))
}

/// The environment `project/env`, if it exists: the item marked as `env` in the folder `Reins/<project>` (exactly that
/// folder, by its exact name). An unmarked item named `project/env` there is never used or changed: it is something of
/// the user's own, and saving an environment of that name is refused until it is renamed.
fn find_env<'a>(snap: &'a Snapshot, project: &str, env: &str) -> Result<Option<&'a Entry>, CoreError> {
    let Some(folder) = project_folder(snap, project)? else {
        return Ok(None);
    };
    let here = |e: &&Entry| e.state() != State::Trash && e.folder() == folder;
    let mut marked = snap.entries.iter().filter(here).filter(|e| env_mark(e).as_deref() == Some(env));
    match (marked.next(), marked.next()) {
        (Some(e), None) => return Ok(Some(e)),
        (Some(_), Some(_)) => {
            return Err(bad(format!("Two items are the environment {project}/{env}. Delete one in the Reins app.")));
        }
        (None, _) => {}
    }
    let item = format!("{project}/{env}");
    if snap.entries.iter().filter(here).any(|e| e.name() == item) {
        return Err(bad(format!(
            "{item} in the vault is an item of your own, not a Reins environment. Rename it in the Reins app."
        )));
    }
    Ok(None)
}

/// The variables of an environment: its custom fields but Reins's own and those whose names are not variable names,
/// sorted by name; and how many were left out for their names.
fn variables(entry: &Entry) -> (Vec<(String, Zeroizing<String>)>, usize) {
    let mut vars = Vec::new();
    let mut skipped = 0;
    for c in entry.custom_fields() {
        if c.name.starts_with(RESERVED_PREFIX) {
            continue;
        }
        if var_ok(&c.name) {
            vars.push((c.name, c.value));
        } else {
            skipped += 1;
        }
    }
    vars.sort_by(|a, b| a.0.cmp(&b.0));
    (vars, skipped)
}

fn revision(entry: &Entry) -> String {
    entry.raw["revisionDate"].as_str().unwrap_or_default().to_owned()
}

/// Every environment in the vault: `(project, env, item)`, by project and environment.
fn all_envs(snap: &Snapshot) -> Vec<(String, String, &Entry)> {
    let mut out: Vec<(String, String, &Entry)> = snap
        .entries
        .iter()
        .filter(|e| e.state() == State::Active)
        .filter_map(|e| env_identity(snap, e).map(|(p, env)| (p, env, e)))
        .collect();
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

// ---- the user's "always ask" ----------------------------------------------------------------------------------------

fn always_ask_choices(store: &Store) -> Result<BTreeMap<String, bool>, CoreError> {
    Ok(store.meta_get(ALWAYS_ASK)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

/// The user's own choice for `project/env` on the phone, if any.
pub(crate) fn always_ask_choice(store: &Store, project: &str, env: &str) -> Result<Option<bool>, CoreError> {
    Ok(always_ask_choices(store)?.get(&format!("{project}/{env}")).copied())
}

/// Sets (or with `None` forgets) the user's choice of asking every time for `project/env`.
pub(crate) fn set_always_ask(store: &Store, project: &str, env: &str, always: Option<bool>) -> Result<(), CoreError> {
    if !name_ok(project) || !name_ok(env) {
        return Err(CoreError::invalid("not a project and an environment"));
    }
    let mut all = always_ask_choices(store)?;
    let key = format!("{project}/{env}");
    match always {
        Some(a) => all.insert(key, a),
        None => all.remove(&key),
    };
    store.meta_set(ALWAYS_ASK, &serde_json::to_string(&all).map_err(|_| CoreError::storage("cannot record it"))?)
}

/// Whether an environment is asked every time: the user's choice on the phone, else `confirm = "always"` in the
/// repository or a production environment. The repository alone can make it stricter, never looser.
fn always_ask(choice: Option<bool>, confirm_always: bool, env: &str) -> bool {
    choice.unwrap_or(confirm_always || PRODUCTION.contains(&env.to_ascii_lowercase().as_str()))
}

// ---- release --------------------------------------------------------------------------------------------------------

/// What the computer asks to have released, checked.
pub(crate) struct ReleaseAsk {
    pub project: String,
    pub env: String,
    /// `None`: every variable.
    pub fields: Option<Vec<String>>,
    pub program: String,
    pub args: Vec<String>,
    pub root: String,
    pub runner: String,
    pub lease: i64,
    pub purpose: Option<String>,
    pub confirm_always: bool,
    pub print: bool,
    /// The program as typed (`npm`), when it was not a path.
    pub typed: Option<String>,
    /// [`reins_proto::desktop::PATH_SAME`] or [`reins_proto::desktop::PATH_OTHER`], when the computer said.
    pub path_check: Option<String>,
}

impl ReleaseAsk {
    /// The exact command asked for.
    pub fn exact(&self) -> CommandScope {
        CommandScope::exact(&self.program, &self.args)
    }

    /// The command as shown: with the program's whole path when it is not in a usual place.
    pub fn shown(&self) -> String {
        self.exact().shown_where()
    }

    /// Why the program deserves a second look, if it does ([`reins_proto::desktop::program_warning`]).
    pub fn program_warning(&self) -> Option<&'static str> {
        reins_proto::desktop::program_warning(&self.program, &self.root, self.path_check.as_deref())
    }

    /// Whether this release is asked every time: `always`, a value to print, or a command that cannot be remembered.
    pub fn one_shot(&self, always: bool) -> bool {
        always || self.print || self.command_options().is_empty()
    }

    /// What a standing permission compares (see `reins_policy::SecretsScope`).
    pub fn facts(&self) -> reins_policy::SecretsFacts<'_> {
        reins_policy::SecretsFacts {
            project: &self.project,
            env: &self.env,
            runner: &self.runner,
            root: &self.root,
            program: &self.program,
            args: &self.args,
            fields: self.fields.as_deref(),
        }
    }

    /// The commands a remembered approval may cover, by id: `exact`, then `prefix:<n>` for each allowed prefix.
    pub fn command_options(&self) -> Vec<(String, CommandScope)> {
        self.exact()
            .offers_as(&self.typed.iter().map(String::as_str).collect::<Vec<_>>())
            .into_iter()
            .map(|c| {
                (
                    if c.prefix {
                        format!("prefix:{}", c.args.len())
                    } else {
                        "exact".to_owned()
                    },
                    c,
                )
            })
            .collect()
    }
}

/// Reads and checks a release's arguments.
pub(crate) fn release_ask(call: &ConnectorCall) -> Result<ReleaseAsk, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let project = call.str_arg("project").unwrap_or_default().to_owned();
    let env = call.str_arg("env").unwrap_or_default().to_owned();
    if !name_ok(&project) || !name_ok(&env) {
        return Err(bad("`project` and `env` must be project and environment names."));
    }
    let fields = if call.args.contains_key("fields") {
        let list: Vec<String> = call.list_arg("fields").iter().map(|f| (*f).to_owned()).collect();
        if list.is_empty() || list.len() > MAX_ENV_VARS || !list.iter().all(|f| var_ok(f)) {
            return Err(bad(format!("`fields` names 1 to {MAX_ENV_VARS} variables.")));
        }
        let mut sorted = list;
        sorted.sort();
        sorted.dedup();
        Some(sorted)
    } else {
        None
    };
    let program = call.str_arg("program").unwrap_or_default().to_owned();
    let args = reins_proto::connector::env_release_args(call).map_err(bad)?;
    CommandScope::exact(&program, &args).validate().map_err(bad)?;
    let root = call.str_arg("root").unwrap_or_default().to_owned();
    if root.is_empty() {
        return Err(bad("`root` must name the project's folder."));
    }
    let runner = call.str_arg("runner").unwrap_or_default().to_owned();
    if !reins_proto::desktop::runner_ok(&runner) {
        return Err(bad("`runner` must be person or agent:<harness>."));
    }
    let lease = call.int_arg("lease_secs").unwrap_or(DEFAULT_LEASE);
    if !(MIN_LEASE..=MAX_LEASE).contains(&lease) {
        return Err(bad(format!("`lease_secs` must be {MIN_LEASE} to {MAX_LEASE}.")));
    }
    Ok(ReleaseAsk {
        project,
        env,
        fields,
        program,
        args,
        root,
        runner,
        lease,
        purpose: call.str_arg("purpose").map(text::one_line).filter(|p| !p.is_empty()),
        confirm_always: call.str_arg("confirm") == Some("always"),
        print: call.bool_arg("print").unwrap_or(false),
        typed: call.str_arg("typed").map(text::one_line).filter(|t| !t.is_empty()),
        path_check: call.str_arg("path_check").map(str::to_owned),
    })
}

/// "You", or the agent's name.
pub(crate) fn runner_label(runner: &str) -> String {
    let Some(harness) = runner.strip_prefix("agent:") else {
        return "You".to_owned();
    };
    match harness {
        "claude-code" | "claude" => "Claude Code".to_owned(),
        "codex" => "Codex".to_owned(),
        "gemini" | "gemini-cli" => "Gemini CLI".to_owned(),
        "cursor" => "Cursor".to_owned(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// The values to release, in name order: every variable, or those asked for (each must exist).
fn chosen(entry: &Entry, ask: &ReleaseAsk) -> Result<Vec<(String, Zeroizing<String>)>, CoreError> {
    let (vars, _) = variables(entry);
    let picked: Vec<(String, Zeroizing<String>)> = match &ask.fields {
        None => vars,
        Some(wanted) => {
            let missing: Vec<&str> =
                wanted.iter().filter(|w| !vars.iter().any(|(n, _)| n == *w)).map(String::as_str).collect();
            if !missing.is_empty() {
                return Err(bad(format!(
                    "{}/{} has no {}. Set it with reins secrets set {}/NAME.",
                    ask.project,
                    ask.env,
                    missing.join(", "),
                    ask.env
                )));
            }
            vars.into_iter().filter(|(n, _)| wanted.contains(n)).collect()
        }
    };
    if picked.len() > MAX_ENV_VARS {
        return Err(bad(format!("At most {MAX_ENV_VARS} variables are released at once.")));
    }
    if picked.iter().map(|(n, v)| n.len() + v.len()).sum::<usize>() > MAX_ENV_BYTES {
        return Err(bad(format!("At most {} KB of values are released at once.", MAX_ENV_BYTES / 1024)));
    }
    Ok(picked)
}

fn env_entry<'a>(snap: &'a Snapshot, ask: &ReleaseAsk) -> Result<&'a Entry, CoreError> {
    find_env(snap, &ask.project, &ask.env)?.ok_or_else(|| {
        bad(format!(
            "No environment {}/{} in the vault. Save one with reins secrets set {}/NAME.",
            ask.project, ask.env, ask.env
        ))
    })
}

pub(super) async fn preview_release(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let ask = release_ask(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let entry = env_entry(&snap, &ask)?;
    let names: Vec<String> = chosen(entry, &ask)?.into_iter().map(|(n, _)| n).collect();
    let choice = always_ask_choice(&vault.store, &ask.project, &ask.env)?;
    let always = always_ask(choice, ask.confirm_always, &ask.env);
    let command = ask.shown();
    let warning = ask.program_warning();
    let one_shot = ask.one_shot(always);
    let who = runner_label(&ask.runner);
    let mut lines = vec![format!("{who}: {} for {command}", ask.env)];
    if let Some(w) = warning {
        lines.push(format!("{w}: {}", text::one_line(&ask.program)));
    }
    lines.push(format!("{}/{}: {}", ask.project, ask.env, names.join(", ")));
    lines.push(format!("In {}", text::one_line(&ask.root)));
    if let Some(p) = &ask.purpose {
        lines.push(format!("Why: {p}"));
    }
    lines.push(if ask.print {
        "Printed in the terminal".to_owned()
    } else {
        format!("Kept for {}", crate::views::duration_text(u64::try_from(ask.lease).unwrap_or(0)))
    });
    let view = EnvReleaseView {
        project: ask.project.clone(),
        env: ask.env.clone(),
        runner: ask.runner.clone(),
        runner_label: who,
        agent: ask.runner != reins_proto::desktop::PERSON,
        root: text::one_line(&ask.root),
        command,
        program: text::one_line(&ask.program),
        program_warning: warning.map(str::to_owned),
        variables: names,
        all_variables: ask.fields.is_none(),
        lease_secs: u64::try_from(ask.lease).unwrap_or(0),
        purpose: ask.purpose.clone(),
        commands: ask
            .command_options()
            .into_iter()
            .map(|(id, c)| CommandOption {
                id,
                label: c.shown_where(),
                prefix: c.prefix,
            })
            .collect(),
        print: ask.print,
        always_ask: one_shot,
        always_ask_set: choice.is_some(),
    };
    Ok(Preview {
        resource: format!("{SECRETS}/{}/{}", ask.project, ask.env),
        resource_label: format!("{} \u{b7} {}", ask.project, ask.env),
        lines,
        parents: vec![(format!("{SECRETS}/{}", ask.project), ask.project.clone())],
        once_only: one_shot,
        secrets: Some(SecretsPreview::Release(view)),
        ..Preview::default()
    })
}

/// Releases the environment, bound to `command`: the exact command, or the prefix the user chose on the phone (the
/// flow decides; never anything the computer proposed).
pub(crate) async fn perform_release(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
    command: CommandScope,
) -> Result<Value, CoreError> {
    let ask = release_ask(call)?;
    if !ask.exact().within(&command) {
        return Err(bad("The release does not match what was approved."));
    }
    // A fresh read: the values are what the vault holds now.
    let snap = Snapshot::load(vault, account).await?;
    let entry = env_entry(&snap, &ask)?;
    let values = chosen(entry, &ask)?;
    let expires_at = unix_now().saturating_add(ask.lease);
    let grant = SecretGrant {
        v: 1,
        nonce: nonce_arg(call)?,
        secrets: values.iter().map(|(n, v)| (n.clone(), v.to_string())).collect(),
        expires_at,
        project: Some(ask.project.clone()),
        env: Some(ask.env.clone()),
        revision: Some(revision(entry)),
        runner: Some(ask.runner.clone()),
        root: Some(ask.root.clone()),
        command: Some(command),
    };
    let sealed = seal(client_key(call)?, &grant);
    for (_, value) in grant.secrets {
        drop(Zeroizing::new(value));
    }
    Ok(json!({ SEALED_FIELD: sealed?, "expires_at": expires_at }))
}

// ---- store ----------------------------------------------------------------------------------------------------------

/// One custom field of the environment being changed: kept as the vault has it, or a new value (or name) to encrypt.
enum Slot {
    Raw {
        name: String,
        json: Value,
        /// The value, for the history and for copies.
        value: Zeroizing<String>,
    },
    New {
        name: String,
        value: Zeroizing<String>,
    },
}

impl Slot {
    fn name(&self) -> &str {
        match self {
            Self::Raw {
                name,
                ..
            }
            | Self::New {
                name,
                ..
            } => name,
        }
    }

    fn value(&self) -> &str {
        match self {
            Self::Raw {
                value,
                ..
            }
            | Self::New {
                value,
                ..
            } => value,
        }
    }
}

/// What one environment's changes come to.
struct EnvPlan {
    project: String,
    env: String,
    /// The folder's id, when it exists.
    folder: Option<String>,
    /// The item's id and the raw item, when it exists.
    entry: Option<(String, Value)>,
    /// The item's key (its own, or the user key).
    slots: Vec<Slot>,
    /// Earlier values for the history: `NAME: value`.
    history: Vec<Zeroizing<String>>,
    view: EnvStoreView,
    /// What the approval is about: the folder and the item as they are now.
    target: String,
}

/// Keeps an earlier value for the item's history, as the Bitwarden apps keep a hidden field's: `NAME: value`.
fn keep_old(name: &str, old: &str, history: &mut Vec<Zeroizing<String>>) {
    if !old.is_empty() {
        history.push(Zeroizing::new(format!("{name}: {old}")));
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn plan_env(snap: &Snapshot, change: &EnvChanges) -> Result<EnvPlan, CoreError> {
    let item = change.item();
    let folder = project_folder(snap, &change.project)?.map(str::to_owned);
    let entry = find_env(snap, &change.project, &change.env)?;
    let mut slots: Vec<Slot> = Vec::new();
    if let Some(e) = entry {
        let raw = e.raw["fields"].as_array().cloned().unwrap_or_default();
        for (json, slot) in raw.into_iter().zip(e.custom_slots()) {
            let (name, value) =
                slot.map_or_else(|| (String::new(), Zeroizing::new(String::new())), |c| (c.name, c.value));
            slots.push(Slot::Raw {
                name,
                json,
                value,
            });
        }
    } else {
        slots.push(Slot::New {
            name: ENV_KIND_FIELD.to_owned(),
            value: Zeroizing::new(ENV_KIND.to_owned()),
        });
        slots.push(Slot::New {
            name: ENV_NAME_FIELD.to_owned(),
            value: Zeroizing::new(change.env.clone()),
        });
    }
    let position = |slots: &[Slot], name: &str| slots.iter().position(|s| s.name() == name);
    let mut view = EnvStoreView {
        project: change.project.clone(),
        env: change.env.clone(),
        new_env: entry.is_none(),
        added: Vec::new(),
        replaced: Vec::new(),
        removed: Vec::new(),
        renamed: Vec::new(),
        copied_from: change.copy_from.clone(),
        copied: 0,
    };
    let mut history: Vec<Zeroizing<String>> = Vec::new();
    if let Some(from) = &change.copy_from {
        let source = find_env(snap, &change.project, from)?
            .ok_or_else(|| bad(format!("No environment {}/{from} to copy from.", change.project)))?;
        let (vars, _) = variables(source);
        for (name, value) in vars {
            if change.set.iter().any(|(n, _)| *n == name) {
                continue;
            }
            match position(&slots, &name) {
                Some(i) => {
                    if slots[i].value() != value.as_str() {
                        keep_old(&name, slots[i].value(), &mut history);
                        slots[i] = Slot::New {
                            name: name.clone(),
                            value,
                        };
                    }
                }
                None => slots.push(Slot::New {
                    name,
                    value,
                }),
            }
            view.copied += 1;
        }
    }
    for (from, to) in &change.rename {
        let i = position(&slots, from).ok_or_else(|| bad(format!("{item} has no {from} to rename.")))?;
        if position(&slots, to).is_some() {
            return Err(bad(format!("{item} has a {to} already.")));
        }
        let value = Zeroizing::new(slots[i].value().to_owned());
        slots[i] = Slot::New {
            name: to.clone(),
            value,
        };
        view.renamed.push(format!("{from} \u{2192} {to}"));
    }
    for name in &change.unset {
        let i = position(&slots, name).ok_or_else(|| bad(format!("{item} has no {name} to remove.")))?;
        keep_old(name, slots[i].value(), &mut history);
        slots.remove(i);
        view.removed.push(name.clone());
    }
    for (name, value) in &change.set {
        if value.is_empty() {
            return Err(bad(format!("{name} has no value.")));
        }
        match position(&slots, name) {
            Some(i) if slots[i].value() == value.as_str() => {}
            Some(i) => {
                if !change.replace {
                    return Err(bad(format!(
                        "{item} has a {name} already. Replace it with reins secrets set {}/{name}.",
                        change.env
                    )));
                }
                keep_old(name, slots[i].value(), &mut history);
                slots[i] = Slot::New {
                    name: name.clone(),
                    value: Zeroizing::new(value.clone()),
                };
                view.replaced.push(name.clone());
            }
            None => {
                slots.push(Slot::New {
                    name: name.clone(),
                    value: Zeroizing::new(value.clone()),
                });
                view.added.push(name.clone());
            }
        }
    }
    let vars = slots.iter().filter(|s| !s.name().starts_with(RESERVED_PREFIX) && !s.name().is_empty()).count();
    if vars > MAX_ENV_VARS {
        return Err(bad(format!("An environment holds at most {MAX_ENV_VARS} variables; {item} would have {vars}.")));
    }
    let target = format!(
        "{}|{}",
        folder.as_deref().unwrap_or("new"),
        entry.map_or_else(|| "new".to_owned(), |e| format!("{}@{}", e.id, revision(e)))
    );
    Ok(EnvPlan {
        project: change.project.clone(),
        env: change.env.clone(),
        folder,
        entry: entry.map(|e| (e.id.clone(), e.raw.clone())),
        slots,
        history,
        view,
        target,
    })
}

/// Opens the box this phone was sent and checks it answers this very call.
fn open_store(vault: &Vault, call: &ConnectorCall, now: i64) -> Result<SecretsToStore, CoreError> {
    let desktop = client_key(call)?;
    let nonce = nonce_arg(call)?;
    let unreadable = || bad(format!("{PHONE_KEY_CHANGED} If this is a new phone, run reins vault list --new-phone."));
    let plain = open_from(desktop, &*vault.store.inbox_key()?, call.str_arg("sealed").unwrap_or_default())
        .ok_or_else(unreadable)?;
    let opened: SecretsToStore = serde_json::from_slice(&plain).map_err(|_| unreadable())?;
    if opened.dir != TO_PHONE || opened.nonce != nonce {
        return Err(bad("The secrets were sealed for another request; refused."));
    }
    if opened.created_at < now - SECRETS_STORE_TTL_SECS || opened.created_at > now + CLOCK_SKEW_SECS {
        return Err(bad("The secrets were sealed too long ago (or the computer's clock is wrong); refused."));
    }
    let boxed: Vec<String> = opened.envs.iter().map(EnvChanges::item).collect();
    let keys: Vec<&str> = opened.ssh.iter().map(|k| k.name.trim()).collect();
    let hosts: Vec<&str> = opened.tokens.iter().map(|t| t.host.as_str()).collect();
    if call.list_arg("envs") != boxed || call.list_arg("ssh") != keys || call.list_arg("hosts") != hosts {
        return Err(bad("The secrets were sealed for other environments, keys or hosts; refused."));
    }
    validate_batch(&opened.envs, &opened.ssh, &opened.tokens).map_err(bad)?;
    Ok(opened)
}

const ALREADY: &str = "This request was seen already; a request is never shown or saved twice. Run it again.";

/// Where imported SSH keys go.
pub(crate) const SSH_FOLDER: &str = "Reins/ssh";

/// The SSH keys of a store: the folder (when it exists), each key's name and fingerprint, and what the approval is
/// about (the folder, and that no key of those names is there yet).
struct SshPlan {
    folder: Option<String>,
    keys: Vec<SshStoreView>,
    target: String,
}

fn plan_ssh(snap: &Snapshot, keys: &[SshKeyToStore]) -> Result<SshPlan, CoreError> {
    let mut same = snap.folders.iter().filter(|f| f.name == SSH_FOLDER);
    let folder = match (same.next(), same.next()) {
        (None, _) => None,
        (Some(f), None) => Some(f.id.clone()),
        _ => return Err(bad(format!("Several vault folders are named {SSH_FOLDER}. Rename one in the Reins app."))),
    };
    let mut views = Vec::new();
    for k in keys {
        let name = text::one_line(k.name.trim());
        let public = public_of_private(&k.private_key)
            .map_err(|_| bad(format!("{name} is not an SSH private key without a passphrase.")))?;
        if snap.entries.iter().any(|e| {
            e.state() != State::Trash
                && e.kind == Kind::SshKey
                && folder.as_deref() == Some(e.folder())
                && e.name() == name
        }) {
            return Err(bad(format!("An SSH key named {name} is in {SSH_FOLDER} already.")));
        }
        views.push(SshStoreView {
            fingerprint: ssh_fingerprint(&public).unwrap_or_default(),
            name,
        });
    }
    Ok(SshPlan {
        target: format!("ssh:{}", folder.as_deref().unwrap_or("new")),
        folder,
        keys: views,
    })
}

fn store_view(plans: &[EnvPlan], ssh: &SshPlan, opened: &SecretsToStore) -> SecretsStoreView {
    let vars: usize = opened.envs.iter().map(|e| e.set.len()).sum();
    SecretsStoreView {
        envs: plans.iter().map(|p| p.view.clone()).collect(),
        check: batch_check(&opened.envs, &opened.ssh, &opened.tokens),
        total: count(vars + opened.ssh.len() + opened.tokens.len()),
        ssh: ssh.keys.clone(),
        tokens: opened
            .tokens
            .iter()
            .map(|t| TokenStoreView {
                service: t.service().unwrap_or_default().to_owned(),
                host: t.host.to_ascii_lowercase(),
                scopes: t.scopes.iter().map(|x| text::one_line(x)).take(30).collect(),
                check: String::new(),
            })
            .collect(),
    }
}

fn plan_all(snap: &Snapshot, envs: &[EnvChanges]) -> Result<Vec<EnvPlan>, CoreError> {
    envs.iter().map(|e| plan_env(snap, e)).collect()
}

fn store_target(plans: &[EnvPlan], ssh: &SshPlan) -> String {
    plans.iter().map(|p| p.target.as_str()).chain(std::iter::once(ssh.target.as_str())).collect::<Vec<_>>().join(";")
}

pub(super) async fn preview_store(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let now = unix_now();
    let opened = open_store(vault, call, now)?;
    let mut seen = seen_all(&vault.store, now)?;
    if seen.contains_key(&opened.nonce) {
        return Err(bad(ALREADY));
    }
    let snap = Snapshot::load(vault, account).await?;
    let plans = plan_all(&snap, &opened.envs)?;
    let ssh = plan_ssh(&snap, &opened.ssh)?;
    let view = store_view(&plans, &ssh, &opened);
    let mut lines = vec![format!(
        "Save {} in your vault",
        match view.total {
            1 => "1 secret".to_owned(),
            n => format!("{n} secrets"),
        }
    )];
    for p in &plans {
        let v = &p.view;
        let mut parts = Vec::new();
        for (n, word) in [(v.added.len(), "new"), (v.replaced.len(), "replaced"), (v.removed.len(), "removed")] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        if !v.renamed.is_empty() {
            parts.push(format!("{} renamed", v.renamed.len()));
        }
        if let Some(from) = &v.copied_from {
            parts.push(format!("{} copied from {from}", v.copied));
        }
        let new = if v.new_env {
            " (new)"
        } else {
            ""
        };
        lines.push(format!("{}/{}{new}: {}", p.project, p.env, parts.join(", ")));
    }
    for k in &view.ssh {
        lines.push(format!("SSH key {} ({}) in {SSH_FOLDER}", k.name, k.fingerprint));
    }
    for t in &view.tokens {
        lines.push(format!("A {} token for the phone's integration", t.host));
    }
    lines.push(format!("Check: {}. The computer shows the same four digits; deny if it does not.", view.check));
    if plans.iter().any(|p| !p.history.is_empty()) {
        lines.push("Earlier values are kept in each environment's history".to_owned());
    }
    seen.insert(
        opened.nonce.clone(),
        Seen {
            target: store_target(&plans, &ssh),
            until: opened.created_at + SECRETS_STORE_TTL_SECS,
            done: false,
        },
    );
    seen_save(&vault.store, &seen)?;
    let (resource, label) = match (plans.as_slice(), opened.ssh.is_empty() && opened.tokens.is_empty()) {
        ([one], true) => {
            (format!("{SECRETS}/{}/{}", one.project, one.env), format!("{} \u{b7} {}", one.project, one.env))
        }
        _ => (SECRETS.to_owned(), "Reins Secrets".to_owned()),
    };
    Ok(Preview {
        resource,
        resource_label: label,
        lines,
        once_only: true,
        secrets: Some(SecretsPreview::Store(view)),
        ..Preview::default()
    })
}

fn field_json(key: &crate::crypto::VaultKey, name: &str, value: &str) -> Result<Value, CoreError> {
    Ok(json!({"name": key.encrypt(name.as_bytes())?, "value": key.encrypt(value.as_bytes())?, "type": 1,
        "linkedId": null}))
}

/// Carries out one environment's plan; its new revision.
async fn carry_out_env(
    vault: &Vault,
    snap: &Snapshot,
    plan: EnvPlan,
    folders: &mut BTreeMap<String, String>,
) -> Result<(EnvStored, String), CoreError> {
    let session = &snap.session;
    let folder = if let Some(id) = plan.folder.clone().or_else(|| folders.get(&plan.project).cloned()) {
        id
    } else {
        let name = folder_name(&plan.project);
        let made = vault
            .api(session, Method::POST, "/api/folders", Some(&json!({"name": snap.user.encrypt(name.as_bytes())?})))
            .await?;
        let id = made["id"].as_str().ok_or_else(|| bad("The vault did not say what folder it made."))?.to_owned();
        folders.insert(plan.project.clone(), id.clone());
        id
    };
    // The item's own key when it has one, else the user key: what its strings are encrypted with.
    let key = match &plan.entry {
        Some((_, raw)) => match raw["key"].as_str() {
            Some(own) => crate::crypto::VaultKey::from_bytes(&snap.user.decrypt(own)?)?,
            None => crate::crypto::VaultKey::from_bytes(&snap.user.to_bytes())?,
        },
        None => crate::crypto::VaultKey::from_bytes(&snap.user.to_bytes())?,
    };
    let mut fields = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        fields.push(match slot {
            Slot::Raw {
                json,
                ..
            } => json.clone(),
            Slot::New {
                name,
                value,
            } => field_json(&key, name, value)?,
        });
    }
    let now = text::iso_utc(unix_now());
    let mut history: Vec<Value> = Vec::new();
    for old in &plan.history {
        history.push(json!({"password": key.encrypt(old.as_bytes())?, "lastUsedDate": now}));
    }
    let item = format!("{}/{}", plan.project, plan.env);
    let answer = match &plan.entry {
        None => {
            let body = json!({
                "type": Kind::Note.code(),
                "folderId": folder,
                "organizationId": null,
                "key": null,
                "name": key.encrypt(item.as_bytes())?,
                "notes": null,
                "favorite": false,
                "reprompt": 0,
                "fields": fields,
                "passwordHistory": null,
                "archivedDate": null,
                "secureNote": {"type": 0},
            });
            vault.api(session, Method::POST, "/api/ciphers", Some(&body)).await?
        }
        Some((id, raw)) => {
            let mut kept = raw["passwordHistory"].as_array().cloned().unwrap_or_default();
            history.append(&mut kept);
            history.truncate(MAX_HISTORY);
            let body = json!({
                "id": id,
                "type": Kind::Note.code(),
                "folderId": raw["folderId"].clone(),
                "organizationId": null,
                "key": raw["key"].clone(),
                "name": raw["name"].clone(),
                "notes": raw["notes"].clone(),
                "favorite": raw["favorite"].as_bool().unwrap_or(false),
                "reprompt": raw["reprompt"].as_i64().unwrap_or(0),
                "fields": fields,
                "passwordHistory": history,
                "archivedDate": raw["archivedDate"].clone(),
                "lastKnownRevisionDate": raw["revisionDate"].clone(),
                "secureNote": if raw["secureNote"].is_object() { raw["secureNote"].clone() } else { json!({"type": 0}) },
            });
            let path = format!("/api/ciphers/{}", crate::connector::calendar::segment(id));
            vault.api(session, Method::PUT, &path, Some(&body)).await?
        }
    };
    let revision = answer["revisionDate"].as_str().unwrap_or_default().to_owned();
    let v = &plan.view;
    Ok((
        EnvStored {
            project: plan.project.clone(),
            env: plan.env.clone(),
            created: plan.entry.is_none(),
            added: count(v.added.len()),
            replaced: count(v.replaced.len()),
            removed: count(v.removed.len()),
            renamed: count(v.renamed.len()),
            copied: v.copied,
            revision: revision.clone(),
            error: None,
        },
        revision,
    ))
}

/// Keeps one SSH key in the folder `Reins/ssh` (made when missing); its fingerprint.
async fn carry_out_ssh(
    vault: &Vault,
    snap: &mut Snapshot,
    folder: &mut Option<String>,
    key: &SshKeyToStore,
) -> Result<String, CoreError> {
    if folder.is_none() {
        let made = vault
            .api(
                &snap.session,
                Method::POST,
                "/api/folders",
                Some(&json!({"name": snap.user.encrypt(SSH_FOLDER.as_bytes())?})),
            )
            .await?;
        let id = made["id"].as_str().ok_or_else(|| bad("The vault did not say what folder it made."))?.to_owned();
        // The vault's own checks look the folder up.
        snap.folders.push(Folder {
            id: id.clone(),
            name: SSH_FOLDER.to_owned(),
        });
        *folder = Some(id);
    }
    let input = VaultItemInput {
        kind: VaultItemKind::Login,
        name: text::one_line(key.name.trim()),
        fields: vec![VaultFieldInput {
            key: "private_key".to_owned(),
            value: key.private_key.clone(),
        }],
    };
    let mut args = args_for(Kind::SshKey, &input, None)?;
    args.insert("folder".to_owned(), json!(folder.clone().unwrap_or_default()));
    let call = ConnectorCall {
        service: reins_proto::connector::VAULT.to_owned(),
        op: "item_create".to_owned(),
        args,
    };
    let fingerprint = ssh_fingerprint(&public_of_private(&key.private_key)?).unwrap_or_default();
    carry_out(vault, &snap.session, plan_with(snap, &call)?).await?;
    Ok(fingerprint)
}

/// Carries out an approved store's environments and SSH keys (the flow adds the tokens and answers): what was opened,
/// and a result per environment and key. A part that fails says why; the others are still saved.
pub(crate) async fn perform_store_parts(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
) -> Result<(SecretsToStore, Vec<EnvStored>, Vec<SshStored>), CoreError> {
    let now = unix_now();
    let opened = open_store(vault, call, now)?;
    let mut seen = seen_all(&vault.store, now)?;
    let Some(asked) = seen.get(&opened.nonce).filter(|s| !s.done).cloned() else {
        return Err(bad(ALREADY));
    };
    let mut snap = Snapshot::load(vault, account).await?;
    let plans = plan_all(&snap, &opened.envs)?;
    let ssh = plan_ssh(&snap, &opened.ssh)?;
    // What happens must be what the user approved: the same folders and items, unchanged since.
    if store_target(&plans, &ssh) != asked.target {
        return Err(bad("The vault changed since you were asked; nothing was saved. Run it again."));
    }
    seen.insert(
        opened.nonce.clone(),
        Seen {
            done: true,
            ..asked
        },
    );
    seen_save(&vault.store, &seen)?;
    let mut folders = BTreeMap::new();
    let mut stored = Vec::new();
    for plan in plans {
        let (project, env) = (plan.project.clone(), plan.env.clone());
        stored.push(match carry_out_env(vault, &snap, plan, &mut folders).await {
            Ok((env, _)) => env,
            Err(e) => EnvStored {
                project,
                env,
                error: Some(e.to_string()),
                ..EnvStored::default()
            },
        });
    }
    let mut keys = Vec::new();
    let mut folder = ssh.folder.clone();
    for key in &opened.ssh {
        let name = text::one_line(key.name.trim());
        keys.push(match carry_out_ssh(vault, &mut snap, &mut folder, key).await {
            Ok(fingerprint) => SshStored {
                name,
                fingerprint,
                error: None,
            },
            Err(e) => SshStored {
                name,
                fingerprint: String::new(),
                error: Some(e.to_string()),
            },
        });
    }
    Ok((opened, stored, keys))
}

// ---- names ----------------------------------------------------------------------------------------------------------

fn listing(snap: &Snapshot, only: Option<&str>) -> Vec<ProjectListing> {
    let mut projects: Vec<ProjectListing> = Vec::new();
    for (project, env, entry) in all_envs(snap) {
        if only.is_some_and(|p| p != project) {
            continue;
        }
        let (vars, _) = variables(entry);
        let listed = EnvListing {
            name: env,
            revision: revision(entry),
            variables: vars.into_iter().map(|(n, _)| n).collect(),
        };
        match projects.last_mut() {
            Some(p) if p.name == project => p.envs.push(listed),
            _ => projects.push(ProjectListing {
                name: project,
                envs: vec![listed],
            }),
        }
    }
    projects
}

fn project_arg(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("project") {
        None | Some("") => Ok(None),
        Some(p) if name_ok(p) => Ok(Some(p.to_owned())),
        Some(_) => Err(bad("`project` must be a project name.")),
    }
}

pub(super) async fn preview_list(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let only = project_arg(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let projects = listing(&snap, only.as_deref());
    let envs: usize = projects.iter().map(|p| p.envs.len()).sum();
    let (resource, label, parents) = match &only {
        Some(p) => (
            format!("{NAMES}/{p}"),
            format!("Secret names of {p}"),
            vec![(NAMES.to_owned(), "Secret names of every project".to_owned())],
        ),
        None => (NAMES.to_owned(), "Secret names of every project".to_owned(), Vec::new()),
    };
    Ok(Preview {
        resource,
        resource_label: label,
        lines: vec![
            match &only {
                Some(p) => format!("Show the secret names of {p} on the computer ({envs} environments)"),
                None => {
                    format!("Show your secret names on the computer ({} projects, {envs} environments)", projects.len())
                }
            },
            "Names only, never a value".to_owned(),
        ],
        parents,
        once_only: false,
        ..Preview::default()
    })
}

pub(super) async fn perform_list(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let only = project_arg(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let answer = SecretsListing {
        v: 1,
        dir: TO_DESKTOP.to_owned(),
        nonce: nonce_arg(call)?,
        projects: listing(&snap, only.as_deref()),
    };
    Ok(json!({ SEALED_FIELD: box_from_phone(client_key(call)?, &*vault.store.inbox_key()?, &answer)? }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_is_asked_every_time_unless_the_user_says_otherwise() {
        assert!(always_ask(None, false, "production") && always_ask(None, false, "Prod"));
        assert!(!always_ask(None, false, "development"));
        assert!(always_ask(None, true, "development"), "confirm = always in reins.toml");
        assert!(!always_ask(Some(false), true, "production"), "the user's choice on the phone wins");
        assert!(always_ask(Some(true), false, "development"));
    }

    #[test]
    fn the_always_ask_choice_is_kept_per_environment() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::open(dir.path());
        assert_eq!(always_ask_choice(&store, "acme", "prod").unwrap(), None);
        set_always_ask(&store, "acme", "prod", Some(false)).unwrap();
        assert_eq!(always_ask_choice(&store, "acme", "prod").unwrap(), Some(false));
        assert_eq!(always_ask_choice(&store, "acme", "dev").unwrap(), None);
        set_always_ask(&store, "acme", "prod", None).unwrap();
        assert_eq!(always_ask_choice(&store, "acme", "prod").unwrap(), None);
        assert!(set_always_ask(&store, "a/b", "prod", Some(true)).is_err());
    }

    #[test]
    fn runners_have_names() {
        assert_eq!(runner_label("person"), "You");
        assert_eq!(runner_label("agent:claude-code"), "Claude Code");
        assert_eq!(runner_label("agent:aider"), "Aider");
    }
}
