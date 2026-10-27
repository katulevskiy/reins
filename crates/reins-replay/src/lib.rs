//! `reins-replay`: replays recorded Reins requests through an engine and a policy (Decisions v2 P0,
//! `docs/design/decisions-v2/07-roadmap.md` §3).
//!
//! The corpus (`corpus/`) is the desktop history of 13–26 October 2026, anonymized and labelled by hand, plus
//! synthetic adversarial cases. Every change to the consequence engine or the default policy is replayed in CI: real-risk
//! recall must stay at 100%, and no known false positive may come back.
//!
//! - `reins-replay run [--engine guard|effects] [--policy F] [--check]`: the numbers, and the gate in CI.
//! - `reins-replay explain ID…`, `list [--ask|--auto]`, `unresolved`: for reviewing labels and the engine.
//! - `reins-replay import --log ~/.local/state/reins/activity.jsonl --labels LABELS --out corpus/desktop.jsonl`:
//!   rebuilds the desktop corpus on the owner's computer. `LABELS` (kept off the repository: it names private hosts)
//!   holds the owner's GitHub name and its alias, private terms to replace, a cut-off time, and the hand labels by
//!   case id; every other case is labelled by kind (git to the owner's repositories, `rm -r` in the workspace,
//!   connector calls on the owner's repositories: routine; questions: needs a human), and a guard hit labelled
//!   routine is a known false positive. `--write-baseline` records the new numbers after a deliberate change.

pub mod anonymize;
pub mod corpus;
pub mod engine;
pub mod report;

use std::path::{Path, PathBuf};

/// The corpus that ships with this crate.
#[must_use]
pub fn default_corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

/// The committed baseline (`baseline.json`).
#[must_use]
pub fn default_baseline() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baseline.json")
}

/// The default policy file.
#[must_use]
pub fn default_policy() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("policy").join("default.json")
}
