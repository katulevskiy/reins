//! Approval routing (`docs/design/decisions-v2/05-organizations.md` §6): who an org review obligation names, resolved
//! from verified sources only.

use reins_policy::org::Reviewers;

use super::OrgError;
use crate::db::{DbConnInner, models::reins_org};

/// A group's name, or "a group".
pub fn group_name(c: &mut DbConnInner, org: &str, group: &str) -> Result<String, OrgError> {
    Ok(reins_org::q_groups(c, org)?
        .into_iter()
        .find(|(id, _, _)| id == group)
        .map_or_else(|| "a group".to_owned(), |g| g.1))
}

/// Who a reviewer set is, in short words ("Manager", "Code owners", "On-call: payments").
pub fn label(c: &mut DbConnInner, org: &str, r: &Reviewers) -> Result<String, OrgError> {
    Ok(match r {
        Reviewers::Requester => "The requester".to_owned(),
        Reviewers::Group(g) => group_name(c, org, g)?,
        Reviewers::Role(role) => role.label().to_owned(),
        Reviewers::Members(_) => "Named reviewers".to_owned(),
        Reviewers::Manager => "Manager".to_owned(),
        Reviewers::CodeOwners => "Code owners".to_owned(),
        Reviewers::ResourceOwner {
            tag,
        } => format!("Owner ({tag})"),
        Reviewers::OnCall {
            alias,
        } => format!("On-call: {alias}"),
        Reviewers::BackupOf(g) => format!("Backup of {}", group_name(c, org, g)?),
        Reviewers::AnyOf(list) => {
            let mut parts = Vec::with_capacity(list.len());
            for x in list {
                parts.push(label(c, org, x)?);
            }
            parts.join(" or ")
        }
    })
}
