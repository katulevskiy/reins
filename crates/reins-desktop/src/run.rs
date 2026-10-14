//! `reins run [-e ENV] [--project NAME] [--env NAME=REF]... [--profile P] [--no-redact] -- <command>`: the
//! phone releases the secrets (sealed to this app), the command runs with them as environment variables, and `reins`
//! exits with the command's code.
//!
//! What it runs with: in a project (a `reins.toml` above the working directory, or `--project`), the variables of
//! `.env.reins` (references are released, literals pass through), or the whole environment (`-e ENV`, else the
//! project's default) when there is no `.env.reins` or `-e` names one (then `.env.reins` gives its literals only); then
//! `[run.profiles.<P>]` of config.toml and `--env` pairs (`-e NAME=REF` too, as before), which win.
//!
//! An environment is asked of the daemon (`POST /_reins/secrets/lease`), which answers from a lease it holds when that
//! lease was released for this very runner, folder, command and variables, else asks the phone and keeps the answer
//! (see `leases`). Without a daemon this process asks the phone itself. Other `vault:Item/field` references are asked
//! for directly, every time.
//!
//! The command's stdout and stderr are filtered: every released value (and its encodings) becomes
//! `[vault:project/env/NAME]` (`redact`). An agent (`REINS_RUNNER=agent:…`) cannot turn that off; a person can,
//! with `--no-redact`, for a tool that needs a real terminal. Values are never printed or logged.
//!
//! Exit codes of `reins` itself: 125 when the secrets are not released (or anything else on its side fails), 126
//! when the command cannot be run, 127 when it is not found; else the command's own code (128 + N after signal N).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::config::{Config, Mode, Paths};
use crate::identity::Identity;
use crate::journal::{Entry, Journal, Kind, Outcome};
use crate::phone::{Phone, awaiting, teller};
use crate::project::{EnvValue, Project};
use crate::secrets::{self, EnvRef, MAX_SECRETS, Reference, Resolved, SecretRef, VarRef, check_var};
use crate::secrets_client::{DEFAULT_LEASE_SECS, LeaseRequest};

/// `reins run` failed before the command ran.
pub const EXIT_FAILED: u8 = 125;
pub const EXIT_CANNOT_RUN: u8 = 126;
pub const EXIT_NOT_FOUND: u8 = 127;
/// A single `vault:Item/field` is only needed until the command starts.
const LEASE_SECS: u32 = 60;

/// `[run.profiles.<name>]` in `config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunConfig {
    pub profiles: BTreeMap<String, Profile>,
}

/// A named set of variables: `env = { OPENAI_API_KEY = "vault:OpenAI/password" }`, and why (shown on the phone).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub env: BTreeMap<String, String>,
    pub purpose: Option<String>,
}

impl RunConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, p) in &self.profiles {
            if p.env.is_empty() || p.env.len() > MAX_SECRETS {
                return Err(format!("`run.profiles.{name}` needs 1 to {MAX_SECRETS} variables in `env`"));
            }
            for (var, reference) in &p.env {
                check_var(var).map_err(|e| format!("`run.profiles.{name}`: {e}"))?;
                Reference::parse_value(reference).map_err(|e| format!("`run.profiles.{name}.env.{var}`: {e}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct RunArgs {
    /// The environment of the project (default: reins.toml's default_env). `-e NAME=REF` sets a variable, as --env.
    #[arg(short = 'e', long = "environment", value_name = "ENV")]
    pub environment: Vec<String>,
    /// A variable from the vault on your phone (repeatable): `NAME=vault:ITEM/FIELD` or `NAME=env:ENV/NAME`.
    #[arg(long = "env", value_name = "NAME=REF")]
    pub env: Vec<String>,
    /// The project (default: the reins.toml above this folder).
    #[arg(long)]
    pub project: Option<String>,
    /// The variables of `[run.profiles.<PROFILE>]` in config.toml (`--env` adds to them or overrides them).
    #[arg(long)]
    pub profile: Option<String>,
    /// Why, shown on your phone.
    #[arg(long)]
    pub purpose: Option<String>,
    /// Show the secrets in the command's output instead of `[vault:…]` (people only: refused for an agent).
    #[arg(long)]
    pub no_redact: bool,
    /// The command and its arguments.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<OsString>,
}

/// Where a variable's value comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// As written in `.env.reins` (not a secret).
    Literal(String),
    /// A variable of an environment.
    Var(VarRef),
    /// Any other vault field.
    Field(SecretRef),
}

/// What to ask for and what to run.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Variables named one by one, by name.
    pub vars: BTreeMap<String, Source>,
    /// Environments whose every variable is set (named ones in `vars` win).
    pub whole: Vec<EnvRef>,
    pub purpose: Option<String>,
    pub command: Vec<OsString>,
    /// The project's folder (canonical), or the working directory without a project.
    pub root: PathBuf,
    /// The project's reins.toml, for leases and `confirm`.
    pub project: Option<Project>,
}

impl Plan {
    pub fn new(args: &RunArgs, config: &Config, cwd: &Path) -> Result<Self, String> {
        if args.command.is_empty() {
            return Err("no command: `reins run -- <command>`".to_owned());
        }
        let (pairs, envs): (Vec<&String>, Vec<&String>) = args.environment.iter().partition(|e| e.contains('='));
        if envs.len() > 1 {
            return Err("give one environment with -e".to_owned());
        }
        let chosen_env = envs.first().map(|e| e.as_str());
        let found = Project::find(cwd)?;
        let name = args.project.clone().or_else(|| found.as_ref().map(|p| p.file.name.clone()));
        let project = found.filter(|p| Some(&p.file.name) == name.as_ref());
        let root = project.as_ref().map_or(cwd, |p| p.root.as_path());
        let root = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let mut vars: BTreeMap<String, Source> = BTreeMap::new();
        let mut whole: Vec<EnvRef> = Vec::new();
        if let Some(name) = &name {
            let default_env = project.as_ref().map_or(crate::project::DEFAULT_ENV, |p| p.file.default_env.as_str());
            let env_file = project.as_ref().map(Project::env_file).transpose()?.flatten();
            match (&env_file, chosen_env) {
                (Some(file), None) => {
                    for v in file.vars() {
                        vars.insert(v.name.clone(), source_of(&v.value, Some(name))?);
                    }
                }
                (file, chosen) => {
                    whole.push(EnvRef::new(name, chosen.unwrap_or(default_env))?);
                    for v in file.iter().flat_map(crate::project::EnvFile::vars) {
                        if let EnvValue::Literal(l) = &v.value {
                            vars.insert(v.name.clone(), Source::Literal(l.clone()));
                        }
                    }
                }
            }
        } else if chosen_env.is_some() {
            return Err(
                "-e names an environment of a project: run it in a folder with a reins.toml, or give --project"
                    .to_owned(),
            );
        }
        let mut purpose = None;
        if let Some(profile) = &args.profile {
            let p = config.run.profiles.get(profile).ok_or_else(|| {
                let known: Vec<&str> = config.run.profiles.keys().map(String::as_str).collect();
                if known.is_empty() {
                    format!("no profile `{profile}`: config.toml has no `[run.profiles.*]`")
                } else {
                    format!("no profile `{profile}` (config.toml has {})", known.join(", "))
                }
            })?;
            for (var, reference) in &p.env {
                let r = Reference::parse_value(reference)?;
                vars.insert(var.clone(), source_of(&EnvValue::Reference(r), name.as_deref())?);
            }
            purpose.clone_from(&p.purpose);
        }
        for pair in pairs.into_iter().chain(&args.env) {
            let (var, reference) =
                pair.split_once('=').ok_or_else(|| format!("`--env {pair}`: write NAME=vault:Item/field"))?;
            check_var(var)?;
            let r = Reference::parse_value(reference)?;
            vars.insert(var.to_owned(), source_of(&EnvValue::Reference(r), name.as_deref())?);
        }
        if args.purpose.is_some() {
            purpose.clone_from(&args.purpose);
        }
        let plan = Self {
            vars,
            whole,
            purpose,
            command: args.command.clone(),
            root,
            project,
        };
        if plan.vars.values().all(|s| matches!(s, Source::Literal(_))) && plan.whole.is_empty() {
            return Err("nothing to release: run it in a project (reins.toml), or give `--env NAME=vault:Item/field` \
                        or `--profile NAME`"
                .to_owned());
        }
        if plan.fields().len() > MAX_SECRETS {
            return Err(format!("at most {MAX_SECRETS} vault:Item/field references per command"));
        }
        Ok(plan)
    }

    /// The command line as shown on the phone: each argument exactly, as a shell would read it back (control and
    /// invisible characters escaped visibly; see `reins_proto::desktop::shell_word`).
    #[must_use]
    pub fn shown(&self) -> String {
        let argv: Vec<String> = self.command.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        reins_proto::desktop::shell_words(&argv)
    }

    /// Distinct `vault:Item/field` secrets, in order (two variables may use the same one).
    fn fields(&self) -> Vec<SecretRef> {
        let mut out: Vec<SecretRef> = Vec::new();
        for s in self.vars.values() {
            if let Source::Field(r) = s
                && !out.contains(r)
            {
                out.push(r.clone());
            }
        }
        out
    }

    /// The environments to ask for and their variables (`None`: every one).
    fn envs(&self) -> BTreeMap<EnvRef, Option<BTreeSet<String>>> {
        let mut out: BTreeMap<EnvRef, Option<BTreeSet<String>>> =
            self.whole.iter().map(|e| (e.clone(), None)).collect();
        for s in self.vars.values() {
            if let Source::Var(v) = s
                && let Some(names) = out.entry(v.env.clone()).or_insert_with(|| Some(BTreeSet::new()))
            {
                names.insert(v.name.clone());
            }
        }
        out
    }

    /// The lease request for one environment.
    fn request(&self, env: &EnvRef, fields: Option<&BTreeSet<String>>, runner: &str) -> LeaseRequest {
        let settings = self
            .project
            .as_ref()
            .filter(|p| p.file.name == env.project)
            .map(|p| p.file.env(&env.env))
            .unwrap_or_default();
        let typed = self.command[0].to_string_lossy();
        let names = crate::secrets_client::program_names(
            &typed,
            &self.root,
            std::env::var("PATH").ok().as_deref(),
            std::env::var("PATH").ok().as_deref(),
        );
        let env_now: Vec<(String, String)> =
            std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
        LeaseRequest {
            project: env.project.clone(),
            env: env.env.clone(),
            fields: fields.map(|f| f.iter().cloned().collect()),
            program: names.program.clone(),
            args: self.command[1..].iter().map(|a| a.to_string_lossy().into_owned()).collect(),
            root: self.root.to_string_lossy().into_owned(),
            runner: runner.to_owned(),
            lease_secs: settings.lease_secs.unwrap_or(DEFAULT_LEASE_SECS),
            purpose: self.purpose.clone(),
            confirm_always: settings.confirm == crate::project::Confirm::Always,
            print: false,
            typed: Some(names.typed),
            aliases: names.aliases,
            path_check: None,
            exec_env: crate::secrets_client::exec_env_digest(&env_now),
        }
    }
}

/// A value as `.env.reins`, a profile or `--env` gives it, resolved against the project.
fn source_of(value: &EnvValue, project: Option<&str>) -> Result<Source, String> {
    Ok(match value {
        EnvValue::Literal(l) => Source::Literal(l.clone()),
        EnvValue::Reference(r) => match r.resolve(project)? {
            Resolved::Field(f) => Source::Field(f),
            Resolved::Var(v) => Source::Var(v),
            Resolved::Env(e) => return Err(format!("{e} is a whole environment; name one variable")),
        },
    })
}

/// One variable for the command, and the label its value is redacted with (`None`: a literal, not redacted).
pub struct Var {
    pub name: String,
    pub value: Zeroizing<String>,
    pub label: Option<String>,
}

/// Asks the phone for `vault:Item/field` references (directly, every time); their values, by reference.
async fn release_fields(
    secrets: &[SecretRef],
    command: &str,
    purpose: Option<&str>,
    phone: &Phone,
    journal: &Journal,
    timeout: Duration,
) -> Result<Vec<(SecretRef, Zeroizing<String>)>, Refusal> {
    let request = secrets::Request {
        secrets,
        command,
        purpose,
        lease_secs: LEASE_SECS,
    };
    let tell = teller(|line: &str| eprintln!("{line}"));
    let what = format!("secrets for `{}`", secrets::cut(command, 80));
    let entry = Entry::new(Kind::Secrets, &what).source(Some("reins run")).detail(Some(&format!(
        "Command: {command}\nSecrets: {}",
        secrets.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )));
    let released = awaiting(
        journal,
        entry,
        Some(tell),
        timeout,
        secrets::release(phone, &request, None, "No answer from your phone in time. Approve it, then run again."),
    )
    .await?;
    Ok(secrets.iter().cloned().zip(released.values).collect())
}

/// The values of one environment for the command, asked of the phone by this process (where the daemon cannot run
/// the command itself: Windows).
#[cfg_attr(unix, allow(dead_code, reason = "Unix runs every environment through the daemon"))]
async fn release_env_here(
    phone: &dyn Fn() -> Result<Arc<Phone>, Refusal>,
    req: &LeaseRequest,
    journal: &Journal,
    timeout: Duration,
) -> Result<Vec<(String, Zeroizing<String>)>, Refusal> {
    let what = format!("{} for `{}`", req.item(), secrets::cut(&req.command().shown(), 80));
    let tell = teller(|line: &str| eprintln!("{line}"));
    let phone = phone()?;
    let entry = Entry::new(Kind::Secrets, &what).source(Some("reins run"));
    let released =
        awaiting(journal, entry, Some(tell), timeout, crate::secrets_client::release_env(&phone, req, None)).await?;
    Ok(released.values)
}

/// The `vault:Item/field` values the plan names, asked of the phone (every time; no lease).
async fn field_values(
    plan: &Plan,
    phone: &dyn Fn() -> Result<Arc<Phone>, Refusal>,
    journal: &Journal,
    timeout: Duration,
) -> Result<Vec<(SecretRef, Zeroizing<String>)>, Refusal> {
    let fields = plan.fields();
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    let phone = phone()?;
    release_fields(&fields, &plan.shown(), plan.purpose.as_deref(), &phone, journal, timeout).await
}

/// The command's variables: every variable of the environments asked for whole under its own name, then the variables
/// named one by one (which win), from what was released.
pub fn assemble(
    whole: &[EnvRef],
    released: &[(VarRef, Zeroizing<String>)],
    named: &[(String, Source)],
    given: &[(SecretRef, Zeroizing<String>)],
) -> Result<Vec<Var>, Refusal> {
    let mut out: Vec<Var> = released
        .iter()
        .filter(|(r, _)| whole.contains(&r.env))
        .map(|(r, v)| Var {
            name: r.name.clone(),
            value: v.clone(),
            label: Some(r.to_string()),
        })
        .collect();
    for (name, source) in named {
        out.retain(|v| v.name != *name);
        let (value, label) = match source {
            Source::Literal(l) => (Zeroizing::new(l.clone()), None),
            Source::Field(r) => {
                let Some((_, v)) = given.iter().find(|(f, _)| f == r) else {
                    return Err(Refusal::Unavailable(format!("{r} was not released")));
                };
                (v.clone(), Some(r.to_string()))
            }
            Source::Var(v) => {
                let Some((_, value)) = released.iter().find(|(r, _)| r == v) else {
                    return Err(Refusal::Unavailable(format!("{v} was not released (no such variable)")));
                };
                (value.clone(), Some(v.to_string()))
            }
        };
        out.push(Var {
            name: name.clone(),
            value,
            label,
        });
    }
    Ok(out)
}

/// Asks for everything the plan needs from this process; the variables for the command. Used where the daemon cannot
/// run the command (Windows), and for `vault:Item/field` references alone.
pub async fn release(
    plan: &Plan,
    paths: &Paths,
    config: &Config,
    runner: &str,
    phone: &dyn Fn() -> Result<Arc<Phone>, Refusal>,
) -> Result<Vec<Var>, Refusal> {
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let journal = Journal::new(paths);
    let mut released: Vec<(VarRef, Zeroizing<String>)> = Vec::new();
    for (env, fields) in &plan.envs() {
        let req = plan.request(env, fields.as_ref(), runner);
        for (name, value) in release_env_here(phone, &req, &journal, timeout).await? {
            released.push((env.var(&name), value));
        }
    }
    let given = field_values(plan, phone, &journal, timeout).await?;
    let named: Vec<(String, Source)> = plan.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    assemble(&plan.whole, &released, &named, &given)
}

/// What the daemon is asked to run (Unix): the plan, with the `vault:Item/field` values this process got.
#[cfg(unix)]
fn run_request(
    plan: &Plan,
    token: &str,
    cwd: &Path,
    given: &[(SecretRef, Zeroizing<String>)],
    no_redact: bool,
) -> Result<crate::run_socket::RunRequest, String> {
    use crate::run_socket::{EnvAsk, RunRequest, RunSource};
    let argv: Vec<String> = plan
        .command
        .iter()
        .map(|a| a.to_str().map(str::to_owned).ok_or_else(|| "the command is not text (UTF-8)".to_owned()))
        .collect::<Result<_, _>>()?;
    let asks = plan
        .envs()
        .iter()
        .map(|(env, fields)| {
            let req = plan.request(env, fields.as_ref(), reins_proto::desktop::PERSON);
            EnvAsk {
                project: req.project,
                env: req.env,
                fields: req.fields,
                lease_secs: req.lease_secs,
                confirm_always: req.confirm_always,
            }
        })
        .collect();
    let vars = plan
        .vars
        .iter()
        .map(|(name, source)| {
            (
                name.clone(),
                match source {
                    Source::Literal(l) => RunSource::Literal {
                        value: l.clone(),
                    },
                    Source::Var(v) => RunSource::Var {
                        project: v.env.project.clone(),
                        env: v.env.env.clone(),
                        name: v.name.clone(),
                    },
                    Source::Field(r) => RunSource::Given {
                        value: given.iter().find(|(f, _)| f == r).map(|(_, v)| v.to_string()).unwrap_or_default(),
                        label: r.to_string(),
                    },
                },
            )
        })
        .collect();
    Ok(RunRequest {
        token: token.to_owned(),
        argv,
        cwd: cwd.to_string_lossy().into_owned(),
        root: plan.root.to_string_lossy().into_owned(),
        env: std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect(),
        asks,
        vars,
        whole: plan.whole.iter().map(|e| (e.project.clone(), e.env.clone())).collect(),
        no_redact,
        purpose: plan.purpose.clone(),
    })
}

/// Starts the command with `vars` and waits for it, passing on SIGTERM and SIGHUP (Ctrl-C reaches it from the
/// terminal directly). With `redactor`, its stdout and stderr go through it. Returns its exit code.
pub async fn execute(command: &[OsString], vars: Vec<Var>, redactor: Option<Arc<crate::redact::Redactor>>) -> u8 {
    let Some((program, args)) = command.split_first() else {
        return EXIT_FAILED;
    };
    // On Windows `npm` is `npm.cmd`, which `Command` does not find by itself.
    #[cfg(windows)]
    let resolved = {
        let dirs: Vec<PathBuf> =
            std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let pathext = std::env::var("PATHEXT").ok();
        crate::win::resolve_program(program, &dirs, pathext.as_deref(), Path::is_file)
    };
    #[cfg(windows)]
    let mut cmd = tokio::process::Command::new(resolved.as_deref().map_or(program.as_os_str(), |p| p.as_os_str()));
    #[cfg(not(windows))]
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    for v in &vars {
        cmd.env(&v.name, v.value.as_str());
    }
    if redactor.is_some() {
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    }
    let spawned = cmd.spawn();
    drop(cmd);
    drop(vars);
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reins: cannot run {}: {e}", program.to_string_lossy());
            return if e.kind() == std::io::ErrorKind::NotFound {
                EXIT_NOT_FOUND
            } else {
                EXIT_CANNOT_RUN
            };
        }
    };
    let pumps = redactor.map(|r| {
        let out = child.stdout.take().map(|s| tokio::spawn(pump(s, tokio::io::stdout(), Arc::clone(&r))));
        let err = child.stderr.take().map(|s| tokio::spawn(pump(s, tokio::io::stderr(), r)));
        (out, err)
    });
    let status = wait(&mut child).await;
    if let Some((out, err)) = pumps {
        for task in [out, err].into_iter().flatten() {
            task.await.ok();
        }
    }
    match status {
        Ok(s) => exit_code(s),
        Err(e) => {
            eprintln!("reins: waiting for the command: {e}");
            EXIT_FAILED
        }
    }
}

/// Copies `from` to `to` through the redactor, chunk by chunk, until the end.
pub(crate) async fn pump(
    mut from: impl tokio::io::AsyncRead + Unpin,
    mut to: impl tokio::io::AsyncWrite + Unpin,
    redactor: Arc<crate::redact::Redactor>,
) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = redactor.stream();
    let mut buf = Zeroizing::new(vec![0u8; 16 * 1024]);
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let out = stream.push(&buf[..n]);
        if to.write_all(&out).await.is_err() {
            return;
        }
        to.flush().await.ok();
    }
    to.write_all(&stream.finish()).await.ok();
    to.flush().await.ok();
}

#[cfg(unix)]
async fn wait(child: &mut tokio::process::Child) -> std::io::Result<std::process::ExitStatus> {
    use tokio::signal::unix::{SignalKind, signal};
    let pid = child.id().and_then(|p| rustix::process::Pid::from_raw(i32::try_from(p).ok()?));
    let (Ok(mut term), Ok(mut hup), Ok(mut int)) =
        (signal(SignalKind::terminate()), signal(SignalKind::hangup()), signal(SignalKind::interrupt()))
    else {
        return child.wait().await;
    };
    loop {
        let forward = tokio::select! {
            s = child.wait() => return s,
            _ = term.recv() => rustix::process::Signal::TERM,
            _ = hup.recv() => rustix::process::Signal::HUP,
            // The terminal sends Ctrl-C to the command as well; reins only keeps waiting for it.
            _ = int.recv() => continue,
        };
        if let Some(pid) = pid {
            rustix::process::kill_process(pid, forward).ok();
        }
    }
}

/// Windows has no signals to pass on. Ctrl-C and Ctrl-Break reach every program in the console, the command too, so
/// reins only keeps waiting for it to finish (instead of quitting and leaving it behind).
#[cfg(windows)]
async fn wait(child: &mut tokio::process::Child) -> std::io::Result<std::process::ExitStatus> {
    use tokio::signal::windows::{ctrl_break, ctrl_c};
    let (Ok(mut c), Ok(mut b)) = (ctrl_c(), ctrl_break()) else {
        return child.wait().await;
    };
    loop {
        tokio::select! {
            s = child.wait() => return s,
            _ = c.recv() => {}
            _ = b.recv() => {}
        }
    }
}

#[cfg(not(any(unix, windows)))]
async fn wait(child: &mut tokio::process::Child) -> std::io::Result<std::process::ExitStatus> {
    child.wait().await
}

/// A Windows exit code as `reins`'s own: 0 to 255 as they are, anything else (an `NTSTATUS` such as
/// `0xC000013A` after Ctrl-C) 1.
fn windows_exit_code(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

pub(crate) fn exit_code(status: std::process::ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        if cfg!(windows) {
            return windows_exit_code(code);
        }
        return u8::try_from(code & 0xff).unwrap_or(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(sig) = status.signal() {
            return u8::try_from(128 + (sig & 0x7f)).unwrap_or(1);
        }
    }
    1
}

/// The whole command: plan, ask, run; the exit code. Messages go to stderr.
pub async fn main(paths: &Paths, config: &Config, args: &RunArgs) -> u8 {
    let fail = |e: &str| {
        eprintln!("reins: {e}");
        EXIT_FAILED
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => return fail(&e.to_string()),
    };
    let plan = match Plan::new(args, config, &cwd) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if config.mode == Mode::Local {
        return fail(
            "secrets live in the vault on your phone, and this app is in local mode (`mode = \"local\"` in \
             config.toml); set `mode = \"auto\"` and run `reins login`",
        );
    }
    if crate::server::oauth::logged_in_server(paths).is_none() {
        return fail("secrets live in the vault on your phone: run `reins login` to pair this app with it");
    }
    let identity = match Identity::load_or_create(&paths.identity_file()) {
        Ok(i) => Arc::new(i),
        Err(e) => return fail(&e.to_string()),
    };
    let timeout = Duration::from_secs(config.approval_timeout_secs);
    let phone_slot: std::sync::OnceLock<Arc<Phone>> = std::sync::OnceLock::new();
    let phone = || -> Result<Arc<Phone>, Refusal> {
        if let Some(p) = phone_slot.get() {
            return Ok(Arc::clone(p));
        }
        let p = Arc::new(Phone::new(paths, Arc::clone(&identity), timeout).map_err(Refusal::Unavailable)?);
        Ok(Arc::clone(phone_slot.get_or_init(|| p)))
    };
    let journal = Journal::new(paths);
    let mut entry = Entry::new(Kind::Secrets, &format!("run `{}`", secrets::cut(&plan.shown(), 80)))
        .source(Some("reins run"))
        .detail(Some(&format!(
            "Command: {}\nIn: {}\nVariables: {}",
            plan.shown(),
            plan.root.display(),
            plan.vars
                .iter()
                .map(|(name, s)| match s {
                    Source::Literal(_) => name.clone(),
                    Source::Var(v) => format!("{name}={v}"),
                    Source::Field(r) => format!("{name}={r}"),
                })
                .chain(plan.whole.iter().map(ToString::to_string))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    // On Unix the daemon runs the command: it decides who runs it from the process asking, and the values reach only
    // what it starts. Every environment goes that way; `vault:Item/field` references alone do too when it runs.
    #[cfg(unix)]
    {
        let socket = crate::run_socket::socket_path(paths);
        let token =
            std::fs::read_to_string(paths.control_token_file()).ok().map(|t| Zeroizing::new(t.trim().to_owned()));
        match token {
            Some(token) if socket.exists() => {
                let given = match field_values(&plan, &phone, &journal, timeout).await {
                    Ok(g) => g,
                    Err(r) => return fail(r.message()),
                };
                let req = match run_request(&plan, &token, &cwd, &given, args.no_redact) {
                    Ok(r) => r,
                    Err(e) => return fail(&e),
                };
                drop(given);
                entry.end(Outcome::Allowed, None);
                journal.record(&entry);
                return match crate::run_socket::run(&socket, &req).await {
                    Ok(code) => code,
                    Err(e) => fail(&e),
                };
            }
            _ if !plan.envs().is_empty() => {
                return fail(
                    "a project's secrets are used through the Reins daemon, which is not running: `reins \
                             resume` starts it",
                );
            }
            _ => {}
        }
    }
    // Here, this process asks the phone itself and starts the command: who runs it is what its own ancestry says.
    let mut env: BTreeMap<String, String> =
        std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    env.retain(|k, _| {
        k.starts_with("REINS_")
            || k.starts_with("CLAUDE")
            || k.starts_with("CODEX")
            || k.starts_with("GEMINI")
            || k.starts_with("CURSOR")
    });
    let runner = crate::runner::runner_of(std::process::id(), &env, &crate::runner::process_info);
    let agent = runner != reins_proto::desktop::PERSON;
    if agent && args.no_redact {
        return fail("--no-redact is for people at a terminal; for an agent the secrets stay out of the output");
    }
    let vars = match release(&plan, paths, config, &runner, &phone).await {
        Ok(v) => v,
        Err(r) => return fail(r.message()),
    };
    // The filter is built before anything starts: a run that asked for it never runs without it.
    let redactor = if agent || !args.no_redact {
        let secrets: Vec<(String, &str)> =
            vars.iter().filter_map(|v| v.label.as_ref().map(|l| (l.clone(), v.value.as_str()))).collect();
        match crate::redact::Redactor::new(&secrets) {
            Ok(r) => Some(Arc::new(r)).filter(|r| !r.is_empty()),
            Err(e) => return fail(&e.to_string()),
        }
    } else {
        None
    };
    entry.end(Outcome::Allowed, None);
    journal.record(&entry);
    execute(&plan.command, vars, redactor).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ENV_FILE, PROJECT_FILE, ProjectFile};

    fn args(env: &[&str], environment: &[&str], profile: Option<&str>, command: &[&str]) -> RunArgs {
        RunArgs {
            environment: environment.iter().map(|s| (*s).to_owned()).collect(),
            env: env.iter().map(|s| (*s).to_owned()).collect(),
            project: None,
            profile: profile.map(str::to_owned),
            purpose: None,
            no_redact: false,
            command: command.iter().map(OsString::from).collect(),
        }
    }

    fn config_with_profile() -> Config {
        let c: Config = toml::from_str(
            "[run.profiles.deploy]\npurpose = \"deploy the site\"\nenv = { AWS_KEY = \"vault:AWS/username\", AWS_SECRET = \"vault:AWS/password\" }\n",
        )
        .unwrap();
        c.validate().unwrap();
        c
    }

    fn shown(p: &Plan) -> Vec<(String, String)> {
        p.vars
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v {
                        Source::Literal(l) => format!("literal {l}"),
                        Source::Var(r) => r.to_string(),
                        Source::Field(r) => r.to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_plan_merges_the_profile_and_the_flags() {
        let dir = tempfile::tempdir().unwrap();
        let c = config_with_profile();
        let p = Plan::new(
            &args(&["AWS_SECRET=vault:Other/password"], &["X=vault:X/y"], Some("deploy"), &["make", "it"]),
            &c,
            dir.path(),
        )
        .unwrap();
        assert_eq!(
            shown(&p),
            [
                ("AWS_KEY".to_owned(), "vault:AWS/username".to_owned()),
                ("AWS_SECRET".to_owned(), "vault:Other/password".to_owned()),
                ("X".to_owned(), "vault:X/y".to_owned())
            ]
        );
        assert!(p.whole.is_empty() && p.project.is_none());
        assert_eq!(p.purpose.as_deref(), Some("deploy the site"));
        assert_eq!(p.shown(), "make it");
        let d = dir.path();
        assert!(Plan::new(&args(&[], &[], Some("nope"), &["x"]), &c, d).unwrap_err().contains("deploy"));
        assert!(Plan::new(&args(&[], &[], None, &["x"]), &c, d).is_err(), "nothing to release");
        assert!(Plan::new(&args(&["1X=vault:a/b"], &[], None, &["x"]), &c, d).is_err());
        assert!(Plan::new(&args(&["X=OpenAI/password"], &[], None, &["x"]), &c, d).is_err());
        assert!(Plan::new(&args(&["X"], &[], None, &["x"]), &c, d).is_err());
        assert!(Plan::new(&args(&[], &["dev"], None, &["x"]), &c, d).unwrap_err().contains("reins.toml"));
        assert!(Plan::new(&args(&["X=env:dev/X"], &[], None, &["x"]), &c, d).unwrap_err().contains("project"));
    }

    #[test]
    fn the_same_secret_is_asked_for_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = Plan::new(
            &args(&["A=vault:K/password", "B=vault:K/password"], &[], None, &["x"]),
            &Config::default(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(p.fields().len(), 1);
    }

    #[test]
    fn the_command_is_shown_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let p = Plan::new(
            &args(&["A=vault:K/p"], &[], None, &["sh", "-c", "echo \"$A\"", ""]),
            &Config::default(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(p.shown(), r#"sh -c 'echo "$A"' ''"#);
    }

    fn project(dir: &Path, env_reins: Option<&str>) -> PathBuf {
        let root = dir.join("acme");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let mut f = ProjectFile::new("acme", "development").unwrap();
        f.envs.insert(
            "production".to_owned(),
            crate::project::EnvSettings {
                lease_secs: Some(900),
                confirm: crate::project::Confirm::Always,
            },
        );
        f.write(&root.join(PROJECT_FILE)).unwrap();
        if let Some(text) = env_reins {
            std::fs::write(root.join(ENV_FILE), text).unwrap();
        }
        root
    }

    #[test]
    fn in_a_project_the_whole_environment_is_asked_for_unless_env_reins_names_variables() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(dir.path(), None);
        let c = Config::default();
        let p = Plan::new(&args(&[], &[], None, &["npm", "run", "dev"]), &c, &root.join("src")).unwrap();
        assert_eq!(p.whole, [EnvRef::new("acme", "development").unwrap()]);
        assert_eq!(p.root, std::fs::canonicalize(&root).unwrap(), "the project's folder, from below it");
        let prod = Plan::new(&args(&[], &["production"], None, &["npm", "start"]), &c, &root).unwrap();
        assert_eq!(prod.whole, [EnvRef::new("acme", "production").unwrap()]);
        let req = prod.request(&prod.whole[0], None, "person");
        assert_eq!((req.lease_secs, req.confirm_always, req.fields.clone()), (900, true, None));
        assert_eq!(req.args, ["start"]);
        assert_eq!(req.root, std::fs::canonicalize(&root).unwrap().to_string_lossy());

        let root = project(dir.path(), Some("KEY=env:development/KEY\nOTHER=vault:shared/ci/TOKEN\nLOG=debug\n"));
        let p = Plan::new(&args(&[], &[], None, &["npm", "run", "dev"]), &c, &root).unwrap();
        assert!(p.whole.is_empty());
        assert_eq!(
            shown(&p),
            [
                ("KEY".to_owned(), "vault:acme/development/KEY".to_owned()),
                ("LOG".to_owned(), "literal debug".to_owned()),
                ("OTHER".to_owned(), "vault:shared/ci/TOKEN".to_owned())
            ]
        );
        let envs = p.envs();
        assert_eq!(envs.len(), 2, "one release per environment");
        // -e names the environment: the whole of it, and only the literals of .env.reins.
        let p = Plan::new(&args(&[], &["production"], None, &["x"]), &c, &root).unwrap();
        assert_eq!(shown(&p), [("LOG".to_owned(), "literal debug".to_owned())]);
        assert_eq!(p.whole, [EnvRef::new("acme", "production").unwrap()]);
        assert!(Plan::new(&args(&[], &["a", "b"], None, &["x"]), &c, &root).is_err(), "one environment");
    }

    #[test]
    fn bad_profiles_are_refused_when_the_config_loads() {
        for bad in [
            "[run.profiles.x]\nenv = {}\n",
            "[run.profiles.x]\nenv = { \"A B\" = \"vault:a/b\" }\n",
            "[run.profiles.x]\nenv = { A = \"plain\" }\n",
        ] {
            let c: Config = toml::from_str(bad).unwrap();
            assert!(c.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn windows_exit_codes_fit_or_say_failure() {
        assert_eq!(windows_exit_code(0), 0);
        assert_eq!(windows_exit_code(7), 7);
        assert_eq!(windows_exit_code(255), 255);
        assert_eq!(windows_exit_code(256), 1);
        assert_eq!(windows_exit_code(0xC000_013A_u32.cast_signed()), 1, "Ctrl-C");
    }

    #[test]
    #[cfg(unix)]
    fn exit_codes_pass_through() {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(7 << 8)), 7);
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(9)), 137);
    }
}
