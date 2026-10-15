//! Autopilot in the engine: the pass over newly parked items (spec §7), learning from the user's decisions, and what
//! `ReinsCore`'s Autopilot methods do (spec §9).
//!
//! The pass never loses a request: whatever goes wrong, the item waits for the user like before Autopilot existed,
//! and what went wrong is kept with it (the activity entry of the user's decision shows it).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use reins_proto::blob::BlobInfo;
use reins_proto::pairing::PairingRequest;

use super::adapter::{self, RETRAIN_EVERY};
use super::context::{Decision, deciding};
use super::decide::{self, Pass, Probs};
use super::gates::{self, RATE_PER_10_MIN, RATE_PER_DAY};
use super::memory::{CORRECTION_WEIGHT, Embedding, MemoryRow, Source, Variant};
use super::model::{self, DownloadState, KnownModel, Laya};
use super::modes::{self, DEFAULT_BYPASS_MINUTES};
use super::overnight;
use super::risk::{self, RiskNote};
use super::runtime::{DownloadProgress, ModelRuntime};
use super::situation::{self, Described};
use super::stats;
use super::store::{
    Decided, Judged, NeighbourNote, Profile, SUGGESTION_RETENTION_SECS, StoredProfile, Suggestion, Thresholds,
};
use super::types::{
    AutoDecisionView, AutopilotEvent, AutopilotMode, AutopilotNote, AutopilotSettings, AutopilotStats, ClassView,
    ConnectionAutopilot, ModelStatus, NeighbourView, OVERNIGHT_NAME, OvernightRisky, Preset, ProfileView, RiskLevel,
    SuggestionView, ThresholdReplay, Verdict,
};
use crate::engine::Engine;
use crate::phone_api::check_id;
use crate::store::{AuditAutopilot, AuditRecord, PendingRow, unix_now};
use crate::types::{
    ApprovalChoice, ApprovalKind, ApprovalView, GrantScopeChoice, PendingItem, PendingKind, StandingGrant,
};
use crate::views::{self, ParkedRequest};
use crate::{CoreError, OwnerCheck};

/// Neighbours kept with an evaluation and shown.
const SHOWN_NEIGHBOURS: usize = 5;
/// A "paused" notice is sent at most this often per connection.
const PAUSE_NOTICE_SECS: i64 = 600;
/// Overnight's approvals per connection: at most this many in 10 minutes…
pub const OVERNIGHT_RATE_PER_10_MIN: u32 = 60;
/// …and this many in a day.
pub const OVERNIGHT_RATE_PER_DAY: u32 = 600;
/// After something of a connection was held, or this many severe requests of it were approved within
/// [`OVERNIGHT_PAUSE_SECS`], its severe requests wait for the user for that long.
pub const OVERNIGHT_SEVERE_BURST: usize = 5;
pub const OVERNIGHT_PAUSE_SECS: i64 = 30 * 60;
/// Loading the model, or one batch through it, may take this long before the request is left to the user.
#[cfg(not(test))]
const MODEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const MODEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const WIFI_ONLY_KEY: &str = "autopilot.wifi_only";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Autopilot's state on the engine.
#[derive(Default)]
pub struct State {
    runtime: Mutex<Option<Arc<dyn ModelRuntime>>>,
    /// The opened package, and whether the runtime has loaded its model.
    laya: Mutex<Option<Arc<Laya>>>,
    loaded: AtomicBool,
    pub(crate) download: DownloadState,
    /// One pass at a time.
    pass: tokio::sync::Mutex<()>,
    /// Profiles are changed read-modify-write under this, so that no writer undoes another's change.
    profile_writes: Mutex<()>,
    /// Connection → when its "paused" notice was sent.
    paused: Mutex<HashMap<String, i64>>,
}

/// A parked item, as the pass sees it.
struct Subject {
    id: String,
    kind: PendingKind,
    connection_id: String,
    connection_label: String,
    described: Described,
    floor: Option<String>,
    /// Overnight may weigh `floor` with the risk model instead of always asking ([`gates::overnight_weighs`]).
    floor_weighable: bool,
    item: PendingItem,
    /// What approving it once selects (as the approval screen preselects).
    choice: ApprovalChoice,
    /// What a bypass approves: every message or item shown, every account offered.
    everything: ApprovalChoice,
    /// An AI asking for a standing permission (a bypass caps it at its own end).
    grant: bool,
    /// A purchase, approved through its own path (with the defaults its screen would show).
    purchase: bool,
    /// The origin a permission granted while deciding it is stored with (a bypass marks its own).
    grant_origin: Option<String>,
    /// What the approval screen shows (requests only): the risk context is worked out from it.
    view: Option<ApprovalView>,
}

/// What one pass keeps between items.
struct PassCtx {
    now: i64,
    /// The model can run in this process (the app). Without it (the iOS notification extension) only Bypass and
    /// Lockdown are decided; the rest is left unevaluated for the app's next pass.
    model: bool,
    /// Connection → when the server says it was created (fetched once, when needed).
    created: Option<HashMap<String, i64>>,
    /// Profile id → the profile and its memory.
    profiles: HashMap<String, (StoredProfile, Vec<MemoryRow>)>,
}

fn verb(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approved",
        Verdict::Deny => "denied",
        Verdict::Ask => "asked",
    }
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a percentage of a probability")]
fn pct(p: f32) -> u32 {
    (p.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Asks the owner; refuses unless the owner confirmed now.
async fn confirm_owner(owner: &dyn OwnerCheck, reason: String) -> Result<(), CoreError> {
    match owner.confirm(reason).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(CoreError::invalid("the owner did not confirm")),
        Err(e) => Err(CoreError::invalid(format!("the owner check failed: {e}"))),
    }
}

/// Who carries out a decision. An Overnight approval needs the proof its gate made for that very request
/// ([`overnight::gate`]); nothing else can make one.
enum By {
    Lockdown,
    Bypass,
    Autopilot,
    Overnight(Option<overnight::Proof>),
}

impl By {
    fn name(&self) -> &'static str {
        match self {
            Self::Lockdown => "lockdown",
            Self::Bypass => "bypass",
            Self::Autopilot => "autopilot",
            Self::Overnight(_) => "overnight",
        }
    }
}

/// The one line a notification or the pending list shows.
pub(crate) fn suggestion_line(s: &Suggestion) -> Option<String> {
    if s.risk.as_ref().is_some_and(|r| r.held) {
        return Some(s.reason.clone());
    }
    if s.floor.is_some() {
        return Some("Autopilot always asks you for this".to_owned());
    }
    let j = s.judged.as_ref()?;
    Some(match s.verdict {
        Verdict::Approve => format!("Autopilot would approve · {}%", pct(j.probs[0])),
        Verdict::Deny => format!("Autopilot would deny · {}%", pct(j.probs[1])),
        Verdict::Ask => format!("Autopilot would ask you · approve {}%", pct(j.probs[0])),
    })
}

/// Everything a request offers: each message or item shown (sensitive ones too), each account not shared yet.
fn everything(view: &ApprovalView) -> ApprovalChoice {
    let selected_message_ids = if view.kind == ApprovalKind::Accounts {
        view.accounts.iter().filter(|a| !view.shared_accounts.contains(a)).cloned().collect()
    } else {
        view.messages.iter().map(|m| m.id.clone()).collect()
    };
    ApprovalChoice {
        selected_message_ids,
        standing: None,
    }
}

fn default_choice(view: &ApprovalView) -> ApprovalChoice {
    ApprovalChoice {
        selected_message_ids: view
            .messages
            .iter()
            .filter(|m| !m.sensitive && !m.covered_by_grant)
            .map(|m| m.id.clone())
            .collect(),
        standing: None,
    }
}

fn neighbour_lines(j: Option<&Judged>) -> Vec<String> {
    j.map(|j| j.neighbours.iter().map(|n| format!("{}: {}", verb(n.verdict), n.label)).collect()).unwrap_or_default()
}

/// The activity note of a request (what Autopilot did or would have done).
fn note_of(s: &Suggestion, suggested: Verdict) -> AuditAutopilot {
    let j = s.judged.as_ref();
    AuditAutopilot {
        request_id: s.request_id.clone(),
        mode: s.mode,
        suggested,
        p_approve: j.map_or(0.0, |j| j.probs[0]),
        p_deny: j.map_or(0.0, |j| j.probs[1]),
        confidence: j.map_or(0.0, |j| j.confidence),
        profile_id: s.profile_id.clone(),
        profile_name: s.profile_name.clone(),
        neighbours: neighbour_lines(j),
        reason: s.reason.clone(),
        risk: s.risk.clone(),
    }
}

/// What the activity screen shows of a note.
pub(crate) fn activity_note(r: &AuditRecord) -> Option<AutopilotNote> {
    let n = r.info.autopilot.as_ref()?;
    Some(AutopilotNote {
        mode: n.mode,
        suggested: n.suggested,
        p_approve: n.p_approve,
        p_deny: n.p_deny,
        confidence: n.confidence,
        profile_id: n.profile_id.clone(),
        profile_name: n.profile_name.clone(),
        neighbours: n.neighbours.clone(),
        reason: n.reason.clone(),
        correctable: r.info.decided_by != "lockdown"
            && matches!(r.outcome.as_str(), "released" | "sent" | "denied")
            && r.at > unix_now() - SUGGESTION_RETENTION_SECS,
        risk: n.risk.as_ref().map(RiskNote::view),
    })
}

fn suggestion_view(s: &Suggestion) -> SuggestionView {
    let j = s.judged.as_ref();
    SuggestionView {
        request_id: s.request_id.clone(),
        verdict: s.verdict,
        mode: s.mode,
        p_approve: j.map_or(0.0, |j| j.probs[0]),
        p_deny: j.map_or(0.0, |j| j.probs[1]),
        confidence: j.map_or(0.0, |j| j.confidence),
        reason: s.reason.clone(),
        neighbours: j
            .map(|j| {
                j.neighbours
                    .iter()
                    .map(|n| NeighbourView {
                        label: n.label.clone(),
                        verdict: n.verdict,
                        similarity: n.similarity,
                        at: n.at,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        profile_id: s.profile_id.clone(),
        profile_name: s.profile_name.clone(),
        class_key: s.class_key.clone(),
        novel: s.novel,
        floor: s.floor.is_some(),
        judged: j.is_some(),
        risk: s.risk.as_ref().map(RiskNote::view),
    }
}

/// Why it waits, from what the model and the gates said.
fn reason_of(j: &Judged, verdict: Verdict, novel: bool) -> String {
    let approvals = j.neighbours.iter().filter(|n| n.verdict == Verdict::Approve).count();
    let denials = j.neighbours.len() - approvals;
    let like = |v: Verdict, n: usize| {
        j.neighbours.iter().find(|x| x.verdict == v).map(|x| format!("Like {n} times you {}: {}", verb(v), x.label))
    };
    match verdict {
        Verdict::Approve => {
            like(Verdict::Approve, approvals).unwrap_or_else(|| format!("Looks routine ({}% approve)", pct(j.probs[0])))
        }
        Verdict::Deny => like(Verdict::Deny, denials.max(1))
            .filter(|_| denials > 0)
            .unwrap_or_else(|| format!("Looks harmful ({}% deny)", pct(j.probs[1]))),
        Verdict::Ask if novel => "New for this connection: never approved before".to_owned(),
        Verdict::Ask if j.escalate >= decide::ESCALATE_LIMIT => "The model wants you to look at this".to_owned(),
        Verdict::Ask => format!("Not sure ({}% approve, {}% deny)", pct(j.probs[0]), pct(j.probs[1])),
    }
}

/// `service/action/class` in words.
fn class_label(engine: &Engine, key: &str) -> String {
    let mut parts = key.split('/');
    let service = parts.next().unwrap_or_default();
    let name = match service.strip_prefix("mcp:") {
        Some(id) => engine.store.mcp_server(id).ok().flatten().map_or_else(|| "MCP".to_owned(), |s| s.name),
        None => views::service_name(service).to_owned(),
    };
    // A hook's question by its rule's group: "Desktop app · ask · .env files".
    let named = |part: &str| match crate::connector::desktop::guard_group_label(part) {
        Some(label) if service == reins_proto::connector::DESKTOP => label.to_owned(),
        _ => part.to_owned(),
    };
    std::iter::once(name).chain(parts.map(named)).collect::<Vec<_>>().join(" · ")
}

/// A class key from a typed situation (the playground).
fn class_of_text(text: &str) -> (String, bool) {
    let get = |key: &str| {
        text.lines().find_map(|l| l.strip_prefix(&format!("{key}: ")).map(str::trim)).unwrap_or_default().to_owned()
    };
    (situation::class_key(&get("service"), &get("action"), &get("class")), get("target is new") != "no")
}

impl Engine {
    // ---- the model -------------------------------------------------------------------------------------------------

    fn ap_known(&self) -> KnownModel {
        self.cfg.models.first().cloned().unwrap_or_else(|| model::KNOWN_MODELS[0].clone())
    }

    fn ap_installed(&self) -> bool {
        model::installed(&self.data_dir, &self.ap_known())
    }

    /// The app's runtime for the model; replaces (and unloads) an earlier one.
    pub fn set_model_runtime(&self, runtime: Arc<dyn ModelRuntime>) {
        let old = lock(&self.autopilot.runtime).replace(runtime);
        self.autopilot.loaded.store(false, Ordering::SeqCst);
        if let Some(old) = old {
            old.unload();
        }
    }

    fn ap_unload(&self) {
        *lock(&self.autopilot.laya) = None;
        if self.autopilot.loaded.swap(false, Ordering::SeqCst)
            && let Some(rt) = lock(&self.autopilot.runtime).clone()
        {
            rt.unload();
        }
    }

    /// The opened (verified) package, without loading it into the runtime.
    async fn ap_laya(&self) -> Result<Arc<Laya>, String> {
        let known = self.ap_known();
        if let Some(l) = lock(&self.autopilot.laya).clone().filter(|l| l.id == known.id) {
            return Ok(l);
        }
        let dir = self.data_dir.clone();
        let opened = tokio::task::spawn_blocking(move || Laya::open(&dir, &known))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let opened = Arc::new(opened);
        *lock(&self.autopilot.laya) = Some(Arc::clone(&opened));
        self.autopilot.loaded.store(false, Ordering::SeqCst);
        Ok(opened)
    }

    /// The opened package and the runtime with its model loaded (lazily, on first need).
    async fn ap_model(&self) -> Result<(Arc<Laya>, Arc<dyn ModelRuntime>), String> {
        let runtime = lock(&self.autopilot.runtime).clone().ok_or("the app has no model runtime")?;
        let laya = self.ap_laya().await?;
        if !self.autopilot.loaded.load(Ordering::SeqCst) {
            let path = laya.model_path.to_string_lossy().into_owned();
            let rt = Arc::clone(&runtime);
            tokio::time::timeout(MODEL_TIMEOUT, tokio::task::spawn_blocking(move || rt.load(path)))
                .await
                .map_err(|_| "loading the model took too long".to_owned())?
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("the model could not be loaded ({e})"))?;
            self.autopilot.loaded.store(true, Ordering::SeqCst);
        }
        Ok((laya, runtime))
    }

    /// One forward pass per text, off the async runtime. A runtime that does not answer in time is given up on (the
    /// request then waits for the user).
    /// Returns the (verified) package that ran too.
    async fn ap_infer(&self, states: Vec<String>) -> Result<(Arc<Laya>, Vec<Pass>), String> {
        let (laya, runtime) = self.ap_model().await?;
        let id = Arc::clone(&laya);
        let work = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = states.iter().map(String::as_str).collect();
            laya.run(runtime.as_ref(), &refs)
        });
        let passes = tokio::time::timeout(MODEL_TIMEOUT, work)
            .await
            .map_err(|_| "the model took too long".to_owned())?
            .map_err(|e| e.to_string())?;
        match passes {
            Ok(p) => Ok((id, p)),
            Err(e) => {
                // Loaded again next time, in case the runtime lost it.
                self.autopilot.loaded.store(false, Ordering::SeqCst);
                Err(e.to_string())
            }
        }
    }

    // ---- profiles --------------------------------------------------------------------------------------------------

    /// The profiles, creating Personal (the default) and Work on first use.
    fn ap_profiles(&self) -> Result<Vec<StoredProfile>, CoreError> {
        let profiles = self.store.ap_profiles()?;
        if !profiles.is_empty() {
            return Ok(profiles);
        }
        // Under the profile lock and checked again, so that two first uses make the built-ins once.
        let _one = lock(&self.autopilot.profile_writes);
        let profiles = self.store.ap_profiles()?;
        if !profiles.is_empty() {
            return Ok(profiles);
        }
        let now = unix_now();
        let mut first = None;
        for (offset, (name, icon)) in [("Personal", "🙂"), ("Work", "💼")].into_iter().enumerate() {
            let id = uuid::Uuid::new_v4().to_string();
            let profile = Profile {
                name: name.to_owned(),
                icon: Some(icon.to_owned()),
                ..Profile::default()
            };
            self.store.ap_put_profile(&id, now + i64::try_from(offset).unwrap_or(0), &profile)?;
            first.get_or_insert(id);
        }
        let mut global = self.store.ap_mode_row("")?;
        global.profile_id = first;
        self.ap_write_mode("", &global, now)?;
        self.store.ap_profiles()
    }

    /// Changes a stored profile: read, change, write, under the profile lock.
    fn ap_update_profile(&self, id: &str, change: impl FnOnce(&mut Profile)) -> Result<(), CoreError> {
        let _one = lock(&self.autopilot.profile_writes);
        let p = self.store.ap_profiles()?.into_iter().find(|p| p.id == id).ok_or(CoreError::NotFound)?;
        let mut profile = p.profile;
        change(&mut profile);
        self.store.ap_put_profile(&p.id, p.created_at, &profile)
    }

    fn ap_default_profile_id(&self, profiles: &[StoredProfile]) -> Result<String, CoreError> {
        let global = self.store.ap_mode_row("")?;
        Ok(global
            .profile_id
            .filter(|id| profiles.iter().any(|p| &p.id == id))
            .or_else(|| profiles.first().map(|p| p.id.clone()))
            .unwrap_or_default())
    }

    /// The profile a connection's decisions train.
    fn ap_profile_of(&self, connection_id: &str) -> Result<StoredProfile, CoreError> {
        let profiles = self.ap_profiles()?;
        let own = self.store.ap_mode_row(connection_id)?.profile_id;
        let id = match own.filter(|id| profiles.iter().any(|p| &p.id == id)) {
            Some(id) => id,
            None => self.ap_default_profile_id(&profiles)?,
        };
        profiles.into_iter().find(|p| p.id == id).ok_or(CoreError::NotFound)
    }

    pub(crate) fn ap_mode_for(&self, connection_id: &str, now: i64) -> Result<AutopilotMode, CoreError> {
        let global = self.store.ap_mode_row("")?;
        let own = self.store.ap_mode_row(connection_id)?;
        Ok(modes::effective(&global, Some(&own), now))
    }

    // ---- the pass --------------------------------------------------------------------------------------------------

    /// Evaluates every parked item not evaluated yet: decides it when its mode says so, else attaches the suggestion
    /// and tells the app it waits. Runs after every push and sync; never fails (items just wait).
    pub(crate) async fn autopilot_pass(&self) {
        let _one = self.autopilot.pass.lock().await;
        if let Err(e) = self.ap_pass(true).await {
            log::warn!("Autopilot could not look at the waiting items: {e}");
        }
    }

    /// The pass for a process that cannot run the model (the iOS notification extension): Bypass and Lockdown decide
    /// at once, as they need no model; everything else is left unevaluated for the app's next pass.
    pub(crate) async fn autopilot_pass_without_model(&self) {
        let _one = self.autopilot.pass.lock().await;
        if let Err(e) = self.ap_pass(false).await {
            log::warn!("Autopilot could not look at the waiting items: {e}");
        }
    }

    async fn ap_pass(&self, model: bool) -> Result<(), CoreError> {
        let now = unix_now();
        self.ap_expire_bypasses(now)?;
        self.store.ap_prune(now)?;
        let evaluated = self.store.ap_evaluated()?;
        let mut rows: Vec<PendingRow> =
            // Another phone asking for the account's keys is only ever answered by the user, and is not judged.
            self.store
                .pending_rows(now)?
                .into_iter()
                .filter(|r| r.kind != PendingKind::Join && !evaluated.contains(&r.id))
                .collect();
        if rows.is_empty() {
            return Ok(());
        }
        rows.reverse();
        let mut ctx = PassCtx {
            now,
            model,
            created: None,
            profiles: HashMap::new(),
        };
        for row in rows {
            let Some(_claim) = self.claim(&row.id) else {
                continue;
            };
            if self.store.ap_suggestion(&row.id)?.is_some() || self.store.pending_item(&row.id, now)?.is_none() {
                continue;
            }
            if let Err(e) = Box::pin(self.ap_item(&row, &mut ctx)).await {
                // Nothing was decided: the item waits, as it would without Autopilot; under a bypass it is tried
                // again by the next pass, and nobody is told it waits.
                log::warn!("Autopilot could not evaluate a request: {e}");
                let bypassed = Self::ap_subject(&row)
                    .ok()
                    .flatten()
                    .is_some_and(|s| self.ap_mode_for(&s.connection_id, now).ok() == Some(AutopilotMode::Bypass));
                if !bypassed && let Some(item) = Self::ap_pending_item(&row) {
                    self.notifier.item_pending(item);
                }
            }
        }
        Ok(())
    }

    /// The pending-list entry of a parked row.
    fn ap_pending_item(row: &PendingRow) -> Option<PendingItem> {
        match row.kind {
            PendingKind::Request => {
                serde_json::from_slice::<ParkedRequest>(&row.payload).ok().map(|p| views::request_item(&p))
            }
            PendingKind::Pairing => {
                serde_json::from_slice::<PairingRequest>(&row.payload).ok().map(|p| views::pairing_item(&p))
            }
            PendingKind::Blob => Self::blob_pending_item(&row.payload),
            PendingKind::Join => serde_json::from_slice(&row.payload).ok().map(|j| crate::join::join_item(&j)),
        }
    }

    fn ap_subject(row: &PendingRow) -> Result<Option<Subject>, CoreError> {
        let corrupt = || CoreError::storage("corrupt parked item");
        Ok(match row.kind {
            PendingKind::Pairing | PendingKind::Join => None,
            PendingKind::Request => {
                let parked: ParkedRequest = serde_json::from_slice(&row.payload).map_err(|_| corrupt())?;
                let view = views::approval_view(&parked);
                Some(Subject {
                    id: row.id.clone(),
                    kind: PendingKind::Request,
                    connection_id: parked.request.connection_id.0.clone(),
                    connection_label: parked.label(),
                    described: situation::describe_request(&view),
                    floor: gates::request_floor(&view).map(str::to_owned),
                    floor_weighable: gates::overnight_weighs(&view),
                    item: views::request_item(&parked),
                    choice: default_choice(&view),
                    everything: everything(&view),
                    grant: view.kind == ApprovalKind::Grant,
                    purchase: view.purchase.is_some(),
                    grant_origin: None,
                    view: Some(view),
                })
            }
            PendingKind::Blob => {
                let info: BlobInfo = serde_json::from_slice(&row.payload).map_err(|_| corrupt())?;
                let view = crate::blob::blob_view(&info);
                let flagged = gates::blob_flagged(&view);
                Some(Subject {
                    id: row.id.clone(),
                    kind: PendingKind::Blob,
                    connection_id: info.connection_id.0.clone(),
                    connection_label: view.connection_label.clone(),
                    described: situation::describe_blob(&view, flagged),
                    floor: gates::blob_floor(&view).map(str::to_owned),
                    // an upload that may hold something to run: what it does, weighed like the rest of its kind
                    floor_weighable: true,
                    item: crate::blob::blob_item(&info),
                    choice: ApprovalChoice {
                        selected_message_ids: Vec::new(),
                        standing: None,
                    },
                    everything: ApprovalChoice {
                        selected_message_ids: Vec::new(),
                        standing: None,
                    },
                    grant: false,
                    purchase: false,
                    grant_origin: None,
                    view: None,
                })
            }
        })
    }

    /// Seconds since the connection was paired: the later of the server's date and this phone's approval. The server's
    /// dates are kept once read (they never change), so a listing that fails (offline, a server hiccup) does not make
    /// a known connection look new; only one never seen counts as unknown.
    async fn ap_age(&self, ctx: &mut PassCtx, connection_id: &str) -> Option<i64> {
        let cached = self.store.ap_created_at(connection_id).ok().flatten();
        if cached.is_none() && ctx.created.is_none() {
            ctx.created = Some(match self.connections().await {
                Ok(list) => {
                    for c in &list {
                        if let Err(e) = self.store.ap_set_created_at(&c.id, c.created_at) {
                            log::warn!("Autopilot could not keep when a connection was made: {e}");
                        }
                    }
                    list.into_iter().map(|c| (c.id, c.created_at)).collect()
                }
                Err(e) => {
                    log::warn!("Autopilot could not list the connections: {e}");
                    HashMap::new()
                }
            });
        }
        let server = cached.or_else(|| ctx.created.as_ref().and_then(|m| m.get(connection_id).copied()));
        let local = self.store.ap_paired_at(connection_id).ok().flatten();
        let since = match (server, local) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }?;
        Some(ctx.now - since)
    }

    fn ap_profile_loaded<'c>(
        &self,
        ctx: &'c mut PassCtx,
        connection_id: &str,
    ) -> Result<&'c mut (StoredProfile, Vec<MemoryRow>), CoreError> {
        let profile = self.ap_profile_of(connection_id)?;
        let id = profile.id.clone();
        if !ctx.profiles.contains_key(&id) {
            let memory = self.store.ap_memory(&id)?;
            ctx.profiles.insert(id.clone(), (profile, memory));
        }
        ctx.profiles.get_mut(&id).ok_or(CoreError::NotFound)
    }

    async fn ap_item(&self, row: &PendingRow, ctx: &mut PassCtx) -> Result<(), CoreError> {
        let now = ctx.now;
        if !ctx.model && row.kind == PendingKind::Pairing {
            // Left to the app, which tells the user.
            return Ok(());
        }
        let Some(mut subject) = Self::ap_subject(row)? else {
            // A new connection is only ever approved by the user, in every mode.
            let pairing: PairingRequest =
                serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked pairing"))?;
            let global = self.store.ap_mode_row("")?;
            let s = Suggestion {
                deletes: false,
                request_id: row.id.clone(),
                at: now,
                kind: "pairing".to_owned(),
                connection_id: String::new(),
                connection_label: pairing.client_name.clone(),
                mode: modes::global_mode(&global, now),
                profile_id: String::new(),
                profile_name: String::new(),
                class_key: String::new(),
                label: String::new(),
                target_key: String::new(),
                s_facts: String::new(),
                s_full: String::new(),
                novel: true,
                floor: Some("a new connection is only ever approved by you".to_owned()),
                judged: None,
                verdict: Verdict::Ask,
                reason: "Always asks: a new connection is only ever approved by you".to_owned(),
                decided: None,
                risk: None,
            };
            self.store.ap_put_suggestion(&s)?;
            let mut item = views::pairing_item(&pairing);
            item.suggestion = suggestion_line(&s);
            self.notifier.item_pending(item);
            return Ok(());
        };
        let mode = self.ap_mode_for(&subject.connection_id, now)?;
        if mode == AutopilotMode::Bypass {
            // Before anything that could fail or wait (the profile, the connection list): nothing a bypass covers is
            // left waiting.
            return Box::pin(self.ap_bypass_item(subject, now)).await;
        }
        if !ctx.model && matches!(mode, AutopilotMode::Manual | AutopilotMode::Auto | AutopilotMode::Overnight) {
            // Judged by the app's next pass, which has the model.
            return Ok(());
        }
        let stored = self.ap_profile_of(&subject.connection_id)?;
        let thresholds = stored.profile.thresholds();
        let (profile_id, profile_name) = (stored.id, stored.profile.name);
        let target_key = self.store.ap_target_key(&subject.connection_id, &subject.described.facts.target)?;
        let novel = target_key.is_empty() || !self.store.ap_target_seen(&target_key)?;
        let mut s = Suggestion {
            deletes: false,
            request_id: subject.id.clone(),
            at: now,
            kind: match subject.kind {
                PendingKind::Blob => "blob",
                _ => "request",
            }
            .to_owned(),
            connection_id: subject.connection_id.clone(),
            connection_label: subject.connection_label.clone(),
            mode,
            profile_id,
            profile_name,
            class_key: subject.described.class_key.clone(),
            label: subject.described.label.clone(),
            target_key,
            s_facts: String::new(),
            s_full: String::new(),
            novel,
            floor: None,
            judged: None,
            verdict: Verdict::Ask,
            reason: String::new(),
            decided: None,
            risk: None,
        };

        match mode {
            AutopilotMode::Lockdown => {
                s.verdict = Verdict::Deny;
                "Lockdown: every request is denied".clone_into(&mut s.reason);
                return Box::pin(self.ap_conclude(&subject, s, Verdict::Deny, By::Lockdown)).await;
            }
            AutopilotMode::Bypass | AutopilotMode::Manual | AutopilotMode::Auto | AutopilotMode::Overnight => {}
        }

        // A model that rates risk reads a hook's question as the action it is about, with the context line the core
        // works out (docs/autopilot-risk.md §3), the connection's last hour included.
        // (Read from the package's configuration: opening it, which verifies it, is the judging's job. This reading
        // only decides what the model is shown; nothing is decided on it.)
        let rates_risk = mode != AutopilotMode::Bypass && model::rates_risk(&self.data_dir, &self.ap_known());
        let context = if rates_risk {
            let context = match &subject.view {
                Some(view) => {
                    let c = risk::refine(&mut subject.described, view);
                    subject.described.label =
                        situation::label_of(&subject.described.facts.operation, &subject.described.facts.target);
                    s.label.clone_from(&subject.described.label);
                    c
                }
                None => risk::upload_context(),
            };
            s.deletes = context.deletes();
            if let Some(line) = self.ap_recent_line(&subject, context.deletes(), now)? {
                subject.described.facts.context.push(line);
            }
            Some(context)
        } else {
            None
        };

        // What the model reads, also kept for corrections.
        let age = Box::pin(self.ap_age(ctx, &subject.connection_id)).await;
        let mut facts = subject.described.facts.clone();
        facts.connection.clone_from(&subject.connection_label);
        facts.connection_age = age;
        facts.connection_history = self.store.ap_history(&subject.connection_id).ok();
        facts.target_is_new = (!facts.target.is_empty()).then_some(novel);
        (s.s_facts, s.s_full) = situation::render(&facts, &subject.described.ai);
        // The risk's third view: the facts with what the agent wrote into them as placeholders (risk::neutral_facts),
        // read in the same run when it differs; its class can only raise the risk.
        // (the AI's part with them too: a hook's question and detail repeat the command)
        let neutral = rates_risk
            .then(|| {
                let n = risk::neutral_facts(&facts);
                let ai = risk::neutral_ai(&facts, &n, &subject.described.ai);
                situation::render(&n, &ai).1
            })
            .filter(|n| *n != s.s_facts && *n != s.s_full);

        let young = gates::young(age, self.cfg.new_connection_secs)
            .then(|| "a connection paired less than 10 minutes ago".to_owned());
        // Overnight is Auto, except that part of the hard floor (what an action does: a destructive tool, a push that
        // rewrites history, a far-reaching change, an upload that may run) is weighed by the risk model instead of
        // always asked, and only once a verified model rated this very request and the core read it with certainty
        // (below). The rest of the floor (who decides, money, sign-ins, what could not be read) and a just-paired AI
        // wait, as in Auto.
        let weighed = mode == AutopilotMode::Overnight && rates_risk && subject.floor_weighable;
        let floor = subject.floor.clone().filter(|_| !weighed);
        if let Some(floor) = floor.or(young) {
            s.reason = format!("Always asks: {floor}");
            s.floor = Some(floor);
            return self.ap_wait(&subject, &s);
        }

        let judged = {
            let profile = self.ap_profile_loaded(ctx, &subject.connection_id)?;
            Box::pin(self.ap_judge(&s, neutral.as_deref(), profile, now)).await
        };
        let (judged, probs) = match judged {
            Ok(j) => j,
            Err(why) => {
                s.reason = format!("Autopilot could not judge this: {why}");
                return self.ap_wait(&subject, &s);
            }
        };
        s.verdict = decide::would(probs, novel, judged.escalate, thresholds);
        s.reason = reason_of(&judged, s.verdict, novel);
        // Overnight acts on risk when the verified package that judged this request declares it; it then decides only
        // through its gate (overnight::gate), the one place an Overnight approval is made.
        let overnight = mode == AutopilotMode::Overnight && judged.risk_declared;
        let readable = judged.risk.filter(|p| risk::readable(*p));
        let outcome = context.as_ref().map(risk::Context::outcome);
        let tripwire = match &outcome {
            Some(risk::Outcome::Tripwire(line)) => Some(line.clone()),
            _ => None,
        };
        let first_line = subject.described.facts.context.first().cloned();
        if overnight {
            let (stats, manual) = {
                let profile = self.ap_profile_loaded(ctx, &subject.connection_id)?;
                let refs: Vec<&MemoryRow> = profile.1.iter().collect();
                (gates::class_stats(&refs, &s.class_key), profile.0.profile.manual.get(&s.class_key).copied())
            };
            let (may_approve, may_deny) = gates::unlocked(&stats, manual.map(|locked| !locked));
            let view_max = judged.risk_view_max;
            let level = readable
                .map_or(RiskLevel::Catastrophic, |p| risk::level(p, false).max(view_max.unwrap_or(RiskLevel::Benign)));
            let paused = match self.ap_overnight_paused(&subject.connection_id, level, now)? {
                Some(why) => Some(why),
                None => (!self.ap_rate_ok(&subject.connection_id, now)?).then_some("unusual volume"),
            };
            let risk_vec = judged.risk.map(|p| p.to_vec());
            let request = overnight::Request {
                request_id: &subject.id,
                outcome: outcome.as_ref(),
                risk_declared: judged.risk_declared,
                risk: risk_vec.as_deref(),
                threshold: judged.risk_threshold,
                weighed_floor: subject.floor.is_some(),
                why: first_line.as_deref(),
                auto: overnight::AutoChecks {
                    verdict: s.verdict,
                    may_approve,
                    may_deny,
                    young: false,
                    kept_floor: None,
                    paused,
                },
            };
            let decision = overnight::gate(&request);
            let held = match &decision {
                overnight::Gate::Hold(overnight::Hold::Catastrophic(why) | overnight::Hold::Uncertain(why)) => {
                    Some(why.clone())
                }
                _ => None,
            };
            let catastrophic = matches!(decision, overnight::Gate::Hold(overnight::Hold::Catastrophic(_)));
            s.risk = Some(RiskNote {
                level: readable.map_or(RiskLevel::Catastrophic, |p| {
                    risk::level(p, catastrophic).max(view_max.unwrap_or(RiskLevel::Benign))
                }),
                probs: readable.unwrap_or([0.0; 4]),
                why: held.clone().or_else(|| tripwire.clone()).or_else(|| first_line.clone()).unwrap_or_default(),
                held: held.is_some(),
                tripwire: tripwire.is_some(),
            });
            s.judged = Some(judged);
            let line = s.risk.as_ref().map(RiskNote::line).unwrap_or_default();
            return match decision {
                overnight::Gate::Approve(proof) => {
                    log::debug!("Overnight approves a request at P(catastrophic) {:.4}", proof.p_catastrophic());
                    s.verdict = Verdict::Approve;
                    s.reason = format!("Allowed {} · {line}", OVERNIGHT_NAME.to_lowercase());
                    // Marked as this Overnight's: anything granted while carrying it out ends when it does. (It
                    // approves once, as Auto does: it never asks for a standing permission.)
                    let scope = self.ap_overnight_scope(&subject.connection_id, now)?;
                    subject.grant_origin = Some(modes::overnight_origin(&scope));
                    Box::pin(self.ap_conclude(&subject, s, Verdict::Approve, By::Overnight(Some(proof)))).await
                }
                overnight::Gate::Hold(overnight::Hold::Catastrophic(_)) => match self.ap_overnight_risky()? {
                    OvernightRisky::Wait => {
                        s.verdict = Verdict::Ask;
                        s.reason = format!("Waited: {line}");
                        self.ap_wait(&subject, &s)
                    }
                    OvernightRisky::Deny => {
                        s.verdict = Verdict::Deny;
                        s.reason = format!("Denied {}: {line}", OVERNIGHT_NAME.to_lowercase());
                        Box::pin(self.ap_conclude(&subject, s, Verdict::Deny, By::Overnight(None))).await
                    }
                },
                overnight::Gate::Hold(overnight::Hold::Uncertain(why)) => {
                    // whatever the setting: nothing was read to deny it on
                    s.verdict = Verdict::Ask;
                    s.reason = format!("Waited: {why}");
                    self.ap_wait(&subject, &s)
                }
                overnight::Gate::Hold(overnight::Hold::Auto {
                    deny: true,
                    ..
                }) => Box::pin(self.ap_conclude(&subject, s, Verdict::Deny, By::Overnight(None))).await,
                overnight::Gate::Hold(overnight::Hold::Auto {
                    why,
                    deny: false,
                }) => {
                    s.reason = format!("{} — {why}", s.reason);
                    self.ap_wait(&subject, &s)
                }
                overnight::Gate::Hold(overnight::Hold::Paused(why)) => {
                    s.reason = format!("{OVERNIGHT_NAME} paused for this AI ({why}): {line}");
                    self.ap_pause_notice(&subject, now, why);
                    self.ap_wait(&subject, &s)
                }
            };
        }
        if let Some(p) = readable {
            // Manual and Auto show the risk; nothing is decided on it.
            s.risk = Some(RiskNote {
                level: risk::level(p, tripwire.is_some()).max(judged.risk_view_max.unwrap_or(RiskLevel::Benign)),
                probs: p,
                why: tripwire.clone().or_else(|| first_line.clone()).unwrap_or_default(),
                held: false,
                tripwire: tripwire.is_some(),
            });
        }
        s.judged = Some(judged);
        if mode == AutopilotMode::Overnight {
            // Without a verified model that rates risk: Auto, the hard floor included.
            if let Some(floor) = subject.floor.clone() {
                s.reason = format!("Always asks: {floor}");
                s.floor = Some(floor);
                return self.ap_wait(&subject, &s);
            }
            s.reason = format!("{OVERNIGHT_NAME} needs a model that rates risk; decided like Auto: {}", s.reason);
        }

        // Auto's rules: the model's verdict, the class gates, the rate limit.
        if matches!(mode, AutopilotMode::Auto | AutopilotMode::Overnight) && s.verdict != Verdict::Ask {
            let (stats, manual) = {
                let profile = self.ap_profile_loaded(ctx, &subject.connection_id)?;
                let refs: Vec<&MemoryRow> = profile.1.iter().collect();
                (gates::class_stats(&refs, &s.class_key), profile.0.profile.manual.get(&s.class_key).copied())
            };
            let (may_approve, may_deny) = gates::unlocked(&stats, manual.map(|locked| !locked));
            match s.verdict {
                Verdict::Approve if !may_approve => {
                    s.reason = format!(
                        "{} — Autopilot has not earned this kind of request yet ({} of {} decisions)",
                        s.reason,
                        stats.decisions.min(gates::APPROVE_UNLOCK_DECISIONS),
                        gates::APPROVE_UNLOCK_DECISIONS
                    );
                }
                Verdict::Approve => {
                    if self.ap_rate_ok(&subject.connection_id, now)? {
                        return Box::pin(self.ap_conclude(&subject, s, Verdict::Approve, By::Autopilot)).await;
                    }
                    s.reason = format!("{} — paused: unusual volume from this connection", s.reason);
                    self.ap_pause_notice(&subject, now, "unusual volume");
                }
                Verdict::Deny if may_deny => {
                    return Box::pin(self.ap_conclude(&subject, s, Verdict::Deny, By::Autopilot)).await;
                }
                Verdict::Deny => {
                    s.reason = format!("{} — auto-deny is not unlocked for this kind of request", s.reason);
                }
                Verdict::Ask => {}
            }
        }
        self.ap_wait(&subject, &s)
    }

    /// Bypass approves everything, every kind and every item of it, at once: that is what it is for. Only a new
    /// connection (a pairing) and another phone joining the account still reach the user, and neither comes here. A
    /// standing permission an AI asks for ends when the bypass does. What only describes the request (its profile, its
    /// target) is best effort here: none of it may keep the request waiting.
    async fn ap_bypass_item(&self, mut subject: Subject, now: i64) -> Result<(), CoreError> {
        let stored = self.ap_profile_of(&subject.connection_id).ok();
        let target_key =
            self.store.ap_target_key(&subject.connection_id, &subject.described.facts.target).unwrap_or_default();
        let mut facts = subject.described.facts.clone();
        facts.connection.clone_from(&subject.connection_label);
        facts.connection_history = self.store.ap_history(&subject.connection_id).ok();
        let (s_facts, s_full) = situation::render(&facts, &subject.described.ai);
        let s = Suggestion {
            request_id: subject.id.clone(),
            at: now,
            kind: match subject.kind {
                PendingKind::Blob => "blob",
                _ => "request",
            }
            .to_owned(),
            connection_id: subject.connection_id.clone(),
            connection_label: subject.connection_label.clone(),
            mode: AutopilotMode::Bypass,
            profile_id: stored.as_ref().map(|p| p.id.clone()).unwrap_or_default(),
            profile_name: stored.map(|p| p.profile.name).unwrap_or_default(),
            class_key: subject.described.class_key.clone(),
            label: subject.described.label.clone(),
            target_key,
            s_facts,
            s_full,
            novel: false,
            floor: None,
            judged: None,
            verdict: Verdict::Approve,
            reason: "Bypass: approved without asking".to_owned(),
            decided: None,
            risk: None,
            deletes: false,
        };
        subject.choice = subject.everything.clone();
        if subject.grant {
            // Marked as this bypass's, so that it ends when the bypass does, however it ends; a timed one is also
            // capped at its planned end.
            let (scope, cap) = self.ap_bypass_cap(&subject.connection_id, now)?;
            subject.choice.standing = cap;
            subject.grant_origin = Some(modes::bypass_origin(&scope));
        }
        self.ap_conclude(&subject, s, Verdict::Approve, By::Bypass).await
    }

    /// Runs the model on the two texts and scores them against the profile.
    /// `neutral`: the risk's third view (risk::neutral_facts), when it differs from the facts.
    async fn ap_judge(
        &self,
        s: &Suggestion,
        neutral: Option<&str>,
        profile: &mut (StoredProfile, Vec<MemoryRow>),
        now: i64,
    ) -> Result<(Judged, Probs), String> {
        let mut states = vec![s.s_facts.clone()];
        if s.s_full != s.s_facts {
            states.push(s.s_full.clone());
        }
        let full_at = states.len() - 1;
        if let Some(n) = neutral {
            states.push(n.to_owned());
        }
        let n_states = states.len();
        let (laya, passes) = self.ap_infer(states).await?;
        if passes.len() != n_states {
            return Err("the model returned another number of passes".to_owned());
        }
        let model_id = laya.id.clone();
        let risk_config = laya.config.risk();
        let facts = passes.first().cloned().ok_or("the model returned nothing")?;
        let full = passes.get(full_at).cloned().unwrap_or_else(|| facts.clone());
        // The neutral view's risk joins the two-pass rule; a view without a risk leaves none (held in Overnight).
        let neutral_risk = neutral.map(|_| passes.last().and_then(|p| p.risk));
        // The highest class any view gives on its own (the shown class is never below it).
        let risk_view_max = passes.iter().filter_map(|p| p.risk).map(|p| risk::level(p, false)).max();
        self.ap_train_if_due(profile, &model_id, now).await;
        let dim = facts.pooled.len();
        let rows: Vec<&MemoryRow> =
            profile.1.iter().filter(|r| r.model_id == model_id && r.e_facts.dim() == dim).collect();
        let class_examples = rows.iter().filter(|r| r.class_key == s.class_key).count();
        let adapter = profile.0.profile.adapter.as_ref().filter(|a| a.model_id == model_id && a.dim == dim);
        let subject = decide::Subject {
            class_key: &s.class_key,
            target_key: &s.target_key,
            novel: s.novel,
            class_examples,
        };
        let side = |pass: &Pass, v: Variant| decide::side(pass, &rows, v, adapter.map(|a| a.side(v)), subject);
        let (on_facts, on_full) = (side(&facts, Variant::Facts), side(&full, Variant::Full));
        let probs = decide::combine(on_facts.probs, on_full.probs);
        let neighbours = on_facts
            .knn
            .neighbours
            .iter()
            .take(SHOWN_NEIGHBOURS)
            .map(|(i, sim)| NeighbourNote {
                label: rows[*i].short_label.clone(),
                verdict: rows[*i].label,
                similarity: sim.clamp(0.0, 1.0),
                at: rows[*i].at,
            })
            .collect();
        Ok((
            Judged {
                model_id,
                probs: [probs.approve, probs.deny, probs.ask],
                confidence: probs.confidence(),
                escalate: facts.escalate.max(full.escalate),
                base_facts: facts.logits,
                base_full: full.logits,
                e_facts: Embedding::from_vector(&facts.pooled),
                e_full: Embedding::from_vector(&full.pooled),
                neighbours,
                risk: facts.risk.zip(full.risk).map(|(f, u)| decide::risk_combine(f, u)).and_then(
                    |r| match neutral_risk {
                        None => Some(r),
                        Some(n) => n.map(|n| decide::risk_combine(r, n)),
                    },
                ),
                risk_declared: risk_config.is_some(),
                risk_threshold: risk_config.and_then(model::RiskConfig::overnight_threshold),
                risk_view_max,
            },
            probs,
        ))
    }

    /// Retrains the profile's adapter when 5 decisions came in since the last time (or it is for another model).
    async fn ap_train_if_due(&self, profile: &mut (StoredProfile, Vec<MemoryRow>), model_id: &str, now: i64) {
        let p = &profile.0.profile;
        let stale = p.adapter.as_ref().is_some_and(|a| a.model_id != model_id);
        if !(p.since_train >= RETRAIN_EVERY || stale || (p.adapter.is_none() && p.since_train > 0)) {
            return;
        }
        let rows: Vec<MemoryRow> =
            profile.1.iter().filter(|r| r.model_id == model_id).take(adapter::TRAIN_WINDOW).cloned().collect();
        let id = model_id.to_owned();
        let trained = tokio::task::spawn_blocking(move || {
            let refs: Vec<&MemoryRow> = rows.iter().collect();
            adapter::train(&refs, &id, now)
        })
        .await
        .ok()
        .flatten();
        // Only the adapter changes; decisions remembered meanwhile still count toward the next training.
        let counted = profile.0.profile.since_train;
        profile.0.profile.adapter.clone_from(&trained);
        profile.0.profile.since_train = 0;
        let saved = self.ap_update_profile(&profile.0.id, |p| {
            p.adapter = trained;
            p.since_train = p.since_train.saturating_sub(counted);
        });
        if let Err(e) = saved {
            log::warn!("Autopilot could not save a trained profile: {e}");
        }
    }

    fn ap_rate_ok(&self, connection_id: &str, now: i64) -> Result<bool, CoreError> {
        Ok(self.store.ap_rate_count(connection_id, now - 600)? < RATE_PER_10_MIN
            && self.store.ap_rate_count(connection_id, now - 86_400)? < RATE_PER_DAY)
    }

    fn ap_pause_notice(&self, subject: &Subject, now: i64, reason: &str) {
        let mut paused = lock(&self.autopilot.paused);
        if paused.get(&subject.connection_id).is_some_and(|t| now - t < PAUSE_NOTICE_SECS) {
            return;
        }
        paused.insert(subject.connection_id.clone(), now);
        drop(paused);
        self.notifier.autopilot_changed(AutopilotEvent::Paused {
            connection_id: subject.connection_id.clone(),
            connection_label: subject.connection_label.clone(),
            reason: reason.to_owned(),
        });
    }

    /// Why Overnight leaves a connection's request to the user although its risk is not catastrophic: the rate limit
    /// (any request), or, for a severe one, something of the connection held or a burst of severe ones lately.
    fn ap_overnight_paused(
        &self,
        connection_id: &str,
        level: RiskLevel,
        now: i64,
    ) -> Result<Option<&'static str>, CoreError> {
        if self.store.ap_rate_count(connection_id, now - 600)? >= OVERNIGHT_RATE_PER_10_MIN
            || self.store.ap_rate_count(connection_id, now - 86_400)? >= OVERNIGHT_RATE_PER_DAY
        {
            return Ok(Some("unusual volume"));
        }
        if level < RiskLevel::Severe {
            return Ok(None);
        }
        let recent: Vec<Suggestion> = self
            .store
            .ap_suggestions_since(now - OVERNIGHT_PAUSE_SECS)?
            .into_iter()
            .filter(|x| x.connection_id == connection_id)
            .collect();
        if recent.iter().any(|x| x.risk.as_ref().is_some_and(|r| r.held)) {
            return Ok(Some("something of it was held"));
        }
        let severe = recent
            .iter()
            .filter(|x| {
                x.risk.as_ref().is_some_and(|r| r.level >= RiskLevel::Severe)
                    && x.decided.as_ref().is_some_and(|d| d.by == "overnight" && d.verdict == Verdict::Approve)
            })
            .count();
        Ok((severe >= OVERNIGHT_SEVERE_BURST).then_some("several severe requests"))
    }

    /// The connection's last hour, for a deleting or sending request ("this AI in the last hour: 38 requests, 21
    /// deletes, 2 held"); `None` when it is neither or the hour was empty.
    fn ap_recent_line(&self, subject: &Subject, deleting: bool, now: i64) -> Result<Option<String>, CoreError> {
        let sending = risk::sends(&subject.described);
        if !deleting && !sending {
            return Ok(None);
        }
        let recent: Vec<Suggestion> = self
            .store
            .ap_suggestions_since(now - 3_600)?
            .into_iter()
            .filter(|x| x.connection_id == subject.connection_id && x.request_id != subject.id)
            .collect();
        if recent.is_empty() {
            return Ok(None);
        }
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {many}")
            }
        };
        let mut parts = vec![plural(recent.len(), "request", "requests")];
        if deleting {
            // counted from what each request was read to do, never from its label
            let n = recent.iter().filter(|x| x.deletes).count();
            if n > 0 {
                parts.push(plural(n, "delete", "deletes"));
            }
        }
        if sending {
            let n = recent.iter().filter(|x| x.class_key.ends_with("/send")).count();
            if n > 0 {
                parts.push(plural(n, "message sent", "messages sent"));
            }
        }
        let held = recent.iter().filter(|x| x.risk.as_ref().is_some_and(|r| r.held)).count();
        if held > 0 {
            parts.push(format!("{held} held"));
        }
        Ok(Some(format!("this AI in the last hour: {}", parts.join(", "))))
    }

    /// Keeps the evaluation and tells the app the item waits for the user.
    fn ap_wait(&self, subject: &Subject, s: &Suggestion) -> Result<(), CoreError> {
        self.store.ap_put_suggestion(s)?;
        let mut item = subject.item.clone();
        item.suggestion = suggestion_line(s);
        self.notifier.item_pending(item);
        Ok(())
    }

    /// Decides automatically through the user's own approve/deny code; when that fails, the item waits.
    async fn ap_conclude(
        &self,
        subject: &Subject,
        mut s: Suggestion,
        verdict: Verdict,
        by: By,
    ) -> Result<(), CoreError> {
        if verdict == Verdict::Approve {
            let allowed = match &by {
                By::Bypass => true,
                By::Lockdown | By::Overnight(None) => false,
                // Overnight's gate proved it, for this very request
                By::Overnight(Some(proof)) => proof.request_id() == subject.id,
                // Overnight with a model that rates risk approves only through its gate
                By::Autopilot => {
                    !(s.mode == AutopilotMode::Overnight && s.judged.as_ref().is_some_and(|j| j.risk_declared))
                }
            };
            if !allowed {
                log::error!("Autopilot refused an approval it had no right to make ({}); it waits", by.name());
                s.verdict = Verdict::Ask;
                "Autopilot could not approve this on its own; it waits for you".clone_into(&mut s.reason);
                return self.ap_wait(subject, &s);
            }
        }
        let by = by.name();
        let note = note_of(&s, verdict);
        let decision = Decision::new(by, Some(note)).granting_as(subject.grant_origin.clone());
        let (result, seq) = deciding(decision, self.ap_execute(subject, verdict)).await;
        let now = unix_now();
        match result {
            Ok(()) => {
                s.decided = Some(Decided {
                    by: by.to_owned(),
                    verdict,
                    at: now,
                });
                self.store.ap_put_suggestion(&s)?;
                if matches!(by, "autopilot" | "overnight") && verdict == Verdict::Approve {
                    self.store.ap_rate_add(&subject.connection_id, now)?;
                }
                let j = s.judged.as_ref();
                self.notifier.auto_decided(AutoDecisionView {
                    request_id: subject.id.clone(),
                    kind: subject.kind,
                    connection_id: subject.connection_id.clone(),
                    connection_label: subject.connection_label.clone(),
                    title: s.label.clone(),
                    verdict,
                    decided_by: by.to_owned(),
                    p_approve: j.map_or(0.0, |j| j.probs[0]),
                    confidence: j.map_or(0.0, |j| j.confidence),
                    activity_id: seq,
                });
                Ok(())
            }
            Err(e) if by == "bypass" => {
                // Not judged: the next pass (a sync, a push) tries again. A bypass never leaves it waiting for the
                // user, nor says it waits.
                log::warn!("Bypass could not approve a request yet ({e}); it is tried again");
                Ok(())
            }
            Err(e) => {
                log::warn!("Autopilot's decision could not be carried out ({e}); the request waits for the user");
                s.reason =
                    format!("Autopilot tried to {} but it failed ({e}); it waits for you", verb_infinitive(verdict));
                if self.store.pending_item(&subject.id, now)?.is_some() {
                    self.ap_wait(subject, &s)
                } else {
                    self.store.ap_put_suggestion(&s)
                }
            }
        }
    }

    /// The bypass in force for a connection (its own, else the global one, as `modes::effective` reads them) and the
    /// cap of a permission granted during it: until its planned end (one without an end: as asked; it is taken back
    /// when stopped).
    fn ap_bypass_cap(&self, connection_id: &str, now: i64) -> Result<(String, Option<StandingGrant>), CoreError> {
        let own = self.store.ap_mode_row(connection_id)?;
        let (scope, row) = if own.bypassing(now) {
            (connection_id.to_owned(), own)
        } else {
            (String::new(), self.store.ap_mode_row("")?)
        };
        let end = row.bypass_until.filter(|_| row.bypassing(now));
        Ok((
            scope,
            end.filter(|e| *e != modes::BYPASS_UNTIL_OFF).map(|end| StandingGrant {
                duration_secs: Some(u64::try_from((end - now).max(1)).unwrap_or(1)),
                max_uses: None,
                scope: GrantScopeChoice {
                    all_mail: false,
                    selected_messages_only: false,
                    sender_addresses: Vec::new(),
                    sender_domains: Vec::new(),
                    subject_pattern: None,
                    recipient_addresses: Vec::new(),
                    recipient_domains: Vec::new(),
                    resources: Vec::new(),
                    classes: Vec::new(),
                    secrets: None,
                },
            }),
        ))
    }

    /// The Overnight in force for a connection: its own (the connection's id), else the global one (`""`).
    fn ap_overnight_scope(&self, connection_id: &str, now: i64) -> Result<String, CoreError> {
        Ok(if self.store.ap_mode_row(connection_id)?.overnighting(now) {
            connection_id.to_owned()
        } else {
            String::new()
        })
    }

    /// Writes a mode row; in the same transaction, what a bypass granted is taken back once that bypass is no longer
    /// in force (see [`crate::autopilot::store`]), and each such permission gets its Activity entry.
    fn ap_write_mode(&self, scope: &str, row: &modes::ModeRow, now: i64) -> Result<(), CoreError> {
        let revoked = self.store.ap_set_mode_row(scope, row, now)?;
        self.ap_note_ended(&revoked);
        Ok(())
    }

    /// Takes back any permission whose bypass is no longer in force (idempotent; next to the bypasses' own expiry).
    /// Taking back is only cleanup: such a permission already stopped counting when its bypass ended.
    fn ap_sweep_bypass_grants(&self, now: i64) {
        match self.store.ap_sweep_bypass_grants(now) {
            Ok(revoked) => self.ap_note_ended(&revoked),
            Err(e) => log::warn!("Autopilot could not take back what an ended bypass granted: {e}"),
        }
    }

    /// "Ended with the bypass" in Activity, once for each permission just taken back. A failure is logged and the
    /// rest are still written.
    fn ap_note_ended(&self, revoked: &[crate::store::StoredGrant]) {
        let now = unix_now();
        for stored in revoked {
            let view = views::grant_view(stored);
            let record = AuditRecord {
                seq: 0,
                at: now,
                connection_id: view.connection_id,
                connection_label: view.connection_label,
                action: "grant".to_owned(),
                outcome: "revoked".to_owned(),
                detail: "Ended with the bypass".to_owned(),
                grant_id: Some(view.id),
                service: view.service,
                account: view.account,
                count: 1,
                info: crate::store::AuditInfo {
                    grant_summary: Some(view.summary),
                    note: Some("Ended with the bypass".to_owned()),
                    ..crate::store::AuditInfo::default()
                },
                op: String::new(),
            };
            if let Err(e) = self.store.append_audit(&record) {
                log::warn!("Autopilot could not note a permission that ended with its bypass: {e}");
            }
        }
    }

    async fn ap_execute(&self, subject: &Subject, verdict: Verdict) -> Result<(), CoreError> {
        if subject.purchase && verdict == Verdict::Approve {
            // With what its screen would pick; the budgets still apply.
            let session = self.session()?;
            let (_, parked) = self.parked(&subject.id)?;
            let defaults = crate::connector::payments::PurchaseChoice {
                method_id: None,
                address_id: None,
                limit: None,
            };
            return Box::pin(self.approve_parked_purchase(&session, &subject.id, &parked, Some(&defaults))).await;
        }
        // Each answer is boxed: an approval's future holds every integration's write (secrets stores add accounts),
        // and inlined into Autopilot's own futures it outgrows a test thread's stack in debug builds.
        match (subject.kind, verdict) {
            (PendingKind::Request, Verdict::Approve) => {
                Box::pin(self.approve_claimed(&subject.id, subject.choice.clone())).await
            }
            (PendingKind::Request, _) => Box::pin(self.deny_claimed(&subject.id)).await,
            (PendingKind::Blob, v) => Box::pin(self.answer_blob_claimed(&subject.id, v == Verdict::Approve)).await,
            (PendingKind::Pairing | PendingKind::Join, _) => {
                Err(CoreError::invalid("a pairing or another phone is never decided automatically"))
            }
        }
    }

    /// Ends the bypasses that ran out, then takes back what any ended bypass granted. One that cannot be written is
    /// logged and the others still end (what it granted stopped counting at its end either way).
    pub(crate) fn ap_expire_bypasses(&self, now: i64) -> Result<(), CoreError> {
        for (scope, mut row) in self.store.ap_mode_rows()? {
            let bypass_ended = row.bypass_until.is_some_and(|t| t <= now);
            let overnight_ended = row.overnight_ended(now);
            if !bypass_ended && !overnight_ended {
                continue;
            }
            if bypass_ended {
                row.bypass_until = None;
                row.bypass_since = None;
            }
            if overnight_ended {
                // back to the mode it interrupted
                row.mode = row.base(now);
                row.overnight_until = None;
                row.overnight_since = None;
                row.overnight_return = None;
            }
            if let Err(e) = self.ap_write_mode(&scope, &row, now) {
                log::warn!("Autopilot could not end a bypass or an Overnight that ran out: {e}");
                continue;
            }
            let connection_id = (!scope.is_empty()).then_some(scope);
            if bypass_ended {
                self.notifier.autopilot_changed(AutopilotEvent::BypassEnded {
                    connection_id: connection_id.clone(),
                });
            }
            if overnight_ended {
                self.notifier.autopilot_changed(AutopilotEvent::OvernightEnded {
                    connection_id,
                });
            }
        }
        self.ap_sweep_bypass_grants(now);
        Ok(())
    }

    // ---- learning from the user ------------------------------------------------------------------------------------

    /// The note the user's own decision on a request carries: what Autopilot suggested, or why it could not.
    pub(crate) fn ap_human_note(&self, request_id: &str) -> Option<AuditAutopilot> {
        let s = self.store.ap_suggestion(request_id).ok().flatten()?;
        let looked = s.judged.is_some()
            || s.floor.is_some()
            || matches!(
                s.mode,
                AutopilotMode::Manual | AutopilotMode::Auto | AutopilotMode::Overnight | AutopilotMode::Bypass
            );
        looked.then(|| note_of(&s, s.verdict))
    }

    /// The user approved or denied an evaluated request: remember it (never an automatic decision).
    pub(crate) fn ap_learn(&self, request_id: &str, verdict: Verdict) {
        if let Err(e) = self.ap_learn_inner(request_id, verdict) {
            log::warn!("Autopilot could not remember a decision: {e}");
        }
    }

    fn ap_learn_inner(&self, request_id: &str, verdict: Verdict) -> Result<(), CoreError> {
        let Some(mut s) = self.store.ap_suggestion(request_id)? else {
            return Ok(());
        };
        if s.decided.is_some() {
            return Ok(());
        }
        let now = unix_now();
        s.decided = Some(Decided {
            by: String::new(),
            verdict,
            at: now,
        });
        if verdict == Verdict::Approve {
            self.store.ap_target_add(&s.target_key, now)?;
        }
        if let Some(j) = &s.judged {
            let row = memory_row(&s, j, verdict, Source::User, 1.0, s.verdict, now);
            self.ap_remember(&s, &row)?;
        }
        self.store.ap_put_suggestion(&s)
    }

    fn ap_remember(&self, s: &Suggestion, row: &MemoryRow) -> Result<(), CoreError> {
        let profile = match self.store.ap_profiles()?.into_iter().find(|p| p.id == s.profile_id) {
            Some(p) => p,
            None => self.ap_profile_of(&s.connection_id)?,
        };
        self.store.ap_add_memory(&profile.id, row)?;
        self.ap_update_profile(&profile.id, |p| p.since_train += 1)
    }

    /// "This was wrong" on an activity entry: remembered with weight 3; an automatic decision gone wrong re-locks
    /// its class.
    pub async fn correct_decision(&self, activity_id: i64, should_have: Verdict) -> Result<(), CoreError> {
        if should_have == Verdict::Ask {
            return Err(CoreError::invalid("say whether it should have been approved or denied"));
        }
        let entry = self.store.audit_entry(activity_id)?.ok_or(CoreError::NotFound)?;
        let note =
            entry.info.autopilot.clone().ok_or_else(|| CoreError::invalid("Autopilot did not see this request"))?;
        if entry.info.decided_by == "lockdown" {
            return Err(CoreError::invalid("Lockdown denies everything; there is nothing to learn from it"));
        }
        let mut s = self
            .store
            .ap_suggestion(&note.request_id)?
            .ok_or_else(|| CoreError::invalid("this is too old to correct"))?;
        let decided = s.decided.clone().ok_or_else(|| CoreError::invalid("this request was not decided yet"))?;
        if decided.verdict == should_have {
            return Err(CoreError::invalid("that is what was done"));
        }
        let now = unix_now();
        let judged = match s.judged.clone() {
            Some(j) => j,
            None if !s.s_facts.is_empty() => {
                // Bypass decided it without the model: it reads it now.
                let mut profile = {
                    let p = self.ap_profile_of(&s.connection_id)?;
                    let memory = self.store.ap_memory(&p.id)?;
                    (p, memory)
                };
                self.ap_judge(&s, None, &mut profile, now).await.map_err(CoreError::service)?.0
            }
            None => return Err(CoreError::invalid("Autopilot kept nothing to learn from for this request")),
        };
        let automatic = decided.by == "autopilot";
        let would = if automatic {
            decided.verdict
        } else {
            s.verdict
        };
        let row = memory_row(&s, &judged, should_have, Source::Correction, CORRECTION_WEIGHT, would, now);
        self.ap_remember(&s, &row)?;
        if automatic {
            // Back to the numbers, which now hold a wrong automatic answer: the class is locked.
            match self.ap_update_profile(&s.profile_id, |p| {
                p.manual.remove(&s.class_key);
            }) {
                Ok(()) | Err(CoreError::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        if should_have == Verdict::Deny {
            // A target the user did not want is no longer "seen approved".
            self.store.ap_target_forget(&s.target_key)?;
        } else {
            self.store.ap_target_add(&s.target_key, now)?;
        }
        s.judged = Some(judged);
        self.store.ap_put_suggestion(&s)
    }

    // ---- the API ---------------------------------------------------------------------------------------------------

    pub fn model_status(&self) -> ModelStatus {
        let runtime = lock(&self.autopilot.runtime).is_some();
        let installed = self.ap_installed();
        let mut status = self.autopilot.download.status(&self.ap_known(), installed, runtime);
        status.risk = installed && model::rates_risk(&self.data_dir, &self.ap_known());
        status
    }

    /// What Overnight does with what is too risky (the user's setting).
    fn ap_overnight_risky(&self) -> Result<OvernightRisky, CoreError> {
        Ok(self
            .store
            .meta_get(super::store::OVERNIGHT_RISKY_KEY)?
            .as_deref()
            .and_then(OvernightRisky::parse)
            .unwrap_or_default())
    }

    /// A change either way asks the owner first.
    pub async fn set_overnight_when_risky(
        &self,
        risky: OvernightRisky,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        if self.ap_overnight_risky()? == risky {
            return Ok(());
        }
        let reason = match risky {
            OvernightRisky::Wait => format!("{OVERNIGHT_NAME}: wait for you when something is too risky"),
            OvernightRisky::Deny => format!("{OVERNIGHT_NAME}: deny what is too risky"),
        };
        confirm_owner(owner, reason).await?;
        self.store.meta_set(super::store::OVERNIGHT_RISKY_KEY, risky.as_str())
    }

    /// Downloads and verifies the model; on any failure nothing is kept.
    pub async fn download_model(&self, progress: Arc<dyn DownloadProgress>) -> Result<ModelStatus, CoreError> {
        let state = &self.autopilot.download;
        if state.running.swap(true, Ordering::SeqCst) {
            return Err(CoreError::invalid("the model is already being downloaded"));
        }
        *lock(&state.error) = None;
        state.done.store(0, Ordering::SeqCst);
        self.ap_unload();
        let known = self.ap_known();
        let result =
            model::download(&self.http, &self.cfg.models_base, &self.data_dir, &known, progress.as_ref(), state).await;
        if let Err(e) = &result {
            *lock(&state.error) = Some(e.to_string());
        }
        state.running.store(false, Ordering::SeqCst);
        result.map(|()| self.model_status())
    }

    pub fn delete_model(&self) -> Result<(), CoreError> {
        if self.autopilot.download.running.load(Ordering::SeqCst) {
            return Err(CoreError::invalid("the model is being downloaded"));
        }
        self.ap_unload();
        *lock(&self.autopilot.download.error) = None;
        model::remove(&self.data_dir, &self.ap_known().id)
    }

    pub fn set_autopilot_wifi_only(&self, wifi_only: bool) -> Result<(), CoreError> {
        self.store.meta_set(
            WIFI_ONLY_KEY,
            if wifi_only {
                "1"
            } else {
                "0"
            },
        )
    }

    pub fn autopilot_settings(&self) -> Result<AutopilotSettings, CoreError> {
        let now = unix_now();
        self.ap_expire_bypasses(now)?;
        let profiles = self.ap_profiles()?;
        let default_profile_id = self.ap_default_profile_id(&profiles)?;
        let global = self.store.ap_mode_row("")?;
        let connections = self
            .store
            .ap_mode_rows()?
            .into_iter()
            .filter(|(scope, _)| !scope.is_empty())
            .map(|(scope, row)| ConnectionAutopilot {
                mode: modes::effective(&global, Some(&row), now),
                base_mode: row.base(now),
                bypass_until: row.bypass_until.filter(|t| *t > now),
                bypass_since: row.bypass_since.filter(|_| row.bypassing(now)),
                overnight_until: row.overnighting(now).then(|| row.overnight_until.unwrap_or(modes::BYPASS_UNTIL_OFF)),
                overnight_since: row.overnight_since.filter(|_| row.overnighting(now)),
                profile_id: row
                    .profile_id
                    .clone()
                    .filter(|id| profiles.iter().any(|p| &p.id == id))
                    .unwrap_or_else(|| default_profile_id.clone()),
                connection_id: scope,
            })
            .collect();
        Ok(AutopilotSettings {
            mode: modes::global_mode(&global, now),
            base_mode: modes::global_base(&global, now),
            bypass_until: global.bypass_until.filter(|t| *t > now),
            bypass_since: global.bypass_since.filter(|_| global.bypassing(now)),
            overnight_until: global
                .overnighting(now)
                .then(|| global.overnight_until.unwrap_or(modes::BYPASS_UNTIL_OFF)),
            overnight_since: global.overnight_since.filter(|_| global.overnighting(now)),
            overnight_when_risky: self.ap_overnight_risky()?,
            default_profile_id,
            wifi_only: self.store.meta_get(WIFI_ONLY_KEY)?.is_none_or(|v| v != "0"),
            model: self.model_status(),
            connections,
        })
    }

    /// Sets the global mode (`connection_id` = `None`) or a connection's; `mode` = `None` returns to the default
    /// (global) or to following the global mode (connection). A bypass lasts `minutes` (15 by default, a year at
    /// most; [`Self::start_bypass`] also starts one without an end); Lockdown also denies what is waiting.
    pub async fn set_autopilot_mode(
        &self,
        connection_id: Option<String>,
        mode: Option<AutopilotMode>,
        minutes: Option<u32>,
    ) -> Result<(), CoreError> {
        if mode == Some(AutopilotMode::Bypass) {
            return self.start_bypass(connection_id, Some(minutes.unwrap_or(DEFAULT_BYPASS_MINUTES))).await;
        }
        if mode == Some(AutopilotMode::Overnight) {
            return Err(CoreError::invalid(format!(
                "{OVERNIGHT_NAME} starts with start_overnight, after the owner check"
            )));
        }
        if let Some(id) = &connection_id {
            check_id(id)?;
        }
        let now = unix_now();
        let scope = connection_id.clone().unwrap_or_default();
        let mut row = self.store.ap_mode_row(&scope)?;
        if let Some(id) = &connection_id {
            // An AI kept on Manual under a global Overnight that this change would put in it: that is starting
            // Overnight for it, which only start_overnight does (after the owner check).
            let global = self.store.ap_mode_row("")?;
            let mut after = row.clone();
            after.mode = mode;
            after.bypass_until = None;
            after.overnight_until = None;
            if self.ap_mode_for(id, now)? != AutopilotMode::Overnight
                && modes::effective(&global, Some(&after), now) == AutopilotMode::Overnight
            {
                return Err(CoreError::invalid(format!(
                    "this would put the AI in {OVERNIGHT_NAME}: start it with start_overnight, after the owner check"
                )));
            }
        }
        row.mode = mode;
        row.bypass_until = None;
        row.bypass_since = None;
        row.overnight_until = None;
        row.overnight_since = None;
        row.overnight_return = None;
        // Whatever ended a bypass or an Overnight (stopped, another mode, a lockdown), what it granted is taken back in
        // the same transaction.
        self.ap_write_mode(&scope, &row, now)?;
        self.notifier.autopilot_changed(AutopilotEvent::ModeChanged {
            connection_id: connection_id.clone(),
        });
        if mode == Some(AutopilotMode::Lockdown) {
            self.ap_lockdown_sweep(connection_id.as_deref()).await;
        }
        Ok(())
    }

    /// Starts a bypass of every AI (`connection_id` = `None`) or of one: for `minutes` (1 minute to a year), or until
    /// the user turns it off (`None`). It ends a lockdown of the same scope and approves what already waits; a
    /// connection paired less than 10 minutes ago cannot be bypassed.
    pub async fn start_bypass(&self, connection_id: Option<String>, minutes: Option<u32>) -> Result<(), CoreError> {
        if let Some(id) = &connection_id {
            check_id(id)?;
        }
        let now = unix_now();
        let until = modes::bypass_end(minutes, now).map_err(CoreError::invalid)?;
        if let Some(id) = &connection_id {
            let created = self.connections().await?.into_iter().find(|c| &c.id == id).map(|c| c.created_at);
            let paired = self.store.ap_paired_at(id)?;
            let since = created.into_iter().chain(paired).max().ok_or(CoreError::NotFound)?;
            if now - since < self.cfg.new_connection_secs {
                return Err(CoreError::invalid("a connection paired less than 10 minutes ago cannot be put in bypass"));
            }
        }
        let scope = connection_id.clone().unwrap_or_default();
        let mut row = self.store.ap_mode_row(&scope)?;
        if row.mode == Some(AutopilotMode::Lockdown) {
            row.mode = None;
        }
        if row.mode == Some(AutopilotMode::Overnight) {
            // A bypass ends the Overnight of its scope: when the bypass ends, the mode it returns to is the one
            // Overnight interrupted, never Overnight again without the owner.
            row.mode = row.overnight_return.take();
            row.overnight_until = None;
            row.overnight_since = None;
        }
        row.bypass_until = Some(until);
        row.bypass_since = Some(now);
        self.ap_write_mode(&scope, &row, now)?;
        self.notifier.autopilot_changed(AutopilotEvent::ModeChanged {
            connection_id,
        });
        self.ap_sweep_into(AutopilotMode::Bypass, now).await;
        Ok(())
    }

    /// Starts Overnight for every AI (`connection_id` = `None`) or one: for `minutes` (1 minute to a year), or until
    /// the user turns it off (`None`). It returns to the mode it interrupted when it ends (a lockdown it ends is not
    /// returned to), and looks again at what already waits in its scope.
    /// Every start, restart or change of its length asks the owner first; without the owner's confirmation nothing
    /// changes.
    pub async fn start_overnight(
        &self,
        connection_id: Option<String>,
        minutes: Option<u32>,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        if let Some(id) = &connection_id {
            check_id(id)?;
        }
        modes::overnight_end(minutes, unix_now()).map_err(CoreError::invalid)?;
        let reason = match minutes {
            Some(m) if m % 60 == 0 => format!("Start {OVERNIGHT_NAME} for {} h", m / 60),
            Some(m) => format!("Start {OVERNIGHT_NAME} for {m} min"),
            None => format!("Start {OVERNIGHT_NAME} until you turn it off"),
        };
        confirm_owner(owner, reason).await?;
        let now = unix_now();
        let until = modes::overnight_end(minutes, now).map_err(CoreError::invalid)?;
        let scope = connection_id.clone().unwrap_or_default();
        let mut row = self.store.ap_mode_row(&scope)?;
        let back = match row.mode {
            Some(AutopilotMode::Lockdown) => None,
            Some(AutopilotMode::Overnight) => row.overnight_return,
            other => other,
        };
        row.mode = Some(AutopilotMode::Overnight);
        row.overnight_until = Some(until);
        row.overnight_since = Some(now);
        row.overnight_return = back;
        self.ap_write_mode(&scope, &row, now)?;
        self.notifier.autopilot_changed(AutopilotEvent::ModeChanged {
            connection_id,
        });
        self.ap_sweep_into(AutopilotMode::Overnight, now).await;
        Ok(())
    }

    /// A bypass approves what already waits in its scope (as Lockdown denies it), and Overnight judges it: those
    /// requests are looked at again by a pass. Pairings and other phones stay with the user.
    async fn ap_sweep_into(&self, into: AutopilotMode, now: i64) {
        let Ok(rows) = self.store.pending_rows(now) else {
            return;
        };
        let mut again = false;
        for row in rows {
            let Ok(Some(subject)) = Self::ap_subject(&row) else {
                continue;
            };
            if self.ap_mode_for(&subject.connection_id, now).ok() != Some(into) {
                continue;
            }
            // Judged before (it waits with a suggestion): judged again. Never judged (parked where the model could
            // not run): the pass below takes it too.
            match self.store.ap_suggestion(&row.id) {
                Ok(Some(s)) if s.decided.is_none() => again |= self.store.ap_forget_suggestion(&row.id).is_ok(),
                Ok(None) => again = true,
                _ => {}
            }
        }
        if again {
            self.autopilot_pass().await;
        }
    }

    /// Denies what waits (except pairings) for the connection, or for all.
    async fn ap_lockdown_sweep(&self, connection_id: Option<&str>) {
        let Ok(rows) = self.store.pending_rows(unix_now()) else {
            return;
        };
        for row in rows {
            let Ok(Some(subject)) = Self::ap_subject(&row) else {
                continue;
            };
            if connection_id.is_some_and(|c| c != subject.connection_id) {
                continue;
            }
            let Some(_claim) = self.claim(&row.id) else {
                continue;
            };
            let base = self.store.ap_suggestion(&row.id).ok().flatten();
            let mut s = base.unwrap_or_else(|| Suggestion {
                request_id: subject.id.clone(),
                at: unix_now(),
                kind: "request".to_owned(),
                connection_id: subject.connection_id.clone(),
                connection_label: subject.connection_label.clone(),
                mode: AutopilotMode::Lockdown,
                profile_id: String::new(),
                profile_name: String::new(),
                class_key: subject.described.class_key.clone(),
                label: subject.described.label.clone(),
                target_key: String::new(),
                s_facts: String::new(),
                s_full: String::new(),
                novel: true,
                floor: None,
                judged: None,
                verdict: Verdict::Deny,
                reason: String::new(),
                decided: None,
                risk: None,
                deletes: false,
            });
            s.mode = AutopilotMode::Lockdown;
            "Lockdown: every request is denied".clone_into(&mut s.reason);
            if let Err(e) = self.ap_conclude(&subject, s, Verdict::Deny, By::Lockdown).await {
                log::warn!("Lockdown could not deny a waiting request: {e}");
            }
        }
    }

    pub fn autopilot_suggestion(&self, request_id: &str) -> Result<Option<SuggestionView>, CoreError> {
        Ok(self.store.ap_suggestion(request_id)?.map(|s| suggestion_view(&s)))
    }

    /// The "Try it" playground: a typed situation judged by a profile (nothing is kept).
    pub async fn autopilot_evaluate(
        &self,
        profile_id: Option<String>,
        situation_text: String,
    ) -> Result<SuggestionView, CoreError> {
        let (s_facts, s_full) = situation::split(&situation_text);
        if s_facts.is_empty() {
            return Err(CoreError::invalid("describe a request first"));
        }
        let profiles = self.ap_profiles()?;
        let id = match profile_id {
            Some(id) => id,
            None => self.ap_default_profile_id(&profiles)?,
        };
        let stored = profiles.into_iter().find(|p| p.id == id).ok_or(CoreError::NotFound)?;
        let (class_key, novel) = class_of_text(&s_facts);
        let thresholds = stored.profile.thresholds();
        let s = Suggestion {
            deletes: false,
            request_id: String::new(),
            at: unix_now(),
            kind: "playground".to_owned(),
            connection_id: String::new(),
            connection_label: String::new(),
            mode: AutopilotMode::Manual,
            profile_id: stored.id.clone(),
            profile_name: stored.profile.name.clone(),
            class_key,
            label: String::new(),
            target_key: String::new(),
            s_facts,
            s_full,
            novel,
            floor: None,
            judged: None,
            verdict: Verdict::Ask,
            reason: String::new(),
            decided: None,
            risk: None,
        };
        let memory = self.store.ap_memory(&stored.id)?;
        let mut profile = (stored, memory);
        let (judged, probs) = self.ap_judge(&s, None, &mut profile, s.at).await.map_err(CoreError::service)?;
        let verdict = decide::would(probs, s.novel, judged.escalate, thresholds);
        let done = Suggestion {
            verdict,
            reason: reason_of(&judged, verdict, s.novel),
            judged: Some(judged),
            ..s
        };
        Ok(suggestion_view(&done))
    }

    pub fn autopilot_profiles(&self) -> Result<Vec<ProfileView>, CoreError> {
        let profiles = self.ap_profiles()?;
        let default_id = self.ap_default_profile_id(&profiles)?;
        let assigned = self.store.ap_mode_rows()?;
        let mut views = Vec::with_capacity(profiles.len());
        for p in profiles {
            let memory = self.store.ap_memory(&p.id)?;
            let refs: Vec<&MemoryRow> = memory.iter().collect();
            let mut keys: Vec<String> = Vec::new();
            for k in memory.iter().map(|r| r.class_key.clone()).chain(p.profile.manual.keys().cloned()) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            let mut classes: Vec<ClassView> = keys
                .into_iter()
                .map(|key| {
                    let stats = gates::class_stats(&refs, &key);
                    let lock = p.profile.manual.get(&key).copied();
                    let (auto_approve, auto_deny) = gates::unlocked(&stats, lock.map(|locked| !locked));
                    ClassView {
                        label: class_label(self, &key),
                        decisions: u32::try_from(stats.decisions).unwrap_or(u32::MAX),
                        approved: u32::try_from(stats.approved).unwrap_or(u32::MAX),
                        denied: u32::try_from(stats.denied).unwrap_or(u32::MAX),
                        shadow_accuracy: stats.accuracy(),
                        auto_approve,
                        auto_deny,
                        manual: lock,
                        decisions_to_unlock: u32::try_from(
                            gates::APPROVE_UNLOCK_DECISIONS.saturating_sub(stats.decisions),
                        )
                        .unwrap_or(0),
                        class_key: key,
                    }
                })
                .collect();
            classes.sort_by(|a, b| b.decisions.cmp(&a.decisions).then(a.class_key.cmp(&b.class_key)));
            let (approve_at, deny_at) = p.profile.thresholds();
            let preset = p.profile.shown_preset();
            views.push(ProfileView {
                preset,
                is_default: p.id == default_id,
                approve_at,
                deny_at,
                agreement: stats::agreement(&memory).0,
                memory_count: u32::try_from(memory.len()).unwrap_or(u32::MAX),
                connections: assigned
                    .iter()
                    .filter(|(scope, row)| !scope.is_empty() && row.profile_id.as_deref() == Some(p.id.as_str()))
                    .map(|(scope, _)| scope.clone())
                    .collect(),
                classes,
                trained_at: p.profile.adapter.as_ref().map(|a| a.trained_at),
                name: p.profile.name,
                icon: p.profile.icon,
                id: p.id,
            });
        }
        Ok(views)
    }

    fn ap_stored_profile(&self, id: &str) -> Result<StoredProfile, CoreError> {
        self.ap_profiles()?.into_iter().find(|p| p.id == id).ok_or(CoreError::NotFound)
    }

    fn ap_clean_name(name: &str) -> Result<String, CoreError> {
        let name = crate::text::truncate_chars(&crate::text::one_line(name), 40);
        if name.is_empty() {
            return Err(CoreError::invalid("give the profile a name"));
        }
        Ok(name)
    }

    fn ap_clean_icon(icon: Option<String>) -> Option<String> {
        icon.map(|i| crate::text::truncate_chars(&crate::text::one_line(&i), 32)).filter(|i| !i.is_empty())
    }

    pub fn create_profile(&self, name: &str, icon: Option<String>) -> Result<ProfileView, CoreError> {
        // Strictly after the others, so profiles keep the order they were made in.
        let at = self.ap_profiles()?.iter().map(|p| p.created_at + 1).max().unwrap_or(0).max(unix_now());
        let id = uuid::Uuid::new_v4().to_string();
        let profile = Profile {
            name: Self::ap_clean_name(name)?,
            icon: Self::ap_clean_icon(icon),
            ..Profile::default()
        };
        self.store.ap_put_profile(&id, at, &profile)?;
        self.autopilot_profiles()?.into_iter().find(|p| p.id == id).ok_or(CoreError::NotFound)
    }

    pub fn rename_profile(&self, id: &str, name: &str, icon: Option<String>) -> Result<(), CoreError> {
        let name = Self::ap_clean_name(name)?;
        let icon = Self::ap_clean_icon(icon);
        self.ap_profiles()?;
        self.ap_update_profile(id, |p| {
            p.name = name;
            p.icon = icon;
        })
    }

    /// Deletes a profile and what it learned; its connections fall back to the default profile.
    pub fn delete_profile(&self, id: &str) -> Result<(), CoreError> {
        let profiles = self.ap_profiles()?;
        if !profiles.iter().any(|p| p.id == id) {
            return Err(CoreError::NotFound);
        }
        if profiles.len() == 1 {
            return Err(CoreError::invalid("the last profile cannot be deleted"));
        }
        let was_default = self.ap_default_profile_id(&profiles)? == id;
        self.store.ap_delete_profile(id)?;
        if was_default && let Some(next) = profiles.iter().find(|p| p.id != id) {
            self.set_default_profile(&next.id)?;
        }
        Ok(())
    }

    /// Forgets everything a profile learned (memory, adapter, classes unlocked or locked by hand).
    pub fn reset_profile(&self, id: &str) -> Result<(), CoreError> {
        self.ap_stored_profile(id)?;
        self.store.ap_clear_memory(id)?;
        self.ap_update_profile(id, |p| {
            p.adapter = None;
            p.since_train = 0;
            p.manual.clear();
        })
    }

    pub fn set_default_profile(&self, id: &str) -> Result<(), CoreError> {
        self.ap_stored_profile(id)?;
        let mut global = self.store.ap_mode_row("")?;
        global.profile_id = Some(id.to_owned());
        self.ap_write_mode("", &global, unix_now())
    }

    /// The profile a connection's decisions train (`None` = the default profile).
    pub fn assign_profile(&self, connection_id: &str, profile_id: Option<String>) -> Result<(), CoreError> {
        check_id(connection_id)?;
        if let Some(id) = &profile_id {
            self.ap_stored_profile(id)?;
        }
        let mut row = self.store.ap_mode_row(connection_id)?;
        row.profile_id = profile_id;
        self.ap_write_mode(connection_id, &row, unix_now())
    }

    /// Locks (`Some(true)`) or unlocks (`Some(false)`) a class by hand, or returns it to the numbers (`None`).
    pub fn set_class_lock(&self, profile_id: &str, class_key: &str, locked: Option<bool>) -> Result<(), CoreError> {
        let key = class_key.trim().to_lowercase();
        if key.is_empty() || key.len() > 200 {
            return Err(CoreError::invalid("not a class"));
        }
        self.ap_profiles()?;
        self.ap_update_profile(profile_id, |p| {
            match locked {
                Some(l) => p.manual.insert(key, l),
                None => p.manual.remove(&key),
            };
        })
    }

    /// A named preset (its thresholds replace the user's own), or `Custom`: the user's own thresholds, starting
    /// from those in use.
    pub fn set_preset(&self, profile_id: &str, preset: Preset) -> Result<(), CoreError> {
        self.ap_profiles()?;
        self.ap_update_profile(profile_id, |p| {
            if preset == Preset::Custom {
                if p.custom.is_none() {
                    let (approve, deny) = p.thresholds();
                    p.custom = Some(Thresholds {
                        approve,
                        deny,
                    });
                }
            } else {
                p.preset = preset;
                p.custom = None;
            }
        })
    }

    /// The profile's own thresholds (each clamped to 0.80–0.99): Auto approves at or above `approve_at` and denies at
    /// or above `deny_at`.
    pub fn set_thresholds(&self, profile_id: &str, approve_at: f32, deny_at: f32) -> Result<(), CoreError> {
        let t = Thresholds::clamped(approve_at, deny_at).ok_or_else(|| CoreError::invalid("not a threshold"))?;
        self.ap_profiles()?;
        self.ap_update_profile(profile_id, |p| p.custom = Some(t))
    }

    /// The profile's evaluations of the last 30 days replayed at these thresholds (clamped like `set_thresholds`).
    pub fn threshold_replay(
        &self,
        profile_id: &str,
        approve_at: f32,
        deny_at: f32,
    ) -> Result<ThresholdReplay, CoreError> {
        let t = Thresholds::clamped(approve_at, deny_at).ok_or_else(|| CoreError::invalid("not a threshold"))?;
        self.ap_stored_profile(profile_id)?;
        let since = unix_now() - SUGGESTION_RETENTION_SECS;
        let mine: Vec<Suggestion> =
            self.store.ap_suggestions_since(since)?.into_iter().filter(|s| s.profile_id == profile_id).collect();
        let corrections = self
            .store
            .ap_memory(profile_id)?
            .into_iter()
            .filter(|r| r.source == Source::Correction)
            .map(|r| (r.request_id, r.label))
            .collect();
        let days = u32::try_from(SUGGESTION_RETENTION_SECS / 86_400).unwrap_or(30);
        Ok(stats::replay(&mine, &corrections, (t.approve, t.deny), days))
    }

    /// The last `days` (1 to 30): requests scored, how often Autopilot agreed with the user, kinds on Auto.
    pub fn autopilot_stats(&self, days: u32) -> Result<AutopilotStats, CoreError> {
        let days = days.clamp(1, 30);
        let since = unix_now() - i64::from(days) * 86_400;
        let scored = self.store.ap_suggestions_since(since)?.iter().filter(|s| s.judged.is_some()).count();
        let profiles = self.autopilot_profiles()?;
        let mut recent = Vec::new();
        for p in &profiles {
            recent.extend(self.store.ap_memory(&p.id)?.into_iter().filter(|r| r.at > since));
        }
        let (matched, compared) = stats::agreement(&recent);
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(AutopilotStats {
            days,
            scored: count(scored),
            matched,
            compared,
            kinds_on_auto: count(profiles.iter().flat_map(|p| &p.classes).filter(|c| c.auto_approve).count()),
            kinds: count(profiles.iter().map(|p| p.classes.len()).sum()),
        })
    }
}

fn verb_infinitive(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::Deny => "deny",
        Verdict::Ask => "ask",
    }
}

/// A memory row from what was evaluated.
fn memory_row(
    s: &Suggestion,
    j: &Judged,
    label: Verdict,
    source: Source,
    weight: f32,
    would: Verdict,
    now: i64,
) -> MemoryRow {
    MemoryRow {
        request_id: s.request_id.clone(),
        at: now,
        connection_id: s.connection_id.clone(),
        class_key: s.class_key.clone(),
        label,
        weight,
        source,
        would,
        short_label: s.label.clone(),
        facts: s.s_facts.clone(),
        novel: s.novel,
        target_key: s.target_key.clone(),
        model_id: j.model_id.clone(),
        base_facts: j.base_facts,
        base_full: j.base_full,
        e_facts: j.e_facts.clone(),
        e_full: j.e_full.clone(),
    }
}
