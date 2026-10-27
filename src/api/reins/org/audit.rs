//! The org audit log (`docs/design/decisions-v2/06-trust-and-compliance.md` §5): append-only and hash-chained per
//! tenant, with signed checkpoints and a verifiable export.
//!
//! The chain is O1's admin event table (`reins_org_events`), taken over as it is: its rows keep their hashes, so the
//! events O1 wrote verify unchanged, and O2 appends decisions, reviews, executions, leases, policy and executor changes
//! to the same chain ([`kinds`]). Every 10 minutes ([`CHECKPOINT_SECS`]) a chain that grew gets a checkpoint signed by
//! the decision service key, handed to the [`Witness`]. An export ends with a signed head, so `reins-audit verify`
//! (`crates/reins-audit`) catches an edit, a gap, a reordering or a cut tail, and a rewrite against any checkpoint kept
//! from before.

use std::sync::{Arc, LazyLock, RwLock};

use diesel::prelude::*;
use reins_proto::managed::{Checkpoint, Envelope, EnvelopeKind, ExportLine};
use serde_json::Value;

use super::{
    OrgError,
    kms::{self, KeyPurpose},
};
use crate::db::{
    DbConnInner,
    models::{
        reins_org_event,
        reins_org_exec::{self, ReinsOrgCheckpoint},
    },
    schema::reins_orgs,
};

/// How often a grown chain is checkpointed.
pub const CHECKPOINT_SECS: i64 = 600;
/// Events read per page while exporting.
const PAGE: i64 = 1000;

/// The audit event kinds O2 adds (O1's stay as they are).
pub mod kinds {
    pub const REQUEST_RECEIVED: &str = "request.received";
    pub const DECISION_AUTO: &str = "decision.auto";
    pub const DECISION_APPROVED: &str = "decision.approved";
    pub const DECISION_DENIED: &str = "decision.denied";
    pub const REVIEW_OPENED: &str = "review.opened";
    pub const REVIEW_APPROVED: &str = "review.approved";
    pub const REVIEW_DENIED: &str = "review.denied";
    pub const REVIEW_EXPIRED: &str = "review.expired";
    pub const REQUEST_CANCELLED: &str = "request.cancelled";
    pub const REQUEST_EXECUTED: &str = "request.executed";
    pub const REQUEST_FAILED: &str = "request.failed";
    pub const REQUEST_REFUSED: &str = "request.refused";
    pub const REQUEST_STALE: &str = "request.stale";
    pub const REQUEST_UNKNOWN: &str = "request.unknown";
    pub const REQUEST_EXPIRED: &str = "request.expired";
    pub const LEASE_ISSUED: &str = "lease.issued";
    pub const LEASE_REVOKED: &str = "lease.revoked";
    pub const LEASE_REVOKED_UPSTREAM: &str = "lease.revoked_upstream";
    /// Nobody revoked a lease's token upstream (its executor was offline or removed) and GitHub's own lifetime for it
    /// has passed: it is dead, and leaves the revocation queue.
    pub const LEASE_TOKEN_LAPSED: &str = "lease.token_lapsed";
    pub const POLICY_DRAFTED: &str = "policy.drafted";
    pub const POLICY_ACTIVATED: &str = "policy.activated";
    pub const MEMBER_POLICY_SET: &str = "member_policy.set";
    pub const EXECUTOR_REGISTERED: &str = "executor.registered";
    pub const EXECUTOR_REMOVED: &str = "executor.removed";
    pub const RETENTION_SET: &str = "retention.set";
    pub const AUDIT_EXPORTED: &str = "audit.exported";
}

/// Appends an event to `org`'s chain; returns its sequence number.
pub fn record(
    c: &mut DbConnInner,
    org: &str,
    kind: &str,
    actor: Option<&str>,
    subject: Option<&str>,
    detail: &Value,
    now: i64,
) -> Result<i64, OrgError> {
    Ok(reins_org_event::append(c, org, kind, actor, subject, detail, now)?)
}

/// Where checkpoints are published so a rewrite of the whole chain shows (06 §5.2: a transparency log or an RFC 3161
/// timestamp). O2 publishes nowhere by default; O5 adds witnesses.
pub trait Witness: Send + Sync {
    fn publish(&self, org: &str, checkpoint: &Envelope) -> Result<(), String>;
}

/// Publishes nowhere.
pub struct NoWitness;

impl Witness for NoWitness {
    fn publish(&self, _org: &str, _checkpoint: &Envelope) -> Result<(), String> {
        Ok(())
    }
}

static WITNESS: LazyLock<RwLock<Arc<dyn Witness>>> = LazyLock::new(|| RwLock::new(Arc::new(NoWitness)));

/// Sets the witness (once, at launch).
#[allow(dead_code, reason = "the seam O5's witnesses plug into")]
pub fn set_witness(w: Arc<dyn Witness>) {
    *WITNESS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = w;
}

fn witness() -> Arc<dyn Witness> {
    WITNESS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

fn signed_head(
    c: &mut DbConnInner,
    org: &str,
    kind: EnvelopeKind,
    seq: i64,
    hash: &str,
    now: i64,
) -> Result<(Checkpoint, Envelope), OrgError> {
    let cp = Checkpoint {
        org: org.to_owned(),
        seq,
        hash: hash.to_owned(),
        at: now,
    };
    let env = kms::envelope(c, org, KeyPurpose::Decision, kind, &serde_json::to_value(&cp).unwrap_or_default(), now)?;
    Ok((cp, env))
}

/// Checkpoints `org`'s chain when it grew since the last checkpoint: signed, stored, handed to the witness.
pub fn checkpoint(c: &mut DbConnInner, org: &str, now: i64) -> Result<Option<Checkpoint>, OrgError> {
    // The key first: creating it is itself an event, which the checkpoint then covers.
    kms::signing_key(c, org, KeyPurpose::Decision, now)?;
    let Some((seq, hash)) = reins_org_event::q_head(c, org)? else {
        return Ok(None);
    };
    if reins_org_exec::q_last_checkpoint(c, org)?.is_some_and(|last| last.seq >= seq) {
        return Ok(None);
    }
    let (cp, env) = signed_head(c, org, EnvelopeKind::Checkpoint, seq, &hash, now)?;
    reins_org_exec::q_insert_checkpoint(
        c,
        &ReinsOrgCheckpoint {
            org_uuid: org.to_owned(),
            seq,
            hash,
            at: now,
            envelope: serde_json::to_string(&env).unwrap_or_default(),
        },
    )?;
    if let Err(e) = witness().publish(org, &env) {
        warn!("Audit checkpoint {seq} of {org} not witnessed: {e}");
    }
    Ok(Some(cp))
}

/// Checkpoints every org whose chain grew and whose last checkpoint is older than [`CHECKPOINT_SECS`].
pub fn checkpoint_due(c: &mut DbConnInner, now: i64) -> Result<usize, OrgError> {
    let orgs: Vec<String> = reins_orgs::table.select(reins_orgs::org_uuid).load(c)?;
    let mut made = 0;
    for org in orgs {
        if reins_org_exec::q_last_checkpoint(c, &org)?.is_some_and(|last| now - last.at < CHECKPOINT_SECS) {
            continue;
        }
        if checkpoint(c, &org, now)?.is_some() {
            made += 1;
        }
    }
    Ok(made)
}

fn line(l: &ExportLine) -> String {
    serde_json::to_string(l).unwrap_or_default()
}

/// `org`'s audit export as JSON Lines: from the genesis, or from the stored checkpoint at `from` (its first line);
/// the stored checkpoints after the event they cover; and a freshly signed head as the last line. Records
/// `audit.exported` (by `actor`) after building it, so the export's head is the chain just before that event.
pub fn export(
    c: &mut DbConnInner,
    org: &str,
    from: Option<i64>,
    actor: Option<&str>,
    now: i64,
) -> Result<String, OrgError> {
    kms::signing_key(c, org, KeyPurpose::Decision, now)?;
    let mut out: Vec<String> = Vec::new();
    let (mut after, mut last) = match from {
        Some(seq) => {
            let anchor = reins_org_exec::q_checkpoint(c, org, seq)?.ok_or(OrgError::NotFound)?;
            let env: Envelope = serde_json::from_str(&anchor.envelope)
                .map_err(|e| OrgError::Internal(format!("stored checkpoint: {e}")))?;
            out.push(line(&ExportLine::Checkpoint(env)));
            (seq, Some((anchor.seq, anchor.hash)))
        }
        None => (0, None),
    };
    let mut checkpoints = reins_org_exec::q_checkpoints(c, org, after)?.into_iter().peekable();
    loop {
        let page = reins_org_event::q_events_from(c, org, after, PAGE)?;
        if page.is_empty() {
            break;
        }
        for e in &page {
            out.push(line(&ExportLine::Event(e.to_audit())));
            while let Some(cp) = checkpoints.next_if(|cp| cp.seq <= e.seq) {
                if cp.seq == e.seq {
                    let env: Envelope = serde_json::from_str(&cp.envelope)
                        .map_err(|err| OrgError::Internal(format!("stored checkpoint: {err}")))?;
                    out.push(line(&ExportLine::Checkpoint(env)));
                }
            }
            after = e.seq;
            last = Some((e.seq, e.hash.clone()));
        }
    }
    let (seq, hash) = last.ok_or(OrgError::NotFound)?;
    let (_, head) = signed_head(c, org, EnvelopeKind::Head, seq, &hash, now)?;
    out.push(line(&ExportLine::Head(head)));
    record(c, org, kinds::AUDIT_EXPORTED, actor, None, &serde_json::json!({"from": from, "to": seq}), now)?;
    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// Whether `org`'s whole chain verifies as stored (the console's "Chain intact").
pub fn intact(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    let mut after = 0;
    let mut prev = reins_proto::org::GENESIS_HASH.to_owned();
    loop {
        let page = reins_org_event::q_events_from(c, org, after, PAGE)?;
        if page.is_empty() {
            return Ok(true);
        }
        for e in page {
            let a = e.to_audit();
            if e.seq != after + 1 || e.prev_hash != prev || a.expected_hash() != e.hash {
                return Ok(false);
            }
            after = e.seq;
            prev = e.hash;
        }
    }
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_audit::{VerifyError, verify};
    use serde_json::json;

    use super::*;
    use crate::api::reins::org::kms::tests::{trusted, use_test_kms};

    fn keys(c: &mut DbConnInner, org: &str) -> Vec<reins_audit::TrustedKey> {
        vec![trusted(&kms::signing_key(c, org, KeyPurpose::Decision, 1).unwrap())]
    }

    fn some_events(c: &mut DbConnInner, org: &str, n: i64) {
        for i in 0..n {
            let kind = if i % 2 == 0 {
                "member.provisioned"
            } else {
                kinds::DECISION_AUTO
            };
            record(c, org, kind, Some("u1"), Some("s"), &json!({"i": i}), 10 + i).unwrap();
        }
    }

    #[test]
    fn an_export_verifies_with_reins_audit() {
        use_test_kms();
        let mut c = super::super::testing::db();
        some_events(&mut c, "o1", 3);
        let cp = checkpoint(&mut c, "o1", 20).unwrap().unwrap();
        some_events(&mut c, "o1", 2);
        let text = export(&mut c, "o1", None, Some("u9"), 30).unwrap();
        let k = keys(&mut c, "o1");
        let report = verify(text.as_bytes(), &k, Some(&cp)).unwrap();
        assert_eq!((report.first_seq, report.checkpoints), (1, 1));
        // The export itself is recorded after its head.
        let (head_seq, _) = reins_org_event::q_head(&mut c, "o1").unwrap().unwrap();
        assert_eq!(head_seq, report.last_seq + 1);
        // From the checkpoint.
        let partial = export(&mut c, "o1", Some(cp.seq), None, 31).unwrap();
        let r = verify(partial.as_bytes(), &k, Some(&cp)).unwrap();
        assert_eq!(r.first_seq, cp.seq + 1);
        assert!(intact(&mut c, "o1").unwrap());
    }

    #[test]
    fn editing_a_stored_event_breaks_the_export() {
        use crate::db::schema::reins_org_events;
        use_test_kms();
        let mut c = super::super::testing::db();
        some_events(&mut c, "o1", 4);
        diesel::update(
            reins_org_events::table.filter(reins_org_events::org_uuid.eq("o1")).filter(reins_org_events::seq.eq(3)),
        )
        .set(reins_org_events::detail.eq("{\"i\":99}"))
        .execute(&mut c)
        .unwrap();
        let text = export(&mut c, "o1", None, None, 30).unwrap();
        let k = keys(&mut c, "o1");
        assert_eq!(
            verify(text.as_bytes(), &k, None),
            Err(VerifyError::BadHash {
                seq: 3
            })
        );
        assert!(!intact(&mut c, "o1").unwrap());
    }

    #[test]
    fn checkpoints_only_when_there_is_something_new() {
        use_test_kms();
        let mut c = super::super::testing::db();
        assert!(checkpoint(&mut c, "o1", 1).unwrap().is_some(), "the key's creation is the first event");
        assert!(checkpoint(&mut c, "o1", 2).unwrap().is_none());
        some_events(&mut c, "o1", 1);
        assert!(checkpoint(&mut c, "o1", 3).unwrap().is_some());
        assert!(checkpoint(&mut c, "o1", 4).unwrap().is_none());
    }

    #[test]
    fn an_export_never_includes_another_tenant() {
        use_test_kms();
        let mut c = super::super::testing::db();
        some_events(&mut c, "o1", 2);
        some_events(&mut c, "o2", 3);
        checkpoint(&mut c, "o2", 5).unwrap();
        let text = export(&mut c, "o1", None, None, 30).unwrap();
        assert!(!text.contains("\"o2\""), "{text}");
        let k = keys(&mut c, "o1");
        assert_eq!(verify(text.as_bytes(), &k, None).unwrap().org, "o1");
        // Another tenant's checkpoint can't anchor this export.
        assert!(export(&mut c, "o1", Some(4), None, 31).is_err());
    }
}
