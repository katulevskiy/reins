//! `reins-release`: the release key and signed release manifests (`latest.json` for `reins update`, `app.json` for the
//! desktop app's installers), for the release scripts. Not shipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use reins_desktop::update::{self, Asset, Manifest, SIGNING_CONTEXT, Signed};
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
    /// Writes `app.json` (signed) for the desktop app's installers into `out`.
    AppManifest {
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
        /// `platform=path/to/installer`, the platform one of `macos-universal`, `windows-x86_64`, `linux-x86_64`; the
        /// file is published under its own name.
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

/// The manifest of `assets` (`platform=path`) and its signed form, checked with `open` the way the updater will check
/// it, before anything is published.
fn sign(
    key: &Path,
    version: String,
    build: String,
    time: i64,
    assets: &[String],
    open: fn(&[u8], &[u8]) -> Result<Manifest, String>,
) -> Result<(Manifest, Vec<u8>), String> {
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
    open(&json, pair.public_key().as_ref())?;
    Ok((m, json))
}

fn manifest(
    key: &Path,
    version: String,
    build: String,
    time: i64,
    out: &Path,
    assets: &[String],
) -> Result<(), String> {
    let (m, json) = sign(key, version, build, time, assets, update::open)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join("latest.json"), json).map_err(|e| e.to_string())?;
    for (platform, asset) in &m.assets {
        let line = format!("{} {} {} {}\n", asset.file, asset.sha256, m.version, m.build);
        std::fs::write(out.join(format!("latest-{platform}.txt")), line).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `app.json`: the installers the desktop app updates itself from, for the platforms it knows only.
fn app_manifest(
    key: &Path,
    version: String,
    build: String,
    time: i64,
    out: &Path,
    assets: &[String],
) -> Result<(), String> {
    for spec in assets {
        let platform = spec.split_once('=').map_or(spec.as_str(), |(p, _)| p);
        if !update::APP_PLATFORMS.contains(&platform) {
            return Err(format!("{platform}: not an app platform (one of {})", update::APP_PLATFORMS.join(", ")));
        }
    }
    let (_, json) = sign(key, version, build, time, assets, update::open_app)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join("app.json"), json).map_err(|e| e.to_string())
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
        Cli::AppManifest {
            key,
            version,
            build,
            time,
            out,
            assets,
        } => app_manifest(&key, version, build, time, &out, &assets),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reins-release: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release key and two files to list, in `dir`.
    fn fixture(dir: &Path) -> (PathBuf, String, String) {
        let key = dir.join("release.key");
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        std::fs::write(&key, pkcs8.as_ref()).unwrap();
        let dmg = dir.join("Reins-0.2.0-9-abc-macOS.dmg");
        std::fs::write(&dmg, b"disk image").unwrap();
        let msi = dir.join("Reins-0.2.0-9-abc-Windows-x64.msi");
        std::fs::write(&msi, b"installer").unwrap();
        (key, format!("macos-universal={}", dmg.display()), format!("windows-x86_64={}", msi.display()))
    }

    #[test]
    fn the_app_manifest_is_signed_and_lists_only_app_installers() {
        let dir = tempfile::tempdir().unwrap();
        let (key, dmg, msi) = fixture(dir.path());
        let out = dir.path().join("out");
        app_manifest(&key, "0.2.0".into(), "0.2.0-9-abc".into(), 9, &out, &[dmg, msi]).unwrap();
        let written: Vec<_> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(written, ["app.json"], "no latest.json or latest-<platform>.txt");
        let public = load(&key).unwrap().public_key().as_ref().to_vec();
        let m = update::open_app(&std::fs::read(out.join("app.json")).unwrap(), &public).unwrap();
        assert_eq!(m.build_time, 9);
        assert_eq!(m.assets["macos-universal"].file, "Reins-0.2.0-9-abc-macOS.dmg");
        assert_eq!(m.assets["windows-x86_64"].size, 9);
    }

    #[test]
    fn the_app_manifest_refuses_other_platforms() {
        let dir = tempfile::tempdir().unwrap();
        let (key, dmg, _) = fixture(dir.path());
        let out = dir.path().join("out");
        for wrong in ["macos-aarch64", "linux-aarch64", "windows-aarch64", "linux-x86_64-musl"] {
            let spec = dmg.replacen("macos-universal", wrong, 1);
            let err = app_manifest(&key, "0.2.0".into(), "b".into(), 9, &out, &[spec]).unwrap_err();
            assert!(err.contains("not an app platform"), "{err}");
        }
        assert!(!out.exists(), "nothing written");
    }

    #[test]
    fn the_binary_manifest_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (key, dmg, _) = fixture(dir.path());
        let spec = dmg.replacen("macos-universal", "macos-aarch64", 1);
        let out = dir.path().join("out");
        manifest(&key, "0.2.0".into(), "0.2.0-9-abc".into(), 9, &out, &[spec]).unwrap();
        let public = load(&key).unwrap().public_key().as_ref().to_vec();
        let m = update::open(&std::fs::read(out.join("latest.json")).unwrap(), &public).unwrap();
        assert_eq!(m.assets["macos-aarch64"].size, 10);
        let line = std::fs::read_to_string(out.join("latest-macos-aarch64.txt")).unwrap();
        assert!(line.starts_with("Reins-0.2.0-9-abc-macOS.dmg ") && line.ends_with(" 0.2.0 0.2.0-9-abc\n"), "{line}");
        assert!(!out.join("app.json").exists());
    }
}
