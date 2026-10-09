//! Releases and `reins update`.
//!
//! A release is published (by `scripts/release-desktop.sh`) as `<releases>/latest.json`: a manifest naming one static
//! binary per platform with its SHA-256, signed with the release key whose public half is built into this binary. So
//! `reins update` installs only what the release key signed, never an older build than the running one, and never a
//! file whose hash differs, even if the server that hosts the files were taken over.
//!
//! The desktop app's installers (the disk image, the MSI, the AppImage) are listed the same way, signed with the same
//! key, in `<releases>/app.json` ([`Updater::check_app`]); the app downloads the one for its platform into a cache
//! ([`Updater::fetch_app`]) and installs it when the user says so.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use data_encoding::{BASE64URL_NOPAD, HEXLOWER_PERMISSIVE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where releases are published (under the official site, `reins_proto::official_site!`).
pub const DEFAULT_RELEASES: &str = concat!(reins_proto::official_site!(), "/releases");

/// The release key (Ed25519, hex). Its private half signs every published manifest.
pub const RELEASE_KEY: &str = "dec78a718eb46f7f3b676ddc692d1844fbd169a945b88a02d7b87e98e9f107ab";

/// Prefixed to the manifest before signing, so the key signs nothing else by accident.
pub const SIGNING_CONTEXT: &[u8] = b"reins-release/1\n";

/// The same for `app.json`, so that neither list can be passed off as the other (both name `linux-x86_64`, ...).
pub const APP_SIGNING_CONTEXT: &[u8] = b"reins-app/1\n";

/// Largest binary `reins update` downloads.
const MAX_BINARY: u64 = 64 << 20;

/// Largest app installer the app downloads (a universal disk image is about 30 MB).
pub const MAX_APP: u64 = 512 << 20;

/// The platforms `app.json` lists installers for: one disk image for every Mac, the MSI, the AppImage.
pub const APP_PLATFORMS: [&str; 3] = ["macos-universal", "windows-x86_64", "linux-x86_64"];

/// This binary's version, release id and release time (0 for a local build).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("REINS_BUILD");
#[must_use]
pub fn build_time() -> i64 {
    env!("REINS_BUILD_TIME").parse().unwrap_or(0)
}

/// `0.1.0 (0.1.0-202609301800-b20e06b5)`, for `--version`.
pub const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("REINS_BUILD"), ")");

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

/// The `app.json` platform of this app, when installers exist for it.
#[must_use]
pub fn app_platform() -> Option<&'static str> {
    app_platform_of(std::env::consts::OS, std::env::consts::ARCH)
}

fn app_platform_of(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", _) => Some("macos-universal"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
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
    open_limited(signed_json, public_key, SIGNING_CONTEXT, MAX_BINARY)
}

/// Opens `app.json`: as [`open`], with installers up to [`MAX_APP`].
pub fn open_app(signed_json: &[u8], public_key: &[u8]) -> Result<Manifest, String> {
    open_limited(signed_json, public_key, APP_SIGNING_CONTEXT, MAX_APP)
}

fn open_limited(signed_json: &[u8], public_key: &[u8], context: &[u8], max_size: u64) -> Result<Manifest, String> {
    let signed: Signed = serde_json::from_slice(signed_json).map_err(|_| "the release list is malformed".to_owned())?;
    let signature =
        BASE64URL_NOPAD.decode(signed.signature.as_bytes()).map_err(|_| "the release signature is malformed")?;
    let mut message = context.to_vec();
    message.extend_from_slice(signed.manifest.as_bytes());
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(&message, &signature)
        .map_err(|_| "the release is not signed with the Reins release key; refusing it".to_owned())?;
    let manifest: Manifest =
        serde_json::from_str(&signed.manifest).map_err(|_| "the release manifest is malformed".to_owned())?;
    for asset in manifest.assets.values() {
        if !file_name_ok(&asset.file) || asset.sha256.len() != 64 || asset.size == 0 || asset.size > max_size {
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

    /// For this app's installers (`app.json`): the built-in key, [`app_platform`] and this build's release time.
    pub fn for_this_app(releases: &str) -> Result<Self, String> {
        let platform = app_platform().ok_or("there are no app installers for this platform")?;
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

    /// The answer for `url`; `None` when the server has no such file (HTTP 404).
    async fn send(&self, url: &str) -> Result<Option<reqwest::Response>, String> {
        let resp = self.http.get(url).send().await.map_err(|e| format!("cannot reach {url}: {}", e.without_url()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("{url}: HTTP {}", resp.status().as_u16()));
        }
        Ok(Some(resp))
    }

    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, String> {
        self.get_if_there(url, limit).await?.ok_or_else(|| format!("{url}: HTTP 404"))
    }

    async fn get_if_there(&self, url: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        let Some(mut resp) = self.send(url).await? else {
            return Ok(None);
        };
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("{url}: {}", e.without_url()))? {
            out.extend_from_slice(&chunk);
            if out.len() as u64 > limit {
                return Err(format!("{url} is larger than expected"));
            }
        }
        Ok(Some(out))
    }

    fn compare(&self, manifest: Manifest) -> Check {
        if manifest.build_time <= self.current_time {
            return Check::UpToDate(manifest);
        }
        match manifest.assets.get(&self.platform).cloned() {
            Some(asset) => Check::Available(manifest, asset),
            None => Check::NoBuild(manifest),
        }
    }

    pub async fn check(&self) -> Result<Check, String> {
        let raw = self.get(&format!("{}/latest.json", self.releases), 1 << 20).await?;
        Ok(self.compare(open(&raw, &self.public_key)?))
    }

    /// Checks `app.json`; `None` when the feed has none (one published before the app's installers were).
    pub async fn check_app(&self) -> Result<Option<Check>, String> {
        let Some(raw) = self.get_if_there(&format!("{}/app.json", self.releases), 1 << 20).await? else {
            return Ok(None);
        };
        Ok(Some(self.compare(open_app(&raw, &self.public_key)?)))
    }

    /// Downloads `asset` into the file `dest`, checking its size and SHA-256 as it arrives: it is written next to
    /// `dest` and renamed into place only when both match, so `dest` is the signed file or not there at all.
    pub async fn download_to(&self, asset: &Asset, dest: &Path) -> Result<(), String> {
        let url = format!("{}/files/{}", self.releases, asset.file);
        let mut resp = self.send(&url).await?.ok_or_else(|| format!("{url}: HTTP 404"))?;
        let dir = dest.parent().ok_or("the download has no directory")?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".reins-download-")
            .tempfile_in(dir)
            .map_err(|e| format!("cannot write in {}: {e}", dir.display()))?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("{url}: {}", e.without_url()))? {
            size += chunk.len() as u64;
            if size > asset.size {
                return Err(format!("{url} is larger than expected"));
            }
            hash.update(&chunk);
            tmp.write_all(&chunk).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let got = HEXLOWER_PERMISSIVE.encode(&hash.finalize());
        if size != asset.size || !got.eq_ignore_ascii_case(&asset.sha256) {
            return Err("the downloaded file does not match the signed release; nothing was changed".to_owned());
        }
        tmp.as_file().sync_all().map_err(|e| e.to_string())?;
        tmp.persist(dest).map_err(|e| format!("cannot write {}: {}", dest.display(), e.error))?;
        Ok(())
    }

    /// The installer `asset` in the directory `cache`: the copy already there when it is the signed file, else a new
    /// download. Anything else in `cache` (an earlier release's installer) is removed.
    pub async fn fetch_app(&self, asset: &Asset, cache: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
        let file = cache.join(&asset.file);
        if !is_signed_file(&file, asset) {
            self.download_to(asset, &file).await?;
        }
        remove_downloads(cache, Some(&asset.file));
        Ok(file)
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

/// Whether `file` is `asset`: its size and SHA-256 match the signed ones.
#[must_use]
pub fn is_signed_file(file: &Path, asset: &Asset) -> bool {
    let hashed = || -> std::io::Result<String> {
        let mut f = std::fs::File::open(file)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(HEXLOWER_PERMISSIVE.encode(&hash.finalize()));
            }
            hash.update(&buf[..n]);
        }
    };
    std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.len() == asset.size)
        && hashed().is_ok_and(|got| got.eq_ignore_ascii_case(&asset.sha256))
}

/// Removes the files in the download cache `cache` but `keep` (best effort).
pub fn remove_downloads(cache: &Path, keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let kept = keep.is_some_and(|k| entry.file_name() == k);
        if !kept && entry.file_type().is_ok_and(|t| t.is_file()) {
            std::fs::remove_file(entry.path()).ok();
        }
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
    let mut tmp = tempfile::Builder::new().prefix(".reins-update-").tempfile_in(dir).map_err(|e| {
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

/// The name an executable that is moved aside gets: `reins.exe.<pid>-<nanoseconds>.old`, next to it.
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
    let mut tmp = tempfile::Builder::new().prefix(".reins-update-").tempfile_in(dir).map_err(|e| {
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

/// Where the Reins app (which brings its own `reins`) is downloaded.
pub const APP_DOWNLOADS: &str = concat!(reins_proto::official_site!(), "/download");

/// The Reins desktop app's executable names, next to the `reins` it ships (the macOS bundle's `Contents/MacOS`,
/// the Windows install directory, the AppImage's `usr/bin`).
const APP_EXECUTABLES: [&str; 2] = ["reins-app", "reins-app.exe"];

/// Whether `exe` came with the Reins app. The app updates it (a new app brings a new one); replacing it in place
/// would break the macOS bundle's signature and be undone by the Windows installer.
#[must_use]
pub fn installed_with_app(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| APP_EXECUTABLES.iter().any(|name| dir.join(name).is_file()))
}

/// Whether `exe` is a Linux distribution package's (the .deb, .rpm and AUR packages put it in `/usr/bin`; the install
/// script and the archives use `~/.local/bin` or wherever the user puts it, `/usr/local/bin` included). The package
/// manager updates it: writing over its file would leave the package database describing another program.
#[must_use]
pub fn installed_by_package_manager(exe: &Path) -> bool {
    cfg!(target_os = "linux") && exe.starts_with("/usr") && !exe.starts_with("/usr/local")
}

#[cfg(test)]
mod tests {
    use ring::signature::KeyPair as _;

    #[test]
    fn a_reins_in_usr_is_the_package_managers() {
        let packaged = ["/usr/bin/reins", "/usr/lib/reins/reins"];
        let own = ["/usr/local/bin/reins", "/home/me/.local/bin/reins", "/opt/reins/reins", "/usrx/reins"];
        for exe in packaged {
            assert_eq!(installed_by_package_manager(Path::new(exe)), cfg!(target_os = "linux"), "{exe}");
        }
        for exe in own {
            assert!(!installed_by_package_manager(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn a_reins_next_to_the_app_is_the_apps() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("reins");
        std::fs::write(&exe, b"").unwrap();
        assert!(!installed_with_app(&exe));
        std::fs::write(dir.path().join("reins-app"), b"").unwrap();
        assert!(installed_with_app(&exe));
    }

    use super::*;

    fn key() -> ring::signature::Ed25519KeyPair {
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    pub(crate) fn sign(k: &ring::signature::Ed25519KeyPair, m: &Manifest) -> Vec<u8> {
        sign_with(k, m, SIGNING_CONTEXT)
    }

    fn sign_app(k: &ring::signature::Ed25519KeyPair, m: &Manifest) -> Vec<u8> {
        sign_with(k, m, APP_SIGNING_CONTEXT)
    }

    fn sign_with(k: &ring::signature::Ed25519KeyPair, m: &Manifest, context: &[u8]) -> Vec<u8> {
        let manifest = serde_json::to_string(m).unwrap();
        let mut message = context.to_vec();
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
                    file: "reins-0.1.0-1-abc-linux-x86_64".into(),
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
        let exe = dir.path().join("reins");
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
    fn app_installers_may_be_larger_than_binaries_but_not_unbounded() {
        let k = key();
        let mut dmg = manifest(5);
        dmg.assets = BTreeMap::from([(
            "macos-universal".to_owned(),
            Asset {
                file: "Reins-0.1.0-1-abc-macOS.dmg".into(),
                sha256: "b".repeat(64),
                size: 30 << 20,
            },
        )]);
        let pk = k.public_key().as_ref();
        assert_eq!(open_app(&sign_app(&k, &dmg), pk).unwrap(), dmg);
        assert_eq!(open(&sign(&k, &dmg), pk).unwrap(), dmg, "30 MB is within the binary limit too");
        let big = |size, app: bool| {
            let mut m = dmg.clone();
            m.assets.get_mut("macos-universal").unwrap().size = size;
            if app {
                sign_app(&k, &m)
            } else {
                sign(&k, &m)
            }
        };
        assert!(
            open(&big(MAX_BINARY + 1, false), pk).unwrap_err().contains("malformed"),
            "latest.json keeps its limit"
        );
        assert!(open_app(&big(MAX_BINARY + 1, true), pk).is_ok());
        assert!(open_app(&big(MAX_APP, true), pk).is_ok());
        assert!(open_app(&big(MAX_APP + 1, true), pk).unwrap_err().contains("malformed"));
        assert!(open_app(&sign_app(&k, &dmg), key().public_key().as_ref()).unwrap_err().contains("not signed"));
        let mut bad_file = dmg.clone();
        bad_file.assets.get_mut("macos-universal").unwrap().file = ".hidden.dmg".into();
        assert!(open_app(&sign_app(&k, &bad_file), pk).is_err());
        // Each list is signed for its own purpose: one cannot be served as the other.
        assert!(open_app(&sign(&k, &dmg), pk).unwrap_err().contains("not signed"), "latest.json as app.json");
        assert!(open(&sign_app(&k, &dmg), pk).unwrap_err().contains("not signed"), "app.json as latest.json");
    }

    #[test]
    fn app_platforms_are_one_per_installer() {
        assert_eq!(app_platform_of("macos", "aarch64"), Some("macos-universal"));
        assert_eq!(app_platform_of("macos", "x86_64"), Some("macos-universal"));
        assert_eq!(app_platform_of("windows", "x86_64"), Some("windows-x86_64"));
        assert_eq!(app_platform_of("linux", "x86_64"), Some("linux-x86_64"));
        assert_eq!(app_platform_of("linux", "aarch64"), None);
        assert_eq!(app_platform_of("windows", "aarch64"), None);
        assert_eq!(app_platform_of("freebsd", "x86_64"), None);
        assert!(app_platform().is_none_or(|p| APP_PLATFORMS.contains(&p)));
    }

    #[test]
    fn a_cached_installer_counts_only_when_it_is_the_signed_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Reins-1-Linux-x86_64.AppImage");
        let asset = Asset {
            file: "Reins-1-Linux-x86_64.AppImage".into(),
            sha256: HEXLOWER_PERMISSIVE.encode(&Sha256::digest(b"appimage")),
            size: 8,
        };
        assert!(!is_signed_file(&file, &asset), "not there");
        std::fs::write(&file, b"appimage").unwrap();
        assert!(is_signed_file(&file, &asset));
        std::fs::write(&file, b"appimagX").unwrap();
        assert!(!is_signed_file(&file, &asset), "same size, other bytes");
        std::fs::write(&file, b"appimage!").unwrap();
        assert!(!is_signed_file(&file, &asset), "other size");

        std::fs::write(dir.path().join("Reins-0-Linux-x86_64.AppImage"), b"older").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        remove_downloads(dir.path(), Some(&asset.file));
        let mut left: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        left.sort();
        assert_eq!(left, ["Reins-1-Linux-x86_64.AppImage", "sub"]);
        remove_downloads(dir.path(), None);
        assert!(!file.exists());
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
        let exe = dir.path().join("reins.exe");
        std::fs::write(&exe, b"old").unwrap();
        // What an earlier update left next to it, and someone else's file.
        std::fs::write(dir.path().join("reins.exe.1-2.old"), b"older").unwrap();
        std::fs::write(dir.path().join("notes.old"), b"keep").unwrap();
        replace_moving_aside(&exe, b"new binary").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["notes.old", "reins.exe"], "the old copies are gone, nothing else");
        // A first install: nothing to move aside.
        let fresh = dir.path().join("reins-daemon.exe");
        replace_moving_aside(&fresh, b"gui").unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"gui");
        let aside = aside_name("reins.exe");
        assert!(aside.starts_with("reins.exe.") && Path::new(&aside).extension().is_some_and(|e| e == "old"));
    }
}
