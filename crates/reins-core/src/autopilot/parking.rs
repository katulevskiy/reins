//! Overnight's parking on the phone (Decisions v2, `docs/design/decisions-v2/01-decision-engine.md` §11; 08 R3, R12).
//!
//! What Overnight doesn't run waits for the owner like any review, but as a **park**: the suggestion is marked
//! ([`Parked`]) when the pass leaves it, then [`Engine::ap_park_waiting`] makes it a Deferred ticket with the morning's
//! window (the agent is told "Parked until morning" and carries on). Parking never notifies; the apps show one digest
//! in the morning ([`Engine::overnight_digest`]), from which "Approve all" covers only what is soft and below L3.

use reins_policy::effects::Level;

use super::engine::Subject;
use super::overnight::{self, ParkedItem};
use super::store::{Parked, Suggestion};
use super::tasks::Count;
use super::types::{AutopilotMode, OvernightDigest};
use crate::CoreError;
use crate::engine::Engine;
use crate::store::unix_now;
use crate::types::{PendingItem, PendingKind};

/// Whether "Approve all" may cover a request the pass left with `s`: only where the matrix asked it softly, below L3
/// (`Suggestion::batchable`, set at that one place), and nothing about it is always asked. A closer look, a risk hold,
/// a tripwire, a pause, the floor, a computer's question the matrix doesn't read: never.
pub(crate) fn batchable(s: &Suggestion) -> bool {
    s.batchable && s.floor.is_none() && s.matrix.as_ref().is_some_and(|m| !m.outcome.hard() && m.level < Level::L3)
}

impl Engine {
    /// What a wait leaves on the suggestion: the agent's task, and in Overnight the park (its morning and deadline).
    pub(crate) fn ap_mark_wait(&self, subject: &Subject, s: &mut Suggestion, now: i64) {
        if subject.kind != PendingKind::Request {
            return;
        }
        let task = self.ap_live_task(&subject.connection_id, now);
        s.task = task.as_ref().map(|t| t.envelope.id.clone());
        if s.mode == AutopilotMode::Overnight && s.parked.is_none() {
            let until = self
                .ap_overnight_scope(&subject.connection_id, now)
                .and_then(|scope| self.store.ap_mode_row(&scope))
                .ok()
                .and_then(|row| row.overnight_until);
            let expiry = i64::from(self.ap_overnight_expiry_hours()) * 3_600;
            let (until, deadline) = overnight::park_deadline(until, now, expiry);
            s.parked = Some(Parked {
                until,
                deadline,
                batchable: batchable(s),
            });
        }
        if let Some(t) = &task {
            let count = if s.parked.is_some() {
                Count::Parked
            } else {
                Count::Asked
            };
            if let Err(e) = self.ap_task_count(&t.envelope.id, count) {
                log::info!("a task's count was not kept: {e}");
            }
        }
    }

    /// Fills what a waiting item shows of its park and its task (`PendingItem`'s Decisions v2 fields).
    pub(crate) fn ap_fill_item(&self, item: &mut PendingItem) {
        let Ok(Some(s)) = self.store.ap_suggestion(&item.id) else {
            return;
        };
        if s.decided.is_some() {
            return;
        }
        item.parked_until = s.parked.as_ref().map(|p| p.until);
        item.parked = s.parked.is_some();
        item.batchable = s.parked.as_ref().map_or_else(|| batchable(&s), |p| p.batchable);
        if let Some(task) = &s.task {
            item.task_id = Some(task.clone());
            item.task_title = self.tasks().ok().and_then(|ts| ts.into_iter().find(|t| &t.id == task)).map(|t| t.title);
        }
    }

    /// Makes every park the pass left a Deferred ticket (once; one that fails is tried by the next pass, and waits as
    /// an Instant request meanwhile, still quiet until the morning).
    pub(crate) async fn ap_park_waiting(&self) {
        if !self.decides() {
            return;
        }
        let now = unix_now();
        let Ok(rows) = self.store.pending_rows(now) else {
            return;
        };
        for row in rows.into_iter().filter(|r| r.kind == PendingKind::Request) {
            let Ok(Some(s)) = self.store.ap_suggestion(&row.id) else {
                continue;
            };
            let Some(parked) = s.parked.filter(|_| s.decided.is_none()) else {
                continue;
            };
            if !matches!(self.store.ticket(&row.id), Ok(None)) {
                continue;
            }
            let window = u64::try_from(parked.deadline.saturating_sub(now).max(60)).unwrap_or(60);
            match self.decide_later_for(&row.id, Some(window), Some(overnight::PARKED_NOTE)).await {
                Ok(()) => self.notifier_refresh(&row.id),
                Err(e) => log::info!("a parked request could not become a ticket yet: {e}"),
            }
        }
    }

    /// What Overnight parked and still waits, for the digest.
    fn ap_parked_items(&self, now: i64) -> Result<Vec<ParkedItem>, CoreError> {
        let mut items = Vec::new();
        let tasks = self.tasks().unwrap_or_default();
        for row in self.store.pending_rows(now)?.into_iter().filter(|r| r.kind == PendingKind::Request) {
            let Some(s) = self.store.ap_suggestion(&row.id)? else {
                continue;
            };
            let Some(parked) = s.parked.as_ref().filter(|_| s.decided.is_none()) else {
                continue;
            };
            items.push(ParkedItem {
                request_id: row.id.clone(),
                connection_id: s.connection_id.clone(),
                connection_label: s.connection_label.clone(),
                task_id: s.task.clone(),
                task_title: s.task.as_ref().and_then(|id| tasks.iter().find(|t| &t.id == id)).map(|t| t.title.clone()),
                until: parked.until,
                created_at: row.created_at,
                batchable: parked.batchable,
            });
        }
        Ok(items)
    }

    /// The morning digest: what Overnight parked and still waits, grouped by agent and task; `None` when nothing does.
    pub fn overnight_digest(&self) -> Result<Option<OvernightDigest>, CoreError> {
        let now = unix_now();
        Ok(overnight::digest(&self.ap_parked_items(now)?, now))
    }

    /// "Approve all" for one agent's parks under one task (or under none): approves each one "Approve all" may
    /// cover, through the owner's ordinary approval (a ticket's revalidation included), and returns how many were.
    /// What is hard or L3 is left for the owner to decide one by one (03 §5.2).
    pub async fn approve_all(&self, connection_id: &str, task_id: Option<&str>) -> Result<u32, CoreError> {
        let now = unix_now();
        let ids: Vec<String> = self
            .ap_parked_items(now)?
            .into_iter()
            .filter(|i| i.batchable && i.connection_id == connection_id && i.task_id.as_deref() == task_id)
            .map(|i| i.request_id)
            .collect();
        let mut approved = 0;
        for id in ids {
            // With what its screen preselects, as Auto would approve it (never an empty selection), and never a
            // purchase or a standing permission (they have their own screens).
            let Some(row) = self.store.pending_rows(now)?.into_iter().find(|r| r.id == id) else {
                continue;
            };
            let Some(subject) = self.ap_subject(&row)? else {
                continue;
            };
            if subject.purchase || subject.grant {
                continue;
            }
            match self.approve(&id, subject.choice.clone()).await {
                Ok(()) => approved += 1,
                Err(e) => log::info!("Approve all left one for later: {e}"),
            }
        }
        Ok(approved)
    }
}
