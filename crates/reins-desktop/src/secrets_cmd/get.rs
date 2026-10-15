//! `reins secrets get REF`: one value on stdout, for a person's script. The daemon decides who asks (from the calling
//! process and its ancestors, as for `reins run`) and refuses an agent; the phone asks every time (a release with
//! `print: true` is never covered by a permission, Autopilot or Approve all) and says the value is printed; the daemon
//! writes it to this process's stdout and keeps nothing. Before that, the courtesy checks of
//! [`super::person`]: an agent's markers and no terminal stop it early (they are not the lock; the phone is).

use std::path::Path;

use super::manage::Here;
use crate::config::Paths;
use crate::secrets::{Reference, Resolved, VarRef};

/// The variable `reference` names: `ENV/NAME`, `NAME` (the default environment), `env:ENV/NAME`, or
/// `vault:PROJECT/ENV/NAME`.
pub fn target(reference: &str, here: Option<&Here>) -> Result<VarRef, String> {
    if Reference::looks_like(reference) {
        let r = Reference::parse_value(reference)?;
        return match r.resolve(here.map(|h| h.project.as_str()))? {
            Resolved::Var(v) => Ok(v),
            Resolved::Field(_) | Resolved::Env(_) => {
                Err(format!("{reference} is not a variable of a project's environment; reins vault reads other items"))
            }
        };
    }
    let here = here.ok_or("no reins.toml here or above: write vault:PROJECT/ENV/NAME, or give --project")?;
    let (env, name) = here.target(reference)?;
    Ok(crate::secrets::EnvRef::new(&here.project, &env)?.var(&name))
}

/// Asks the daemon to print `var`; `root`: the project's folder (or the working folder).
#[cfg(unix)]
pub async fn print(paths: &Paths, var: &VarRef, cwd: &Path, root: &Path) -> Result<(), String> {
    use crate::run_socket::{EnvAsk, RunRequest};
    let caller = crate::secrets_scan::Caller::from_env();
    caller.check("reins secrets get")?;
    let terminal = super::person::Terminal::now();
    if !terminal.stdin || !terminal.tty {
        return Err(
            "reins secrets get runs only in your own terminal. An agent runs its command with reins run.".into()
        );
    }
    let socket = crate::run_socket::socket_path(paths);
    let token = std::fs::read_to_string(paths.control_token_file())
        .ok()
        .filter(|_| socket.exists())
        .map(|t| zeroize::Zeroizing::new(t.trim().to_owned()))
        .ok_or("the Reins daemon is not running: `reins resume` starts it")?;
    let canonical = |p: &Path| std::fs::canonicalize(p).map(|p| p.to_string_lossy().into_owned());
    let req = RunRequest {
        token: token.to_string(),
        argv: vec!["reins".into(), "secrets".into(), "get".into(), var.to_string()],
        cwd: canonical(cwd).map_err(|e| e.to_string())?,
        root: canonical(root).map_err(|e| e.to_string())?,
        env: std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect(),
        asks: vec![EnvAsk {
            project: var.env.project.clone(),
            env: var.env.env.clone(),
            fields: Some(vec![var.name.clone()]),
            lease_secs: crate::secrets::MIN_LEASE_SECS,
            confirm_always: true,
        }],
        vars: Vec::new(),
        whole: Vec::new(),
        no_redact: false,
        purpose: None,
        print: true,
    };
    match crate::run_socket::run(&socket, &req).await? {
        0 => Ok(()),
        _ => Err("the value could not be written".to_owned()),
    }
}

#[cfg(not(unix))]
#[allow(clippy::unused_async, reason = "the same signature as on Unix, where the daemon is asked")]
pub async fn print(_paths: &Paths, _var: &VarRef, _cwd: &Path, _root: &Path) -> Result<(), String> {
    Err("reins secrets get is not available on Windows yet".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_names_one_variable() {
        let here = Here {
            project: "acme".into(),
            default_env: "development".into(),
            root: None,
        };
        let show = |r: &str, h: Option<&Here>| target(r, h).map(|v| v.to_string());
        assert_eq!(show("API_KEY", Some(&here)).unwrap(), "vault:acme/development/API_KEY");
        assert_eq!(show("production/API_KEY", Some(&here)).unwrap(), "vault:acme/production/API_KEY");
        assert_eq!(show("env:production/API_KEY", Some(&here)).unwrap(), "vault:acme/production/API_KEY");
        assert_eq!(show("vault:blog/development/TOKEN", None).unwrap(), "vault:blog/development/TOKEN");
        assert!(show("API_KEY", None).is_err());
        assert!(show("vault:GitHub/password", None).is_err(), "not a project's variable");
        assert!(show("env:production", Some(&here)).is_err());
    }
}
