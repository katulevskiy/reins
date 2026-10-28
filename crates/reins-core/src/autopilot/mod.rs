//! Autopilot: automatic approvals decided on the phone with the Laya model (spec
//! `docs/superpowers/specs/2026-10-01-autopilot-laya.md`).
//!
//! After every push and sync the engine runs [`Engine::autopilot_pass`](crate::engine::Engine) over newly parked
//! items. Per item: Lockdown denies; Bypass approves everything (new connections and other phones never come here);
//! the hard floor ([`gates`]) waits; Manual and Auto render the
//! situation ([`situation`]), run the model twice ([`model`], [`sequence`]: facts only, and facts plus what the AI
//! wrote), score both against the profile's memory ([`memory`]) and adapter ([`adapter`]), keep the more cautious of
//! the two ([`decide`]) and, in Auto, act only in classes that earned it, within the rate limit (Manual only suggests).
//! Automatic decisions go through the same approve/deny code as the user's taps ([`context`] marks who decided), and
//! never train.

pub mod adapter;
pub mod context;
pub mod decide;
pub mod desktop;
pub(crate) mod desktop_policy;
pub(crate) mod engine;
pub mod gates;
pub mod memory;
pub mod model;
pub mod modes;
pub(crate) mod overnight;
pub(crate) mod parking;
pub mod policy;
pub mod preview;
pub mod risk;
pub mod runtime;
pub mod sequence;
pub mod situation;
pub mod stats;
pub mod store;
pub(crate) mod tasks;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
pub mod types;

pub use engine::State;
pub(crate) use engine::{activity_note, suggestion_line};
pub use runtime::{DownloadProgress, ModelRuntime};
pub use types::{
    AutoDecisionView, AutoQuestion, AutoSetup, AutopilotEvent, AutopilotMode, AutopilotNote, AutopilotSettings,
    AutopilotStats, CellChoice, ClassView, ConnectionAutopilot, DigestGroup, FactChoice, MatrixRowView, MatrixView,
    ModelInput, ModelOutput, ModelState, ModelStatus, ModelUpdateOutcome, NeighbourView, OVERNIGHT_NAME,
    OvernightDigest, Preset, ProfileView, RiskLevel, RiskView, SuggestionView, TaskView, ThresholdReplay, Verdict,
    WorldItem, ZoneChoice,
};
