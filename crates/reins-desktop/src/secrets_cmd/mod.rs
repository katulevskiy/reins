//! `reins secrets …`: finding and moving secrets (`scan`, `import`), and managing them (design: docs/secrets.md,
//! section 4). Each command lives in its own module; this file only names them.

mod doctor;
mod get;
pub mod import;
mod init;
mod leases;
pub mod manage;
pub mod person;
pub mod scan;

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::config::{Config, Paths};

#[derive(Clone, Debug, Subcommand)]
pub enum SecretsCmd {
    /// Find the secrets lying around this computer: names, places and what makes them risky, never their values.
    Scan(scan::ScanArgs),
    /// Move what a scan found into the vault on your phone (one approval there), then use it through Reins.
    Import(import::ImportArgs),
    /// Make this folder a project: writes reins.toml (the vault folder is made on the first save).
    Init {
        /// The project's name (default: this folder's name).
        name: Option<String>,
        /// The environment `reins run` uses when none is given.
        #[arg(long = "default-env", value_name = "ENV", default_value = crate::project::DEFAULT_ENV)]
        default_env: String,
    },
    /// Projects, environments and variable names in the vault (never a value).
    Ls {
        /// PROJECT or PROJECT/ENV (default: this project).
        what: Option<String>,
        #[command(flatten)]
        at: At,
    },
    /// Set a variable: typed without echo, or piped in. One approval on your phone.
    Set {
        /// ENV/NAME, or NAME in the default environment.
        target: String,
        #[command(flatten)]
        at: At,
    },
    /// Remove variables (kept in the item's history).
    Unset {
        /// ENV/NAME or NAME.
        #[arg(required = true)]
        targets: Vec<String>,
        #[command(flatten)]
        at: At,
    },
    /// Rename a variable.
    Rename {
        /// ENV/OLD or OLD.
        from: String,
        to: String,
        #[command(flatten)]
        at: At,
    },
    /// Copy every variable of one environment into another, on your phone.
    Cp {
        from: String,
        to: String,
        #[command(flatten)]
        at: At,
    },
    /// Save a dotenv file's values to an environment, then offer to move the file to the trash.
    Push {
        /// The file (default: .env in the project's folder).
        file: Option<PathBuf>,
        #[arg(short = 'e', long = "environment", value_name = "ENV")]
        env: Option<String>,
        /// Keep the file.
        #[arg(long)]
        keep: bool,
        #[command(flatten)]
        at: At,
    },
    /// Write .env.reins: a reference for each variable of the environment.
    Pull {
        #[arg(short = 'e', long = "environment", value_name = "ENV")]
        env: Option<String>,
        #[command(flatten)]
        at: At,
    },
    /// The names one environment has and the other lacks.
    Diff {
        a: String,
        b: String,
        #[command(flatten)]
        at: At,
    },
    /// What the daemon holds (never a value).
    Leases,
    /// Drop what the daemon holds: ENV, PROJECT/ENV, --id or --all (default: this project).
    Revoke {
        what: Option<String>,
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        at: At,
    },
    /// Print one value, for your own script. Asked on your phone every time; refused for an AI agent.
    Get {
        /// ENV/NAME, NAME, env:ENV/NAME or vault:PROJECT/ENV/NAME.
        reference: String,
        #[command(flatten)]
        at: At,
    },
    /// Check this project: references, plaintext files left, guard, daemon.
    Doctor {
        /// Do not ask the phone for names.
        #[arg(long)]
        offline: bool,
        #[command(flatten)]
        at: At,
    },
}

/// The project a command is about.
#[derive(Clone, Debug, Default, Args)]
pub struct At {
    /// The project (default: the reins.toml here or above).
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,
}

/// Runs `reins secrets …`; the lines to print.
pub async fn run(cmd: &SecretsCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    match cmd {
        SecretsCmd::Scan(args) => scan::run(args, paths, config).await,
        SecretsCmd::Import(args) => import::run(args, paths, config).await,
        SecretsCmd::Init {
            name,
            default_env,
        } => init::run(&cwd()?, name.as_deref(), default_env),
        SecretsCmd::Leases => {
            let client = control(paths, config)?;
            Ok(leases::lines(&client.leases().await.map_err(|e| e.to_string())?, crate::now_unix()))
        }
        SecretsCmd::Revoke {
            what,
            id,
            all,
            at,
        } => {
            let here = manage::Here::find(&cwd()?, at.project.as_deref()).ok();
            let req = leases::request(what.as_deref(), id.as_deref(), *all, here.as_ref().map(|h| h.project.as_str()))?;
            let dropped = control(paths, config)?.revoke(&req).await.map_err(|e| e.to_string())?;
            Ok(vec![format!("Dropped {dropped}.")])
        }
        SecretsCmd::Get {
            reference,
            at,
        } => {
            let cwd = cwd()?;
            let here = manage::Here::find(&cwd, at.project.as_deref()).ok();
            let var = get::target(reference, here.as_ref())?;
            let root = here.and_then(|h| h.root).unwrap_or_else(|| cwd.clone());
            get::print(paths, &var, &cwd, &root).await?;
            Ok(Vec::new())
        }
        other => manage_cmd(other, paths, config).await,
    }
}

fn cwd() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| e.to_string())
}

fn control(paths: &Paths, config: &Config) -> Result<crate::control::Client, String> {
    crate::control::Client::new(paths, config.listen).map_err(|e| e.to_string())
}

/// The commands that read names from the phone or store through it.
async fn manage_cmd(cmd: &SecretsCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let cwd = cwd()?;
    let backend = crate::secrets_client::PhoneVaultWriter::new(paths, config, None);
    let here = |at: &At| manage::Here::find(&cwd, at.project.as_deref());
    match cmd {
        SecretsCmd::Ls {
            what,
            at,
        } => manage::ls(&backend, here(at).ok().as_ref(), what.as_deref()).await,
        SecretsCmd::Set {
            target,
            at,
        } => {
            let here = here(at)?;
            let value = manage::read_value(&here, target)?;
            manage::set(&backend, &here, target, value).await
        }
        SecretsCmd::Unset {
            targets,
            at,
        } => manage::unset(&backend, &here(at)?, targets).await,
        SecretsCmd::Rename {
            from,
            to,
            at,
        } => manage::rename(&backend, &here(at)?, from, to).await,
        SecretsCmd::Cp {
            from,
            to,
            at,
        } => manage::cp(&backend, &here(at)?, from, to).await,
        SecretsCmd::Push {
            file,
            env,
            keep,
            at,
        } => {
            let (mut lines, path) = manage::push(&backend, &here(at)?, env.as_deref(), file.as_deref(), &cwd).await?;
            if !keep {
                lines.extend(offer_trash(&path));
            }
            Ok(lines)
        }
        SecretsCmd::Pull {
            env,
            at,
        } => manage::pull(&backend, &here(at)?, env.as_deref()).await,
        SecretsCmd::Diff {
            a,
            b,
            at,
        } => manage::diff(&backend, &here(at)?, a, b).await,
        SecretsCmd::Doctor {
            offline,
            at,
        } => {
            let here = here(at)?;
            let facts = doctor::Facts {
                guard: config.guard.clone(),
                hooked: hooked(paths, config),
                daemon: match control(paths, config) {
                    Ok(c) => c.status().await.map(|_| ()).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                },
            };
            let names: Option<&dyn manage::Backend> = if *offline {
                None
            } else {
                Some(&backend)
            };
            Ok(doctor::run(&here, names, &facts).await)
        }
        _ => Err("not a command of reins secrets".to_owned()),
    }
}

/// After a push, at a terminal: the plaintext file goes to the trash if the user says so.
fn offer_trash(path: &Path) -> Vec<String> {
    let shown = path.display();
    if !person::Terminal::now().stdin {
        return vec![format!("{shown} still holds the values: delete it")];
    }
    match person::ask_tty(&format!("Move {shown} to the trash? [y/N] ")).as_deref() {
        Some("y" | "Y" | "yes") => match trash::delete(path) {
            Ok(()) => vec![format!("{shown}: in the trash")],
            Err(e) => vec![format!("{shown} could not go to the trash ({e}): delete it")],
        },
        _ => vec![format!("{shown} still holds the values")],
    }
}

/// The AI tools with Reins' hooks.
fn hooked(paths: &Paths, config: &Config) -> Vec<&'static str> {
    let Ok(setup) = crate::agents_cli::setup(config) else {
        return Vec::new();
    };
    crate::harness::Harness::ALL
        .into_iter()
        .filter(|h| crate::harness::registered(paths, &setup, *h).is_ok_and(|r| r.hooks.0 > 0))
        .map(crate::harness::Harness::label)
        .collect()
}
