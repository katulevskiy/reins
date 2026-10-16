//! The model package on the phone (spec §6): which models this build trusts, downloading and verifying one, and
//! running it through the app's [`ModelRuntime`].
//!
//! A package is `model.onnx`, `tokenizer.json` and `laya_config.json` under `<data_dir>/models/<id>/`, downloaded from
//! `<base>/<id>/<file>` and checked against the SHA-256 pinned here. A file that does not match is deleted with the
//! rest of the package and never loaded. Once every file matched, `staged.json` is written; the engine then loads the
//! package and runs it once (its self-check) and only then renames that to `installed.json`, the one step that puts it
//! in use. The two small files are hashed again whenever the package is opened, the model by its size.
//!
//! A download cut off by the network keeps what it has (`<file>.part`) and goes on from there with a range request
//! next time; the whole file is still hashed against its pin before it is kept.
//!
//! The host also lists each package's hashes in `<base>/<id>/SHA256SUMS`. That list is not trusted (it is not signed):
//! it only tells the automatic update whether the package this build pins is published yet, so that it does not
//! fetch a few hundred megabytes that would fail their check.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::decide::{Pass, softmax, softmax4};
use super::memory::normalized;
use super::runtime::{DownloadProgress, ModelRuntime};
use super::sequence::{LayaTokenizer, QTYPE_CHOICE, Special, build_sequence};
use super::types::{ModelInput, ModelState, ModelStatus};
use crate::CoreError;

pub const MODEL_FILE: &str = "model.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const CONFIG_FILE: &str = "laya_config.json";
const MARKER_FILE: &str = "installed.json";
/// Every file matched its pin; not in use until the self-check passed and it is renamed to [`MARKER_FILE`].
const STAGED_FILE: &str = "staged.json";
/// The host's (unsigned) list of a package's hashes, in `sha256sum` format.
pub const SUMS_FILE: &str = "SHA256SUMS";
/// The largest `SHA256SUMS` read.
const MAX_SUMS_BYTES: usize = 16 * 1024;
const SUMS_TIMEOUT: Duration = Duration::from_secs(30);
/// Where packages are downloaded from (`<base>/<id>/<file>`), under the official site (`reins_proto::official_site!`);
/// `CoreConfig::models_base` overrides it in tests.
pub const DEFAULT_MODELS_BASE: &str = concat!(reins_proto::official_site!(), "/models");
/// The largest file accepted when its size is not pinned.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(30);
/// Progress is reported at least every this many bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// One file of a package, pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownFile {
    pub name: Cow<'static, str>,
    /// Lowercase hex.
    pub sha256: Cow<'static, str>,
    /// Bytes; 0 = not pinned (then at most 2 GB).
    pub size: u64,
}

/// A model this build trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownModel {
    pub id: Cow<'static, str>,
    pub label: Cow<'static, str>,
    pub version: Cow<'static, str>,
    pub files: Cow<'static, [KnownFile]>,
}

impl KnownModel {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    fn file(&self, name: &str) -> Option<&KnownFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// This model's version is newer than `other`'s ([`compare_versions`]); versions that cannot be compared are not.
    pub fn newer_than(&self, other: &Self) -> bool {
        compare_versions(&self.version, &other.version) == Some(std::cmp::Ordering::Greater)
    }
}

/// Compares two model versions made of dot-separated numbers ("2", "2.1", "10" > "9"; "2" = "2.0"). `None` when either
/// is not such a version.
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    fn parts(v: &str) -> Option<Vec<u64>> {
        let v = v.trim();
        if v.is_empty() {
            return None;
        }
        v.split('.')
            .map(|p| (!p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())).then(|| p.parse().ok()).flatten())
            .collect()
    }
    let (mut a, mut b) = (parts(a)?, parts(b)?);
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    Some(a.cmp(&b))
}

/// Reads a `SHA256SUMS` file (`<hex>  <name>` per line, `sha256sum`'s format, binary-mode `*name` too): name → lowercase
/// hash. Anything else in it, or a name twice, makes the whole list unreadable.
pub fn parse_sums(text: &str) -> Result<BTreeMap<String, String>, CoreError> {
    let bad = || CoreError::service("the model server's list of files cannot be read");
    let mut sums = BTreeMap::new();
    for line in text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        let (hash, rest) = line.split_once(' ').ok_or_else(bad)?;
        let name = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*')).ok_or_else(bad)?;
        let name = name.strip_prefix('*').unwrap_or(name);
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) || !safe_name(name) {
            return Err(bad());
        }
        if sums.insert(name.to_owned(), hash.to_ascii_lowercase()).is_some() {
            return Err(bad());
        }
    }
    if sums.is_empty() {
        return Err(bad());
    }
    Ok(sums)
}

/// Whether the host has the package this build pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Published {
    /// Its `SHA256SUMS` lists every pinned file with the pinned hash.
    Yes,
    /// Not there yet (no `SHA256SUMS`).
    No,
    /// There, but not the package this build trusts: downloading it would only fail its check.
    Different,
}

/// Every pinned file is listed with its pinned hash (other files listed are ignored).
pub fn sums_match(known: &KnownModel, sums: &BTreeMap<String, String>) -> bool {
    known.files.iter().all(|pin| {
        pin.sha256 != PLACEHOLDER_SHA256
            && sums.get(pin.name.as_ref()).is_some_and(|h| pin.sha256.eq_ignore_ascii_case(h))
    })
}

/// Asks the host whether it has `known` (`<base>/<id>/SHA256SUMS`). Only a hint: whatever it says, every file is
/// still checked against its pin when it is downloaded.
pub async fn published(http: &reqwest::Client, base: &str, known: &KnownModel) -> Result<Published, CoreError> {
    if !safe_name(&known.id) {
        return Err(CoreError::invalid("not a model id"));
    }
    let url = format!("{}/{}/{SUMS_FILE}", base.trim_end_matches('/'), known.id);
    let resp = http.get(&url).timeout(SUMS_TIMEOUT).send().await?;
    match resp.status().as_u16() {
        404 | 410 => return Ok(Published::No),
        s if !(200..300).contains(&s) => return Err(refused(s)),
        _ => {}
    }
    if resp.content_length().is_some_and(|n| n > MAX_SUMS_BYTES as u64) {
        return Err(CoreError::service("the model server's list of files is too long"));
    }
    let body = resp.bytes().await?;
    if body.len() > MAX_SUMS_BYTES {
        return Err(CoreError::service("the model server's list of files is too long"));
    }
    let text = std::str::from_utf8(&body)
        .map_err(|_| CoreError::service("the model server's list of files cannot be read"))?;
    Ok(if sums_match(known, &parse_sums(text)?) {
        Published::Yes
    } else {
        Published::Different
    })
}

/// PLACEHOLDER — the real hashes and sizes are filled in when the ML tooling (`tools/laya/export_onnx.py`) publishes
/// the first package. With these all-zero hashes no download can ever be accepted, which is the point: nothing
/// unverified is installed.
const PLACEHOLDER_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The models this build accepts, the first being the one `download_model` installs. An older one already installed
/// keeps working until the first is downloaded (`Engine::ap_known`).
pub const KNOWN_MODELS: &[KnownModel] = &[
    // approvals and risk (Overnight): tools/laya/MODEL_CARD.md, "laya-approvals-ml-v2"
    KnownModel {
        id: Cow::Borrowed("laya-approvals-ml-v2"),
        label: Cow::Borrowed("Laya approvals and risk"),
        version: Cow::Borrowed("2"),
        files: Cow::Borrowed(&[
            KnownFile {
                name: Cow::Borrowed(MODEL_FILE),
                sha256: Cow::Borrowed("f84efce3db931f136562dc57060f2bd7154a471ba1415974ff6e4ed1c318afad"),
                size: 336_786_417,
            },
            KnownFile {
                name: Cow::Borrowed(TOKENIZER_FILE),
                sha256: Cow::Borrowed("609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
                size: 34_363_188,
            },
            KnownFile {
                name: Cow::Borrowed(CONFIG_FILE),
                sha256: Cow::Borrowed("f13633d037e29b3e4a6cf31580211e23ab46605f264e16a46922539c43908fbd"),
                size: 1247,
            },
        ]),
    },
    KnownModel {
        id: Cow::Borrowed("laya-approvals-ml-v1"),
        label: Cow::Borrowed("Laya approvals"),
        version: Cow::Borrowed("1"),
        files: Cow::Borrowed(&[
            KnownFile {
                name: Cow::Borrowed(MODEL_FILE),
                sha256: Cow::Borrowed("c90c4b7bc51609b799bbb5c27ebceddc089e5ac13e6cd08dff2756e2a4f1c727"),
                size: 336_774_982,
            },
            KnownFile {
                name: Cow::Borrowed(TOKENIZER_FILE),
                sha256: Cow::Borrowed("609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
                size: 34_363_188,
            },
            KnownFile {
                name: Cow::Borrowed(CONFIG_FILE),
                sha256: Cow::Borrowed("0fa399a2b85ce30a2eb937c427adf3b187592f02af61973d7b9dbc39e6b30ce2"),
                size: 989,
            },
        ]),
    },
];

/// `laya_config.json`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct LayaConfig {
    #[serde(default)]
    pub id: String,
    pub max_len: usize,
    pub head_max_len: usize,
    /// Per question type (choice, score, noul).
    #[serde(default)]
    pub temperature: Vec<f32>,
    /// Per question type and option count ("choice:3-5").
    #[serde(default)]
    pub temperature_by_options: BTreeMap<String, f32>,
    #[serde(default)]
    pub hidden: usize,
    pub mask_id: u32,
    pub cls_id: u32,
    pub sep_id: u32,
    pub pad_id: u32,
    /// The risk output (`docs/autopilot-risk.md`), when the model has one.
    #[serde(default)]
    pub risk: Option<RiskConfig>,
}

/// `laya_config.json` → `risk`: what the phone needs to read the severity output.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RiskConfig {
    /// The ONNX output's name ("risk").
    #[serde(default)]
    pub output: String,
    /// The classes in output order; must be benign, risky, severe, catastrophic.
    pub labels: Vec<String>,
    pub temperature: f32,
    /// Per mode: Overnight holds at or above `thresholds["overnight"]` of P(catastrophic).
    #[serde(default)]
    pub thresholds: BTreeMap<String, f32>,
}

/// Risk classes in the order the model returns them.
pub const RISK_LABELS: [&str; 4] = ["benign", "risky", "severe", "catastrophic"];

impl RiskConfig {
    /// The classes are the four this core knows, in its order.
    pub fn valid(&self) -> bool {
        self.labels.iter().map(String::as_str).eq(RISK_LABELS)
    }

    pub fn temperature(&self) -> f32 {
        Some(self.temperature).filter(|t| t.is_finite() && *t > 0.0).unwrap_or(1.0).clamp(0.05, 20.0)
    }

    /// Overnight holds a request at or above this P(catastrophic). `None` when the package names none, or one that is
    /// not a number in (0, 1]: there is no default, and Overnight then holds everything it judges.
    pub fn overnight_threshold(&self) -> Option<f32> {
        self.thresholds.get("overnight").copied().filter(|t| t.is_finite() && *t > 0.0 && *t <= 1.0)
    }
}

impl LayaConfig {
    /// The temperature of a three-option choice.
    pub fn temperature(&self) -> f32 {
        let t = self.temperature_by_options.get("choice:3-5").or_else(|| self.temperature.first()).copied();
        t.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(1.0).clamp(0.05, 20.0)
    }

    /// The risk output, when the model has one this core can read.
    pub fn risk(&self) -> Option<&RiskConfig> {
        self.risk.as_ref().filter(|r| r.valid())
    }
}

/// What `installed.json` records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    id: String,
    version: String,
    files: Vec<MarkerFile>,
    /// When it was put in use (Unix seconds); absent in packages installed before this was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct MarkerFile {
    name: String,
    sha256: String,
    size: u64,
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `<data_dir>/models/<id>`.
pub fn model_dir(data_dir: &Path, id: &str) -> Result<PathBuf, CoreError> {
    if !safe_name(id) {
        return Err(CoreError::invalid("not a model id"));
    }
    Ok(data_dir.join("models").join(id))
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// SHA-256 (lowercase hex) and size of a file.
fn hash_file(path: &Path) -> Result<(String, u64), CoreError> {
    let (ctx, size) = hash_prefix(path)?;
    Ok((hex(ctx.finish().as_ref()), size))
}

fn matches_pin(pin: &KnownFile, sha256: &str, size: u64) -> bool {
    pin.sha256 != PLACEHOLDER_SHA256 && pin.sha256.eq_ignore_ascii_case(sha256) && (pin.size == 0 || pin.size == size)
}

/// A marker (`installed.json` or `staged.json`) of `known` that matches its pins, with every file present at its size.
fn marker(data_dir: &Path, known: &KnownModel, file: &str) -> Option<Marker> {
    let dir = model_dir(data_dir, &known.id).ok()?;
    let marker = fs::read(dir.join(file)).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok())?;
    let complete = marker.id == known.id
        && known.files.len() == marker.files.len()
        && marker.files.iter().all(|m| {
            known.file(&m.name).is_some_and(|pin| matches_pin(pin, &m.sha256, m.size))
                && fs::metadata(dir.join(&m.name)).is_ok_and(|md| md.len() == m.size)
        });
    complete.then_some(marker)
}

/// Whether `known` is installed completely (marker written, files present with their sizes).
pub fn installed(data_dir: &Path, known: &KnownModel) -> bool {
    marker(data_dir, known, MARKER_FILE).is_some()
}

/// Whether `known` is downloaded and verified but not in use yet (its self-check has not passed).
pub fn staged(data_dir: &Path, known: &KnownModel) -> bool {
    marker(data_dir, known, STAGED_FILE).is_some()
}

/// When the installed `known` was put in use (Unix seconds): as recorded, else when its marker was written.
pub fn installed_at(data_dir: &Path, known: &KnownModel) -> Option<i64> {
    let m = marker(data_dir, known, MARKER_FILE)?;
    m.installed_at.or_else(|| {
        let path = model_dir(data_dir, &known.id).ok()?.join(MARKER_FILE);
        let modified = fs::metadata(path).ok()?.modified().ok()?;
        i64::try_from(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()).ok()
    })
}

/// Puts a staged package in use: `installed.json` takes the place of `staged.json` in one rename, so the package is
/// either in use or not, never half.
pub fn commit(data_dir: &Path, known: &KnownModel, now: i64) -> Result<(), CoreError> {
    let mut m = marker(data_dir, known, STAGED_FILE).ok_or_else(|| CoreError::service("the new model is not ready"))?;
    m.installed_at = Some(now);
    let dir = model_dir(data_dir, &known.id)?;
    write_marker(&dir, &m, MARKER_FILE)?;
    fs::remove_file(dir.join(STAGED_FILE)).ok();
    Ok(())
}

fn write_marker(dir: &Path, marker: &Marker, name: &str) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec(marker).map_err(|e| CoreError::storage(e.to_string()))?;
    let tmp = dir.join(format!("{name}.part"));
    fs::write(&tmp, bytes).map_err(|e| io(&e))?;
    fs::rename(&tmp, dir.join(name)).map_err(|e| io(&e))
}

/// Whether the installed package declares the risk output (read from its configuration, without opening it).
pub fn rates_risk(data_dir: &Path, known: &KnownModel) -> bool {
    model_dir(data_dir, &known.id)
        .ok()
        .and_then(|dir| fs::read(dir.join(CONFIG_FILE)).ok())
        .and_then(|b| serde_json::from_slice::<LayaConfig>(&b).ok())
        .is_some_and(|c| c.risk().is_some())
}

/// Deletes a package (whatever state it is in).
pub fn remove(data_dir: &Path, id: &str) -> Result<(), CoreError> {
    let dir = model_dir(data_dir, id)?;
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CoreError::storage(format!("cannot delete the model: {e}"))),
    }
}

/// A download in progress, as `model_status` reports it.
#[derive(Debug, Default)]
pub struct DownloadState {
    pub running: AtomicBool,
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub error: Mutex<Option<String>>,
    /// The app asked the download running now to stop (the phone left Wi-Fi): it ends as a dropped connection would.
    paused: AtomicBool,
    wake: tokio::sync::Notify,
}

/// What a paused download ends with: a network error, so that what arrived is kept and the next attempt goes on.
fn paused_error() -> CoreError {
    CoreError::Network {
        reason: "the download was paused; it goes on from here next time".to_owned(),
    }
}

impl DownloadState {
    /// Stops the download running now (if any) at its next step; what arrived is kept.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    /// A new download starts: an earlier pause no longer applies.
    pub fn unpause(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    async fn until_paused(&self) {
        loop {
            let woken = self.wake.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if self.paused.load(Ordering::SeqCst) {
                return;
            }
            woken.await;
        }
    }

    pub fn status(&self, known: &KnownModel, installed: bool, runtime_ready: bool) -> ModelStatus {
        let error = self.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let state = if self.running.load(Ordering::SeqCst) {
            ModelState::Downloading
        } else if installed {
            ModelState::Installed
        } else if error.is_some() {
            ModelState::Failed
        } else {
            ModelState::NotInstalled
        };
        ModelStatus {
            update: false,
            state,
            id: known.id.to_string(),
            label: known.label.to_string(),
            version: known.version.to_string(),
            size_bytes: known.size(),
            downloaded_bytes: if state == ModelState::Downloading {
                self.done.load(Ordering::SeqCst)
            } else {
                0
            },
            error: if state == ModelState::Failed {
                error
            } else {
                None
            },
            runtime_ready,
            risk: false,
            updated_at: None,
        }
    }
}

fn io(e: &std::io::Error) -> CoreError {
    CoreError::storage(format!("cannot save the model: {e}"))
}

/// Downloads every file of `known` into its directory, verifying each against its pin, and stages the package
/// ([`staged`]; [`commit`] puts it in use). A download the network cut off keeps its partial files and continues from
/// them next time; on any other failure (a file that does not match, a server that refuses) nothing is kept.
pub async fn download(
    http: &reqwest::Client,
    base: &str,
    data_dir: &Path,
    known: &KnownModel,
    progress: &dyn DownloadProgress,
    state: &DownloadState,
) -> Result<(), CoreError> {
    let dir = model_dir(data_dir, &known.id)?;
    // A pause drops the download between two writes: every chunk written is whole, and the part file goes on from there.
    let result = tokio::select! {
        r = fetch_all(http, base, &dir, known, progress, state) => r,
        () = state.until_paused() => Err(paused_error()),
    };
    match &result {
        // the connection or the server, not the files: what arrived is kept for next time
        Ok(())
        | Err(
            CoreError::Network {
                ..
            }
            | CoreError::Server {
                ..
            },
        ) => {}
        Err(_) => {
            remove(data_dir, &known.id).ok();
        }
    }
    result
}

const MISMATCH: &str = "the downloaded model did not match the one this app trusts; it was deleted";
const TOO_LARGE: &str = "the model file is larger than expected; it was not kept";

/// The download failed its check (a file that does not match its pin): it is not worth fetching again by itself.
pub fn failed_check(e: &CoreError) -> bool {
    matches!(e, CoreError::Service { reason } if reason == MISMATCH || reason == TOO_LARGE)
}

fn refused(status: u16) -> CoreError {
    CoreError::Server {
        status,
        reason: format!("the model server answered {status}"),
    }
}

fn read_err(e: &std::io::Error) -> CoreError {
    CoreError::storage(format!("cannot read the model: {e}"))
}

fn too_large() -> CoreError {
    CoreError::service(TOO_LARGE)
}

/// SHA-256 of a file's first bytes as a running digest, for a download that goes on after them.
fn hash_prefix(path: &Path) -> Result<(ring::digest::Context, u64), CoreError> {
    let mut file = fs::File::open(path).map_err(|e| read_err(&e))?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| read_err(&e))?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        size += n as u64;
    }
    Ok((ctx, size))
}

/// `Content-Range: bytes <start>-<end>/<total>` starts at `from`.
fn range_starts_at(resp: &reqwest::Response, from: u64) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes "))
        .and_then(|v| v.split_once('-'))
        .and_then(|(start, _)| start.trim().parse::<u64>().ok())
        == Some(from)
}

async fn fetch_all(
    http: &reqwest::Client,
    base: &str,
    dir: &Path,
    known: &KnownModel,
    progress: &dyn DownloadProgress,
    state: &DownloadState,
) -> Result<(), CoreError> {
    if known.files.iter().any(|f| !safe_name(&f.name)) {
        return Err(CoreError::invalid("not a model file name"));
    }
    // Whatever marker there was is written again only once every file matched.
    for marker in [MARKER_FILE, STAGED_FILE] {
        match fs::remove_file(dir.join(marker)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(&e)),
            _ => {}
        }
    }
    fs::create_dir_all(dir).map_err(|e| io(&e))?;
    let total = known.size();
    state.total.store(total, Ordering::SeqCst);
    let mut done = 0_u64;
    let mut files = Vec::new();
    for pin in known.files.iter() {
        let limit = if pin.size == 0 {
            MAX_FILE_BYTES
        } else {
            pin.size
        };
        let path = dir.join(pin.name.as_ref());
        // Finished in an earlier attempt: kept if it still matches.
        if path.exists() {
            let (sha, size) = hash_file(&path)?;
            if matches_pin(pin, &sha, size) {
                done += size;
                state.done.store(done, Ordering::SeqCst);
                files.push(MarkerFile {
                    name: pin.name.to_string(),
                    sha256: sha,
                    size,
                });
                continue;
            }
            fs::remove_file(&path).map_err(|e| io(&e))?;
        }
        let part = dir.join(format!("{}.part", pin.name));
        // What an earlier attempt left; its bytes are checked with the rest once the file is whole.
        let mut size = match fs::metadata(&part) {
            Ok(md) if md.len() <= limit => md.len(),
            _ => 0,
        };
        if size == 0 || size < limit {
            let url = format!("{}/{}/{}", base.trim_end_matches('/'), known.id, pin.name);
            let mut req = http.get(&url).timeout(DOWNLOAD_TIMEOUT);
            if size > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={size}-"));
            }
            let mut resp = req.send().await?;
            let status = resp.status().as_u16();
            let append = size > 0 && status == 206 && range_starts_at(&resp, size);
            if !append {
                if !resp.status().is_success() || status == 206 {
                    return Err(refused(status));
                }
                // The server sent the whole file: start it again.
                size = 0;
            }
            if resp.content_length().is_some_and(|n| size + n > limit) {
                return Err(too_large());
            }
            let mut out = if append {
                fs::OpenOptions::new().append(true).open(&part)
            } else {
                fs::File::create(&part)
            }
            .map_err(|e| io(&e))?;
            let mut reported = done + size;
            state.done.store(done + size, Ordering::SeqCst);
            // Written as it comes, so that a dropped connection keeps what arrived.
            loop {
                let chunk = match resp.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(e) => {
                        out.sync_all().ok();
                        return Err(e.into());
                    }
                };
                size += chunk.len() as u64;
                if size > limit {
                    return Err(too_large());
                }
                out.write_all(&chunk).map_err(|e| io(&e))?;
                state.done.store(done + size, Ordering::SeqCst);
                if done + size - reported >= PROGRESS_STEP {
                    reported = done + size;
                    progress.progress(done + size, total);
                }
            }
            out.sync_all().map_err(|e| io(&e))?;
        }
        // The whole file as it is on disk (what an earlier attempt left included), not what this attempt saw.
        let (sha, size) = hash_file(&part)?;
        if !matches_pin(pin, &sha, size) {
            log::warn!("a downloaded model file did not match its pinned hash; the package was deleted");
            return Err(CoreError::service(MISMATCH));
        }
        fs::rename(&part, &path).map_err(|e| io(&e))?;
        done += size;
        files.push(MarkerFile {
            name: pin.name.to_string(),
            sha256: sha,
            size,
        });
    }
    let marker = Marker {
        id: known.id.to_string(),
        version: known.version.to_string(),
        files,
        installed_at: None,
    };
    write_marker(dir, &marker, STAGED_FILE)?;
    progress.progress(done, total.max(done));
    Ok(())
}

/// Writes a package's files and marker directly, as a finished download leaves them (tests).
#[cfg(any(test, feature = "testing"))]
pub(crate) fn install_files(data_dir: &Path, known: &KnownModel, files: &[(String, Vec<u8>)]) -> Result<(), CoreError> {
    let dir = model_dir(data_dir, &known.id)?;
    fs::create_dir_all(&dir).map_err(|e| io(&e))?;
    let mut marker = Marker {
        id: known.id.to_string(),
        version: known.version.to_string(),
        files: Vec::new(),
        installed_at: None,
    };
    for (name, bytes) in files {
        fs::write(dir.join(name), bytes).map_err(|e| io(&e))?;
        let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
        marker.files.push(MarkerFile {
            name: name.clone(),
            sha256: hex(digest.as_ref()),
            size: bytes.len() as u64,
        });
    }
    let bytes = serde_json::to_vec(&marker).map_err(|e| CoreError::storage(e.to_string()))?;
    fs::write(dir.join(MARKER_FILE), bytes).map_err(|e| io(&e))
}

/// An opened, verified package: its tokenizer, its configuration and where the ONNX file is.
pub struct Laya {
    pub id: String,
    pub config: LayaConfig,
    tokenizer: LayaTokenizer,
    special: Special,
    pub model_path: PathBuf,
}

impl Laya {
    /// Opens an installed package, hashing its small files again. A package that does not match is deleted.
    pub fn open(data_dir: &Path, known: &KnownModel) -> Result<Self, CoreError> {
        if !installed(data_dir, known) {
            return Err(CoreError::service("no model is installed"));
        }
        Self::open_verified(data_dir, known)
    }

    /// Opens a staged package (downloaded and verified, not in use yet) for its self-check, just before it is loaded and
    /// committed: every file, the model too, is hashed again from disk, so that nothing changed since the download is
    /// loaded or put in use. A package that does not match is deleted.
    pub fn open_staged(data_dir: &Path, known: &KnownModel) -> Result<Self, CoreError> {
        if !staged(data_dir, known) {
            return Err(CoreError::service("the new model is not ready"));
        }
        let dir = model_dir(data_dir, &known.id)?;
        for pin in known.files.iter() {
            let (sha, size) = hash_file(&dir.join(pin.name.as_ref()))?;
            if !matches_pin(pin, &sha, size) {
                log::warn!("a downloaded model file changed before it was put in use; the package was deleted");
                remove(data_dir, &known.id).ok();
                return Err(CoreError::service(MISMATCH));
            }
        }
        Self::from_dir(&dir, &known.id)
    }

    fn open_verified(data_dir: &Path, known: &KnownModel) -> Result<Self, CoreError> {
        let dir = model_dir(data_dir, &known.id)?;
        for name in [TOKENIZER_FILE, CONFIG_FILE] {
            let (sha, size) = hash_file(&dir.join(name))?;
            if !known.file(name).is_some_and(|pin| matches_pin(pin, &sha, size)) {
                log::warn!("an installed model file changed; the package was deleted");
                remove(data_dir, &known.id).ok();
                return Err(CoreError::service("the installed model was damaged and has been deleted"));
            }
        }
        Self::from_dir(&dir, &known.id)
    }

    /// Opens a package directory as it is, without checking it against the pins: for desktop tools and tests
    /// (`reins-laya`). The phone only ever opens verified packages with [`Laya::open`].
    pub fn open_unverified(dir: &Path) -> Result<Self, CoreError> {
        let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self::from_dir(dir, &id)
    }

    fn from_dir(dir: &Path, id: &str) -> Result<Self, CoreError> {
        let bad = |why: &str| CoreError::storage(format!("the model's configuration {why}"));
        let config: LayaConfig =
            serde_json::from_slice(&fs::read(dir.join(CONFIG_FILE)).map_err(|e| CoreError::storage(e.to_string()))?)
                .map_err(|_| bad("cannot be read"))?;
        if !(16..=8_192).contains(&config.max_len) || config.head_max_len >= config.max_len {
            return Err(bad("has impossible lengths"));
        }
        let tokenizer = LayaTokenizer::from_bytes(
            &fs::read(dir.join(TOKENIZER_FILE)).map_err(|e| CoreError::storage(e.to_string()))?,
        )?;
        let mask_token =
            tokenizer.token(config.mask_id).ok_or_else(|| bad("names a mask token the tokenizer lacks"))?;
        for id in [config.cls_id, config.sep_id, config.pad_id] {
            tokenizer.token(id).ok_or_else(|| bad("names a token the tokenizer lacks"))?;
        }
        let special = Special {
            cls: config.cls_id,
            sep: config.sep_id,
            mask: config.mask_id,
            pad: config.pad_id,
            mask_token,
        };
        Ok(Self {
            id: id.to_owned(),
            config,
            tokenizer,
            special,
            model_path: dir.join(MODEL_FILE),
        })
    }

    /// The batch for `states`, padded to the longest.
    pub fn input(&self, states: &[&str]) -> Result<ModelInput, CoreError> {
        let mut seqs = Vec::with_capacity(states.len());
        for s in states {
            let (ids, markers) =
                build_sequence(&self.tokenizer, &self.special, s, self.config.max_len, self.config.head_max_len)?;
            if markers.len() != 3 {
                return Err(CoreError::storage("the question does not fit the model's length"));
            }
            seqs.push((ids, markers));
        }
        let len = seqs.iter().map(|(ids, _)| ids.len()).max().unwrap_or(0);
        let mut input = ModelInput {
            batch: u32::try_from(seqs.len()).unwrap_or(u32::MAX),
            seq_len: u32::try_from(len).unwrap_or(u32::MAX),
            k: 3,
            input_ids: Vec::with_capacity(seqs.len() * len),
            attention_mask: Vec::with_capacity(seqs.len() * len),
            marker_pos: Vec::with_capacity(seqs.len() * 3),
            marker_mask: vec![1; seqs.len() * 3],
            qtype: vec![QTYPE_CHOICE; seqs.len()],
        };
        for (ids, markers) in seqs {
            for i in 0..len {
                let (id, on) = ids.get(i).map_or((self.special.pad, 0), |id| (*id, 1));
                input.input_ids.push(i64::from(id));
                input.attention_mask.push(on);
            }
            input.marker_pos.extend(markers.iter().map(|m| i64::try_from(*m).unwrap_or(i64::MAX)));
        }
        Ok(input)
    }

    /// The model rates risk (and reads the `context` line of the situation).
    pub fn has_risk(&self) -> bool {
        self.config.risk().is_some()
    }

    /// One forward pass per state (batched), calibrated. The risk of each pass is read when the package declares the
    /// output and the runtime returned it (an app whose runtime does not read the output gives none: no risk).
    pub fn run(&self, runtime: &dyn ModelRuntime, states: &[&str]) -> Result<Vec<Pass>, CoreError> {
        let input = self.input(states)?;
        let batch = states.len();
        let out = runtime.run(input).map_err(|e| CoreError::service(format!("the model failed: {e}")))?;
        let hidden = out.hidden as usize;
        if hidden == 0
            || hidden > 16_384
            || out.logits.len() != batch * 3
            || out.act.len() != batch * 2
            || out.pooled.len() != batch * hidden
        {
            return Err(CoreError::service("the model returned something of the wrong shape"));
        }
        if out.logits.iter().chain(&out.act).chain(&out.pooled).chain(&out.risk).any(|x| !x.is_finite()) {
            return Err(CoreError::service("the model returned something that is not a number"));
        }
        let t = self.config.temperature();
        let risk =
            self.config.risk().filter(|_| out.risk.len() == batch * RISK_LABELS.len()).map(RiskConfig::temperature);
        if self.config.risk().is_some() && risk.is_none() {
            log::warn!("the model has a risk output but the runtime returned none; risk is not rated");
        }
        Ok((0..batch)
            .map(|b| {
                let logits = [out.logits[b * 3] / t, out.logits[b * 3 + 1] / t, out.logits[b * 3 + 2] / t];
                let (answer, escalate) = (out.act[b * 2], out.act[b * 2 + 1]);
                Pass {
                    logits,
                    probs: softmax(logits),
                    escalate: 1.0 / (1.0 + (answer - escalate).exp()),
                    pooled: normalized(&out.pooled[b * hidden..(b + 1) * hidden]),
                    risk: risk.map(|tr| {
                        let z: [f32; 4] = std::array::from_fn(|i| out.risk[b * 4 + i] / tr);
                        softmax4(z)
                    }),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_never_accepted() {
        let known = &KNOWN_MODELS[0];
        for f in known.files.iter() {
            assert!(!matches_pin(f, PLACEHOLDER_SHA256, f.size), "an all-zero hash matches nothing");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(!installed(dir.path(), known));
    }

    #[test]
    fn versions_compare_as_numbers() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        for (a, b, want) in [
            ("2", "1", Some(Greater)),
            ("1", "2", Some(Less)),
            ("10", "9", Some(Greater)),
            ("2", "2.0", Some(Equal)),
            ("2.1", "2", Some(Greater)),
            ("2.0.1", "2.1", Some(Less)),
            (" 3 ", "3", Some(Equal)),
            ("test", "1", None),
            ("", "1", None),
            ("1..2", "1", None),
            ("1.", "1", None),
            ("-1", "1", None),
            ("1e3", "1", None),
            ("99999999999999999999999", "1", None),
        ] {
            assert_eq!(compare_versions(a, b), want, "{a} vs {b}");
        }
        // Each model this build trusts is newer than the ones after it: the first is what an update installs.
        for pair in KNOWN_MODELS.windows(2) {
            assert!(pair[0].newer_than(&pair[1]), "{} is newer than {}", pair[0].id, pair[1].id);
        }
        assert!(!KNOWN_MODELS[1].newer_than(&KNOWN_MODELS[0]));
    }

    const A: &str = "f84efce3db931f136562dc57060f2bd7154a471ba1415974ff6e4ed1c318afad";
    const B: &str = "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f";

    #[test]
    fn the_hosts_list_of_hashes_is_read_strictly() {
        let sums = parse_sums(&format!("{A}  model.onnx\n{}  *tokenizer.json\n\n", B.to_uppercase())).unwrap();
        assert_eq!(sums.get("model.onnx").map(String::as_str), Some(A));
        assert_eq!(sums.get("tokenizer.json").map(String::as_str), Some(B), "lower case, binary mode read");
        for bad in [
            String::new(),
            "\n\n".to_owned(),
            format!("{A} model.onnx"),
            format!("{}  model.onnx", &A[..63]),
            format!("{}g  model.onnx", &A[..63]),
            format!("{A}  ../model.onnx"),
            format!("{A}  dir/model.onnx"),
            format!("{A}  model.onnx\n{B}  model.onnx"),
            format!("{A}\tmodel.onnx"),
            format!("{A}  "),
        ] {
            assert!(parse_sums(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_list_says_published_only_with_every_pinned_hash() {
        let v2 = &KNOWN_MODELS[0];
        let all = v2.files.iter().map(|f| format!("{}  {}", f.sha256, f.name)).collect::<Vec<_>>().join("\n");
        assert!(sums_match(v2, &parse_sums(&all).unwrap()));
        // What the server serves today (/srv/reins-models/laya-approvals-ml-v2/SHA256SUMS).
        let served = "f84efce3db931f136562dc57060f2bd7154a471ba1415974ff6e4ed1c318afad  model.onnx\n\
                      609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f  tokenizer.json\n\
                      f13633d037e29b3e4a6cf31580211e23ab46605f264e16a46922539c43908fbd  laya_config.json\n";
        assert!(sums_match(v2, &parse_sums(served).unwrap()));
        assert!(!sums_match(&KNOWN_MODELS[1], &parse_sums(served).unwrap()), "v1 pins another model file");
        let extra = format!("{all}\n{A}  README.md\n");
        assert!(sums_match(v2, &parse_sums(&extra).unwrap()), "files besides the pinned ones are ignored");
        let missing = all.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(!sums_match(v2, &parse_sums(&missing).unwrap()));
        let other = all.replacen(&v2.files[0].sha256.to_string(), B, 1);
        assert!(!sums_match(v2, &parse_sums(&other).unwrap()));
        // A placeholder pin matches no list, even one that names it.
        let mut placeholder = v2.clone();
        placeholder.files.to_mut()[0].sha256 = Cow::Borrowed(PLACEHOLDER_SHA256);
        let zeros = all.replacen(&v2.files[0].sha256.to_string(), PLACEHOLDER_SHA256, 1);
        assert!(!sums_match(&placeholder, &parse_sums(&zeros).unwrap()));
    }

    #[test]
    fn a_staged_package_is_in_use_only_once_committed() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![(MODEL_FILE.to_owned(), b"m".to_vec()), (CONFIG_FILE.to_owned(), b"{}".to_vec())];
        let known = KnownModel {
            id: Cow::Borrowed("staged-1"),
            label: Cow::Borrowed("x"),
            version: Cow::Borrowed("1"),
            files: Cow::Owned(
                files
                    .iter()
                    .map(|(name, bytes)| KnownFile {
                        name: Cow::Owned(name.clone()),
                        sha256: Cow::Owned(hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())),
                        size: bytes.len() as u64,
                    })
                    .collect(),
            ),
        };
        install_files(dir.path(), &known, &files).unwrap();
        let pkg = model_dir(dir.path(), "staged-1").unwrap();
        fs::rename(pkg.join(MARKER_FILE), pkg.join(STAGED_FILE)).unwrap();
        assert!(staged(dir.path(), &known) && !installed(dir.path(), &known));
        assert_eq!(installed_at(dir.path(), &known), None);
        commit(dir.path(), &known, 1_800_000_000).unwrap();
        assert!(installed(dir.path(), &known) && !staged(dir.path(), &known));
        assert_eq!(installed_at(dir.path(), &known), Some(1_800_000_000));
        assert!(commit(dir.path(), &known, 1).is_err(), "nothing staged any more");
    }

    #[test]
    fn ids_and_file_names_stay_inside_the_models_directory() {
        let dir = Path::new("/data");
        assert!(model_dir(dir, "../x").is_err());
        assert!(model_dir(dir, "a/b").is_err());
        assert!(model_dir(dir, "..").is_err());
        assert_eq!(model_dir(dir, "laya-1.0").unwrap(), Path::new("/data/models/laya-1.0"));
    }

    #[test]
    fn the_risk_output_is_read_only_with_the_classes_this_core_knows() {
        let cfg: LayaConfig = serde_json::from_value(serde_json::json!({
            "max_len": 512, "head_max_len": 256, "mask_id": 4, "cls_id": 2, "sep_id": 1, "pad_id": 0,
            "risk": {"output": "risk", "labels": RISK_LABELS, "temperature": 1.7, "thresholds": {"overnight": 0.31}}
        }))
        .unwrap();
        let risk = cfg.risk().unwrap();
        assert!((risk.temperature() - 1.7).abs() < 1e-6 && (risk.overnight_threshold().unwrap() - 0.31).abs() < 1e-6);
        let mut other = cfg.clone();
        other.risk.as_mut().unwrap().labels.reverse();
        assert!(other.risk().is_none(), "classes in another order are not read");
        let mut none = cfg.clone();
        none.risk.as_mut().unwrap().thresholds.clear();
        assert_eq!(none.risk().unwrap().overnight_threshold(), None, "no default threshold");
        for odd in [f32::NAN, 0.0, -0.5, 1.5] {
            none.risk.as_mut().unwrap().thresholds.insert("overnight".to_owned(), odd);
            assert_eq!(none.risk().unwrap().overnight_threshold(), None, "{odd}");
        }
        let v1: LayaConfig = serde_json::from_value(serde_json::json!({
            "max_len": 512, "head_max_len": 256, "mask_id": 4, "cls_id": 2, "sep_id": 1, "pad_id": 0}))
        .unwrap();
        assert!(v1.risk().is_none(), "laya-approvals-ml-v1 has no risk output");
    }

    #[test]
    fn temperature_of_a_three_option_choice() {
        let mut cfg = LayaConfig {
            id: String::new(),
            max_len: 512,
            head_max_len: 192,
            temperature: vec![1.6, 1.2, 1.9],
            temperature_by_options: BTreeMap::new(),
            hidden: 0,
            mask_id: 0,
            cls_id: 0,
            sep_id: 0,
            pad_id: 0,
            risk: None,
        };
        assert!((cfg.temperature() - 1.6).abs() < 1e-6);
        cfg.temperature_by_options.insert("choice:3-5".to_owned(), 1.76);
        assert!((cfg.temperature() - 1.76).abs() < 1e-6);
        cfg.temperature_by_options.insert("choice:3-5".to_owned(), f32::NAN);
        assert!((cfg.temperature() - 1.0).abs() < 1e-6);
    }
}
