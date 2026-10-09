//! `reins run [--env NAME=vault:Item/field]... [--profile P] -- <command>`: the phone releases the secrets (sealed to
//! this app), the command runs with them as environment variables, and `reins` exits with the command's code. The
//! values are only in this process's memory until the command starts, then wiped; they are never printed or logged.
//!
//! Exit codes of `reins` itself: 125 when the secrets are not released (or anything else on its side fails), 126
//! when the command cannot be run, 127 when it is not found; else the command's own code (128 + N after signal N).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::auth::Refusal;
use crate::config::{Config, Mode, Paths};
use crate::identity::Identity;
use crate::journal::{Entry, Journal, Kind};
use crate::phone::{Phone, awaiting, teller};
use crate::secrets::{self, MAX_SECRETS, SecretRef};

/// `reins run` failed before the command ran.
pub const EXIT_FAILED: u8 = 125;
pub const EXIT_CANNOT_RUN: u8 = 126;
pub const EXIT_NOT_FOUND: u8 = 127;
/// The secrets are only needed until the command starts.
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
                SecretRef::parse(reference).map_err(|e| format!("`run.profiles.{name}.env.{var}`: {e}"))?;
            }
        }
        Ok(())
    }
}

fn check_var(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 100;
    if ok {
        Ok(())
    } else {
        Err(format!("`{name}` is not an environment variable name (letters, digits, `_`)"))
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct RunArgs {
    /// A variable for the command, from the vault on your phone (repeatable).
    #[arg(long = "env", short = 'e', value_name = "NAME=vault:ITEM/FIELD")]
    pub env: Vec<String>,
    /// The variables of `[run.profiles.<PROFILE>]` in config.toml (`--env` adds to them or overrides them).
    #[arg(long)]
    pub profile: Option<String>,
    /// Why, shown on your phone.
    #[arg(long)]
    pub purpose: Option<String>,
    /// The command and its arguments.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<OsString>,
}

/// What to ask for and what to run, from the arguments and the profile.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Variable names and the secrets that fill them, sorted by name.
    pub vars: Vec<(String, SecretRef)>,
    pub purpose: Option<String>,
    pub command: Vec<OsString>,
}

impl Plan {
    pub fn new(args: &RunArgs, config: &Config) -> Result<Self, String> {
        let mut env: BTreeMap<String, SecretRef> = BTreeMap::new();
        let mut purpose = None;
        if let Some(name) = &args.profile {
            let p = config.run.profiles.get(name).ok_or_else(|| {
                let known: Vec<&str> = config.run.profiles.keys().map(String::as_str).collect();
                if known.is_empty() {
                    format!("no profile `{name}`: config.toml has no `[run.profiles.*]`")
                } else {
                    format!("no profile `{name}` (config.toml has {})", known.join(", "))
                }
            })?;
            for (var, reference) in &p.env {
                env.insert(var.clone(), SecretRef::parse(reference)?);
            }
            purpose.clone_from(&p.purpose);
        }
        for pair in &args.env {
            let (var, reference) =
                pair.split_once('=').ok_or_else(|| format!("`--env {pair}`: write NAME=vault:Item/field"))?;
            check_var(var)?;
            env.insert(var.to_owned(), SecretRef::parse(reference)?);
        }
        if env.is_empty() {
            return Err("nothing to release: give `--env NAME=vault:Item/field` or `--profile NAME`".to_owned());
        }
        if env.len() > MAX_SECRETS {
            return Err(format!("at most {MAX_SECRETS} secrets per command"));
        }
        if args.command.is_empty() {
            return Err("no command: `reins run --env … -- <command>`".to_owned());
        }
        if args.purpose.is_some() {
            purpose.clone_from(&args.purpose);
        }
        Ok(Self {
            vars: env.into_iter().collect(),
            purpose,
            command: args.command.clone(),
        })
    }

    /// The command line as shown on the phone.
    #[must_use]
    pub fn shown(&self) -> String {
        self.command
            .iter()
            .map(|a| {
                let a = a.to_string_lossy();
                if a.is_empty() || a.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
                    format!("'{}'", a.replace('\'', r"'\''"))
                } else {
                    a.into_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Distinct secrets, in order (two variables may use the same one).
    fn secrets(&self) -> Vec<SecretRef> {
        let mut out: Vec<SecretRef> = Vec::new();
        for (_, r) in &self.vars {
            if !out.contains(r) {
                out.push(r.clone());
            }
        }
        out
    }
}

/// Asks the phone for the plan's secrets; the variables to set, in the plan's order.
pub async fn release(
    plan: &Plan,
    phone: &Phone,
    journal: &Journal,
    timeout: Duration,
) -> Result<Vec<(String, Zeroizing<String>)>, Refusal> {
    let secrets = plan.secrets();
    let command = plan.shown();
    let request = secrets::Request {
        secrets: &secrets,
        command: &command,
        purpose: plan.purpose.as_deref(),
        lease_secs: LEASE_SECS,
    };
    let tell = teller(|line: &str| eprintln!("{line}"));
    let what = format!("secrets for `{}`", secrets::cut(&command, 80));
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
    Ok(plan
        .vars
        .iter()
        .map(|(var, r)| {
            let i = secrets.iter().position(|s| s == r).unwrap_or_default();
            (var.clone(), released.values.get(i).cloned().unwrap_or_default())
        })
        .collect())
}

/// Starts the command with `vars` and waits for it, passing on SIGTERM and SIGHUP (Ctrl-C reaches it from the
/// terminal directly). Returns its exit code.
pub async fn execute(command: &[OsString], vars: Vec<(String, Zeroizing<String>)>) -> u8 {
    let Some((program, args)) = command.split_first() else {
        return EXIT_FAILED;
    };
    // On Windows `npm` is `npm.cmd`, which `Command` does not find by itself.
    #[cfg(windows)]
    let resolved = {
        let dirs: Vec<std::path::PathBuf> =
            std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let pathext = std::env::var("PATHEXT").ok();
        crate::win::resolve_program(program, &dirs, pathext.as_deref(), std::path::Path::is_file)
    };
    #[cfg(windows)]
    let mut cmd = tokio::process::Command::new(resolved.as_deref().map_or(program.as_os_str(), |p| p.as_os_str()));
    #[cfg(not(windows))]
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    for (k, v) in &vars {
        cmd.env(k, v.as_str());
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
    let status = wait(&mut child).await;
    match status {
        Ok(s) => exit_code(s),
        Err(e) => {
            eprintln!("reins: waiting for the command: {e}");
            EXIT_FAILED
        }
    }
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

fn exit_code(status: std::process::ExitStatus) -> u8 {
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

/// The whole command: plan, ask, run; the exit code. Messages go to stderr; the command's own output is untouched.
pub async fn main(paths: &Paths, config: &Config, args: &RunArgs) -> u8 {
    let fail = |e: &str| {
        eprintln!("reins: {e}");
        EXIT_FAILED
    };
    let plan = match Plan::new(args, config) {
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
    let phone = match Phone::new(paths, identity, Duration::from_secs(config.approval_timeout_secs)) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let vars =
        match release(&plan, &phone, &Journal::new(paths), Duration::from_secs(config.approval_timeout_secs)).await {
            Ok(v) => v,
            Err(r) => return fail(r.message()),
        };
    drop(phone);
    execute(&plan.command, vars).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(env: &[&str], profile: Option<&str>, command: &[&str]) -> RunArgs {
        RunArgs {
            env: env.iter().map(|s| (*s).to_owned()).collect(),
            profile: profile.map(str::to_owned),
            purpose: None,
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

    #[test]
    fn a_plan_merges_the_profile_and_the_flags() {
        let c = config_with_profile();
        let p =
            Plan::new(&args(&["AWS_SECRET=vault:Other/password", "X=vault:X/y"], Some("deploy"), &["make", "it"]), &c)
                .unwrap();
        let vars: Vec<(&str, String)> = p.vars.iter().map(|(k, v)| (k.as_str(), v.to_string())).collect();
        assert_eq!(
            vars,
            [
                ("AWS_KEY", "vault:AWS/username".to_owned()),
                ("AWS_SECRET", "vault:Other/password".to_owned()),
                ("X", "vault:X/y".to_owned())
            ]
        );
        assert_eq!(p.purpose.as_deref(), Some("deploy the site"));
        assert_eq!(p.shown(), "make it");
        assert!(Plan::new(&args(&[], Some("nope"), &["x"]), &c).unwrap_err().contains("deploy"));
        assert!(Plan::new(&args(&[], None, &["x"]), &c).is_err(), "nothing to release");
        assert!(Plan::new(&args(&["1X=vault:a/b"], None, &["x"]), &c).is_err());
        assert!(Plan::new(&args(&["X=OpenAI/password"], None, &["x"]), &c).is_err());
        assert!(Plan::new(&args(&["X"], None, &["x"]), &c).is_err());
    }

    #[test]
    fn the_same_secret_is_asked_for_once() {
        let p =
            Plan::new(&args(&["A=vault:K/password", "B=vault:K/password"], None, &["x"]), &Config::default()).unwrap();
        assert_eq!(p.secrets().len(), 1);
    }

    #[test]
    fn the_command_is_shown_quoted() {
        let p = Plan::new(&args(&["A=vault:K/p"], None, &["sh", "-c", "echo \"$A\"", ""]), &Config::default()).unwrap();
        assert_eq!(p.shown(), r#"sh -c 'echo "$A"' ''"#);
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
