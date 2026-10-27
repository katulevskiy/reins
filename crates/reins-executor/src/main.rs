//! `reins-executor`: the org executor program.
//!
//! - `reins-executor keygen --out <file>`: makes the executor's key and prints its public key (register it in the
//!   console).
//! - `reins-executor pin --config <file> [--accept]`: fetches the org's issuer and decision keys and prints their
//!   fingerprints to compare with the console; with `--accept` it writes them to the configuration as the pins.
//! - `reins-executor run --config <file>`: runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use reins_audit::TrustedKey;
use reins_executor::config::{ExecutorConfig, pin_text};
use reins_executor::{Client, Context};

const USAGE: &str = "usage:
  reins-executor keygen --out <file>
  reins-executor pin --config <file> [--accept]
  reins-executor run --config <file>";

struct StderrLog;

static LOGGER: StderrLog = StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::max_level() && m.target().starts_with("reins_executor")
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            eprintln!("{} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

fn init_log() {
    let level = match std::env::var("REINS_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("warn") => log::LevelFilter::Warn,
        _ => log::LevelFilter::Info,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

/// The value after `flag`, if given.
fn flag(args: &[String], name: &str) -> Option<PathBuf> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(PathBuf::from)
}

/// `SHA256:<hex>` of a public key, for comparing with the console.
fn fingerprint(public: &[u8; 32]) -> String {
    format!("SHA256:{}", reins_proto::canon::hex32(&reins_proto::canon::sha256(public)))
}

fn trusted((key_id, b64): &(String, String)) -> Result<TrustedKey, String> {
    TrustedKey::parse(&format!("{key_id}={b64}")).ok_or_else(|| format!("the server's key {key_id} is not valid"))
}

fn write_pins(path: &Path, issuer: &TrustedKey, decision: &TrustedKey) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut doc: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut pins = toml::Table::new();
    pins.insert("issuer".into(), toml::Value::String(pin_text(issuer)));
    pins.insert("decision".into(), toml::Value::String(pin_text(decision)));
    doc.insert("pins".into(), toml::Value::Table(pins));
    let out = toml::to_string(&doc).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("writing {}: {e}", path.display()))
}

async fn pin(config: &Path, accept: bool) -> Result<(), String> {
    let cfg = ExecutorConfig::load(config)?;
    let key = Arc::new(reins_executor::client::load_key(&cfg.key_file)?);
    let client = Client::new(&cfg.server, &cfg.org, &cfg.executor_id, key)?;
    let keys = client.keys().await?;
    let (issuer, decision) = (trusted(&keys.issuer)?, trusted(&keys.decision)?);
    let pinned = cfg.pins().ok();
    for (what, k, old) in [
        ("issuer", &issuer, pinned.as_ref().map(|p| &p.issuer)),
        ("decision", &decision, pinned.as_ref().map(|p| &p.decision)),
    ] {
        let note = match old {
            Some(o) if o == k => " (pinned)",
            Some(_) => " (DIFFERS from the pinned key)",
            None => "",
        };
        println!("{what:<9} {}  {}{note}", k.key_id, fingerprint(&k.public));
    }
    if accept {
        write_pins(config, &issuer, &decision)?;
        println!("Pinned in {}.", config.display());
    } else {
        println!("Compare these with the console, then run again with --accept to pin them.");
    }
    Ok(())
}

async fn run(config: &Path) -> Result<(), String> {
    let cfg = ExecutorConfig::load(config)?;
    let ctx = Context::from_config(&cfg)?;
    log::info!("executor {} for {} ({:?}) running", cfg.executor_id, cfg.org, cfg.mode);
    reins_executor::run(&ctx).await;
    Ok(())
}

fn main() {
    init_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = flag(&args, "--config");
    let result = match (args.first().map(String::as_str), config) {
        (Some("keygen"), _) => match flag(&args, "--out") {
            Some(out) => reins_executor::client::generate_key(&out).map(|public| println!("{public}")),
            None => Err(USAGE.to_owned()),
        },
        (Some(cmd @ ("pin" | "run")), Some(config)) => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
            .and_then(|rt| {
                rt.block_on(async {
                    if cmd == "pin" {
                        pin(&config, args.iter().any(|a| a == "--accept")).await
                    } else {
                        run(&config).await
                    }
                })
            }),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
