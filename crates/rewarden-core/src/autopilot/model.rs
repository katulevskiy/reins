//! The model package on the phone (spec §6): which models this build trusts, downloading and verifying one, and
//! running it through the app's [`ModelRuntime`].
//!
//! A package is `model.onnx`, `tokenizer.json` and `laya_config.json` under `<data_dir>/models/<id>/`, downloaded from
//! `<base>/<id>/<file>` and checked against the SHA-256 pinned here. A file that does not match is deleted with the
//! rest of the package and never loaded. `installed.json` is written last, once every file matched; the two small
//! files are hashed again whenever the package is opened, the model by its size.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::decide::{Pass, softmax};
use super::memory::normalized;
use super::runtime::{DownloadProgress, ModelRuntime};
use super::sequence::{LayaTokenizer, QTYPE_CHOICE, Special, build_sequence};
use super::types::{ModelInput, ModelState, ModelStatus};
use crate::CoreError;

pub const MODEL_FILE: &str = "model.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const CONFIG_FILE: &str = "laya_config.json";
const MARKER_FILE: &str = "installed.json";
/// Where packages are downloaded from (`<base>/<id>/<file>`), under the official site (`rewarden_proto::official_site!`);
/// `CoreConfig::models_base` overrides it in tests.
pub const DEFAULT_MODELS_BASE: &str = concat!(rewarden_proto::official_site!(), "/models");
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
}

/// PLACEHOLDER — the real hashes and sizes are filled in when the ML tooling (`tools/laya/export_onnx.py`) publishes
/// the first package. With these all-zero hashes no download can ever be accepted, which is the point: nothing
/// unverified is installed.
const PLACEHOLDER_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The models this build accepts, the first being the one `download_model` installs.
pub const KNOWN_MODELS: &[KnownModel] = &[KnownModel {
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
}];

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
}

impl LayaConfig {
    /// The temperature of a three-option choice.
    pub fn temperature(&self) -> f32 {
        let t = self.temperature_by_options.get("choice:3-5").or_else(|| self.temperature.first()).copied();
        t.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(1.0).clamp(0.05, 20.0)
    }
}

/// What `installed.json` records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    id: String,
    version: String,
    files: Vec<MarkerFile>,
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
    let mut file = fs::File::open(path).map_err(|e| CoreError::storage(format!("cannot read the model: {e}")))?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| CoreError::storage(format!("cannot read the model: {e}")))?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex(ctx.finish().as_ref()), size))
}

fn matches_pin(pin: &KnownFile, sha256: &str, size: u64) -> bool {
    pin.sha256 != PLACEHOLDER_SHA256 && pin.sha256.eq_ignore_ascii_case(sha256) && (pin.size == 0 || pin.size == size)
}

/// Whether `known` is installed completely (marker written, files present with their sizes).
pub fn installed(data_dir: &Path, known: &KnownModel) -> bool {
    let Ok(dir) = model_dir(data_dir, &known.id) else {
        return false;
    };
    let Some(marker) = fs::read(dir.join(MARKER_FILE)).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok())
    else {
        return false;
    };
    marker.id == known.id
        && known.files.len() == marker.files.len()
        && marker.files.iter().all(|m| {
            known.file(&m.name).is_some_and(|pin| matches_pin(pin, &m.sha256, m.size))
                && fs::metadata(dir.join(&m.name)).is_ok_and(|md| md.len() == m.size)
        })
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
}

impl DownloadState {
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
        }
    }
}

fn io(e: &std::io::Error) -> CoreError {
    CoreError::storage(format!("cannot save the model: {e}"))
}

/// Downloads every file of `known` into its directory, verifying each; on any failure nothing is kept.
pub async fn download(
    http: &reqwest::Client,
    base: &str,
    data_dir: &Path,
    known: &KnownModel,
    progress: &dyn DownloadProgress,
    state: &DownloadState,
) -> Result<(), CoreError> {
    let dir = model_dir(data_dir, &known.id)?;
    let result = fetch_all(http, base, &dir, known, progress, state).await;
    if result.is_err() {
        remove(data_dir, &known.id).ok();
    }
    result
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
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| io(&e))?;
    }
    fs::create_dir_all(dir).map_err(|e| io(&e))?;
    let total = known.size();
    state.total.store(total, Ordering::SeqCst);
    let mut done = 0_u64;
    let mut files = Vec::new();
    for pin in known.files.iter() {
        let url = format!("{}/{}/{}", base.trim_end_matches('/'), known.id, pin.name);
        let mut resp = http.get(&url).timeout(DOWNLOAD_TIMEOUT).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::service(format!("the model server answered {}", resp.status().as_u16())));
        }
        let limit = if pin.size == 0 {
            MAX_FILE_BYTES
        } else {
            pin.size
        };
        if resp.content_length().is_some_and(|n| n > limit) {
            return Err(CoreError::service("the model file is larger than expected; it was not kept"));
        }
        let part = dir.join(format!("{}.part", pin.name));
        let mut out = fs::File::create(&part).map_err(|e| io(&e))?;
        let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
        let mut size = 0_u64;
        let mut reported = 0_u64;
        while let Some(chunk) = resp.chunk().await? {
            size += chunk.len() as u64;
            if size > limit {
                return Err(CoreError::service("the model file is larger than expected; it was not kept"));
            }
            ctx.update(&chunk);
            out.write_all(&chunk).map_err(|e| io(&e))?;
            done += chunk.len() as u64;
            state.done.store(done, Ordering::SeqCst);
            if done - reported >= PROGRESS_STEP {
                reported = done;
                progress.progress(done, total);
            }
        }
        out.sync_all().map_err(|e| io(&e))?;
        drop(out);
        let sha = hex(ctx.finish().as_ref());
        if !matches_pin(pin, &sha, size) {
            log::warn!("a downloaded model file did not match its pinned hash; the package was deleted");
            return Err(CoreError::service(
                "the downloaded model did not match the one this app trusts; it was deleted",
            ));
        }
        fs::rename(&part, dir.join(pin.name.as_ref())).map_err(|e| io(&e))?;
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
    };
    let bytes = serde_json::to_vec(&marker).map_err(|e| CoreError::storage(e.to_string()))?;
    let tmp = dir.join(format!("{MARKER_FILE}.part"));
    fs::write(&tmp, bytes).map_err(|e| io(&e))?;
    fs::rename(&tmp, dir.join(MARKER_FILE)).map_err(|e| io(&e))?;
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
    /// (`rewarden-laya`). The phone only ever opens verified packages with [`Laya::open`].
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

    /// One forward pass per state (batched), calibrated.
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
        if out.logits.iter().chain(&out.act).chain(&out.pooled).any(|x| !x.is_finite()) {
            return Err(CoreError::service("the model returned something that is not a number"));
        }
        let t = self.config.temperature();
        Ok((0..batch)
            .map(|b| {
                let logits = [out.logits[b * 3] / t, out.logits[b * 3 + 1] / t, out.logits[b * 3 + 2] / t];
                let (answer, escalate) = (out.act[b * 2], out.act[b * 2 + 1]);
                Pass {
                    logits,
                    probs: softmax(logits),
                    escalate: 1.0 / (1.0 + (answer - escalate).exp()),
                    pooled: normalized(&out.pooled[b * hidden..(b + 1) * hidden]),
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
    fn ids_and_file_names_stay_inside_the_models_directory() {
        let dir = Path::new("/data");
        assert!(model_dir(dir, "../x").is_err());
        assert!(model_dir(dir, "a/b").is_err());
        assert!(model_dir(dir, "..").is_err());
        assert_eq!(model_dir(dir, "laya-1.0").unwrap(), Path::new("/data/models/laya-1.0"));
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
        };
        assert!((cfg.temperature() - 1.6).abs() < 1e-6);
        cfg.temperature_by_options.insert("choice:3-5".to_owned(), 1.76);
        assert!((cfg.temperature() - 1.76).abs() < 1e-6);
        cfg.temperature_by_options.insert("choice:3-5".to_owned(), f32::NAN);
        assert!((cfg.temperature() - 1.0).abs() < 1e-6);
    }
}
