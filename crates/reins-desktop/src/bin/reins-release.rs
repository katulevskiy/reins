//! `reins-release`: the release key and signed release manifests, for `scripts/release-desktop.sh`. Not shipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use reins_desktop::update::{Asset, Manifest, SIGNING_CONTEXT, Signed};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use sha2::{Digest, Sha256};

#[derive(Parser)]
#[command(name = "reins-release", about = "Release key and signed manifests for the Reins desktop app")]
enum Cli {
    /// Makes a new release key (PKCS#8, 0600) and prints its public half (hex) for `update::RELEASE_KEY`.
    Keygen {
        key: PathBuf,
    },
    /// Prints the public half of a release key.
    Public {
        key: PathBuf,
    },
    /// Writes `latest.json` (signed) and `latest-<platform>.txt` for the given binaries into `out`.
    Manifest {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        build: String,
        #[arg(long)]
        time: i64,
        #[arg(long)]
        out: PathBuf,
        /// `platform=path/to/binary`; the file is published under its own name.
        #[arg(required = true)]
        assets: Vec<String>,
    },
}

fn load(key: &Path) -> Result<Ed25519KeyPair, String> {
    let pkcs8 = std::fs::read(key).map_err(|e| format!("{}: {e}", key.display()))?;
    Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| format!("{} is not an Ed25519 PKCS#8 key", key.display()))
}

fn keygen(key: &Path) -> Result<(), String> {
    if key.exists() {
        return Err(format!("{} exists; refusing to replace a release key", key.display()));
    }
    if let Some(dir) = key.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let pkcs8 =
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).map_err(|_| "key generation failed")?;
    reins_desktop::config::write_private(key, pkcs8.as_ref()).map_err(|e| e.to_string())?;
    println!("{}", HEXLOWER.encode(load(key)?.public_key().as_ref()));
    Ok(())
}

fn manifest(
    key: &Path,
    version: String,
    build: String,
    time: i64,
    out: &Path,
    assets: &[String],
) -> Result<(), String> {
    let pair = load(key)?;
    let mut by_platform = BTreeMap::new();
    for spec in assets {
        let (platform, path) = spec.split_once('=').ok_or_else(|| format!("{spec}: expected platform=path"))?;
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let file = Path::new(path).file_name().and_then(|f| f.to_str()).ok_or("bad file name")?.to_owned();
        by_platform.insert(
            platform.to_owned(),
            Asset {
                file,
                sha256: HEXLOWER.encode(&Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        );
    }
    let m = Manifest {
        version,
        build,
        build_time: time,
        assets: by_platform,
    };
    let text = serde_json::to_string(&m).map_err(|e| e.to_string())?;
    let mut message = SIGNING_CONTEXT.to_vec();
    message.extend_from_slice(text.as_bytes());
    let signed = Signed {
        signature: BASE64URL_NOPAD.encode(pair.sign(&message).as_ref()),
        manifest: text,
    };
    let json = serde_json::to_vec_pretty(&signed).map_err(|e| e.to_string())?;
    // Check it the way `reins update` will before anything is published.
    reins_desktop::update::open(&json, pair.public_key().as_ref())?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join("latest.json"), json).map_err(|e| e.to_string())?;
    for (platform, asset) in &m.assets {
        let line = format!("{} {} {} {}\n", asset.file, asset.sha256, m.version, m.build);
        std::fs::write(out.join(format!("latest-{platform}.txt")), line).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let result = match Cli::parse() {
        Cli::Keygen {
            key,
        } => keygen(&key),
        Cli::Public {
            key,
        } => load(&key).map(|p| println!("{}", HEXLOWER.encode(p.public_key().as_ref()))),
        Cli::Manifest {
            key,
            version,
            build,
            time,
            out,
            assets,
        } => manifest(&key, version, build, time, &out, &assets),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reins-release: {e}");
            ExitCode::FAILURE
        }
    }
}
