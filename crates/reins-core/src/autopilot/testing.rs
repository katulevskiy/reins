//! Deterministic stand-ins for Autopilot's model, for tests (feature `testing`): a one-character-per-token
//! tokenizer, a package around it, and a [`ModelRuntime`] whose embeddings come from hashed words and whose logits
//! come from keywords.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use super::model::{self, CONFIG_FILE, KnownFile, KnownModel, MODEL_FILE, TOKENIZER_FILE};
use super::runtime::{DownloadProgress, ModelRuntime};
use super::types::{ModelInput, ModelOutput};
use crate::error::ForeignError;

pub const PAD: u32 = 0;
pub const UNK: u32 = 1;
pub const CLS: u32 = 2;
pub const SEP: u32 = 3;
pub const MASK: u32 = 4;
/// Printable ASCII starts here; a newline is `NEWLINE`.
const FIRST_CHAR: u32 = 5;
const NEWLINE: u32 = FIRST_CHAR + (0x7e - 0x20) + 1;
/// The fake model's embedding size.
pub const HIDDEN: usize = 64;

/// A `tokenizer.json` with one token per printable ASCII character (anything else is `[UNK]`).
pub fn char_tokenizer_json() -> String {
    let mut vocab = Map::new();
    for (id, name) in [(PAD, "[PAD]"), (UNK, "[UNK]"), (CLS, "[CLS]"), (SEP, "[SEP]"), (MASK, "[MASK]")] {
        vocab.insert(name.to_owned(), json!(id));
    }
    for c in 0x20_u8..=0x7e {
        vocab.insert(char::from(c).to_string(), json!(FIRST_CHAR + u32::from(c - 0x20)));
    }
    vocab.insert("\n".to_owned(), json!(NEWLINE));
    let added: Vec<Value> = [(PAD, "[PAD]"), (UNK, "[UNK]"), (CLS, "[CLS]"), (SEP, "[SEP]"), (MASK, "[MASK]")]
        .iter()
        .map(|(id, content)| {
            json!({"id": id, "content": content, "single_word": false, "lstrip": false, "rstrip": false,
                   "normalized": false, "special": true})
        })
        .collect();
    json!({
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": added,
        "normalizer": null,
        "pre_tokenizer": {"type": "Split", "pattern": {"Regex": "(?s)."}, "behavior": "Isolated", "invert": false},
        "post_processor": null,
        "decoder": null,
        "model": {"type": "WordLevel", "vocab": vocab, "unk_token": "[UNK]"}
    })
    .to_string()
}

/// `laya_config.json` for the character tokenizer.
pub fn laya_config_json() -> String {
    json!({"id": "fake-laya", "max_len": 1536, "head_max_len": 192, "temperature": [1.0, 1.0, 1.0],
           "temperature_by_options": {"choice:3-5": 1.0}, "hidden": HIDDEN,
           "mask_id": MASK, "cls_id": CLS, "sep_id": SEP, "pad_id": PAD})
    .to_string()
}

/// `laya_config.json` of a fake model with the risk output (`docs/autopilot-risk.md`): Overnight holds at
/// P(catastrophic) ≥ `threshold`.
pub fn laya_config_with_risk_json(threshold: f32) -> String {
    let mut cfg: Value = serde_json::from_str(&laya_config_json()).expect("config");
    cfg["id"] = json!("fake-laya-risk");
    cfg["risk"] = json!({"output": "risk", "labels": ["benign", "risky", "severe", "catastrophic"], "temperature": 1.0,
                         "thresholds": {"overnight": threshold}, "situation": "context"});
    cfg.to_string()
}

/// The files of a fake package.
pub fn package() -> Vec<(String, Vec<u8>)> {
    vec![
        (MODEL_FILE.to_owned(), b"not really an onnx model".to_vec()),
        (TOKENIZER_FILE.to_owned(), char_tokenizer_json().into_bytes()),
        (CONFIG_FILE.to_owned(), laya_config_json().into_bytes()),
    ]
}

/// The files of a fake package that rates risk (Overnight holds at P(catastrophic) ≥ 0.5).
pub fn package_with_risk() -> Vec<(String, Vec<u8>)> {
    package_with_config(&laya_config_with_risk_json(0.5))
}

/// The files of a fake package with this `laya_config.json`.
pub fn package_with_config(config: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (MODEL_FILE.to_owned(), b"not really an onnx model, with risk".to_vec()),
        (TOKENIZER_FILE.to_owned(), char_tokenizer_json().into_bytes()),
        (CONFIG_FILE.to_owned(), config.as_bytes().to_vec()),
    ]
}

fn sha256(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

/// A model entry pinning exactly these files.
pub fn known_model(id: &str, files: &[(String, Vec<u8>)]) -> KnownModel {
    known_model_version(id, "test", files)
}

/// The same with a version ("1", "2": a newer one is an update).
pub fn known_model_version(id: &str, version: &str, files: &[(String, Vec<u8>)]) -> KnownModel {
    KnownModel {
        id: Cow::Owned(id.to_owned()),
        label: Cow::Borrowed("Fake Laya"),
        version: Cow::Owned(version.to_owned()),
        files: Cow::Owned(
            files
                .iter()
                .map(|(name, bytes)| KnownFile {
                    name: Cow::Owned(name.clone()),
                    sha256: Cow::Owned(sha256(bytes)),
                    size: bytes.len() as u64,
                })
                .collect(),
        ),
    }
}

/// `SHA256SUMS` of these files, as the model host serves it.
pub fn sums(files: &[(String, Vec<u8>)]) -> String {
    files.iter().flat_map(|(name, bytes)| [sha256(bytes), "  ".to_owned(), name.clone(), "\n".to_owned()]).collect()
}

/// Installs a package as a finished download would.
pub fn install(data_dir: &Path, known: &KnownModel, files: &[(String, Vec<u8>)]) {
    model::install_files(data_dir, known, files).expect("install the fake model");
}

/// A download listener that keeps what it heard.
#[derive(Default)]
pub struct Progress(pub Mutex<Vec<(u64, u64)>>);

impl DownloadProgress for Progress {
    fn progress(&self, downloaded: u64, total: u64) {
        self.0.lock().expect("progress").push((downloaded, total));
    }
}

/// A model that reads the state text back from the character ids: logits from the first keyword rule that matches
/// it (else `default`), the act head escalating on "escalate-me", the embedding from its hashed words.
pub struct FakeRuntime {
    rules: Mutex<Vec<(String, [f32; 3])>>,
    default: Mutex<[f32; 3]>,
    /// Severity logits by keyword, and the default; returned only when `risk_default` is set (a runtime that reads
    /// the risk output).
    risk_rules: Mutex<Vec<(String, [f32; 4])>>,
    risk_default: Mutex<Option<[f32; 4]>>,
    pub fail: AtomicBool,
    /// Each run first takes this long (milliseconds): a phone too slow or a runtime that hangs.
    pub stall_ms: AtomicU64,
    /// The risk output gets one value too many per request (a model whose output is not what it declares).
    pub risk_extra: AtomicBool,
    /// Each run panics (a runtime that crashes).
    pub panic: AtomicBool,
    pub loads: AtomicU32,
    pub runs: AtomicU32,
    pub loaded: Mutex<Option<String>>,
    /// The state texts seen, in order.
    pub seen: Mutex<Vec<String>>,
}

impl Default for FakeRuntime {
    fn default() -> Self {
        Self {
            rules: Mutex::new(Vec::new()),
            default: Mutex::new([0.0, 0.0, 1.0]),
            risk_rules: Mutex::new(Vec::new()),
            risk_default: Mutex::new(None),
            fail: AtomicBool::new(false),
            stall_ms: AtomicU64::new(0),
            risk_extra: AtomicBool::new(false),
            panic: AtomicBool::new(false),
            loads: AtomicU32::new(0),
            runs: AtomicU32::new(0),
            loaded: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        }
    }
}

fn fnv(word: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in word.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The text of character ids.
pub fn decode(ids: &[i64]) -> String {
    ids.iter()
        .map(|id| match u32::try_from(*id).unwrap_or(UNK) {
            NEWLINE => '\n',
            i if (FIRST_CHAR..NEWLINE).contains(&i) => char::from(u8::try_from(i - FIRST_CHAR + 0x20).unwrap_or(b'?')),
            _ => ' ',
        })
        .collect()
}

/// Logits (approve, deny, ask) as probabilities would come out of them.
pub fn logits(approve: f32, deny: f32, ask: f32) -> [f32; 3] {
    [approve.ln(), deny.ln(), ask.ln()]
}

/// Severity logits (benign, risky, severe, catastrophic) as probabilities would come out of them.
pub fn risk(benign: f32, risky: f32, severe: f32, catastrophic: f32) -> [f32; 4] {
    [benign.ln(), risky.ln(), severe.ln(), catastrophic.ln()]
}

impl FakeRuntime {
    /// When the state text contains `keyword` (lower case), these logits.
    pub fn rule(&self, keyword: &str, logits: [f32; 3]) {
        self.rules.lock().expect("rules").push((keyword.to_lowercase(), logits));
    }

    pub fn set_default(&self, logits: [f32; 3]) {
        *self.default.lock().expect("default") = logits;
    }

    /// When the state text contains `keyword` (lower case), these severity logits.
    pub fn risk_rule(&self, keyword: &str, logits: [f32; 4]) {
        self.risk_rules.lock().expect("risk rules").push((keyword.to_lowercase(), logits));
    }

    /// Reads the risk output (as an app whose runtime knows it), with these logits when no rule matches.
    pub fn set_risk_default(&self, logits: [f32; 4]) {
        *self.risk_default.lock().expect("risk default") = Some(logits);
    }

    /// The state part of one sequence: after the separator closing the options, up to the next one.
    fn state(ids: &[i64]) -> String {
        let seps: Vec<usize> =
            ids.iter().enumerate().filter(|(_, id)| **id == i64::from(SEP)).map(|(i, _)| i).collect();
        match (seps.get(1), seps.get(2)) {
            (Some(a), Some(b)) => decode(&ids[a + 1..*b]),
            _ => String::new(),
        }
    }

    fn embed(text: &str) -> Vec<f32> {
        let mut v = vec![0.0_f32; HIDDEN];
        let words: Vec<String> = text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        for w in &words {
            let h = fnv(w);
            let sign = if h & 0x1_0000 == 0 {
                1.0
            } else {
                -1.0
            };
            v[(h as usize) % HIDDEN] += sign;
        }
        v
    }
}

/// The owner check, answered as told; what it was asked is kept.
pub struct FakeOwner {
    pub answer: Mutex<Result<bool, String>>,
    pub asked: Mutex<Vec<String>>,
}

impl FakeOwner {
    pub fn new(answer: Result<bool, String>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            answer: Mutex::new(answer),
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl crate::OwnerCheck for FakeOwner {
    async fn confirm(&self, reason: String) -> Result<bool, ForeignError> {
        self.asked.lock().expect("asked").push(reason);
        self.answer.lock().expect("answer").clone().map_err(|reason| ForeignError::Failed {
            reason,
        })
    }
}

impl ModelRuntime for FakeRuntime {
    fn load(&self, path: String) -> Result<(), ForeignError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(ForeignError::Failed {
                reason: "fake load failure".to_owned(),
            });
        }
        self.loads.fetch_add(1, Ordering::SeqCst);
        *self.loaded.lock().expect("loaded") = Some(path);
        Ok(())
    }

    fn run(&self, input: ModelInput) -> Result<ModelOutput, ForeignError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(ForeignError::Failed {
                reason: "fake inference failure".to_owned(),
            });
        }
        assert!(!self.panic.load(Ordering::SeqCst), "fake runtime crash");
        let stall = self.stall_ms.load(Ordering::SeqCst);
        if stall > 0 {
            std::thread::sleep(std::time::Duration::from_millis(stall));
        }
        self.runs.fetch_add(1, Ordering::SeqCst);
        let (batch, len) = (input.batch as usize, input.seq_len as usize);
        let mut out = ModelOutput {
            logits: Vec::with_capacity(batch * 3),
            act: Vec::with_capacity(batch * 2),
            pooled: Vec::with_capacity(batch * HIDDEN),
            hidden: u32::try_from(HIDDEN).unwrap_or(0),
            risk: Vec::new(),
        };
        let rules = self.rules.lock().expect("rules").clone();
        let default = *self.default.lock().expect("default");
        let risk_rules = self.risk_rules.lock().expect("risk rules").clone();
        let risk_default = *self.risk_default.lock().expect("risk default");
        for b in 0..batch {
            let state = Self::state(&input.input_ids[b * len..(b + 1) * len]);
            let lower = state.to_lowercase();
            let l = rules.iter().find(|(k, _)| lower.contains(k)).map_or(default, |(_, l)| *l);
            out.logits.extend(l);
            out.act.extend(if lower.contains("escalate-me") {
                [0.0, 4.0]
            } else {
                [0.0, -4.0]
            });
            out.pooled.extend(Self::embed(&state));
            if let Some(d) = risk_default {
                out.risk.extend(risk_rules.iter().find(|(k, _)| lower.contains(k)).map_or(d, |(_, l)| *l));
                if self.risk_extra.load(Ordering::SeqCst) {
                    out.risk.push(0.0);
                }
            }
            self.seen.lock().expect("seen").push(state);
        }
        Ok(out)
    }

    fn unload(&self) {
        *self.loaded.lock().expect("loaded") = None;
    }
}
