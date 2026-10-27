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
    /// Autopilot approves what it is confident about from the first request (stricter while the user's answers to a
    /// kind are few, and only what the risk model calls clearly benign), and denies what it is sure about where it
    /// learned to; the rest waits.
    Auto,
    /// "Auto while you're away" (Decisions v2, 01 §11): what Auto would ask is parked for the morning (a ticket: the
    /// agent carries on), people are stricter and the owner's own L1 work is looser. It never denies (only explicit
    /// rules do). Runs until turned off or until `overnight_until`, then returns to the mode it interrupted.
    Overnight,
    /// Everything is approved at once (only pairings and other phones still ask), until `bypass_until` (or until the
    /// user turns it off).
    Bypass,
    /// Everything is denied at once (pairings still reach the user).
    Lockdown,
}

impl AutopilotMode {
    /// The mode as the desktop apps are told it.
    #[must_use]
    pub fn wire(self) -> reins_proto::autopilot::Mode {
        use reins_proto::autopilot::Mode;
        match self {
            Self::Manual => Mode::Manual,
            Self::Auto => Mode::Auto,
            Self::Overnight => Mode::Overnight,
            Self::Bypass => Mode::Bypass,
            Self::Lockdown => Mode::Lockdown,
        }
    }

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

/// How bad it would be if a request were a mistake, in its context (`docs/autopilot-risk.md` §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum RiskLevel {
    /// Routine; nothing is lost if it is wrong.
    Benign,
    /// Changes or deletes something easily undone, or limited to the agent's own work.
    Risky,
    /// Hard to undo, shared or production things, or one secret exposed; bounded.
    Severe,
    /// Irreversible loss, outages, mass messages, secrets leaving, persistence, protections turned off.
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
    /// How long a parked request waits after the morning before it expires, not approved (1 to 48 h; 12 by default).
    #[uniffi(default = 12)]
    pub overnight_expiry_hours: u32,
    /// The profile of every connection that has none of its own.
    pub default_profile_id: String,
    /// Download the model on Wi-Fi only (the app's download job honours it).
    pub wifi_only: bool,
    /// A newer model this build trusts is downloaded and put in use by itself (on Wi-Fi; the app decides when).
    #[uniffi(default = true)]
    pub auto_update: bool,
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
    /// The installed model is an older one this build still trusts; `download_model` installs the newer one (and
    /// then removes the older).
    #[uniffi(default)]
    pub update: bool,
    /// When the installed model was put in use (unix seconds), if known.
    #[uniffi(default)]
    pub updated_at: Option<i64>,
}

/// What `update_model` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ModelUpdateOutcome {
    /// The model in use is the newest this build trusts.
    UpToDate,
    /// No model is installed: the first download is the user's to start.
    NotInstalled,
    /// Updating automatically is off (an automatic check only).
    Off,
    /// The host does not have the newer model yet (or has another package under its name).
    Unavailable,
    /// The newer model failed its check before; automatic checks leave it alone until "Check now".
    Rejected,
    /// The newer model was downloaded, verified, passed its self-check and is in use.
    Updated,
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

/// One action class of a profile and where it stands (spec §5.4). Its state, for the app: `manual == Some(false)`
/// locked by you; else `paused`; else `auto_approve` Auto; else it asks.
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
    /// Auto mode approves on its own in this class when it is sure (from the first request: the bar is stricter while
    /// the user's answers are few). False when paused, locked by hand, or, with no model that rates risk, before the
    /// user's answers show full measured agreement (`decide::trust`), as the engine reads it.
    pub auto_approve: bool,
    /// Auto mode may deny any request of this class it would deny (else only what it rates surely catastrophic).
    pub auto_deny: bool,
    /// The user's override: `Some(true)` unlocked by hand, `Some(false)` locked by hand, `None` automatic.
    pub manual: Option<bool>,
    /// Deprecated, always 0: Auto no longer waits for a number of decisions. Kept for apps that still read it.
    #[uniffi(default)]
    pub decisions_to_unlock: u32,
    /// Auto-approve is paused here after a wrong approval (the user said "This was wrong", or denied what Autopilot
    /// would have approved); it resumes after enough answers that agree with the user.
    #[uniffi(default)]
    pub paused: bool,
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
    /// What the matrix decided, or would have decided in shadow (Decisions v2).
    #[uniffi(default)]
    pub matrix: Option<MatrixView>,
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
    /// What the matrix decided, or would have decided in shadow (Decisions v2).
    #[uniffi(default)]
    pub matrix: Option<MatrixView>,
    /// The request it was about ("Ask me next time" takes it).
    #[uniffi(default)]
    pub request_id: String,
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
    /// Kinds of request Auto approves on its own when sure (not paused or locked), over every profile…
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

// ---- Decisions v2: zones, the matrix, setup and questions (`docs/design/decisions-v2/01-decision-engine.md` §3–§9) --

/// Whose a thing is (01 §3), least trusted last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ZoneChoice {
    Mine,
    Friends,
    Work,
    Outside,
}

impl From<reins_policy::context::Zone> for ZoneChoice {
    fn from(z: reins_policy::context::Zone) -> Self {
        use reins_policy::context::Zone;
        match z {
            Zone::Mine => Self::Mine,
            Zone::Friends => Self::Friends,
            Zone::Work => Self::Work,
            Zone::Outside => Self::Outside,
        }
    }
}

impl From<ZoneChoice> for reins_policy::context::Zone {
    fn from(z: ZoneChoice) -> Self {
        match z {
            ZoneChoice::Mine => Self::Mine,
            ZoneChoice::Friends => Self::Friends,
            ZoneChoice::Work => Self::Work,
            ZoneChoice::Outside => Self::Outside,
        }
    }
}

/// A matrix cell (01 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CellChoice {
    Auto,
    Ask,
    /// Ask the first time per resource, then Auto.
    AskFirst,
}

impl From<reins_policy::matrix::Cell> for CellChoice {
    fn from(c: reins_policy::matrix::Cell) -> Self {
        use reins_policy::matrix::Cell;
        match c {
            Cell::Auto => Self::Auto,
            Cell::Ask => Self::Ask,
            Cell::AskFirst => Self::AskFirst,
        }
    }
}

impl From<CellChoice> for reins_policy::matrix::Cell {
    fn from(c: CellChoice) -> Self {
        match c {
            CellChoice::Auto => Self::Auto,
            CellChoice::Ask => Self::Ask,
            CellChoice::AskFirst => Self::AskFirst,
        }
    }
}

/// What a fact is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum FactChoice {
    /// A GitHub user or organization.
    Owner,
    /// One repository (`owner/name`).
    Repo,
    /// An email domain.
    Domain,
    /// A person (an address, a number, a handle).
    Person,
    /// A computer or server.
    Host,
    /// A chat or group.
    Chat,
}

impl From<reins_policy::context::FactKind> for FactChoice {
    fn from(k: reins_policy::context::FactKind) -> Self {
        use reins_policy::context::FactKind;
        match k {
            FactKind::Owner => Self::Owner,
            FactKind::Repo => Self::Repo,
            FactKind::Domain => Self::Domain,
            FactKind::Person => Self::Person,
            FactKind::Host => Self::Host,
            FactKind::Chat => Self::Chat,
        }
    }
}

impl From<FactChoice> for reins_policy::context::FactKind {
    fn from(k: FactChoice) -> Self {
        match k {
            FactChoice::Owner => Self::Owner,
            FactChoice::Repo => Self::Repo,
            FactChoice::Domain => Self::Domain,
            FactChoice::Person => Self::Person,
            FactChoice::Host => Self::Host,
            FactChoice::Chat => Self::Chat,
        }
    }
}

/// One thing on the "Your world" screen.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorldItem {
    pub kind: FactChoice,
    pub key: String,
    /// What the screen shows ("acme", "ann@example.com").
    pub label: String,
    /// "github" | "email" | "messages" | "computers": the section it is listed in.
    pub service: String,
    pub zone: ZoneChoice,
    /// The owner confirmed it (else it is a proposal, which decides nothing until confirmed).
    pub confirmed: bool,
    /// Requests that touched it in the last 30 days.
    pub count: u32,
}

/// One row of the matrix, with real names from the owner's world.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MatrixRowView {
    pub zone: ZoneChoice,
    /// L0 (undoable), L1 (recoverable), L2 (reaches people), L3 (can't be undone).
    pub cells: Vec<CellChoice>,
    /// Up to three names in this zone ("dkat", "acme").
    pub names: Vec<String>,
}

/// Auto's setup (01 §9.1) and where it stands.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AutoSetup {
    /// The owner finished the setup: turning on Auto no longer guides first.
    pub done: bool,
    /// The matrix decides in Auto once the setup is done (else, or switched off, it runs in shadow: what it would
    /// decide is recorded on each entry).
    pub matrix_decides: bool,
    pub world: Vec<WorldItem>,
    pub matrix: Vec<MatrixRowView>,
    /// Open "Auto needs information" questions.
    pub questions: u32,
}

/// "Auto needs information": a party a request named that Auto doesn't know (01 §3, later questions).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AutoQuestion {
    pub id: String,
    pub kind: FactChoice,
    /// What it is about ("octo", "bob@acme.io").
    pub key: String,
    /// "github" | "email" | "messages" | "computers".
    pub service: String,
    /// The AI connection whose request named it.
    pub connection_label: String,
    pub request_id: Option<String>,
    pub at: i64,
    /// The answer the card preselects (Outside unless a fact suggests otherwise).
    pub proposed: ZoneChoice,
}

/// What the matrix decided, or would have decided in shadow, for a request.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MatrixView {
    /// `Approve` = Auto, `Ask`, or `Deny` (an explicit rule).
    pub outcome: Verdict,
    /// Asked because of a hard obligation (no permit could cover it).
    pub hard: bool,
    pub zone: ZoneChoice,
    /// 0..=3.
    pub level: u8,
    /// "Auto · Mine · Undoable".
    pub line: String,
    /// Recorded only: something else decided.
    pub shadow: bool,
}

/// The morning digest of what Overnight parked (01 §11): one notification at `at`, grouped by agent and task.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct OvernightDigest {
    /// When to show it: the earliest morning of what waits that is still to come; `None` once it came (the app shows
    /// the digest when opened instead of notifying).
    pub at: Option<i64>,
    pub total: u32,
    /// "3 waiting".
    pub title: String,
    /// "1 from Claude Code (Fix CI), 2 from ChatGPT".
    pub body: String,
    pub groups: Vec<DigestGroup>,
}

/// What one agent parked under one task (or none).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DigestGroup {
    pub connection_id: String,
    pub connection_label: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    /// Oldest first.
    pub request_ids: Vec<String>,
    /// How many of them "Approve all" covers (not hard, not L3); the rest are decided one by one.
    pub batchable: u32,
}

/// A task (01 §12): an envelope inside which Auto may do more for one agent, up to a ceiling, for a while. Also the
/// draft the owner starts or narrows. Lists name what is inside; an empty list bounds nothing, except `people`: empty
/// means nobody.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TaskView {
    /// Empty in a draft.
    pub id: String,
    pub connection_id: String,
    pub connection_label: String,
    pub title: String,
    /// The agent's words (a proposal): shown as the agent's, never read for policy.
    pub reason: Option<String>,
    /// `owner/name`, or an owner followed by a slash and a star (all its repositories).
    pub repos: Vec<String>,
    /// Branch names, or prefixes ending in a star.
    pub branches: Vec<String>,
    /// Absolute folders.
    pub paths: Vec<String>,
    /// Addresses, ids or `@domain`.
    pub people: Vec<String>,
    /// Kinds of effect ("git_push", "git_ref_delete", "file_write", "message", ...); empty: any.
    pub operations: Vec<String>,
    /// The highest level it makes Auto: 0 undoable, 1 recoverable, 2 reaches people.
    pub ceiling: u8,
    /// Uses it may make Auto; `None`: no limit.
    pub budget: Option<u32>,
    pub uses_left: Option<u32>,
    pub started_at: i64,
    /// When it ends (in a draft: `started_at` + its length).
    pub ends_at: i64,
    pub ended_at: Option<i64>,
    /// "agent" | "you" | "overnight"
    pub started_by: String,
    /// What ran under it: decided Auto by it, asked, parked.
    pub auto: u32,
    pub asked: u32,
    pub parked: u32,
    /// The owner chose that it covers any repository, folder, person and kind (shown as "Any"); otherwise it must
    /// name at least one of them.
    #[uniffi(default)]
    pub any: bool,
    /// The ceiling the agent asked for. A proposal's `ceiling` starts at the default for its repositories (01 §12: L2
    /// only when every one is the owner's) and the owner may raise it up to this.
    #[uniffi(default)]
    pub requested_ceiling: u8,
}
