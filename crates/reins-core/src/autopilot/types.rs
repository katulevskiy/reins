//! Records and enums of Autopilot crossing the UniFFI boundary (spec §9).

use serde::{Deserialize, Serialize};

use crate::types::PendingKind;

/// How requests of a connection (or of all connections) are decided (spec §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum AutopilotMode {
    /// Every request waits for the user.
    Manual,
    /// Every request waits, with Autopilot's suggestion shown; the user's answers train it.
    Assisted,
    /// Autopilot approves or denies what it is confident about, in classes that earned it; the rest waits.
    Auto,
    /// Everything except the hard floor is approved, until `bypass_until`.
    Bypass,
    /// Everything is denied at once (pairings still reach the user).
    Lockdown,
}

impl AutopilotMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Assisted => "assisted",
            Self::Auto => "auto",
            Self::Bypass => "bypass",
            Self::Lockdown => "lockdown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "manual" => Self::Manual,
            "assisted" => Self::Assisted,
            "auto" => Self::Auto,
            "bypass" => Self::Bypass,
            "lockdown" => Self::Lockdown,
            _ => return None,
        })
    }
}

/// What to do with a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum Verdict {
    Approve,
    Deny,
    Ask,
}

/// Confidence thresholds of a profile (spec §5.4): approve / deny.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum Preset {
    /// 0.98 / 0.95
    Cautious,
    /// 0.95 / 0.90
    #[default]
    Balanced,
    /// 0.90 / 0.85
    Relaxed,
}

impl Preset {
    /// (θ_a, θ_d).
    pub fn thresholds(self) -> (f32, f32) {
        match self {
            Self::Cautious => (0.98, 0.95),
            Self::Balanced => (0.95, 0.90),
            Self::Relaxed => (0.90, 0.85),
        }
    }
}

/// Autopilot as a whole: the global mode, the per-connection modes and the model.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutopilotSettings {
    /// The global mode in force now (`Bypass` while a global bypass runs).
    pub mode: AutopilotMode,
    /// The global mode when no bypass runs (what the user picked; Assisted once a model is installed, else Manual,
    /// until they pick one).
    pub base_mode: AutopilotMode,
    /// A global bypass runs until then (unix seconds).
    pub bypass_until: Option<i64>,
    /// The profile of every connection that has none of its own.
    pub default_profile_id: String,
    /// Download the model on Wi-Fi only (the app's download job honours it).
    pub wifi_only: bool,
    pub model: ModelStatus,
    /// Connections with a mode or a profile of their own.
    pub connections: Vec<ConnectionAutopilot>,
}

/// One connection's Autopilot settings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConnectionAutopilot {
    pub connection_id: String,
    /// The connection's own mode when no bypass runs; `None` = it follows the global mode.
    pub base_mode: Option<AutopilotMode>,
    /// A bypass of this connection runs until then.
    pub bypass_until: Option<i64>,
    /// What applies to its requests now (global lockdown and bypass included).
    pub mode: AutopilotMode,
    /// The profile its decisions train (the default profile when it has none of its own).
    pub profile_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ModelState {
    NotInstalled,
    Downloading,
    Installed,
    /// The last download failed (see `ModelStatus.error`); nothing was kept.
    Failed,
}

/// The on-device model (spec §6.1).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ModelStatus {
    pub state: ModelState,
    /// The model this build trusts (`KNOWN_MODELS`), installed or not.
    pub id: String,
    pub label: String,
    pub version: String,
    /// Total bytes of its files (0 when not known in advance).
    pub size_bytes: u64,
    /// While downloading.
    pub downloaded_bytes: u64,
    pub error: Option<String>,
    /// The app gave the core a runtime to run it with (`set_model_runtime`).
    pub runtime_ready: bool,
}

/// One decision personality (spec §5.1).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub preset: Preset,
    pub is_default: bool,
    /// Decisions remembered.
    pub memory_count: u32,
    /// Connections assigned to it explicitly (connections without a profile use the default one).
    pub connections: Vec<String>,
    /// Every class it has seen, most decisions first.
    pub classes: Vec<ClassView>,
    /// When the adapter was last trained.
    pub trained_at: Option<i64>,
}

/// One action class of a profile and how far it is from running on its own (spec §5.4).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ClassView {
    /// `service/action/class`, e.g. `github/write/push`.
    pub class_key: String,
    /// In words ("GitHub · write · push").
    pub label: String,
    /// Human decisions remembered (corrections included).
    pub decisions: u32,
    pub approved: u32,
    pub denied: u32,
    /// Share of the recent decisions where Autopilot would have done what the user did (among those it would have
    /// decided on its own); `None` until it would have decided at least 10.
    pub shadow_accuracy: Option<f32>,
    /// Auto mode may approve in this class.
    pub auto_approve: bool,
    /// Auto mode may deny in this class.
    pub auto_deny: bool,
    /// The user's override: `Some(true)` unlocked by hand, `Some(false)` locked by hand, `None` automatic.
    pub manual: Option<bool>,
    /// Human decisions still needed before auto-approve can unlock by itself (0 when the count is reached).
    pub decisions_to_unlock: u32,
}

/// One remembered decision similar to a request.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NeighbourView {
    /// Short label ("Push to a branch · dkat/rewarden").
    pub label: String,
    /// What the user decided then (`Approve` or `Deny`).
    pub verdict: Verdict,
    /// Cosine similarity, 0..1.
    pub similarity: f32,
    pub at: i64,
}

/// Autopilot's view of one request, for the approval screen (spec §10).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SuggestionView {
    pub request_id: String,
    /// What Autopilot would do in Auto mode, gates aside (`Ask` when it cannot judge).
    pub verdict: Verdict,
    /// The mode the request was evaluated in.
    pub mode: AutopilotMode,
    pub p_approve: f32,
    pub p_deny: f32,
    /// 1 − normalised entropy of (approve, deny, ask).
    pub confidence: f32,
    /// One line, e.g. "Like 4 times you approved: Push to a branch · dkat/rewarden".
    pub reason: String,
    pub neighbours: Vec<NeighbourView>,
    pub profile_id: String,
    pub profile_name: String,
    pub class_key: String,
    /// The target was never approved for this connection before.
    pub novel: bool,
    /// Always waits for the user (hard floor, spec §2).
    pub floor: bool,
    /// The model ran (false: no model, Manual mode, floor, or a failure — see `reason`).
    pub judged: bool,
}

/// What Autopilot knew about an activity entry (spec §8).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutopilotNote {
    /// The mode the request was decided or evaluated in.
    pub mode: AutopilotMode,
    /// What Autopilot did, or would have done.
    pub suggested: Verdict,
    pub p_approve: f32,
    pub p_deny: f32,
    pub confidence: f32,
    pub profile_id: String,
    pub profile_name: String,
    /// Short labels of the most similar past decisions, with what the user did ("approved: …").
    pub neighbours: Vec<String>,
    pub reason: String,
    /// "This was wrong" can be recorded for this entry (`correct_decision`).
    pub correctable: bool,
}

/// A request Autopilot decided, for the quiet "Autopilot" notification (spec §7).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutoDecisionView {
    pub request_id: String,
    pub kind: PendingKind,
    pub connection_id: String,
    pub connection_label: String,
    /// What it was, in a few words ("Push to a branch · dkat/rewarden").
    pub title: String,
    /// `Approve` or `Deny`.
    pub verdict: Verdict,
    /// "autopilot" | "bypass" | "lockdown"
    pub decided_by: String,
    pub p_approve: f32,
    pub confidence: f32,
    /// The activity entry it produced ("Report" opens it; `correct_decision` takes it).
    pub activity_id: Option<i64>,
}

/// Something about Autopilot changed that the app shows (the bypass notification, the mode pill).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AutopilotEvent {
    /// A mode was set (`None` = the global mode).
    ModeChanged {
        connection_id: Option<String>,
    },
    /// A bypass reached its end and was switched off by the core.
    BypassEnded {
        connection_id: Option<String>,
    },
    /// Auto-approvals stopped for a connection (unusual volume); its requests wait for the user.
    Paused {
        connection_id: String,
        connection_label: String,
        reason: String,
    },
}

/// One batch for the model (spec §6.2), row-major.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ModelInput {
    pub batch: u32,
    pub seq_len: u32,
    /// Markers per row (3: approve, deny, ask).
    pub k: u32,
    /// int64 [batch, seq_len]
    pub input_ids: Vec<i64>,
    /// int64 [batch, seq_len]
    pub attention_mask: Vec<i64>,
    /// int64 [batch, k]
    pub marker_pos: Vec<i64>,
    /// bool [batch, k] (0 or 1)
    pub marker_mask: Vec<u8>,
    /// int64 [batch] (0 = choice)
    pub qtype: Vec<i64>,
}

/// What the model returned for a [`ModelInput`].
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ModelOutput {
    /// f32 [batch, k], uncalibrated.
    pub logits: Vec<f32>,
    /// f32 [batch, 2], raw logits of (answer, escalate).
    pub act: Vec<f32>,
    /// f32 [batch, hidden], the [CLS] state after the decision head.
    pub pooled: Vec<f32>,
    pub hidden: u32,
}
