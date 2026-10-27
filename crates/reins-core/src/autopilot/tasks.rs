//! Tasks (Decisions v2, `docs/design/decisions-v2/01-decision-engine.md` §12; 08 R3): an envelope inside which Auto
//! may do more for one agent connection, up to a ceiling, for a while.
//!
//! - Started by the agent (`reins_task_propose`, `desktop_task_propose`: a request the owner approves, and may only
//!   narrow), by the owner, or with Overnight. One live task per connection: a new one ends the one before.
//! - A task is an AND bound (`reins_policy::matrix::Task::holds`): it satisfies only soft cells, never a hard
//!   obligation, and only for its connection, inside every list of its envelope, below its ceiling, before its end and
//!   while uses are left. Uses are spent before the effect.
//! - Ending it (time, the agent, Stop) only takes away: never asked. What ran under it stays as its digest.
//!
//! Records are sealed with the data key, next to the policy snapshot; the server never sees them.

use reins_policy::context::{self as graph, Zone};
use reins_policy::effects::{EffectKind, Level};
use reins_policy::matrix::{self, Snapshot};
use reins_proto::connector::ConnectorCall;
use reins_proto::connector::tasks::{self as proto, TaskDraft};
use serde::{Deserialize, Serialize};

use super::types::TaskView;
use crate::engine::Engine;
use crate::store::unix_now;
use crate::{CoreError, OwnerCheck};

const TASKS_KEY: &str = "autopilot.tasks";
const EDITS_KEY: &str = "autopilot.task_edits";
/// Ended tasks are listed (with their digest) this long.
pub const KEEP_ENDED_SECS: i64 = 86_400;
/// Records kept at most.
const MAX_RECORDS: usize = 50;

/// A task and what ran under it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskRecord {
    pub envelope: matrix::Task,
    #[serde(default)]
    pub connection_label: String,
    pub title: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub budget: Option<u32>,
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub started_by: String,
    #[serde(default)]
    pub auto: u32,
    #[serde(default)]
    pub asked: u32,
    #[serde(default)]
    pub parked: u32,
}

impl TaskRecord {
    pub fn live(&self, now: i64) -> bool {
        self.ended_at.is_none() && now < self.envelope.not_after
    }

    pub fn view(&self, now: i64) -> TaskView {
        let e = &self.envelope;
        TaskView {
            id: e.id.clone(),
            connection_id: e.connection.clone(),
            connection_label: self.connection_label.clone(),
            title: self.title.clone(),
            reason: self.reason.as_deref().map(crate::text::mark_invisible),
            repos: e.resources.clone(),
            branches: e.branches.clone(),
            paths: e.paths.clone(),
            people: e.people.clone(),
            operations: self.operations.clone(),
            ceiling: e.ceiling.min(matrix::MAX_CEILING) as u8,
            budget: self.budget,
            uses_left: e.uses_left,
            started_at: self.started_at,
            ends_at: e.not_after,
            ended_at: self.ended_at.or_else(|| (now >= e.not_after).then_some(e.not_after)),
            started_by: self.started_by.clone(),
            auto: self.auto,
            asked: self.asked,
            parked: self.parked,
        }
    }
}

/// What a task counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Count {
    /// Decided Auto by it: a use is spent.
    Auto,
    Asked,
    Parked,
}

fn level_of(ceiling: u8) -> Result<Level, CoreError> {
    match ceiling {
        0 => Ok(Level::L0),
        1 => Ok(Level::L1),
        2 => Ok(Level::L2),
        _ => Err(CoreError::invalid("A task goes up to Reaches people at most.")),
    }
}

fn operation_of(op: &str) -> Result<EffectKind, CoreError> {
    if !proto::OPERATIONS.contains(&op) {
        return Err(CoreError::invalid(format!("A task can't name {op:?}.")));
    }
    serde_json::from_value(serde_json::Value::String(op.to_owned()))
        .map_err(|_| CoreError::invalid(format!("A task can't name {op:?}.")))
}

/// Checks every list of a view against the same bounds a proposal has.
fn check_lists(v: &TaskView) -> Result<(), CoreError> {
    let bad = |what: &str| Err(CoreError::invalid(format!("A task's {what} has an entry that is not allowed.")));
    let max = proto::MAX_TASK_ITEMS;
    if [&v.repos, &v.branches, &v.paths, &v.people, &v.operations].iter().any(|l| l.len() > max) {
        return Err(CoreError::invalid(format!("A task lists at most {max} of each.")));
    }
    if !v.repos.iter().all(|r| proto::valid_repo(r)) {
        return bad("repositories");
    }
    if !v.branches.iter().all(|b| proto::valid_branch(b)) {
        return bad("branches");
    }
    if !v.paths.iter().all(|p| proto::valid_path(p)) {
        return bad("folders");
    }
    if !v.people.iter().all(|p| proto::valid_person(p)) {
        return bad("people");
    }
    if v.title.trim().is_empty() || v.title.chars().any(char::is_control) || v.title.chars().count() > 80 {
        return Err(CoreError::invalid("A task needs a title, one line."));
    }
    Ok(())
}

fn sorted(list: &[String], lower: bool) -> Vec<String> {
    let mut v: Vec<String> = list
        .iter()
        .map(|s| {
            if lower {
                s.trim().to_ascii_lowercase()
            } else {
                s.trim().to_owned()
            }
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The ceiling a task gets when none is asked: L2 when every repository it names is the owner's own (Mine), else L1
/// (01 §12).
pub(crate) fn default_ceiling(repos: &[String], facts: &graph::Facts) -> u8 {
    let mine = !repos.is_empty()
        && repos.iter().all(|r| {
            let zone = match r.strip_suffix("/*") {
                Some(owner) => facts.get(graph::FactKind::Owner, owner).unwrap_or(Zone::Outside),
                None => {
                    graph::parse_repo(r, Some(graph::GITHUB)).map_or(Zone::Outside, |rr| graph::repo_zone(facts, &rr).0)
                }
            };
            zone == Zone::Mine
        });
    if mine {
        2
    } else {
        1
    }
}

/// A proposal as the card shows it (before the owner approves it).
pub(crate) fn view_of_draft(
    d: &TaskDraft,
    connection_id: &str,
    connection_label: &str,
    facts: &graph::Facts,
    now: i64,
) -> TaskView {
    TaskView {
        id: String::new(),
        connection_id: connection_id.to_owned(),
        connection_label: connection_label.to_owned(),
        title: d.title.clone(),
        reason: d.reason.as_deref().map(crate::text::mark_invisible),
        repos: d.repos.clone(),
        branches: d.branches.clone(),
        paths: d.paths.clone(),
        people: d.people.clone(),
        operations: d.operations.clone(),
        ceiling: d.ceiling.unwrap_or_else(|| default_ceiling(&d.repos, facts)),
        budget: d.budget,
        uses_left: d.budget,
        started_at: now,
        ends_at: now.saturating_add(d.minutes.saturating_mul(60)),
        ended_at: None,
        started_by: "agent".to_owned(),
        auto: 0,
        asked: 0,
        parked: 0,
    }
}

/// `edit` as the owner narrowed `proposed`: every list a subset, the ceiling no higher, the length no longer, the
/// budget no larger (none only if none was proposed), the same connection. Anything else is refused: the owner widens
/// a task by starting one of their own.
pub(crate) fn narrow(proposed: &TaskView, edit: &TaskView) -> Result<TaskView, &'static str> {
    let subset = |a: &[String], b: &[String]| a.iter().all(|x| b.contains(x));
    if edit.connection_id != proposed.connection_id {
        return Err("A task stays with the agent that proposed it.");
    }
    if !subset(&edit.repos, &proposed.repos)
        || !subset(&edit.branches, &proposed.branches)
        || !subset(&edit.paths, &proposed.paths)
        || !subset(&edit.people, &proposed.people)
        || !subset(&edit.operations, &proposed.operations)
    {
        return Err("Only remove from what the agent asked for.");
    }
    // An emptied list would mean "any": never as a narrowing of a list that named something.
    let emptied = |a: &[String], b: &[String]| a.is_empty() && !b.is_empty();
    if emptied(&edit.repos, &proposed.repos)
        || emptied(&edit.branches, &proposed.branches)
        || emptied(&edit.paths, &proposed.paths)
        || emptied(&edit.operations, &proposed.operations)
    {
        return Err("Leave at least one, or deny the task.");
    }
    if edit.ceiling > proposed.ceiling {
        return Err("The ceiling can only go down.");
    }
    let length = |v: &TaskView| v.ends_at.saturating_sub(v.started_at);
    if length(edit) > length(proposed) || length(edit) < proto::MIN_TASK_MINUTES * 60 {
        return Err("The length can only be shortened, to 15 minutes at least.");
    }
    match (proposed.budget, edit.budget) {
        (Some(p), Some(e)) if e > p || e == 0 => return Err("The budget can only go down."),
        (Some(_), None) => return Err("The budget can only go down."),
        _ => {}
    }
    let mut out = proposed.clone();
    out.title.clone_from(&edit.title);
    out.repos.clone_from(&edit.repos);
    out.branches.clone_from(&edit.branches);
    out.paths.clone_from(&edit.paths);
    out.people.clone_from(&edit.people);
    out.operations.clone_from(&edit.operations);
    out.ceiling = edit.ceiling;
    out.ends_at = proposed.started_at.saturating_add(length(edit));
    out.budget = edit.budget;
    out.uses_left = edit.budget;
    Ok(out)
}

/// A random id an agent can name: `tk_` and 16 hex digits.
fn new_id() -> String {
    format!("tk_{}", &uuid::Uuid::new_v4().simple().to_string()[..16])
}

impl Engine {
    fn ap_task_records(&self) -> Result<Vec<TaskRecord>, CoreError> {
        match self.store.meta_get_sealed(TASKS_KEY)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| CoreError::storage("corrupt tasks")),
            None => Ok(Vec::new()),
        }
    }

    fn ap_put_task_records(&self, records: &[TaskRecord]) -> Result<(), CoreError> {
        let json = serde_json::to_vec(records).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.meta_set_sealed(TASKS_KEY, &json)
    }

    /// Changes the records under one lock-free read-modify-write (the account's calls run one at a time).
    fn ap_with_tasks<T>(&self, f: impl FnOnce(&mut Vec<TaskRecord>) -> T) -> Result<T, CoreError> {
        let mut records = self.ap_task_records()?;
        let out = f(&mut records);
        let now = unix_now();
        records.retain(|r| r.live(now) || r.ended_at.unwrap_or(r.envelope.not_after) > now - KEEP_ENDED_SECS);
        if records.len() > MAX_RECORDS {
            let drop = records.len() - MAX_RECORDS;
            records.drain(..drop);
        }
        self.ap_put_task_records(&records)?;
        Ok(out)
    }

    /// The task live for a connection now.
    pub(crate) fn ap_live_task(&self, connection_id: &str, now: i64) -> Option<TaskRecord> {
        let records = self.ap_task_records().ok()?;
        records.into_iter().rev().find(|r| r.envelope.connection == connection_id && r.live(now))
    }

    /// Puts the live tasks' envelopes into `snap`, so that the evaluation (and its hash) includes them.
    pub(crate) fn ap_tasks_into(&self, snap: &mut Snapshot, now: i64) {
        match self.ap_task_records() {
            Ok(records) => {
                snap.tasks.extend(records.into_iter().filter(|r| r.live(now)).map(|r| r.envelope));
            }
            // A task that can't be read covers nothing.
            Err(e) => log::warn!("tasks could not be read: {e}"),
        }
    }

    /// Counts a request against task `id`. `Count::Auto` spends a use: it runs before the effect, and fails when
    /// none is left (the request then waits).
    pub(crate) fn ap_task_count(&self, id: &str, count: Count) -> Result<(), CoreError> {
        let spent = self.ap_with_tasks(|records| {
            let Some(r) = records.iter_mut().find(|r| r.envelope.id == id) else {
                return count != Count::Auto;
            };
            match count {
                Count::Auto => {
                    if r.envelope.uses_left == Some(0) {
                        return false;
                    }
                    r.envelope.uses_left = r.envelope.uses_left.map(|u| u - 1);
                    r.auto = r.auto.saturating_add(1);
                }
                Count::Asked => r.asked = r.asked.saturating_add(1),
                Count::Parked => r.parked = r.parked.saturating_add(1),
            }
            true
        })?;
        if spent {
            Ok(())
        } else {
            Err(CoreError::invalid("the task has no uses left"))
        }
    }

    /// Live tasks, then those that ended in the last day (with what ran under them), newest first.
    pub fn tasks(&self) -> Result<Vec<TaskView>, CoreError> {
        let now = unix_now();
        let mut records = self.ap_task_records()?;
        records.retain(|r| r.live(now) || r.ended_at.unwrap_or(r.envelope.not_after) > now - KEEP_ENDED_SECS);
        records.sort_by_key(|r| (!r.live(now), std::cmp::Reverse(r.started_at)));
        Ok(records.iter().map(|r| r.view(now)).collect())
    }

    /// Stores a task from a checked view; the connection's live task ends first. Returns its record.
    pub(crate) fn ap_begin_task(&self, v: &TaskView, started_by: &str, now: i64) -> Result<TaskRecord, CoreError> {
        check_lists(v)?;
        let ceiling = level_of(v.ceiling)?;
        let length = v.ends_at.saturating_sub(v.started_at);
        if !(proto::MIN_TASK_MINUTES * 60..=proto::MAX_TASK_MINUTES * 60).contains(&length) {
            return Err(CoreError::invalid("A task lasts 15 minutes to 24 hours."));
        }
        if v.budget == Some(0) {
            return Err(CoreError::invalid("A budget is at least one."));
        }
        let effects = v.operations.iter().map(|o| operation_of(o)).collect::<Result<Vec<_>, _>>()?;
        let record = TaskRecord {
            envelope: matrix::Task {
                id: new_id(),
                connection: v.connection_id.clone(),
                ceiling,
                zones: Vec::new(),
                effects,
                resources: sorted(&v.repos, true),
                branches: sorted(&v.branches, false),
                paths: sorted(&v.paths, false),
                people: sorted(&v.people, true),
                uses_left: v.budget,
                not_after: now.saturating_add(length),
            },
            connection_label: v.connection_label.clone(),
            title: v.title.trim().to_owned(),
            reason: v.reason.clone(),
            operations: sorted(&v.operations, false),
            budget: v.budget,
            started_at: now,
            ended_at: None,
            started_by: started_by.to_owned(),
            auto: 0,
            asked: 0,
            parked: 0,
        };
        let stored = record.clone();
        self.ap_with_tasks(|records| {
            for r in records.iter_mut().filter(|r| r.envelope.connection == stored.envelope.connection && r.live(now)) {
                r.ended_at = Some(now);
            }
            records.push(stored);
        })?;
        self.notifier.autopilot_changed(super::types::AutopilotEvent::ModeChanged {
            connection_id: Some(v.connection_id.clone()),
        });
        Ok(record)
    }

    /// The owner starts a task (from Activity, or with Overnight): a loosening, so the owner check comes first.
    pub async fn start_task(&self, draft: TaskView, owner: &dyn OwnerCheck) -> Result<TaskView, CoreError> {
        if draft.connection_id.is_empty() {
            return Err(CoreError::invalid("A task is for one AI."));
        }
        crate::phone_api::check_id(&draft.connection_id)?;
        check_lists(&draft)?;
        let by = if draft.started_by == "overnight" {
            "overnight"
        } else {
            "you"
        };
        let reason = format!("Start the task \"{}\"", crate::text::one_line(draft.title.trim()));
        match owner.confirm(reason).await {
            Ok(true) => {}
            Ok(false) => return Err(CoreError::invalid("the owner did not confirm")),
            Err(e) => return Err(CoreError::invalid(format!("the owner check failed: {e}"))),
        }
        let now = unix_now();
        let mut v = draft;
        let length = v.ends_at.saturating_sub(v.started_at);
        v.started_at = now;
        v.ends_at = now.saturating_add(length);
        let record = self.ap_begin_task(&v, by, now)?;
        Ok(record.view(now))
    }

    /// Ends a task now (Stop). Takes away only: never asked.
    pub fn end_task(&self, id: &str) -> Result<(), CoreError> {
        let now = unix_now();
        let found = self.ap_with_tasks(|records| {
            let mut found = None;
            for r in records.iter_mut().filter(|r| r.envelope.id == id && r.live(now)) {
                r.ended_at = Some(now);
                found = Some(r.envelope.connection.clone());
            }
            found
        })?;
        if let Some(connection) = found {
            self.notifier.autopilot_changed(super::types::AutopilotEvent::ModeChanged {
                connection_id: Some(connection),
            });
        }
        Ok(())
    }

    /// The agent ends its own task (`reins_task_end`): only one of its own connection; the number ended.
    pub(crate) fn end_task_of(&self, connection_id: &str, id: &str) -> Result<u32, CoreError> {
        let now = unix_now();
        self.ap_with_tasks(|records| {
            let mut n = 0;
            for r in records
                .iter_mut()
                .filter(|r| r.envelope.connection == connection_id && r.envelope.id == id && r.live(now))
            {
                r.ended_at = Some(now);
                n += 1;
            }
            n
        })
    }

    /// What a proposal asks, as the card shows it.
    pub(crate) fn task_proposal_view(
        &self,
        connection_id: &str,
        label: &str,
        call: &ConnectorCall,
        at: i64,
    ) -> Option<TaskView> {
        let draft = proto::draft_of(call).ok()?;
        let facts = self.ap_snapshot().map(|s| s.facts).unwrap_or_default();
        Some(view_of_draft(&draft, connection_id, label, &facts, at))
    }

    /// The owner approves a proposal, as they narrowed it (`draft`): kept for the approval to apply, then approved
    /// through the ordinary path (the owner check first: a task loosens).
    pub async fn approve_task(
        &self,
        request_id: &str,
        draft: TaskView,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        let (_, parked) = self.parked(request_id)?;
        let reins_proto::gmail::ToolCall::Connector(call) = &parked.request.call else {
            return Err(CoreError::invalid("This request doesn't propose a task."));
        };
        if call.service != reins_proto::connector::TASKS || !proto::proposes(&call.op) {
            return Err(CoreError::invalid("This request doesn't propose a task."));
        }
        let proposed = self
            .task_proposal_view(&parked.request.connection_id.0, &parked.label(), call, parked.request.created_at)
            .ok_or_else(|| CoreError::invalid("The proposal can't be read."))?;
        let narrowed = narrow(&proposed, &draft).map_err(CoreError::invalid)?;
        check_lists(&narrowed)?;
        let reason = format!("Allow the task \"{}\"", crate::text::one_line(&narrowed.title));
        match owner.confirm(reason).await {
            Ok(true) => {}
            Ok(false) => return Err(CoreError::invalid("the owner did not confirm")),
            Err(e) => return Err(CoreError::invalid(format!("the owner check failed: {e}"))),
        }
        self.ap_put_task_edit(request_id, Some(&narrowed))?;
        let choice = crate::types::ApprovalChoice {
            selected_message_ids: Vec::new(),
            standing: None,
        };
        let approved = self.approve(request_id, choice).await;
        if approved.is_err() {
            self.ap_put_task_edit(request_id, None)?;
        }
        approved
    }

    /// Whether the owner approved proposal `request_id` through its card (`approve_task`, after the owner check).
    pub(crate) fn ap_task_checked(&self, request_id: &str) -> Result<bool, CoreError> {
        Ok(self.ap_task_edits()?.contains_key(request_id))
    }

    fn ap_task_edits(&self) -> Result<std::collections::BTreeMap<String, TaskViewStored>, CoreError> {
        match self.store.meta_get_sealed(EDITS_KEY)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| CoreError::storage("corrupt task edits")),
            None => Ok(std::collections::BTreeMap::new()),
        }
    }

    fn ap_put_task_edit(&self, request_id: &str, edit: Option<&TaskView>) -> Result<(), CoreError> {
        let mut edits = self.ap_task_edits()?;
        match edit {
            Some(v) => {
                edits.insert(request_id.to_owned(), TaskViewStored::from(v));
            }
            None => {
                edits.remove(request_id);
            }
        }
        // Few at a time: an edit lives from the tap to the approval.
        while edits.len() > 20 {
            let first = edits.keys().next().cloned().unwrap_or_default();
            edits.remove(&first);
        }
        let json = serde_json::to_vec(&edits).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.meta_set_sealed(EDITS_KEY, &json)
    }

    /// Applies an approved proposal: the owner's narrowed envelope when they edited it, else the proposal as asked.
    /// Returns what the agent is told: the envelope as approved (02 §9), nothing else of the policy.
    pub(crate) fn ap_task_from_request(
        &self,
        request: &reins_proto::relay::RelayRequest,
        label: &str,
        call: &ConnectorCall,
    ) -> Result<serde_json::Value, CoreError> {
        let now = unix_now();
        let proposed = self
            .task_proposal_view(&request.connection_id.0, label, call, request.created_at)
            .ok_or_else(|| CoreError::invalid("The proposal can't be read."))?;
        let edit = self.ap_task_edits()?.remove(&request.id.0).map(TaskView::from);
        let mut v = match edit {
            Some(e) => narrow(&proposed, &e).map_err(CoreError::invalid)?,
            None => proposed,
        };
        // Its length runs from now, not from when it was asked.
        let length = v.ends_at.saturating_sub(v.started_at);
        v.started_at = now;
        v.ends_at = now.saturating_add(length);
        let record = self.ap_begin_task(&v, "agent", now)?;
        self.ap_put_task_edit(&request.id.0, None)?;
        let e = &record.envelope;
        Ok(serde_json::json!({
            "task": e.id,
            "title": record.title,
            "repos": e.resources,
            "branches": e.branches,
            "paths": e.paths,
            "people": e.people,
            "operations": record.operations,
            "ceiling": proto::CEILINGS[(e.ceiling.min(matrix::MAX_CEILING) as usize).min(2)],
            "budget": e.uses_left,
            "ends_at": e.not_after,
            "note": "Inside this envelope Reins decides more on its own. What can't be undone, secrets, money and \
                     publishing are still asked."
        }))
    }
}

/// [`TaskView`] as kept between the owner's tap and the approval (UniFFI records don't derive serde).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TaskViewStored {
    connection_id: String,
    title: String,
    repos: Vec<String>,
    branches: Vec<String>,
    paths: Vec<String>,
    people: Vec<String>,
    operations: Vec<String>,
    ceiling: u8,
    budget: Option<u32>,
    started_at: i64,
    ends_at: i64,
}

impl From<&TaskView> for TaskViewStored {
    fn from(v: &TaskView) -> Self {
        Self {
            connection_id: v.connection_id.clone(),
            title: v.title.clone(),
            repos: v.repos.clone(),
            branches: v.branches.clone(),
            paths: v.paths.clone(),
            people: v.people.clone(),
            operations: v.operations.clone(),
            ceiling: v.ceiling,
            budget: v.budget,
            started_at: v.started_at,
            ends_at: v.ends_at,
        }
    }
}

impl From<TaskViewStored> for TaskView {
    fn from(s: TaskViewStored) -> Self {
        Self {
            id: String::new(),
            connection_id: s.connection_id,
            connection_label: String::new(),
            title: s.title,
            reason: None,
            repos: s.repos,
            branches: s.branches,
            paths: s.paths,
            people: s.people,
            operations: s.operations,
            ceiling: s.ceiling,
            budget: s.budget,
            uses_left: s.budget,
            started_at: s.started_at,
            ends_at: s.ends_at,
            ended_at: None,
            started_by: "agent".to_owned(),
            auto: 0,
            asked: 0,
            parked: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal() -> TaskView {
        TaskView {
            id: String::new(),
            connection_id: "c1".to_owned(),
            connection_label: "Claude Code".to_owned(),
            title: "Fix the CI".to_owned(),
            reason: None,
            repos: vec!["acme/app".to_owned(), "acme/lib".to_owned()],
            branches: vec!["fix/*".to_owned()],
            paths: Vec::new(),
            people: vec!["ann@acme.io".to_owned()],
            operations: vec!["git_push".to_owned(), "git_ref_delete".to_owned()],
            ceiling: 2,
            budget: Some(20),
            uses_left: Some(20),
            started_at: 1_000,
            ends_at: 1_000 + 7_200,
            ended_at: None,
            started_by: "agent".to_owned(),
            auto: 0,
            asked: 0,
            parked: 0,
        }
    }

    type Change = Box<dyn Fn(&mut TaskView)>;

    #[test]
    fn narrowing_only() {
        let p = proposal();
        let mut e = p.clone();
        e.repos = vec!["acme/app".to_owned()];
        e.people.clear();
        e.ceiling = 1;
        e.ends_at = 1_000 + 3_600;
        e.budget = Some(5);
        let n = narrow(&p, &e).unwrap();
        assert_eq!(n.repos, vec!["acme/app"]);
        assert_eq!(n.ceiling, 1);
        assert_eq!(n.uses_left, Some(5));
        let changes: [(&str, Change); 9] = [
            ("an added repo", Box::new(|v: &mut TaskView| v.repos.push("octo/x".to_owned()))),
            ("a raised ceiling", Box::new(|v: &mut TaskView| v.ceiling = 3)),
            ("a longer length", Box::new(|v: &mut TaskView| v.ends_at += 60)),
            ("a larger budget", Box::new(|v: &mut TaskView| v.budget = Some(21))),
            ("no budget", Box::new(|v: &mut TaskView| v.budget = None)),
            ("an emptied repo list", Box::new(|v: &mut TaskView| v.repos.clear())),
            ("another agent", Box::new(|v: &mut TaskView| "c2".clone_into(&mut v.connection_id))),
            ("an added person", Box::new(|v: &mut TaskView| v.people.push("bob@acme.io".to_owned()))),
            ("too short", Box::new(|v: &mut TaskView| v.ends_at = v.started_at + 60)),
        ];
        for (what, change) in changes {
            let mut e = p.clone();
            change(&mut e);
            assert!(narrow(&p, &e).is_err(), "{what}");
        }
    }

    #[test]
    fn l2_ceiling_by_default_only_for_mine_repos() {
        let mut facts = graph::Facts::default();
        facts.set(graph::FactKind::Owner, "dkat", Zone::Mine);
        facts.set(graph::FactKind::Owner, "acme", Zone::Work);
        assert_eq!(default_ceiling(&["dkat/reins".to_owned(), "dkat/*".to_owned()], &facts), 2);
        assert_eq!(default_ceiling(&["dkat/reins".to_owned(), "acme/app".to_owned()], &facts), 1);
        assert_eq!(default_ceiling(&[], &facts), 1);
    }

    #[test]
    fn operations_map_to_effect_kinds() {
        assert_eq!(operation_of("git_push").unwrap(), EffectKind::GitPush);
        assert_eq!(operation_of("git_ref_delete").unwrap(), EffectKind::GitRefDelete);
        assert!(operation_of("publish").is_err(), "never a kind the proposal can't name");
        assert!(operation_of("money").is_err());
    }
}
