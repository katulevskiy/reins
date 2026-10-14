//! `reins secrets …`: finding and moving secrets (`scan`, `import`), and managing them.

pub mod person;
pub mod scan;

use clap::Subcommand;

use crate::config::{Config, Paths};

#[derive(Clone, Debug, Subcommand)]
pub enum SecretsCmd {
    /// Find the secrets lying around this computer: names, places and what makes them risky, never their values.
    Scan(scan::ScanArgs),
}

/// Runs `reins secrets …`; the lines to print.
pub async fn run(cmd: &SecretsCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    match cmd {
        SecretsCmd::Scan(args) => scan::run(args, paths, config).await,
    }
}
