//! Records and enums of Autopilot crossing the UniFFI boundary (spec §9).

use serde::{Deserialize, Serialize};

use crate::types::PendingKind;

/// The name of the Overnight mode wherever the core writes it (notes, reasons). The apps have their own constant;
/// renaming the mode is these constants plus strings.
pub const OVERNIGHT_NAME: &str = "Overnight";

/// How requests of a connection (or of all connections) are decided (spec §1, `docs/autopilot-risk.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum AutopilotMode {
    /// Every request waits, with Autopilot's suggestion shown when a model is installed; the user's answers train it.
    // Older versions stored the mode this one replaced as "Assisted".
    #[serde(alias = "Assisted")]
    Manual,
    /// Autopilot approves or denies what it is confident about, in classes that earned it; the rest waits.
    Auto,
    /// For agents left working: everything is approved, the hard floor included, except what the risk model rates
    /// catastrophic in context, which waits for the user (or is denied, `OvernightRisky`). Needs a model with the risk
    /// output; without one it decides like Auto. Runs until turned off or until `overnight_until`, then returns to the
    /// mode it interrupted.
    Overnight,
    /// Everything is approved at once (only pairings and other phones still ask), until `bypass_until` (or until the
    /// user turns it off).
    Bypass,
    /// Everything is denied at once (pairings still reach the user).
    Lockdown,
}

impl AutopilotMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
            Self::Overnight => "overnight",
            Self::Bypass => "bypass",
            Self::Lockdown => "lockdown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            // Older versions had an "assisted" mode: what Manual does now.
            "manual" | "assisted" => Self::Manual,
            "auto" => Self::Auto,
            "overnight" => Self::Overnight,
            "bypass" => Self::Bypass,
            "lockdown" => Self::Lockdown,
            _ => return None,
        })
    }
}

/// What Overnight does with a request its risk model rates catastrophic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum OvernightRisky {
    /// It waits for the user (the default).
    #[default]
    Wait,
    /// It is denied at once.
    Deny,
}

impl OvernightRisky {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wait => "wait",
            Self::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "wait" => Some(Self::Wait),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// How bad it would be if a request were a mistake, in its context (`docs/autopilot-risk.md` §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum RiskLevel {
    /// Routine; nothing is lost if it is wrong.
    Benign,
    /// Changes or deletes something easily undone, or limited to the agent's own work.
    Risky,
    /// Hard to undo, shared or production things, or one secret exposed; bounded.
    Severe,
    /// Irreversible loss, outages, mass messages, secrets leaving, or text steering the approver.
    Catastrophic,
}

impl RiskLevel {
    pub const ALL: [Self; 4] = [Self::Benign, Self::Risky, Self::Severe, Self::Catastrophic];

    /// In a note ("risk low", "risk catastrophic").
    pub fn word(self) -> &'static str {
        match self {
            Self::Benign => "low",
            Self::Risky => "risky",
            Self::Severe => "severe",
            Self::Catastrophic => "catastrophic",
        }
    }
}

/// The risk of a request: its class, how likely catastrophic it is, and why in a few words.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RiskView {
    pub level: RiskLevel,
    /// Calibrated P(catastrophic), with the AI's text only able to raise it.
    pub p_catastrophic: f32,
    /// The fact that weighs most ("deletes the default branch main"); may be empty.
    pub why: String,
    /// Overnight held it (it waited for the user or was denied).
    pub held: bool,
}

/// What to do with a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum Verdict {
    Approve,
    Deny,
    Ask,
}

/// The lowest confidence threshold a profile may have…
pub const MIN_THRESHOLD: f32 = 0.80;
/// …and the highest.
pub const MAX_THRESHOLD: f32 = 0.99;

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
    /// The user's own (`ProfileView.approve_at` / `deny_at`, set with `set_thresholds`), each 0.80 to 0.99.
    Custom,
}

impl Preset {
    /// (θ_a, θ_d). `Custom` has no values of its own here (a profile keeps them); it reads as Balanced.
    pub fn thresholds(self) -> (f32, f32) {
        match self {
            Self::Cautious => (0.98, 0.95),
            Self::Balanced | Self::Custom => (0.95, 0.90),
            Self::Relaxed => (0.90, 0.85),
        }
    }
}

/// Autopilot as a whole: the global mode, the per-connection modes and the model.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutopilotSettings {
    /// The global mode in force now (`Bypass` while a global bypass runs).
    pub mode: AutopilotMode,
    /// The global mode when no bypass runs (what the user picked; Manual until they pick one).
    pub base_mode: AutopilotMode,
    /// A global bypass runs until then (unix seconds); `i64::MAX` while it runs until the user turns it off.
    pub bypass_until: Option<i64>,
    /// When the global bypass started.
    pub bypass_since: Option<i64>,
    /// A global Overnight runs until then (`i64::MAX` while it runs until the user turns it off); `None` when the
    /// global mode is not Overnight.
    #[uniffi(default)]
    pub overnight_until: Option<i64>,
    /// When the global Overnight started.
    #[uniffi(default)]
    pub overnight_since: Option<i64>,
    /// What Overnight does with what its risk model rates catastrophic.
    pub overnight_when_risky: OvernightRisky,
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
    /// A bypass of this connection runs until then (`i64::MAX`: until the user turns it off).
    pub bypass_until: Option<i64>,
    /// When that bypass started.
    pub bypass_since: Option<i64>,
    /// Its own Overnight runs until then (`i64::MAX`: until the user turns it off).
    #[uniffi(default)]
    pub overnight_until: Option<i64>,
    #[uniffi(default)]
    pub overnight_since: Option<i64>,
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
    /// The model rates risk, which Overnight needs (without it Overnight decides like Auto).
    #[uniffi(default)]
    pub risk: bool,
}

/// One decision personality (spec §5.1).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// `Custom` when the user set thresholds of their own.
    pub preset: Preset,
    /// The thresholds in use: Auto approves at or above `approve_at` and denies at or above `deny_at`.
    pub approve_at: f32,
    pub deny_at: f32,
    /// Share of the remembered decisions where Autopilot would have done what the user did (among those it would
    /// have decided on its own); `None` before there is one.
    pub agreement: Option<f32>,
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
    /// Short label ("Push to a branch · dkat/reins").
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
    /// One line, e.g. "Like 4 times you approved: Push to a branch · dkat/reins".
    pub reason: String,
    pub neighbours: Vec<NeighbourView>,
    pub profile_id: String,
    pub profile_name: String,
    pub class_key: String,
    /// The target was never approved for this connection before.
    pub novel: bool,
    /// Always waits for the user (hard floor, spec §2).
    pub floor: bool,
    /// The model ran (false: no model, floor, or a failure — see `reason`).
    pub judged: bool,
    /// The risk, when the model rates it.
    #[uniffi(default)]
    pub risk: Option<RiskView>,
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
    /// The risk, when the model rated it.
    #[uniffi(default)]
    pub risk: Option<RiskView>,
}

/// A request Autopilot decided, for the quiet "Autopilot" notification (spec §7).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutoDecisionView {
    pub request_id: String,
    pub kind: PendingKind,
    pub connection_id: String,
    pub connection_label: String,
    /// What it was, in a few words ("Push to a branch · dkat/reins").
    pub title: String,
    /// `Approve` or `Deny`.
    pub verdict: Verdict,
    /// "autopilot" | "overnight" | "bypass" | "lockdown"
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
    /// An Overnight reached its end and the core returned to the mode it interrupted.
    OvernightEnded {
        connection_id: Option<String>,
    },
    /// Auto-approvals stopped for a connection (unusual volume); its requests wait for the user.
    Paused {
        connection_id: String,
        connection_label: String,
        reason: String,
    },
}

/// Autopilot over the last days, for the strip at the top of its page.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AutopilotStats {
    /// The window, in days.
    pub days: u32,
    /// Requests the model scored.
    pub scored: u32,
    /// Share of the user's decisions Autopilot would have made the same way (among those it would have decided on its
    /// own); `None` before there is one.
    pub matched: Option<f32>,
    /// How many decisions `matched` is out of.
    pub compared: u32,
    /// Kinds of request Auto may approve on its own, over every profile…
    pub kinds_on_auto: u32,
    /// …out of the kinds seen.
    pub kinds: u32,
}

/// What a profile's last days would have been at other thresholds (the stored evaluations replayed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct ThresholdReplay {
    pub days: u32,
    /// Requests it would have approved or denied on its own.
    pub decided: u32,
    /// Requests it would have left to the user (always-ask ones included).
    pub asked: u32,
    /// Of `decided`, those where the user did, or later said, otherwise.
    pub undone: u32,
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
    /// f32 [batch, 4], uncalibrated severity logits (benign, risky, severe, catastrophic), from a model whose
    /// `laya_config.json` declares the `risk` output; empty otherwise.
    #[uniffi(default)]
    pub risk: Vec<f32>,
}
