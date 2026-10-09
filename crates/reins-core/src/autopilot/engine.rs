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
use super::modes::{self, DEFAULT_BYPASS_MINUTES, MAX_BYPASS_MINUTES};
use super::runtime::{DownloadProgress, ModelRuntime};
use super::situation::{self, Described};
use super::store::{Decided, Judged, NeighbourNote, Profile, SUGGESTION_RETENTION_SECS, StoredProfile, Suggestion};
use super::types::{
    AutoDecisionView, AutopilotEvent, AutopilotMode, AutopilotNote, AutopilotSettings, ClassView, ConnectionAutopilot,
    ModelStatus, NeighbourView, Preset, ProfileView, SuggestionView, Verdict,
};
use crate::CoreError;
use crate::engine::Engine;
use crate::phone_api::check_id;
use crate::store::{AuditAutopilot, AuditRecord, PendingRow, unix_now};
use crate::types::{ApprovalChoice, ApprovalView, PendingItem, PendingKind};
use crate::views::{self, ParkedRequest};

/// Neighbours kept with an evaluation and shown.
const SHOWN_NEIGHBOURS: usize = 5;
/// A "paused" notice is sent at most this often per connection.
const PAUSE_NOTICE_SECS: i64 = 600;
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
    item: PendingItem,
    /// What approving it once selects (as the approval screen preselects).
    choice: ApprovalChoice,
}

/// What one pass keeps between items.
struct PassCtx {
    now: i64,
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

/// The one line a notification or the pending list shows.
pub(crate) fn suggestion_line(s: &Suggestion) -> Option<String> {
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
    std::iter::once(name).chain(parts.map(str::to_owned)).collect::<Vec<_>>().join(" · ")
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

    /// The opened package and the runtime with its model loaded (lazily, on first need).
    async fn ap_model(&self) -> Result<(Arc<Laya>, Arc<dyn ModelRuntime>), String> {
        let runtime = lock(&self.autopilot.runtime).clone().ok_or("the app has no model runtime")?;
        let known = self.ap_known();
        let cached = lock(&self.autopilot.laya).clone().filter(|l| l.id == known.id);
        let laya = if let Some(l) = cached {
            l
        } else {
            let dir = self.data_dir.clone();
            let opened = tokio::task::spawn_blocking(move || Laya::open(&dir, &known))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let opened = Arc::new(opened);
            *lock(&self.autopilot.laya) = Some(Arc::clone(&opened));
            self.autopilot.loaded.store(false, Ordering::SeqCst);
            opened
        };
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
    async fn ap_infer(&self, states: Vec<String>) -> Result<(String, Vec<Pass>), String> {
        let (laya, runtime) = self.ap_model().await?;
        let id = laya.id.clone();
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
        self.store.ap_set_mode_row("", &global, now)?;
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

    fn ap_mode_for(&self, connection_id: &str, now: i64) -> Result<AutopilotMode, CoreError> {
        let global = self.store.ap_mode_row("")?;
        let own = self.store.ap_mode_row(connection_id)?;
        Ok(modes::effective(&global, Some(&own), self.ap_installed(), now))
    }

    // ---- the pass --------------------------------------------------------------------------------------------------

    /// Evaluates every parked item not evaluated yet: decides it when its mode says so, else attaches the suggestion
    /// and tells the app it waits. Runs after every push and sync; never fails (items just wait).
    pub(crate) async fn autopilot_pass(&self) {
        let _one = self.autopilot.pass.lock().await;
        if let Err(e) = self.ap_pass().await {
            log::warn!("Autopilot could not look at the waiting items: {e}");
        }
    }

    async fn ap_pass(&self) -> Result<(), CoreError> {
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
            if let Err(e) = self.ap_item(&row, &mut ctx).await {
                // Nothing was decided: the item waits, as it would without Autopilot.
                log::warn!("Autopilot could not evaluate a request: {e}");
                if let Some(item) = Self::ap_pending_item(&row) {
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
                    item: views::request_item(&parked),
                    choice: default_choice(&view),
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
                    item: crate::blob::blob_item(&info),
                    choice: ApprovalChoice {
                        selected_message_ids: Vec::new(),
                        standing: None,
                    },
                })
            }
        })
    }

    /// Seconds since the connection was paired: the later of the server's date and this phone's approval.
    async fn ap_age(&self, ctx: &mut PassCtx, connection_id: &str) -> Option<i64> {
        if ctx.created.is_none() {
            ctx.created = Some(match self.connections().await {
                Ok(list) => list.into_iter().map(|c| (c.id, c.created_at)).collect(),
                Err(e) => {
                    log::warn!("Autopilot could not list the connections: {e}");
                    HashMap::new()
                }
            });
        }
        let server = ctx.created.as_ref().and_then(|m| m.get(connection_id).copied());
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
        let Some(subject) = Self::ap_subject(row)? else {
            // A new connection is only ever approved by the user, in every mode.
            let pairing: PairingRequest =
                serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked pairing"))?;
            let global = self.store.ap_mode_row("")?;
            let s = Suggestion {
                request_id: row.id.clone(),
                at: now,
                kind: "pairing".to_owned(),
                connection_id: String::new(),
                connection_label: pairing.client_name.clone(),
                mode: modes::global_mode(&global, self.ap_installed(), now),
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
            };
            self.store.ap_put_suggestion(&s)?;
            let mut item = views::pairing_item(&pairing);
            item.suggestion = suggestion_line(&s);
            self.notifier.item_pending(item);
            return Ok(());
        };
        let mode = self.ap_mode_for(&subject.connection_id, now)?;
        let stored = self.ap_profile_of(&subject.connection_id)?;
        let (profile_id, profile_name, preset) = (stored.id, stored.profile.name, stored.profile.preset);
        let target_key = self.store.ap_target_key(&subject.connection_id, &subject.described.facts.target)?;
        let novel = target_key.is_empty() || !self.store.ap_target_seen(&target_key)?;
        let mut s = Suggestion {
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
        };

        match mode {
            AutopilotMode::Lockdown => {
                s.verdict = Verdict::Deny;
                "Lockdown: every request is denied".clone_into(&mut s.reason);
                return self.ap_conclude(&subject, s, Verdict::Deny, "lockdown").await;
            }
            AutopilotMode::Manual => {
                "Autopilot is off (Manual)".clone_into(&mut s.reason);
                return self.ap_wait(&subject, &s);
            }
            AutopilotMode::Bypass | AutopilotMode::Assisted | AutopilotMode::Auto => {}
        }

        // What the model reads, also kept for corrections.
        let age = self.ap_age(ctx, &subject.connection_id).await;
        let mut facts = subject.described.facts.clone();
        facts.connection.clone_from(&subject.connection_label);
        facts.connection_age = age;
        facts.connection_history = self.store.ap_history(&subject.connection_id).ok();
        facts.target_is_new = (!facts.target.is_empty()).then_some(novel);
        (s.s_facts, s.s_full) = situation::render(&facts, &subject.described.ai);

        let young = gates::young(age, self.cfg.new_connection_secs)
            .then(|| "a connection paired less than 10 minutes ago".to_owned());
        if let Some(floor) = subject.floor.clone().or(young) {
            s.reason = format!("Always asks: {floor}");
            s.floor = Some(floor);
            return self.ap_wait(&subject, &s);
        }
        if mode == AutopilotMode::Bypass {
            s.verdict = Verdict::Approve;
            "Bypass: everything outside the hard floor is approved".clone_into(&mut s.reason);
            return self.ap_conclude(&subject, s, Verdict::Approve, "bypass").await;
        }

        let judged = {
            let profile = self.ap_profile_loaded(ctx, &subject.connection_id)?;
            self.ap_judge(&s, profile, now).await
        };
        let (judged, probs) = match judged {
            Ok(j) => j,
            Err(why) => {
                s.reason = format!("Autopilot could not judge this: {why}");
                return self.ap_wait(&subject, &s);
            }
        };
        s.verdict = decide::would(probs, novel, judged.escalate, preset.thresholds());
        s.reason = reason_of(&judged, s.verdict, novel);
        s.judged = Some(judged);

        if mode == AutopilotMode::Auto && s.verdict != Verdict::Ask {
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
                        return self.ap_conclude(&subject, s, Verdict::Approve, "autopilot").await;
                    }
                    s.reason = format!("{} — paused: unusual volume from this connection", s.reason);
                    self.ap_pause_notice(&subject, now);
                }
                Verdict::Deny if may_deny => return self.ap_conclude(&subject, s, Verdict::Deny, "autopilot").await,
                Verdict::Deny => {
                    s.reason = format!("{} — auto-deny is not unlocked for this kind of request", s.reason);
                }
                Verdict::Ask => {}
            }
        }
        self.ap_wait(&subject, &s)
    }

    /// Runs the model on the two texts and scores them against the profile.
    async fn ap_judge(
        &self,
        s: &Suggestion,
        profile: &mut (StoredProfile, Vec<MemoryRow>),
        now: i64,
    ) -> Result<(Judged, Probs), String> {
        let states = if s.s_full == s.s_facts {
            vec![s.s_facts.clone()]
        } else {
            vec![s.s_facts.clone(), s.s_full.clone()]
        };
        let (model_id, passes) = self.ap_infer(states).await?;
        let facts = passes.first().cloned().ok_or("the model returned nothing")?;
        let full = passes.get(1).cloned().unwrap_or_else(|| facts.clone());
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

    fn ap_pause_notice(&self, subject: &Subject, now: i64) {
        let mut paused = lock(&self.autopilot.paused);
        if paused.get(&subject.connection_id).is_some_and(|t| now - t < PAUSE_NOTICE_SECS) {
            return;
        }
        paused.insert(subject.connection_id.clone(), now);
        drop(paused);
        self.notifier.autopilot_changed(AutopilotEvent::Paused {
            connection_id: subject.connection_id.clone(),
            connection_label: subject.connection_label.clone(),
            reason: "unusual volume".to_owned(),
        });
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
        by: &str,
    ) -> Result<(), CoreError> {
        let note = note_of(&s, verdict);
        let (result, seq) = deciding(Decision::new(by, Some(note)), self.ap_execute(subject, verdict)).await;
        let now = unix_now();
        match result {
            Ok(()) => {
                s.decided = Some(Decided {
                    by: by.to_owned(),
                    verdict,
                    at: now,
                });
                self.store.ap_put_suggestion(&s)?;
                if by == "autopilot" && verdict == Verdict::Approve {
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

    async fn ap_execute(&self, subject: &Subject, verdict: Verdict) -> Result<(), CoreError> {
        match (subject.kind, verdict) {
            (PendingKind::Request, Verdict::Approve) => self.approve_claimed(&subject.id, subject.choice.clone()).await,
            (PendingKind::Request, _) => self.deny_claimed(&subject.id).await,
            (PendingKind::Blob, v) => self.answer_blob_claimed(&subject.id, v == Verdict::Approve).await,
            (PendingKind::Pairing | PendingKind::Join, _) => {
                Err(CoreError::invalid("a pairing or another phone is never decided automatically"))
            }
        }
    }

    fn ap_expire_bypasses(&self, now: i64) -> Result<(), CoreError> {
        for (scope, mut row) in self.store.ap_mode_rows()? {
            if row.bypass_until.is_some_and(|t| t <= now) {
                row.bypass_until = None;
                self.store.ap_set_mode_row(&scope, &row, now)?;
                self.notifier.autopilot_changed(AutopilotEvent::BypassEnded {
                    connection_id: (!scope.is_empty()).then_some(scope),
                });
            }
        }
        Ok(())
    }

    // ---- learning from the user ------------------------------------------------------------------------------------

    /// The note the user's own decision on a request carries: what Autopilot suggested, or why it could not (no
    /// note when Autopilot was off).
    pub(crate) fn ap_human_note(&self, request_id: &str) -> Option<AuditAutopilot> {
        let s = self.store.ap_suggestion(request_id).ok().flatten()?;
        let looked = s.judged.is_some()
            || s.floor.is_some()
            || matches!(s.mode, AutopilotMode::Assisted | AutopilotMode::Auto | AutopilotMode::Bypass);
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
                self.ap_judge(&s, &mut profile, now).await.map_err(CoreError::service)?.0
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
        self.autopilot.download.status(&self.ap_known(), self.ap_installed(), runtime)
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
        let installed = self.ap_installed();
        let profiles = self.ap_profiles()?;
        let default_profile_id = self.ap_default_profile_id(&profiles)?;
        let global = self.store.ap_mode_row("")?;
        let connections = self
            .store
            .ap_mode_rows()?
            .into_iter()
            .filter(|(scope, _)| !scope.is_empty())
            .map(|(scope, row)| ConnectionAutopilot {
                mode: modes::effective(&global, Some(&row), installed, now),
                base_mode: row.mode,
                bypass_until: row.bypass_until.filter(|t| *t > now),
                profile_id: row
                    .profile_id
                    .clone()
                    .filter(|id| profiles.iter().any(|p| &p.id == id))
                    .unwrap_or_else(|| default_profile_id.clone()),
                connection_id: scope,
            })
            .collect();
        Ok(AutopilotSettings {
            mode: modes::global_mode(&global, installed, now),
            base_mode: modes::global_base(&global, installed),
            bypass_until: global.bypass_until.filter(|t| *t > now),
            default_profile_id,
            wifi_only: self.store.meta_get(WIFI_ONLY_KEY)?.is_none_or(|v| v != "0"),
            model: self.model_status(),
            connections,
        })
    }

    /// Sets the global mode (`connection_id` = `None`) or a connection's; `mode` = `None` returns to the default
    /// (global) or to following the global mode (connection). A bypass lasts `minutes` (15 by default, 60 at most);
    /// Lockdown also denies what is waiting.
    pub async fn set_autopilot_mode(
        &self,
        connection_id: Option<String>,
        mode: Option<AutopilotMode>,
        minutes: Option<u32>,
    ) -> Result<(), CoreError> {
        if let Some(id) = &connection_id {
            check_id(id)?;
        }
        let now = unix_now();
        let scope = connection_id.clone().unwrap_or_default();
        let mut row = self.store.ap_mode_row(&scope)?;
        match mode {
            Some(AutopilotMode::Bypass) => {
                let minutes = minutes.unwrap_or(DEFAULT_BYPASS_MINUTES);
                if minutes == 0 || minutes > MAX_BYPASS_MINUTES {
                    return Err(CoreError::invalid("a bypass lasts between 1 and 60 minutes"));
                }
                if let Some(id) = &connection_id {
                    let created = self.connections().await?.into_iter().find(|c| &c.id == id).map(|c| c.created_at);
                    let paired = self.store.ap_paired_at(id)?;
                    let since = created.into_iter().chain(paired).max().ok_or(CoreError::NotFound)?;
                    if now - since < self.cfg.new_connection_secs {
                        return Err(CoreError::invalid(
                            "a connection paired less than 10 minutes ago cannot be put in bypass",
                        ));
                    }
                }
                if row.mode == Some(AutopilotMode::Lockdown) {
                    row.mode = None;
                }
                row.bypass_until = Some(now + i64::from(minutes) * 60);
            }
            other => {
                row.mode = other;
                row.bypass_until = None;
            }
        }
        self.store.ap_set_mode_row(&scope, &row, now)?;
        self.notifier.autopilot_changed(AutopilotEvent::ModeChanged {
            connection_id: connection_id.clone(),
        });
        if mode == Some(AutopilotMode::Lockdown) {
            self.ap_lockdown_sweep(connection_id.as_deref()).await;
        }
        Ok(())
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
            });
            s.mode = AutopilotMode::Lockdown;
            "Lockdown: every request is denied".clone_into(&mut s.reason);
            if let Err(e) = self.ap_conclude(&subject, s, Verdict::Deny, "lockdown").await {
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
        let preset = stored.profile.preset;
        let s = Suggestion {
            request_id: String::new(),
            at: unix_now(),
            kind: "playground".to_owned(),
            connection_id: String::new(),
            connection_label: String::new(),
            mode: AutopilotMode::Assisted,
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
        };
        let memory = self.store.ap_memory(&stored.id)?;
        let mut profile = (stored, memory);
        let (judged, probs) = self.ap_judge(&s, &mut profile, s.at).await.map_err(CoreError::service)?;
        let verdict = decide::would(probs, s.novel, judged.escalate, preset.thresholds());
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
            views.push(ProfileView {
                is_default: p.id == default_id,
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
                preset: p.profile.preset,
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
        self.store.ap_set_mode_row("", &global, unix_now())
    }

    /// The profile a connection's decisions train (`None` = the default profile).
    pub fn assign_profile(&self, connection_id: &str, profile_id: Option<String>) -> Result<(), CoreError> {
        check_id(connection_id)?;
        if let Some(id) = &profile_id {
            self.ap_stored_profile(id)?;
        }
        let mut row = self.store.ap_mode_row(connection_id)?;
        row.profile_id = profile_id;
        self.store.ap_set_mode_row(connection_id, &row, unix_now())
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

    pub fn set_preset(&self, profile_id: &str, preset: Preset) -> Result<(), CoreError> {
        self.ap_profiles()?;
        self.ap_update_profile(profile_id, |p| p.preset = preset)
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
