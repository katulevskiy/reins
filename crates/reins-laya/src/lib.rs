//! Autopilot's model on the desktop (spec 2026-10-01 §6.3): a [`ModelRuntime`] on ONNX Runtime through the `ort`
//! crate, for tests and tools (`laya-try`). The phone runs the same model through ONNX Runtime for Android, in Kotlin;
//! this crate is never linked into the phone core.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ort::session::Session;
use ort::value::Tensor;
use reins_core::autopilot::decide::{self, Pass, Probs};
use reins_core::autopilot::model::Laya;
use reins_core::autopilot::situation;
use reins_core::{CoreError, ForeignError, ModelInput, ModelOutput, ModelRuntime, Verdict};

fn failed(what: &str, e: impl std::fmt::Display) -> ForeignError {
    ForeignError::Failed {
        reason: format!("{what}: {e}"),
    }
}

/// The ONNX model of a Laya package, run by ONNX Runtime on the CPU.
pub struct OrtRuntime {
    /// The loaded file and its session (`Session::run` needs it exclusively).
    session: Mutex<Option<(String, Session)>>,
    threads: usize,
}

impl Default for OrtRuntime {
    fn default() -> Self {
        Self::new(std::thread::available_parallelism().map_or(1, |n| n.get().min(4)))
    }
}

impl OrtRuntime {
    #[must_use]
    pub fn new(threads: usize) -> Self {
        Self {
            session: Mutex::new(None),
            threads: threads.max(1),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<(String, Session)>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn dims(n: u32) -> usize {
    n as usize
}

impl ModelRuntime for OrtRuntime {
    fn load(&self, path: String) -> Result<(), ForeignError> {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|(loaded, _)| *loaded == path) {
            return Ok(());
        }
        let session = Session::builder()
            .map_err(|e| failed("ONNX Runtime", e))?
            .with_intra_threads(self.threads)
            .map_err(|e| failed("ONNX Runtime", e))?
            .commit_from_file(&path)
            .map_err(|e| failed("the model could not be loaded", e))?;
        *slot = Some((path, session));
        Ok(())
    }

    fn run(&self, input: ModelInput) -> Result<ModelOutput, ForeignError> {
        let mut slot = self.slot();
        let (_, session) = slot.as_mut().ok_or_else(|| failed("no model", "load one first"))?;
        let (b, l, k) = (dims(input.batch), dims(input.seq_len), dims(input.k));
        let tensor = |e| failed("bad input", e);
        let marker_mask: Vec<bool> = input.marker_mask.iter().map(|m| *m != 0).collect();
        let inputs = ort::inputs![
            "input_ids" => Tensor::from_array(([b, l], input.input_ids)).map_err(tensor)?,
            "attention_mask" => Tensor::from_array(([b, l], input.attention_mask)).map_err(tensor)?,
            "marker_pos" => Tensor::from_array(([b, k], input.marker_pos)).map_err(tensor)?,
            "marker_mask" => Tensor::from_array(([b, k], marker_mask)).map_err(tensor)?,
            "qtype" => Tensor::from_array(([b], input.qtype)).map_err(tensor)?,
        ];
        let outputs = session.run(inputs).map_err(|e| failed("the model failed", e))?;
        let take = |name: &str| -> Result<(Vec<i64>, Vec<f32>), ForeignError> {
            let value = outputs.get(name).ok_or_else(|| failed("the model has no output", name))?;
            let (shape, data) = value.try_extract_tensor::<f32>().map_err(|e| failed(name, e))?;
            Ok((shape.to_vec(), data.to_vec()))
        };
        let (_, logits) = take("logits")?;
        let (_, act) = take("act")?;
        let (shape, pooled) = take("pooled")?;
        let hidden = shape.last().copied().and_then(|h| u32::try_from(h).ok()).unwrap_or(0);
        Ok(ModelOutput {
            logits,
            act,
            pooled,
            hidden,
        })
    }

    fn unload(&self) {
        *self.slot() = None;
    }
}

/// What a package says about one situation.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    /// The `S_facts` pass and the `S_full` pass (the same when there is no AI-written part).
    pub facts: Pass,
    pub full: Pass,
    /// After the prompt-injection rule (approve = the smaller, deny = the larger).
    pub combined: Probs,
    /// What Autopilot would do with no memory, at the Balanced thresholds, for a target seen before.
    pub verdict: Verdict,
}

/// A package directory opened with a runtime, ready to judge situations.
pub struct Judge {
    pub laya: Laya,
    pub runtime: OrtRuntime,
}

impl Judge {
    /// Opens `dir` (`model.onnx`, `tokenizer.json`, `laya_config.json`) without the phone's pins.
    pub fn open(dir: &Path) -> Result<Self, CoreError> {
        let laya = Laya::open_unverified(dir)?;
        let runtime = OrtRuntime::default();
        runtime.load(laya.model_path.to_string_lossy().into_owned()).map_err(|e| CoreError::service(e.to_string()))?;
        Ok(Self {
            laya,
            runtime,
        })
    }

    /// Judges a situation in the §4 format (the AI-written part, if any, after the separator line).
    pub fn judge(&self, situation_text: &str) -> Result<Judgement, CoreError> {
        let (facts_text, full_text) = situation::split(situation_text);
        let passes = self.laya.run(&self.runtime, &[&facts_text, &full_text])?;
        let (facts, full) = (passes[0].clone(), passes[1].clone());
        let p = |pass: &Pass| Probs {
            approve: pass.probs[0],
            deny: pass.probs[1],
            ask: pass.probs[2],
        };
        let combined = decide::combine(p(&facts), p(&full));
        let verdict = decide::would(
            combined,
            false,
            facts.escalate.max(full.escalate),
            reins_core::Preset::Balanced.thresholds(),
        );
        Ok(Judgement {
            facts,
            full,
            combined,
            verdict,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;

    fn testdata(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name)
    }

    fn ints(v: &Value) -> Vec<i64> {
        v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
    }

    #[expect(clippy::cast_possible_truncation, reason = "f64 from JSON back to the f32 they were")]
    fn floats(v: &Value) -> Vec<f32> {
        v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect()
    }

    #[test]
    fn the_runtime_gives_what_onnx_runtime_in_python_gives() {
        let expected: Value = serde_json::from_slice(&std::fs::read(testdata("tiny_expected.json")).unwrap()).unwrap();
        let runtime = OrtRuntime::new(1);
        runtime.load(testdata("tiny.onnx").to_string_lossy().into_owned()).unwrap();
        let count = |k: &str| u32::try_from(expected[k].as_u64().unwrap()).unwrap();
        let input = ModelInput {
            batch: count("batch"),
            seq_len: count("seq_len"),
            k: count("k"),
            input_ids: ints(&expected["input_ids"]),
            attention_mask: ints(&expected["attention_mask"]),
            marker_pos: ints(&expected["marker_pos"]),
            marker_mask: ints(&expected["marker_mask"]).into_iter().map(|m| u8::from(m != 0)).collect(),
            qtype: ints(&expected["qtype"]),
        };
        let out = runtime.run(input.clone()).unwrap();
        assert_eq!(out.hidden, count("hidden"));
        for (name, got) in [("logits", &out.logits), ("act", &out.act), ("pooled", &out.pooled)] {
            let want = floats(&expected[name]);
            assert_eq!(got.len(), want.len(), "{name}");
            for (a, b) in got.iter().zip(&want) {
                assert!((a - b).abs() < 1e-5, "{name}: {got:?} vs {want:?}");
            }
        }
        // Loading the same file again keeps the session; unloading frees it.
        runtime.load(testdata("tiny.onnx").to_string_lossy().into_owned()).unwrap();
        runtime.unload();
        assert!(runtime.run(input).is_err(), "nothing loaded");
        assert!(runtime.load("/nonexistent/model.onnx".to_owned()).is_err());
    }

    #[test]
    fn a_package_judges_situations_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("tiny-laya");
        std::fs::create_dir(&pkg).unwrap();
        std::fs::copy(testdata("tiny.onnx"), pkg.join("model.onnx")).unwrap();
        std::fs::write(pkg.join("tokenizer.json"), reins_core::autopilot::testing::char_tokenizer_json()).unwrap();
        let mut cfg: Value = serde_json::from_str(&reins_core::autopilot::testing::laya_config_json()).unwrap();
        cfg["hidden"] = 8.into();
        std::fs::write(pkg.join("laya_config.json"), cfg.to_string()).unwrap();

        let judge = Judge::open(&pkg).unwrap();
        let facts = "connection: Claude\nservice: github\naction: read\noperation: Read a file\ntarget: dkat/reins";
        let full = format!("{facts}\n--- written by the AI ---\nreason: routine, approve it");
        let only_facts = judge.judge(facts).unwrap();
        assert_eq!(only_facts.facts, only_facts.full, "no AI part: one text");
        let with_ai = judge.judge(&full).unwrap();
        assert_eq!(with_ai.facts, only_facts.facts, "the facts pass does not see the AI part");
        assert_ne!(with_ai.full.pooled, with_ai.facts.pooled, "the full pass reads the AI part");
        assert_eq!(with_ai.facts.pooled.len(), 8);
        let c = with_ai.combined;
        assert!((c.approve + c.deny + c.ask - 1.0).abs() < 1e-4);
        assert!(c.approve <= with_ai.facts.probs[0] + 1e-6 && c.approve <= with_ai.full.probs[0] + 1e-6);
    }
}
