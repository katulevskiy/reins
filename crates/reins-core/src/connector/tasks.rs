//! Tasks as an integration (`reins_proto::connector::tasks`): what the approval card shows of a proposal. Applying
//! one, and ending one, are the core's own (`crate::autopilot::tasks`, `connector::flow`).

use reins_proto::connector::ConnectorCall;
use reins_proto::connector::tasks::{CEILINGS, draft_of};
use serde_json::Value;

use super::{Connector, Preview};
use crate::CoreError;

/// The account a task is under: the connection itself.
pub const TASKS_ACCOUNT: &str = "tasks";

#[derive(Default)]
pub struct Tasks;

fn join(list: &[String]) -> String {
    list.join(", ")
}

#[async_trait::async_trait]
impl Connector for Tasks {
    fn service(&self) -> &'static str {
        reins_proto::connector::TASKS
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let d = draft_of(call).map_err(CoreError::invalid)?;
        let mut lines = vec![format!("Task: {}", crate::text::one_line(&crate::text::mark_invisible(&d.title)))];
        if !d.repos.is_empty() {
            lines.push(format!("Repos: {}", join(&d.repos)));
        }
        if !d.branches.is_empty() {
            lines.push(format!("Branches: {}", join(&d.branches)));
        }
        if !d.paths.is_empty() {
            lines.push(format!("Folders: {}", join(&d.paths)));
        }
        lines.push(if d.people.is_empty() {
            "People: none".to_owned()
        } else {
            format!("People: {}", join(&d.people))
        });
        if !d.operations.is_empty() {
            lines.push(format!("Kinds: {}", join(&d.operations)));
        }
        let ceiling = d.ceiling.map_or("default", |c| CEILINGS[usize::from(c).min(2)]);
        lines.push(format!("Up to: {}", ceiling.replace('_', " ")));
        lines.push(format!("For {} min", d.minutes));
        if let Some(b) = d.budget {
            lines.push(format!("At most {b} requests"));
        }
        Ok(Preview {
            resource: "task".to_owned(),
            resource_label: "Task".to_owned(),
            lines,
            once_only: true,
            ..Preview::default()
        })
    }

    async fn status(&self, _account: &str) -> crate::types::GmailStatus {
        crate::types::GmailStatus::Ready
    }

    async fn perform(&self, _account: &str, _call: &ConnectorCall) -> Result<Value, CoreError> {
        // Applied by the core from the approval (`connector::flow`), never through here.
        Err(CoreError::service("a task is started by its approval"))
    }
}
