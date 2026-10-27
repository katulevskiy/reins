//! Personal preview (`docs/design/decisions-v2/02-policy-and-authority.md` §4): before the owner changes a matrix cell
//! or adds a rule, the phone replays the change over its own last 30 days and says what it would have changed: "Last
//! 30 days: 12 Auto → Ask, 0 Ask → Auto".
//!
//! Each evaluated request keeps what the matrix judged it by ([`PreviewInput`]) inside its sealed suggestion, which
//! the store sweep drops after 30 days. The replay is [`reins_policy::simulate::simulate_personal`], on the phone
//! only: nothing is sent, and the input is never copied into Activity.

use reins_policy::context::Resolved;
use reins_policy::effects::Analysis;
use reins_policy::simulate::{PersonalCase, simulate_personal};
use serde::{Deserialize, Serialize};

use super::store::SUGGESTION_RETENTION_SECS;
use super::types::{CellChoice, ZoneChoice};
use crate::CoreError;
use crate::engine::Engine;
use crate::store::unix_now;

/// What the matrix judged one request by (no body: its effects, zone and the connection asking).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewInput {
    pub analysis: Analysis,
    pub resolved: Resolved,
    pub connection: String,
    #[serde(default)]
    pub seen_resource: bool,
}

/// A change to preview.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PolicyChange {
    /// One matrix cell (`level` 0 to 3).
    SetCell { zone: ZoneChoice, level: u8, cell: CellChoice },
    /// The rules "Ask me next time" on this Activity entry would add.
    AskMeNextTime { request_id: String },
}

/// What a change would have done on the phone's own history.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PreviewView {
    /// "Last 30 days: 12 Auto → Ask, 0 Ask → Auto", or "No history yet".
    pub line: String,
    /// Requests whose outcome would have differed.
    pub changed: u32,
    /// The days looked at.
    pub days: u32,
}

/// The days a preview looks back (what suggestions are kept for).
pub const PREVIEW_DAYS: u32 = 30;

impl Engine {
    /// Replays `change` over this phone's evaluated requests of the last [`PREVIEW_DAYS`] days. Reads only.
    pub fn preview_policy_change(&self, change: &PolicyChange) -> Result<PreviewView, CoreError> {
        let now = unix_now();
        let mut base = self.ap_snapshot()?;
        self.ap_tasks_into(&mut base, now);
        let mut draft = base.clone();
        match change {
            PolicyChange::SetCell {
                zone,
                level,
                cell,
            } => {
                let level = match level {
                    0 => reins_policy::effects::Level::L0,
                    1 => reins_policy::effects::Level::L1,
                    2 => reins_policy::effects::Level::L2,
                    3 => reins_policy::effects::Level::L3,
                    _ => return Err(CoreError::invalid("a level is 0 to 3")),
                };
                draft.matrix.set((*zone).into(), level, (*cell).into());
            }
            PolicyChange::AskMeNextTime {
                request_id,
            } => {
                let s = self.store.ap_suggestion(request_id)?.ok_or(CoreError::NotFound)?;
                let note = s.matrix.ok_or_else(|| CoreError::invalid("Auto has nothing to change for this entry"))?;
                draft.rules.extend(super::policy::exception_rules(request_id, &note, now));
            }
        }
        let cases: Vec<PersonalCase> = self
            .store
            .ap_suggestions_since(now - SUGGESTION_RETENTION_SECS)?
            .into_iter()
            .filter_map(|s| {
                let input = *s.matrix?.input?;
                Some(PersonalCase {
                    id: s.request_id,
                    at: s.at,
                    analysis: input.analysis,
                    resolved: input.resolved,
                    connection: input.connection,
                    seen_resource: input.seen_resource,
                })
            })
            .collect();
        if cases.is_empty() {
            return Ok(PreviewView {
                line: "No history yet".to_owned(),
                changed: 0,
                days: PREVIEW_DAYS,
            });
        }
        let r = simulate_personal(&cases, &base, &draft);
        let changed = r.auto_to_review + r.review_to_auto + r.to_deny + r.from_deny;
        Ok(PreviewView {
            line: format!(
                "Last {PREVIEW_DAYS} days: {} Auto → Ask, {} Ask → Auto",
                r.auto_to_review, r.review_to_auto
            ),
            changed,
            days: PREVIEW_DAYS,
        })
    }
}
