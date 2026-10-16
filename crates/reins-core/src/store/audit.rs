//! Append-only audit log. Details are encrypted.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Store;
use crate::CoreError;
use crate::autopilot::{AutopilotMode, Verdict};

/// Rows kept; older rows are deleted on append.
pub const AUDIT_RETENTION: i64 = 5_000;
/// Shown when a detail can no longer be decrypted (data key lost).
pub const DETAIL_UNAVAILABLE: &str = "(details unavailable)";
const DETAIL_AAD: &str = "audit.detail";
pub(super) const INFO_AAD: &str = "audit.info";
const SNAPSHOT_AAD: &str = "audit.snapshot";
/// The most bytes an entry's snapshot may take (JSON, before sealing); a larger one is not kept.
pub const MAX_SNAPSHOT: usize = 256 * 1024;
/// Snapshots kept: those of the newest entries only (each can be up to [`MAX_SNAPSHOT`]).
pub const SNAPSHOT_RETENTION: i64 = 1_000;
/// The most characters of an item's text kept in a snapshot: what its approval row shows.
const SNAPSHOT_ITEM_TEXT: usize = 400;

/// What is kept of a request's approval: as it was shown, without what must not be kept (a secret item's text, a
/// vault call's values), at most [`MAX_SNAPSHOT`] bytes of JSON.
pub(crate) fn snapshot_json(parked: &crate::views::ParkedRequest) -> Option<Vec<u8>> {
    let mut p = parked.clone();
    if let reins_proto::gmail::ToolCall::Connector(call) = &mut p.request.call
        && call.service == reins_proto::connector::VAULT
    {
        call.args.retain(|name, _| crate::titles::VAULT_ARGS.contains(&name.as_str()));
    }
    if let Some(held) = p.connector.as_mut() {
        for item in &mut held.items {
            if item.secret {
                item.body = None;
                item.extra.clear();
            } else {
                item.body = item.body.take().map(|b| crate::text::truncate_chars(&b, SNAPSHOT_ITEM_TEXT));
            }
        }
    }
    let json = serde_json::to_vec(&p).ok()?;
    (json.len() <= MAX_SNAPSHOT).then_some(json)
}

/// Stores entry `seq`'s sealed snapshot (if any) and prunes those past [`SNAPSHOT_RETENTION`].
fn keep_snapshot(conn: &rusqlite::Connection, seq: i64, sealed: Option<&[u8]>) -> rusqlite::Result<()> {
    if let Some(sealed) = sealed {
        conn.execute("INSERT OR REPLACE INTO audit_snapshots (seq, data) VALUES (?1, ?2)", params![seq, sealed])?;
    }
    conn.execute("DELETE FROM audit_snapshots WHERE seq <= ?1 - ?2", params![seq, SNAPSHOT_RETENTION])?;
    Ok(())
}

/// One message of a released search/read, as shown in the activity details (headers only, never bodies).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditMessage {
    /// Gmail message id (empty for entries from before ids were kept).
    #[serde(default)]
    pub id: String,
    /// Another integration: what was shared of the item (message text, event details). Sealed with the rest.
    #[serde(default)]
    pub text: String,
    pub from: String,
    pub subject: String,
    pub date: i64,
}

/// The email that was sent (or approved to be sent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEmail {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// What Autopilot knew when the entry's request was decided (spec §8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditAutopilot {
    /// The request, so that a correction can find what Autopilot saw.
    pub request_id: String,
    pub mode: AutopilotMode,
    /// What Autopilot did, or would have done.
    pub suggested: Verdict,
    pub p_approve: f32,
    pub p_deny: f32,
    pub confidence: f32,
    pub profile_id: String,
    pub profile_name: String,
    #[serde(default)]
    pub neighbours: Vec<String>,
    #[serde(default)]
    pub reason: String,
    /// The risk, when the model rated it (`docs/autopilot-risk.md`).
    #[serde(default)]
    pub risk: Option<crate::autopilot::risk::RiskNote>,
}

/// What the activity screen shows when an entry is opened. Sealed with the data key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditInfo {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub messages: Vec<AuditMessage>,
    #[serde(default)]
    pub email: Option<AuditEmail>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub grant_summary: Option<String>,
    /// Accounts shown to the AI.
    #[serde(default)]
    pub accounts: Vec<String>,
    /// Who decided: "" (the user, or nobody: a grant covered it), "autopilot", "bypass" or "lockdown".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub decided_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot: Option<AuditAutopilot>,
    /// Another integration: the things the request touched (chats, calendars, a repository), sorted; how an approval
    /// is matched with earlier ones of the same thing (`crate::quick`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    /// What the request was, as the screens show it (`crate::titles`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Box<crate::types::RequestLabel>>,
    /// What it asked, in full (the entry's "Full request").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Box<AuditRequest>>,
    /// A push: its refs, commits and files (and their changes while small), kept with the entry so that every device
    /// of the account can show them (see [`super::AuditPush`]). Shown only: nothing decides by it.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "super::audit_push::lenient")]
    pub git: Option<Box<super::AuditPush>>,
    /// The request as its approval showed it (or would have, when it was decided without asking), so the entry's page
    /// can show the same thing. Kept apart from the rest (see [`Store::audit_snapshot`]), never in this JSON.
    #[serde(skip)]
    pub snapshot: Option<Box<crate::views::ParkedRequest>>,
}

/// What a request asked, in full, kept with its entry. Each text is cleaned ([`crate::text::neutralize`]) and at most
/// [`crate::titles::MAX_FIELD`] bytes; a secret's value is never among them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub full: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default)]
    pub asked_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuditRecord {
    /// Row number; 0 when appending.
    pub seq: i64,
    pub at: i64,
    pub connection_id: String,
    pub connection_label: String,
    /// "search" | "read" | "send" | "pair"
    pub action: String,
    /// "released" | "sent" | "granted" | "denied" | "error"
    pub outcome: String,
    pub detail: String,
    pub grant_id: Option<String>,
    pub service: String,
    pub account: Option<String>,
    /// How many messages / recipients the operation covered.
    pub count: u32,
    pub info: AuditInfo,
    /// The operation on a connector besides Gmail ("read", "send", ...), else empty.
    pub op: String,
}

impl Store {
    /// Appends an entry (stamped with who is deciding, see [`crate::autopilot::context`]); returns its row number.
    pub fn append_audit(&self, record: &AuditRecord) -> Result<i64, CoreError> {
        let detail = self.seal(DETAIL_AAD, record.detail.as_bytes())?;
        let mut info = record.info.clone();
        crate::autopilot::context::stamp(&mut info);
        let info_json = serde_json::to_vec(&info).map_err(|e| CoreError::storage(e.to_string()))?;
        let info = self.seal(INFO_AAD, &info_json)?;
        // The approval's snapshot is extra: losing it never fails the entry (nor the decision that writes it).
        let snapshot = record.info.snapshot.as_deref().and_then(snapshot_json).and_then(|json| {
            self.seal(SNAPSHOT_AAD, &json)
                .map_err(|e| log::error!("an activity entry is kept without its approval: {e}"))
                .ok()
        });
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO audit (at, connection_id, connection_label, action, outcome, grant_id, detail, service, account, \
             item_count, info, op) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                record.at,
                record.connection_id,
                record.connection_label,
                record.action,
                record.outcome,
                record.grant_id,
                detail,
                record.service,
                record.account,
                record.count,
                info,
                record.op
            ],
        )?;
        let seq = conn.last_insert_rowid();
        conn.execute("DELETE FROM audit WHERE seq <= (SELECT MAX(seq) FROM audit) - ?1", params![AUDIT_RETENTION])?;
        if let Err(e) = keep_snapshot(&conn, seq, snapshot.as_deref()) {
            log::error!("an activity entry is kept without its approval: {e}");
        }
        drop(conn);
        crate::autopilot::context::written(seq);
        Ok(seq)
    }

    /// One entry by its row number.
    pub fn audit_entry(&self, seq: i64) -> Result<Option<AuditRecord>, CoreError> {
        Ok(self.audit_rows("WHERE seq = ?1", params![seq])?.pop())
    }

    /// The request of entry `seq` as its approval showed it; `None` for an entry without one (logged before they were
    /// kept, too large, pruned, or not about a request).
    pub fn audit_snapshot(&self, seq: i64) -> Result<Option<crate::views::ParkedRequest>, CoreError> {
        let blob: Option<Vec<u8>> = {
            let conn = self.read()?;
            // Extra like its writing: an entry whose approval cannot be read still opens.
            conn.query_row("SELECT data FROM audit_snapshots WHERE seq = ?1", params![seq], |r| r.get(0))
                .optional()
                .unwrap_or_else(|e| {
                    log::error!("an activity entry's approval cannot be read: {e}");
                    None
                })
        };
        Ok(blob.and_then(|b| self.open_snapshot(&b)))
    }

    /// Newest first, at most `limit` rows.
    pub fn activity(&self, limit: u32) -> Result<Vec<AuditRecord>, CoreError> {
        self.audit_rows("ORDER BY seq DESC LIMIT ?1", params![i64::from(limit)])
    }

    /// One connection's entries logged at or after `since`, newest first (at most 500).
    pub fn audit_since(&self, connection_id: &str, since: i64) -> Result<Vec<AuditRecord>, CoreError> {
        self.audit_rows(
            "WHERE connection_id = ?1 AND at >= ?2 ORDER BY seq DESC LIMIT 500",
            params![connection_id, since],
        )
    }

    pub(super) fn audit_rows(&self, clause: &str, param: impl rusqlite::Params) -> Result<Vec<AuditRecord>, CoreError> {
        let rows = sealed_rows(&*self.read()?, clause, param)?;
        Ok(rows.into_iter().map(|row| self.open_audit(row)).collect())
    }

    /// An entry as read from its row: its sealed parts opened (what cannot be is shown as unavailable).
    pub(super) fn open_audit(&self, row: SealedAudit) -> AuditRecord {
        let detail = row
            .detail
            .and_then(|blob| self.unseal(DETAIL_AAD, &blob).ok())
            .and_then(|plain| String::from_utf8(plain).ok())
            .unwrap_or_else(|| DETAIL_UNAVAILABLE.to_owned());
        let info = row
            .info
            .and_then(|blob| self.unseal(INFO_AAD, &blob).ok())
            .and_then(|plain| serde_json::from_slice(&plain).ok())
            .unwrap_or_default();
        AuditRecord {
            seq: row.seq,
            at: row.at,
            connection_id: row.connection_id,
            connection_label: row.connection_label,
            action: row.action,
            outcome: row.outcome,
            detail,
            grant_id: row.grant_id,
            service: row.service,
            account: row.account,
            count: row.count,
            info,
            op: row.op,
        }
    }

    /// An entry's sealed approval, opened; `None` when it cannot be.
    pub(super) fn open_snapshot(&self, blob: &[u8]) -> Option<crate::views::ParkedRequest> {
        self.unseal(SNAPSHOT_AAD, blob).ok().and_then(|plain| serde_json::from_slice(&plain).ok())
    }
}

/// An entry's row as stored, its details still sealed.
pub(super) struct SealedAudit {
    pub seq: i64,
    at: i64,
    connection_id: String,
    connection_label: String,
    action: String,
    outcome: String,
    grant_id: Option<String>,
    detail: Option<Vec<u8>>,
    service: String,
    account: Option<String>,
    count: u32,
    info: Option<Vec<u8>>,
    op: String,
}

/// The rows `clause` selects from `audit`, sealed: read under the lock, opened after letting it go.
pub(super) fn sealed_rows(
    conn: &rusqlite::Connection,
    clause: &str,
    param: impl rusqlite::Params,
) -> rusqlite::Result<Vec<SealedAudit>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT seq, at, connection_id, connection_label, action, outcome, grant_id, detail, service, account, \
         item_count, info, op FROM audit {clause}"
    ))?;
    stmt.query_map(param, |r| {
        Ok(SealedAudit {
            seq: r.get(0)?,
            at: r.get(1)?,
            connection_id: r.get(2)?,
            connection_label: r.get(3)?,
            action: r.get(4)?,
            outcome: r.get(5)?,
            grant_id: r.get(6)?,
            detail: r.get(7)?,
            service: r.get(8)?,
            account: r.get(9)?,
            count: r.get(10)?,
            info: r.get(11)?,
            op: r.get(12)?,
        })
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::open;

    fn record(at: i64) -> AuditRecord {
        AuditRecord {
            seq: 0,
            at,
            connection_id: "c1".to_owned(),
            connection_label: "ChatGPT".to_owned(),
            action: "send".to_owned(),
            outcome: "sent".to_owned(),
            detail: format!("to a@work.com #{at}"),
            grant_id: Some("g1".to_owned()),
            service: "gmail".to_owned(),
            account: Some("me@gmail.com".to_owned()),
            count: 1,
            op: String::new(),
            info: AuditInfo {
                email: Some(AuditEmail {
                    to: vec!["a@work.com".to_owned()],
                    cc: vec![],
                    subject: format!("Report {at}"),
                    body: "Hello".to_owned(),
                }),
                ..AuditInfo::default()
            },
        }
    }

    #[test]
    fn newest_first_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        for at in 1..=3 {
            store.append_audit(&record(at)).unwrap();
        }
        let rows = store.activity(2).unwrap();
        let strip = |mut r: AuditRecord| {
            r.seq = 0;
            r
        };
        assert_eq!(rows.into_iter().map(strip).collect::<Vec<_>>(), vec![record(3), record(2)]);
    }

    #[test]
    fn retention_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        for at in 0..AUDIT_RETENTION + 3 {
            store.append_audit(&record(at)).unwrap();
        }
        let count: i64 = store.lock().unwrap().query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0)).unwrap();
        assert_eq!(count, AUDIT_RETENTION);
        assert_eq!(store.activity(1).unwrap()[0].at, AUDIT_RETENTION + 2);
    }

    fn parked(call: serde_json::Value, items: Vec<crate::connector::Item>) -> crate::views::ParkedRequest {
        let call: reins_proto::connector::ConnectorCall = serde_json::from_value(call).unwrap();
        crate::views::ParkedRequest {
            request: reins_proto::relay::RelayRequest {
                v: 1,
                id: "r1".into(),
                connection_id: "c1".into(),
                connection_label: "laptop".to_owned(),
                created_at: 9,
                wait_until: None,
                account: None,
                call: reins_proto::gmail::ToolCall::Connector(call),
            },
            messages: Vec::new(),
            covered: std::collections::BTreeMap::default(),
            account: None,
            accounts: Vec::new(),
            shared_accounts: Vec::new(),
            connector: Some(crate::views::ParkedConnector {
                items,
                ..crate::views::ParkedConnector::default()
            }),
        }
    }

    #[test]
    fn an_entry_keeps_its_approval_apart_and_without_secret_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let mut rec = record(1);
        let call = serde_json::json!({"service": "vault", "op": "item_create",
            "args": {"name": "Bank", "password": "hunter2", "notes": "pin 1234"}});
        let item = crate::connector::Item {
            id: "i1".into(),
            title: "Bank".into(),
            snippet: "Password for Bank".into(),
            body: Some("hunter2".into()),
            secret: true,
            ..crate::connector::Item::default()
        };
        rec.info.snapshot = Some(Box::new(parked(call, vec![item])));
        let seq = store.append_audit(&rec).unwrap();
        let kept = store.audit_snapshot(seq).unwrap().expect("kept");
        let shown = format!("{kept:?}");
        assert!(shown.contains("Bank") && !shown.contains("hunter2") && !shown.contains("1234"), "{shown}");
        // Never in the entry's own info (loaded with every list).
        let info: Vec<u8> =
            store.lock().unwrap().query_row("SELECT info FROM audit WHERE seq = ?1", [seq], |r| r.get(0)).unwrap();
        let info: AuditInfo = serde_json::from_slice(&store.unseal(INFO_AAD, &info).unwrap()).unwrap();
        assert!(info.snapshot.is_none());
    }

    #[test]
    fn a_snapshot_is_bounded_and_older_entries_have_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let old = store.append_audit(&record(1)).unwrap();
        assert!(store.audit_snapshot(old).unwrap().is_none(), "an entry from before snapshots loads without one");
        assert!(store.audit_entry(old).unwrap().is_some());
        let big = (0..100)
            .map(|i| crate::connector::Item {
                id: format!("i{i}"),
                title: "x".repeat(3_000),
                ..crate::connector::Item::default()
            })
            .collect();
        let mut rec = record(2);
        rec.info.snapshot =
            Some(Box::new(parked(serde_json::json!({"service": "telegram", "op": "read", "args": {}}), big)));
        let seq = store.append_audit(&rec).unwrap();
        assert!(store.audit_snapshot(seq).unwrap().is_none(), "more than {MAX_SNAPSHOT} bytes is not kept");
        let mut small = record(3);
        let long_body = crate::connector::Item {
            id: "i".into(),
            body: Some("y".repeat(10_000)),
            ..crate::connector::Item::default()
        };
        small.info.snapshot = Some(Box::new(parked(
            serde_json::json!({"service": "telegram", "op": "read", "args": {}}),
            vec![long_body],
        )));
        let seq = store.append_audit(&small).unwrap();
        let kept = store.audit_snapshot(seq).unwrap().unwrap();
        assert_eq!(kept.connector.unwrap().items[0].body.as_ref().unwrap().len(), SNAPSHOT_ITEM_TEXT);
    }

    /// Entries written before pushes were kept read without one; a version without the field (the same type without
    /// `git`) reads entries that have it, as it ignores what it does not know; and a push that does not read never
    /// costs the entry its other details.
    #[test]
    fn a_kept_push_is_compatible_both_ways_and_never_costs_the_entry_its_details() {
        let old: AuditInfo = serde_json::from_str(r#"{"note": "n", "decided_by": "bypass"}"#).unwrap();
        assert!(old.git.is_none());
        let call = crate::store::audit_push::tests::push_call(&crate::store::audit_push::tests::summary(1, 1, 2));
        let info = AuditInfo {
            git: crate::store::AuditPush::of_call(&call).map(Box::new),
            decided_by: "bypass".to_owned(),
            ..AuditInfo::default()
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json.get("git").is_some());
        let mut newer = json.clone();
        newer["from_a_newer_version"] = serde_json::json!({"x": 1});
        assert_eq!(serde_json::from_value::<AuditInfo>(newer).unwrap(), info, "unknown fields are ignored");
        let mut broken = json;
        broken["git"] = serde_json::json!({"repo": "me/app", "summary": {"updates": "not a list"}});
        let read: AuditInfo = serde_json::from_value(broken).unwrap();
        assert_eq!((read.git, read.decided_by.as_str()), (None, "bypass"));
        // Written and read back through the store.
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let mut rec = record(1);
        rec.info.git.clone_from(&info.git);
        let seq = store.append_audit(&rec).unwrap();
        assert_eq!(store.audit_entry(seq).unwrap().unwrap().info.git, info.git);
        assert!(store.audit_entry(store.append_audit(&record(2)).unwrap()).unwrap().unwrap().info.git.is_none());
    }

    #[test]
    fn snapshots_are_pruned_with_their_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        for at in 0..SNAPSHOT_RETENTION + 5 {
            let mut rec = record(at);
            rec.info.snapshot = Some(Box::new(parked(
                serde_json::json!({"service": "telegram", "op": "read", "args": {}}),
                Vec::new(),
            )));
            store.append_audit(&rec).unwrap();
        }
        let count: i64 =
            store.lock().unwrap().query_row("SELECT COUNT(*) FROM audit_snapshots", [], |r| r.get(0)).unwrap();
        assert_eq!(count, SNAPSHOT_RETENTION);
    }
}
