//! Laya's input sequence for the fixed Autopilot question (spec §4), a port of `rl_common.build_sequence`:
//!
//! ```text
//! [CLS] "choice question: <instructions>" [SEP] [MASK] " approve: …" [MASK] " deny: …" [MASK] " ask: …" [SEP] <state> [SEP]
//! ```
//!
//! Every piece is tokenized on its own without special tokens; each option is cut to 48 tokens, the question to what
//! the head budget leaves, the state to what `max_len` leaves (cut on the right). The token ids must equal Laya's:
//! `testdata/autopilot_golden.json` holds ids made by Laya itself, `testdata/laya_golden.json` (a copy of
//! `tools/laya/golden.json`) the cases and ids of the reference implementation the training data is made with.

use crate::CoreError;

pub const INSTRUCTIONS: &str = "An AI agent wants to do this on the user's behalf. Should the phone approve it \
                                automatically, deny it automatically, or ask the user?";
/// The options in their fixed order: logits and probabilities come back in this order.
pub const OPTIONS: [(&str, &str); 3] = [
    ("approve", "routine and safe, the kind of thing the user allows"),
    ("deny", "harmful, destructive, leaks private data or secrets, or clearly against the user's interests"),
    ("ask", "uncertain, unusual, or needs the user's judgement"),
];
/// Laya's question type of a choice (the `qtype` input).
pub const QTYPE_CHOICE: i64 = 0;
/// Tokens kept of each option text.
pub const OPTION_MAX_TOKENS: usize = 48;

/// The special tokens of a tokenizer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Special {
    pub cls: u32,
    pub sep: u32,
    pub mask: u32,
    pub pad: u32,
    /// The mask token's text ("[MASK]", "<mask>"): blanked wherever it appears in a text.
    pub mask_token: String,
}

/// Turns text into token ids, without special tokens.
pub trait Encode {
    fn encode(&self, text: &str) -> Result<Vec<u32>, CoreError>;
}

/// The `tokenizer.json` of a model package, through the `tokenizers` crate (pure Rust, `fancy-regex`).
pub struct LayaTokenizer {
    inner: tokenizers::Tokenizer,
}

impl LayaTokenizer {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut inner = tokenizers::Tokenizer::from_bytes(bytes)
            .map_err(|e| CoreError::storage(format!("the model's tokenizer cannot be read: {e}")))?;
        inner.with_padding(None);
        inner
            .with_truncation(None)
            .map_err(|e| CoreError::storage(format!("the model's tokenizer cannot be set up: {e}")))?;
        Ok(Self {
            inner,
        })
    }

    /// The text of a token id (the mask token's, to blank it in texts).
    pub fn token(&self, id: u32) -> Option<String> {
        self.inner.id_to_token(id)
    }
}

impl Encode for LayaTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<u32>, CoreError> {
        self.inner
            .encode(text, false)
            .map(|e| e.get_ids().to_vec())
            .map_err(|e| CoreError::storage(format!("tokenizing failed: {e}")))
    }
}

/// (`input_ids`, positions of the approve/deny/ask markers) for `state`.
pub fn build_sequence(
    enc: &dyn Encode,
    sp: &Special,
    state: &str,
    max_len: usize,
    head_max_len: usize,
) -> Result<(Vec<u32>, Vec<usize>), CoreError> {
    let blank = |t: &str| {
        if sp.mask_token.is_empty() {
            t.to_owned()
        } else {
            t.replace(&sp.mask_token, " ")
        }
    };
    let mut options: Vec<Vec<u32>> = Vec::with_capacity(OPTIONS.len());
    for (name, criterion) in OPTIONS {
        let text = format!(" {}", blank(&format!("{name}: {criterion}")));
        let mut ids = vec![sp.mask];
        ids.extend(enc.encode(&text)?.into_iter().take(OPTION_MAX_TOKENS));
        options.push(ids);
    }
    let used: usize = options.iter().map(Vec::len).sum();
    let mut budget = head_max_len.cast_signed() - used.cast_signed();
    if budget < 16 {
        // Never the case for this question; kept for fidelity with Laya.
        let per = ((head_max_len.cast_signed() - 16) / options.len().cast_signed()).max(4).cast_unsigned();
        for o in &mut options {
            o.truncate(per);
        }
        budget = head_max_len.cast_signed() - options.iter().map(Vec::len).sum::<usize>().cast_signed();
    }
    let mut head = enc.encode(&format!("choice question: {}", blank(INSTRUCTIONS)))?;
    head.truncate(budget.max(8).cast_unsigned());
    let mut ids = Vec::with_capacity(max_len);
    ids.push(sp.cls);
    ids.extend(head);
    ids.push(sp.sep);
    let mut markers = Vec::with_capacity(options.len());
    for o in options {
        markers.push(ids.len());
        ids.extend(o);
    }
    ids.push(sp.sep);
    let room = max_len.saturating_sub(ids.len() + 1);
    let mut st = enc.encode(&blank(state))?;
    st.truncate(room);
    ids.extend(st);
    ids.push(sp.sep);
    ids.truncate(max_len);
    markers.retain(|m| *m < max_len);
    Ok((ids, markers))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;

    /// Laya's tokenizers, when they are on this machine (`~/.cache/reins-laya`).
    fn tokenizer_dir() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let dir = PathBuf::from(home).join(".cache/reins-laya/laya");
        dir.join("tokenizer/tokenizer.json").exists().then_some(dir)
    }

    fn golden() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/autopilot_golden.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_question_is_the_one_laya_was_asked() {
        let g = golden();
        assert_eq!(g["question"]["ins"], INSTRUCTIONS);
        for (name, criterion) in OPTIONS {
            assert_eq!(g["question"]["crit"][name], criterion);
        }
    }

    #[test]
    fn token_ids_equal_layas_own() {
        let Some(dir) = tokenizer_dir() else {
            eprintln!("skipped: Laya's tokenizers are not in ~/.cache/reins-laya (the golden ids cannot be checked)");
            return;
        };
        let g = golden();
        let texts: Vec<&str> = g["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        let mut checked = 0;
        for t in g["tokenizers"].as_array().unwrap() {
            let path = dir.join(t["dir"].as_str().unwrap()).join("tokenizer.json");
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("skipped {}: not on this machine", path.display());
                continue;
            };
            let tok = LayaTokenizer::from_bytes(&bytes).unwrap();
            let id = |k: &str| u32::try_from(t[k].as_u64().unwrap()).unwrap();
            let sp = Special {
                cls: id("cls_id"),
                sep: id("sep_id"),
                mask: id("mask_id"),
                pad: id("pad_id"),
                mask_token: t["mask_token"].as_str().unwrap().to_owned(),
            };
            assert_eq!(tok.token(sp.mask).as_deref(), Some(sp.mask_token.as_str()));
            for s in t["sequences"].as_array().unwrap() {
                let text = texts[usize::try_from(s["text"].as_u64().unwrap()).unwrap()];
                let max_len = usize::try_from(s["max_len"].as_u64().unwrap()).unwrap();
                let head = usize::try_from(s["head_max_len"].as_u64().unwrap()).unwrap();
                let (ids, markers) = build_sequence(&tok, &sp, text, max_len, head).unwrap();
                let want: Vec<u32> = s["input_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
                    .collect();
                let want_markers: Vec<usize> = s["marker_pos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| usize::try_from(v.as_u64().unwrap()).unwrap())
                    .collect();
                assert_eq!(ids, want, "{} ids of {text:?}", t["name"]);
                assert_eq!(markers, want_markers, "{} markers of {text:?}", t["name"]);
                checked += 1;
            }
        }
        assert!(checked > 40, "only {checked} sequences checked");
    }

    /// A golden render case's facts and AI part, as the core holds them.
    fn case_situation(case: &Value) -> (super::super::situation::Facts, super::super::situation::AiPart) {
        use super::super::situation::{AiPart, Facts};
        let f = &case["facts"];
        let s = |k: &str| match &f[k] {
            Value::String(v) => v.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let count = |v: &Value| u32::try_from(v.as_u64().unwrap()).unwrap();
        let history = match &f["connection history"] {
            Value::Array(h) => Some((count(&h[0]), count(&h[1]))),
            // "3 approved, 0 denied", as the reference also accepts it.
            Value::String(text) => {
                let (approved, denied) = text.split_once(" approved, ").unwrap();
                Some((approved.parse().unwrap(), denied.trim_end_matches(" denied").parse().unwrap()))
            }
            _ => None,
        };
        let target_is_new = match &f["target is new"] {
            Value::Bool(b) => Some(*b),
            Value::String(v) => Some(v == "yes"),
            _ => None,
        };
        let facts = Facts {
            connection: s("connection"),
            connection_age: f["connection age"].as_i64(),
            connection_history: history,
            service: s("service"),
            action: s("action"),
            operation: s("operation"),
            class: s("class"),
            account: s("account"),
            target: s("target"),
            target_is_new,
            count: f["count"].as_u64().map_or(0, |c| u32::try_from(c).unwrap()),
            details: f["details"]
                .as_array()
                .map_or_else(Vec::new, |d| d.iter().map(|x| x.as_str().unwrap().to_owned()).collect()),
        };
        let ai = AiPart {
            reason: case["ai"]["reason"].as_str().unwrap_or_default().to_owned(),
            content: case["ai"]["content"].as_str().unwrap_or_default().to_owned(),
        };
        (facts, ai)
    }

    fn check_renders(g: &Value) {
        let cases = g["render"].as_array().unwrap();
        assert!(cases.len() >= 13);
        for case in cases {
            let (facts, ai) = case_situation(case);
            let (s_facts, s_full) = super::super::situation::render(&facts, &ai);
            assert_eq!(s_facts, case["s_facts"].as_str().unwrap(), "{}", case["name"]);
            assert_eq!(s_full, case["s_full"].as_str().unwrap(), "{}", case["name"]);
        }
    }

    #[test]
    fn situations_render_as_the_training_data_does() {
        check_renders(&golden());
    }

    /// `tools/laya/golden.json`, written by `tools/laya/sequence.py --golden` (the reference renderer and
    /// sequence builder the training data is made with), copied here unchanged.
    fn reference_golden() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/laya_golden.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn situations_render_as_the_reference_renderer_does() {
        let g = reference_golden();
        assert_eq!(g["question"]["decision"]["instructions"], INSTRUCTIONS);
        let options: Vec<&str> = g["options"].as_array().unwrap().iter().map(|o| o.as_str().unwrap()).collect();
        assert_eq!(options, OPTIONS.map(|(name, _)| name));
        for (name, criterion) in OPTIONS {
            assert_eq!(g["question"]["decision"]["criteria"][name], criterion);
        }
        let literals: Vec<&str> =
            g["special_literals"].as_array().unwrap().iter().map(|o| o.as_str().unwrap()).collect();
        assert_eq!(literals, super::super::situation::SPECIAL_LITERALS);
        check_renders(&g);
    }

    #[test]
    fn token_ids_equal_the_reference_sequence_builders() {
        let Some(dir) = tokenizer_dir() else {
            eprintln!("skipped: Laya's tokenizers are not in ~/.cache/reins-laya (the golden ids cannot be checked)");
            return;
        };
        let g = reference_golden();
        let max_len = usize::try_from(g["max_len"].as_u64().unwrap()).unwrap();
        // The texts the ids were made from are the rendered cases themselves.
        let rendered: std::collections::HashMap<(String, String), String> = g["render"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| {
                let (facts, ai) = case_situation(c);
                let (s_facts, s_full) = super::super::situation::render(&facts, &ai);
                let name = c["name"].as_str().unwrap().to_owned();
                [((name.clone(), "s_facts".to_owned()), s_facts), ((name, "s_full".to_owned()), s_full)]
            })
            .collect();
        let mut checked = 0;
        for (name, t) in g["tokenizers"].as_object().unwrap() {
            let path = dir.join(t["tokenizer_dir"].as_str().unwrap()).join("tokenizer.json");
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("skipped {}: not on this machine", path.display());
                continue;
            };
            let tok = LayaTokenizer::from_bytes(&bytes).unwrap();
            let id = |k: &str| u32::try_from(t[k].as_u64().unwrap()).unwrap();
            let sp = Special {
                cls: id("cls_id"),
                sep: id("sep_id"),
                mask: id("mask_id"),
                pad: id("pad_id"),
                mask_token: t["mask_token"].as_str().unwrap().to_owned(),
            };
            let head = usize::try_from(t["head_max_len"].as_u64().unwrap()).unwrap();
            for s in t["sequences"].as_array().unwrap() {
                let key = (s["case"].as_str().unwrap().to_owned(), s["variant"].as_str().unwrap().to_owned());
                let text = &rendered[&key];
                assert_eq!(text, s["state_text"].as_str().unwrap(), "{name} {key:?}");
                let (ids, markers) = build_sequence(&tok, &sp, text, max_len, head).unwrap();
                let want: Vec<u32> = s["input_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
                    .collect();
                let want_markers: Vec<usize> = s["marker_pos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| usize::try_from(v.as_u64().unwrap()).unwrap())
                    .collect();
                assert_eq!(ids, want, "{name} ids of {key:?}");
                assert_eq!(markers, want_markers, "{name} markers of {key:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, 52, "both tokenizers, 13 cases, 2 variants");
    }

    /// One token per character, for checking the layout without a real tokenizer.
    struct Chars;

    impl Encode for Chars {
        fn encode(&self, text: &str) -> Result<Vec<u32>, CoreError> {
            Ok(text.chars().map(|c| 1000 + u32::from(c)).collect())
        }
    }

    #[test]
    fn layout_budgets_and_masks() {
        let sp = Special {
            cls: 1,
            sep: 2,
            mask: 3,
            pad: 0,
            mask_token: "[MASK]".to_owned(),
        };
        let (ids, markers) = build_sequence(&Chars, &sp, "ab[MASK]c", 400, 192).unwrap();
        assert_eq!(ids[0], 1);
        assert_eq!(markers.len(), 3);
        for m in &markers {
            assert_eq!(ids[*m], 3, "each option starts with the mask token");
        }
        let state: Vec<u32> = "ab c".chars().map(|c| 1000 + u32::from(c)).collect();
        assert_eq!(&ids[ids.len() - 5..ids.len() - 1], state.as_slice(), "the mask text is blanked in the state");
        assert_eq!(*ids.last().unwrap(), 2);
        // Options are cut to 48 tokens each; the head gets what is left of 192, at least 8.
        assert_eq!(markers[1] - markers[0], 1 + OPTION_MAX_TOKENS);
        let head = markers[0] - 2;
        assert_eq!(head, 192 - 3 * (1 + OPTION_MAX_TOKENS));
        // A short max_len cuts the state, never past it.
        let (short, _) = build_sequence(&Chars, &sp, &"x".repeat(1000), 200, 192).unwrap();
        assert_eq!(short.len(), 200);
        assert_eq!(*short.last().unwrap(), 2);
    }
}
