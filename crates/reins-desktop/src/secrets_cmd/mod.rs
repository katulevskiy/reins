//! `reins secrets …`: finding and moving secrets (`scan`, `import`), and managing them (design: docs/secrets.md,
//! section 4). Each command lives in its own module; this file only names them.

pub mod import;
mod init;
pub mod person;
pub mod scan;

use clap::Subcommand;

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
}

/// Runs `reins secrets …`; the lines to print.
pub async fn run(cmd: &SecretsCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    if !crate::secrets_scan::preview(config) {
        return Err(crate::secrets_scan::PREVIEW_OFF.to_owned());
    }
    match cmd {
        SecretsCmd::Scan(args) => scan::run(args, paths, config).await,
        SecretsCmd::Import(args) => import::run(args, paths, config).await,
        SecretsCmd::Init {
            name,
            default_env,
        } => init::run(&std::env::current_dir().map_err(|e| e.to_string())?, name.as_deref(), default_env),
    }
}
