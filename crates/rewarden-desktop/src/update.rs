//! Releases and `rewarden update`.
//!
//! A release is published (by `scripts/release-desktop.sh`) as `<releases>/latest.json`: a manifest naming one static
//! binary per platform with its SHA-256, signed with the release key whose public half is built into this binary. So
//! `rewarden update` installs only what the release key signed, never an older build than the running one, and never a
//! file whose hash differs, even if the server that hosts the files were taken over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use data_encoding::{BASE64URL_NOPAD, HEXLOWER_PERMISSIVE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where releases are published (under the official site, `rewarden_proto::official_site!`).
pub const DEFAULT_RELEASES: &str = concat!(rewarden_proto::official_site!(), "/releases");

/// The release key (Ed25519, hex). Its private half signs every published manifest.
pub const RELEASE_KEY: &str = "dec78a718eb46f7f3b676ddc692d1844fbd169a945b88a02d7b87e98e9f107ab";

/// Prefixed to the manifest before signing, so the key signs nothing else by accident.
pub const SIGNING_CONTEXT: &[u8] = b"rewarden-release/1\n";

/// Largest binary `rewarden update` downloads.
const MAX_BINARY: u64 = 64 << 20;

/// This binary's version, release id and release time (0 for a local build).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("REWARDEN_BUILD");
#[must_use]
pub fn build_time() -> i64 {
    env!("REWARDEN_BUILD_TIME").parse().unwrap_or(0)
}

/// `0.1.0 (0.1.0-202609301800-b20e06b5)`, for `--version`.
pub const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("REWARDEN_BUILD"), ")");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// File name under `<releases>/files/`.
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub build: String,
    /// Unix seconds; a release is newer when this is larger.
    pub build_time: i64,
    /// By platform: `linux-x86_64`, `linux-aarch64`, ...
    pub assets: BTreeMap<String, Asset>,
}

/// `latest.json`: the manifest exactly as signed, and the signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signed {
    pub manifest: String,
    /// Ed25519 over [`SIGNING_CONTEXT`] + `manifest`, base64url without padding.
    pub signature: String,
}

/// The platform name of this binary, when releases exist for it.
#[must_use]
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        _ => None,
    }
}

fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        && !name.starts_with('.')
}

/// Opens `latest.json`: the signature must verify with `public_key` and the manifest must be well formed.
pub fn open(signed_json: &[u8], public_key: &[u8]) -> Result<Manifest, String> {
    let signed: Signed = serde_json::from_slice(signed_json).map_err(|_| "the release list is malformed".to_owned())?;
    let signature =
        BASE64URL_NOPAD.decode(signed.signature.as_bytes()).map_err(|_| "the release signature is malformed")?;
    let mut message = SIGNING_CONTEXT.to_vec();
    message.extend_from_slice(signed.manifest.as_bytes());
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(&message, &signature)
        .map_err(|_| "the release is not signed with the Rewarden release key; refusing it".to_owned())?;
    let manifest: Manifest =
        serde_json::from_str(&signed.manifest).map_err(|_| "the release manifest is malformed".to_owned())?;
    for asset in manifest.assets.values() {
        if !file_name_ok(&asset.file) || asset.sha256.len() != 64 || asset.size == 0 || asset.size > MAX_BINARY {
            return Err("the release manifest is malformed".to_owned());
        }
    }
    Ok(manifest)
}

/// The built-in release key.
#[must_use]
pub fn release_key() -> Vec<u8> {
    HEXLOWER_PERMISSIVE.decode(RELEASE_KEY.as_bytes()).unwrap_or_default()
}

/// What checking for an update found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// This binary is the latest release (or newer).
    UpToDate(Manifest),
    /// A newer release exists for this platform.
    Available(Manifest, Asset),
    /// A newer release exists, but not for this platform.
    NoBuild(Manifest),
}

pub struct Updater {
    pub releases: String,
    pub public_key: Vec<u8>,
    pub platform: String,
    /// The running binary's release time.
    pub current_time: i64,
    http: reqwest::Client,
}

impl Updater {
    /// For this binary: the built-in key, platform and release time.
    pub fn for_this_binary(releases: &str) -> Result<Self, String> {
        let platform = platform().ok_or("there are no releases for this platform")?;
        Self::new(releases, release_key(), platform, build_time())
    }

    pub fn new(releases: &str, public_key: Vec<u8>, platform: &str, current_time: i64) -> Result<Self, String> {
        Ok(Self {
            releases: releases.trim_end_matches('/').to_owned(),
            public_key,
            platform: platform.to_owned(),
            current_time,
            http: crate::http::client(Some(Duration::from_secs(300)))?,
        })
    }

    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut resp =
            self.http.get(url).send().await.map_err(|e| format!("cannot reach {url}: {}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("{url}: HTTP {}", resp.status().as_u16()));
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("{url}: {}", e.without_url()))? {
            out.extend_from_slice(&chunk);
            if out.len() as u64 > limit {
                return Err(format!("{url} is larger than expected"));
            }
        }
        Ok(out)
    }

    pub async fn check(&self) -> Result<Check, String> {
        let raw = self.get(&format!("{}/latest.json", self.releases), 1 << 20).await?;
        let manifest = open(&raw, &self.public_key)?;
        if manifest.build_time <= self.current_time {
            return Ok(Check::UpToDate(manifest));
        }
        match manifest.assets.get(&self.platform).cloned() {
            Some(asset) => Ok(Check::Available(manifest, asset)),
            None => Ok(Check::NoBuild(manifest)),
        }
    }

    /// Downloads `asset` and checks its size and SHA-256.
    pub async fn download(&self, asset: &Asset) -> Result<Vec<u8>, String> {
        let bytes = self.get(&format!("{}/files/{}", self.releases, asset.file), asset.size).await?;
        let got = HEXLOWER_PERMISSIVE.encode(&Sha256::digest(&bytes));
        if bytes.len() as u64 != asset.size || !got.eq_ignore_ascii_case(&asset.sha256) {
            return Err("the downloaded file does not match the signed release; nothing was changed".to_owned());
        }
        Ok(bytes)
    }
}

/// Puts `bytes` in place of the executable at `exe`: written next to it, made executable, then renamed over it (the
/// running process keeps its old copy). Nothing is changed on failure. On Windows a running program cannot be
/// replaced, only renamed: see [`replace_moving_aside`].
pub fn replace_executable(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    if cfg!(windows) {
        return replace_moving_aside(exe, bytes);
    }
    let dir = exe.parent().ok_or("the executable has no directory")?;
    let mut tmp = tempfile::Builder::new().prefix(".rewarden-update-").tempfile_in(dir).map_err(|e| {
        format!(
            "cannot write next to {}: {e} (reinstall with the install script, or run with permission to write there)",
            exe.display()
        )
    })?;
    std::io::Write::write_all(&mut tmp, bytes).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    tmp.persist(exe).map_err(|e| format!("cannot replace {}: {}", exe.display(), e.error))?;
    Ok(())
}

/// The name an executable that is moved aside gets: `rewarden.exe.<pid>-<nanoseconds>.old`, next to it.
fn aside_name(file_name: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("{file_name}.{}-{nanos}.old", std::process::id())
}

/// The Windows way to replace a program that may be running (Windows refuses to overwrite or delete a running
/// program's file, but lets it be renamed): the new file is written next to it, the old one renamed aside, the new one
/// renamed into its place. If that last step fails the old one is put back, so nothing is changed on failure. The
/// renamed old file goes when nothing runs it any more ([`remove_set_aside`], at the next update or service start).
pub fn replace_moving_aside(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = exe.parent().ok_or("the executable has no directory")?;
    let name = exe.file_name().and_then(|n| n.to_str()).ok_or("the executable has no file name")?;
    remove_set_aside(exe);
    let mut tmp = tempfile::Builder::new().prefix(".rewarden-update-").tempfile_in(dir).map_err(|e| {
        format!(
            "cannot write next to {}: {e} (reinstall with the install script, or run with permission to write there)",
            exe.display()
        )
    })?;
    std::io::Write::write_all(&mut tmp, bytes).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    let aside = dir.join(aside_name(name));
    let moved = match std::fs::rename(exe, &aside) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("cannot move {} aside: {e}", exe.display())),
    };
    if let Err(e) = tmp.persist_noclobber(exe) {
        if moved {
            std::fs::rename(&aside, exe).ok();
        }
        return Err(format!("cannot replace {}: {}", exe.display(), e.error));
    }
    // Gone at once when the old program was not running.
    if moved {
        std::fs::remove_file(&aside).ok();
    }
    Ok(())
}

/// Removes the old copies [`replace_moving_aside`] left next to `exe`, those no program runs any more (best effort).
pub fn remove_set_aside(exe: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let prefix = format!("{name}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let old = Path::new(&file).extension().is_some_and(|e| e.eq_ignore_ascii_case("old"));
        if old && file.to_string_lossy().starts_with(&prefix) {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

/// The running executable, following symlinks to the real file (on Windows without the `\\?\` prefix).
pub fn current_executable() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this executable: {e}"))?;
    let real = std::fs::canonicalize(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    Ok(if cfg!(windows) {
        crate::win::strip_verbatim(&real)
    } else {
        real
    })
}

/// Where the Reins app (which brings its own `rewarden`) is downloaded.
pub const APP_DOWNLOADS: &str = concat!(rewarden_proto::official_site!(), "/download");

/// The Reins desktop app's executable names, next to the `rewarden` it ships (the macOS bundle's `Contents/MacOS`,
/// the Windows install directory, the AppImage's `usr/bin`).
const APP_EXECUTABLES: [&str; 3] = ["Reins", "Reins.exe", "reins-app"];

/// Whether `exe` came with the Reins app. The app updates it (a new app brings a new one); replacing it in place
/// would break the macOS bundle's signature and be undone by the Windows installer.
#[must_use]
pub fn installed_with_app(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| APP_EXECUTABLES.iter().any(|name| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use ring::signature::KeyPair as _;

    #[test]
    fn a_rewarden_next_to_the_app_is_the_apps() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rewarden");
        std::fs::write(&exe, b"").unwrap();
        assert!(!installed_with_app(&exe));
        std::fs::write(dir.path().join("Reins"), b"").unwrap();
        assert!(installed_with_app(&exe));
    }

    use super::*;

    fn key() -> ring::signature::Ed25519KeyPair {
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    pub(crate) fn sign(k: &ring::signature::Ed25519KeyPair, m: &Manifest) -> Vec<u8> {
        let manifest = serde_json::to_string(m).unwrap();
        let mut message = SIGNING_CONTEXT.to_vec();
        message.extend_from_slice(manifest.as_bytes());
        let signature = BASE64URL_NOPAD.encode(k.sign(&message).as_ref());
        serde_json::to_vec(&Signed {
            manifest,
            signature,
        })
        .unwrap()
    }

    fn manifest(time: i64) -> Manifest {
        Manifest {
            version: "0.1.0".into(),
            build: "0.1.0-1-abc".into(),
            build_time: time,
            assets: BTreeMap::from([(
                "linux-x86_64".to_owned(),
                Asset {
                    file: "rewarden-0.1.0-1-abc-linux-x86_64".into(),
                    sha256: "a".repeat(64),
                    size: 3,
                },
            )]),
        }
    }

    #[test]
    fn only_manifests_signed_with_the_release_key_open() {
        let k = key();
        let signed = sign(&k, &manifest(5));
        assert_eq!(open(&signed, k.public_key().as_ref()).unwrap(), manifest(5));
        assert!(open(&signed, key().public_key().as_ref()).unwrap_err().contains("not signed"));
        let mut tampered: Signed = serde_json::from_slice(&signed).unwrap();
        tampered.manifest = tampered.manifest.replace("\"build_time\":5", "\"build_time\":6");
        assert!(open(&serde_json::to_vec(&tampered).unwrap(), k.public_key().as_ref()).is_err());
        let mut bad_file = manifest(5);
        bad_file.assets.get_mut("linux-x86_64").unwrap().file = "../../etc/passwd".into();
        assert!(open(&sign(&k, &bad_file), k.public_key().as_ref()).unwrap_err().contains("malformed"));
        assert!(open(b"{}", k.public_key().as_ref()).is_err());
    }

    #[test]
    fn this_build_knows_its_platform_and_the_key_is_built_in() {
        assert!(platform().is_some());
        assert_eq!(release_key().len(), 32, "RELEASE_KEY must be a 32-byte hex Ed25519 key");
    }

    #[test]
    fn the_executable_is_replaced_whole_and_kept_executable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rewarden");
        std::fs::write(&exe, b"old").unwrap();
        replace_executable(&exe, b"new binary").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file left behind");
    }

    #[test]
    fn windows_platforms_have_release_names() {
        let names =
            ["linux-x86_64", "linux-aarch64", "macos-x86_64", "macos-aarch64", "windows-x86_64", "windows-aarch64"];
        assert!(platform().is_some_and(|p| names.contains(&p)));
    }

    #[test]
    fn a_program_is_replaced_by_moving_the_old_one_aside() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rewarden.exe");
        std::fs::write(&exe, b"old").unwrap();
        // What an earlier update left next to it, and someone else's file.
        std::fs::write(dir.path().join("rewarden.exe.1-2.old"), b"older").unwrap();
        std::fs::write(dir.path().join("notes.old"), b"keep").unwrap();
        replace_moving_aside(&exe, b"new binary").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["notes.old", "rewarden.exe"], "the old copies are gone, nothing else");
        // A first install: nothing to move aside.
        let fresh = dir.path().join("rewarden-daemon.exe");
        replace_moving_aside(&fresh, b"gui").unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"gui");
        let aside = aside_name("rewarden.exe");
        assert!(aside.starts_with("rewarden.exe.") && Path::new(&aside).extension().is_some_and(|e| e == "old"));
    }
}
