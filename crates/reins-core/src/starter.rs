//! The starting policy: what a newly connected AI may do before it has asked for anything. Chosen in one screen during
//! setup and changed under Grants. The default, "ask every time", gives it nothing. "Headlines for its first hour"
//! gives the new connection ordinary grants (listed under Grants, revocable, logged) to see, for one hour, the headlines
//! of what its searches and lists find: for Gmail the sender, recipients, subject and date of each message (never a
//! snippet, a body or an attachment), elsewhere the names and titles a list shows (never what is inside). Opening
//! anything still asks, as do sending, changing or deleting anything, the vault, the desktop app's questions and secrets,
//! and the hard floor. Nothing that looks like a code or a password is ever released by a grant, these included.
//!
//! The grants are [`reins_policy::Scope::Headlines`], which no read, write or other check ever looks at: only the Gmail
//! search (`Engine::decide_read`) and the lists of other integrations (`connector::flow`) consult them, and both strip
//! what they release down to the metadata ([`strip_to_headline`], [`headlines_only`]).

use reins_policy::{Grant, HeadlinesScope, Scope};
use reins_proto::connector::{ConnectorCall, Effect};
use reins_proto::gmail::MessageSummary;
use reins_proto::ids::{ConnectionId, GrantId};

use crate::CoreError;
use crate::connector::Item;
use crate::engine::Engine;
use crate::store::{AuditInfo, AuditRecord, Store};
use crate::types::StartingPolicy;
use crate::views;

/// How long the headlines of a new connection are allowed.
pub const STARTER_SECS: i64 = 3_600;
/// The grants' origin, as `GrantView.origin` shows it.
pub const ORIGIN: &str = "starter";
const META_KEY: &str = "starting_policy";
const ASK_EVERY_TIME: &str = "ask_every_time";
const HEADLINES_FIRST_HOUR: &str = "headlines_first_hour";
/// The rule older versions offered ("let it read for a day"). It is no longer given: whoever chose it now asks every
/// time (never something wider), and the grants it gave stay until they end.
const RETIRED_READS_FOR_A_DAY: &str = "reads_for_a_day";
/// Integrations a starting grant never covers: the vault (names of logins, secrets), the desktop app (its questions,
/// secrets and SSH sign-ins are asked every time anyway), files uploaded for one operation and payments (cards and
/// purchases are not headlines).
const NEVER: [&str; 4] =
    ["vault", reins_proto::connector::DESKTOP, crate::blob::SERVICE_FILES, reins_proto::connector::PAYMENTS];

/// Whether the headlines of a call may be covered by a starting grant: a list (names only) on an integration a
/// starting grant may cover. Gmail's own lists (labels, filters, attachments) are not headlines: only its search is.
pub(crate) fn headlines_cover(call: &ConnectorCall) -> bool {
    call.effect() == Some(Effect::List)
        && call.service != views::SERVICE_GMAIL
        && !NEVER.contains(&call.service.as_str())
        && !call.spec().is_some_and(|s| s.desktop_only || s.once_only)
}

/// Reduces a message found by a search to its headline: who, to whom, what about and when. The snippet (taken from
/// the body) is dropped.
pub(crate) fn strip_to_headline(m: &mut MessageSummary) {
    m.snippet.clear();
}

/// An item of a list, reduced to its name: the id, the title, who it is from and when. Its text, its content and every
/// other field are dropped.
pub(crate) fn headlines_only(item: &Item) -> Item {
    Item {
        id: item.id.clone(),
        resource: item.resource.clone(),
        resource_label: item.resource_label.clone(),
        from: item.from.clone(),
        title: item.title.clone(),
        date: item.date,
        sensitive: item.sensitive,
        ..Item::default()
    }
}

impl Engine {
    /// What a new connection may do at first; `None` until the user chose (setup or Grants), which acts as "ask every
    /// time". The retired "reads for a day" (stored by an older version, or brought back with a backup) becomes "ask
    /// every time" here, and is stored so.
    pub fn starting_policy(&self) -> Result<Option<StartingPolicy>, CoreError> {
        read_policy(&self.store)
    }

    /// Applies to connections made from now on; grants already given stay until they end or are revoked.
    pub fn set_starting_policy(&self, policy: StartingPolicy) -> Result<(), CoreError> {
        write_policy(&self.store, policy)
    }

    /// The grants "headlines for its first hour" gives a connection the user just approved: one per connected
    /// integration that has headlines (every account of it), each for [`STARTER_SECS`]. Returns how many.
    pub(crate) fn grant_starter_headlines(&self, connection: &str, label: &str, now: i64) -> Result<usize, CoreError> {
        let connection_id = ConnectionId(connection.to_owned());
        let expires = Some(now + STARTER_SECS);
        let mut grants = Vec::new();
        for service in self.integrations() {
            if !has_headlines(&service) {
                continue;
            }
            let scope = Scope::Headlines(HeadlinesScope {
                service,
            });
            grants.push(
                Grant::new(GrantId(uuid::Uuid::new_v4().to_string()), connection_id.clone(), scope, now, expires, None)
                    .map_err(|e| CoreError::invalid(e.to_string()))?,
            );
        }
        for g in &grants {
            self.store.insert_grant_from(g, label, ORIGIN)?;
        }
        if !grants.is_empty() {
            let summary = format!("headlines for 1 hour ({})", plural(grants.len()));
            self.store.append_audit(&AuditRecord {
                seq: 0,
                at: now,
                connection_id: connection.to_owned(),
                connection_label: label.to_owned(),
                action: "grant".to_owned(),
                outcome: "granted".to_owned(),
                detail: format!("allowed: {summary}"),
                grant_id: grants.first().map(|g| g.id.0.clone()),
                service: String::new(),
                account: None,
                count: u32::try_from(grants.len()).unwrap_or(u32::MAX),
                op: String::new(),
                info: AuditInfo {
                    grant_summary: Some(summary),
                    note: Some(
                        "From your starting rule: headlines for its first hour. Opening anything still asks."
                            .to_owned(),
                    ),
                    ..AuditInfo::default()
                },
            })?;
        }
        Ok(grants.len())
    }
}

fn read_policy(store: &Store) -> Result<Option<StartingPolicy>, CoreError> {
    Ok(match store.meta_get(META_KEY)?.as_deref() {
        Some(HEADLINES_FIRST_HOUR) => Some(StartingPolicy::HeadlinesFirstHour),
        Some(ASK_EVERY_TIME) => Some(StartingPolicy::AskEveryTime),
        Some(RETIRED_READS_FOR_A_DAY) => {
            write_policy(store, StartingPolicy::AskEveryTime)?;
            Some(StartingPolicy::AskEveryTime)
        }
        _ => None,
    })
}

fn write_policy(store: &Store, policy: StartingPolicy) -> Result<(), CoreError> {
    store.meta_set(
        META_KEY,
        match policy {
            StartingPolicy::AskEveryTime => ASK_EVERY_TIME,
            StartingPolicy::HeadlinesFirstHour => HEADLINES_FIRST_HOUR,
        },
    )
}

/// Whether a starting grant may cover `service`: Gmail (its search), or an integration with a list.
fn has_headlines(service: &str) -> bool {
    if NEVER.contains(&service) {
        return false;
    }
    service == views::SERVICE_GMAIL
        || reins_proto::connector::specs()
            .iter()
            .any(|s| s.service == service && s.effect == Effect::List && !s.desktop_only && !s.once_only)
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 integration".to_owned()
    } else {
        format!("{n} integrations")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lists_outside_gmail_and_never_the_vault_have_headlines() {
        let call = |service: &str, op: &str| ConnectorCall {
            service: service.to_owned(),
            op: op.to_owned(),
            args: serde_json::Map::new(),
        };
        assert!(headlines_cover(&call("telegram", "list_chats")));
        assert!(!headlines_cover(&call("telegram", "read")));
        assert!(!headlines_cover(&call("telegram", "search")));
        assert!(!headlines_cover(&call("telegram", "send")));
        assert!(!headlines_cover(&call("gmail", "list_attachments")), "attachments are not headlines");
        assert!(!headlines_cover(&call("gmail", "list_filters")));
        assert!(has_headlines("gmail") && has_headlines("telegram") && has_headlines("github"));
        assert!(!has_headlines("vault") && !has_headlines("payments") && !has_headlines("desktop"));
    }

    #[test]
    fn the_retired_reads_for_a_day_becomes_ask_every_time_and_is_stored_so() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::open(dir.path());
        assert_eq!(read_policy(&store).unwrap(), None, "not chosen: asks every time");
        store.meta_set(META_KEY, RETIRED_READS_FOR_A_DAY).unwrap();
        assert_eq!(read_policy(&store).unwrap(), Some(StartingPolicy::AskEveryTime), "never kept, never widened");
        assert_eq!(store.meta_get(META_KEY).unwrap().as_deref(), Some(ASK_EVERY_TIME), "migrated in place");
        write_policy(&store, StartingPolicy::HeadlinesFirstHour).unwrap();
        assert_eq!(read_policy(&store).unwrap(), Some(StartingPolicy::HeadlinesFirstHour));
        store.meta_set(META_KEY, "something_newer").unwrap();
        assert_eq!(read_policy(&store).unwrap(), None, "an unknown rule gives nothing");
    }

    #[test]
    fn a_headline_keeps_the_metadata_and_drops_the_content() {
        let item = Item {
            id: "9".to_owned(),
            title: "Family".to_owned(),
            from: "Anna".to_owned(),
            snippet: "the door code is 4821".to_owned(),
            body: Some("all of it".to_owned()),
            date: 7,
            extra: serde_json::json!({"last": "hello"}).as_object().unwrap().clone(),
            ..Item::default()
        };
        let h = headlines_only(&item);
        assert_eq!((h.id.as_str(), h.title.as_str(), h.from.as_str(), h.date), ("9", "Family", "Anna", 7));
        assert!(h.snippet.is_empty() && h.body.is_none() && h.extra.is_empty());
    }
}
