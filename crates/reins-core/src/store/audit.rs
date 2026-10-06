//! Append-only audit log. Details are encrypted.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Store;
use crate::CoreError;
use crate::autopilot::{AutopilotMode, Verdict};

/// Rows kept; older rows are deleted on append.
pub const AUDIT_RETENTION: i64 = 5_000;
/// Shown when a detail can no longer be decrypted (data key lost).
pub const DETAIL_UNAVAILABLE: &str = "(details unavailable)";
const DETAIL_AAD: &str = "audit.detail";
const INFO_AAD: &str = "audit.info";

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
        drop(conn);
        crate::autopilot::context::written(seq);
        Ok(seq)
    }

    /// One entry by its row number.
    pub fn audit_entry(&self, seq: i64) -> Result<Option<AuditRecord>, CoreError> {
        Ok(self.audit_rows("WHERE seq = ?1", seq)?.pop())
    }

    /// Newest first, at most `limit` rows.
    pub fn activity(&self, limit: u32) -> Result<Vec<AuditRecord>, CoreError> {
        self.audit_rows("ORDER BY seq DESC LIMIT ?1", i64::from(limit))
    }

    fn audit_rows(&self, clause: &str, param: i64) -> Result<Vec<AuditRecord>, CoreError> {
        struct Row {
            seq: i64,
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
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT seq, at, connection_id, connection_label, action, outcome, grant_id, detail, service, account, \
             item_count, info, op FROM audit {clause}"
        ))?;
        let rows = stmt
            .query_map(params![param], |r| {
                Ok(Row {
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
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);
        Ok(rows
            .into_iter()
            .map(|row| {
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
            })
            .collect())
    }
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
}
