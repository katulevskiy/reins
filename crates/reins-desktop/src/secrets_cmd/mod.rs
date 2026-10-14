//! `reins secrets …`: finding and moving secrets (`scan`, `import`), and managing them.

pub mod import;
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
}

/// Runs `reins secrets …`; the lines to print.
pub async fn run(cmd: &SecretsCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    match cmd {
        SecretsCmd::Scan(args) => scan::run(args, paths, config).await,
        SecretsCmd::Import(args) => import::run(args, paths, config).await,
    }
}
