//! The org audit log (`docs/design/decisions-v2/06-trust-and-compliance.md` §5): append-only and hash-chained per
//! tenant, with signed checkpoints and a verifiable export.
//!
//! The chain is O1's admin event table (`reins_org_events`), taken over as it is: its rows keep their hashes, so the
//! events O1 wrote verify unchanged, and O2 appends decisions, reviews, executions, leases, policy and executor changes
//! to the same chain ([`kinds`]). Every 10 minutes ([`CHECKPOINT_SECS`]) a chain that grew gets a checkpoint signed by
//! the decision service key, which an RFC 3161 authority then timestamps ([`super::witness`]). An export ends with a signed head, so `reins-audit verify`
//! (`crates/reins-audit`) catches an edit, a gap, a reordering or a cut tail, and a rewrite against any checkpoint kept
//! from before.

use std::sync::LazyLock;

use diesel::prelude::*;
use reins_proto::{
    managed::{Checkpoint, Envelope, EnvelopeKind, ExportLine},
    org::Tier,
};
use serde_json::Value;

use super::{
    OrgError,
    kms::{self, KeyPurpose},
};
use crate::db::{
    DbConnInner,
    models::{
        reins_org, reins_org_compliance, reins_org_event,
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
    /// O3: whom an obligation's relationships, on-call schedule or escalation step named, with the source's evidence.
    pub const REVIEW_RESOLVED: &str = "review.resolved";
    /// O3: an obligation's window passed and it reached its next reviewers.
    pub const REVIEW_ESCALATED: &str = "review.escalated";
    /// O3: managers, group backups, on-call sources, channels or the GitHub reader changed (a two-person change).
    pub const ROUTING_CHANGED: &str = "routing.changed";
    /// O3: a review answered through a signed web approval link (the link spent).
    pub const WEB_APPROVAL: &str = "review.web";
    /// O3: a member's web passkey added (unconfirmed), confirmed on their phone, or revoked.
    pub const PASSKEY_ADDED: &str = "passkey.added";
    /// O3: a Slack or Teams app's secrets posted (still off until a two-person change).
    pub const CHANNEL_INSTALLED: &str = "channel.installed";
    pub const PASSKEY_CONFIRMED: &str = "passkey.confirmed";
    pub const PASSKEY_REVOKED: &str = "passkey.revoked";
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
    pub const AUDIT_READ: &str = "audit.read";
    /// O5: events below a signed checkpoint deleted after the tier's retention (the checkpoint stays as the anchor).
    pub const AUDIT_PRUNED: &str = "audit.pruned";
    pub const POLICY_SIMULATED: &str = "policy.simulated";
    pub const POLICY_STAGE_ADVANCED: &str = "policy.stage_advanced";
    pub const POLICY_ROLLED_BACK: &str = "policy.rolled_back";
    pub const POLICY_ROLLOUT_COMPLETED: &str = "policy.rollout_completed";
    pub const POLICY_ROLLOUT_PAUSED: &str = "policy.rollout_paused";
    pub const POLICY_ROLLOUT_RESUMED: &str = "policy.rollout_resumed";
    pub const POLICY_REPO_BOUND: &str = "policy_repo.bound";
    pub const POLICY_REPO_CHECKED: &str = "policy_repo.checked";
    pub const GITHUB_READER_SET: &str = "github_reader.set";
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

/// Checkpoints `org`'s chain when it grew since the last checkpoint: signed and stored ([`super::witness`] has it
/// timestamped later, outside any transaction).
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

fn stored_envelope(cp: &ReinsOrgCheckpoint) -> Result<Envelope, OrgError> {
    serde_json::from_str(&cp.envelope).map_err(|e| OrgError::Internal(format!("stored checkpoint: {e}")))
}

/// Export lines and the last event's sequence number and hash.
type Lines = (Vec<String>, Option<(i64, String)>);

/// The export lines of `org`'s events after `after` up to `upto` (all when `None`), each stored checkpoint after the
/// event it covers; with the anchor checkpoint at `after` first when `after` isn't the genesis. Returns the lines and
/// the last event's sequence number and hash.
fn lines_between(c: &mut DbConnInner, org: &str, after: i64, upto: Option<i64>) -> Result<Lines, OrgError> {
    let mut out: Vec<String> = Vec::new();
    let mut last = if after > 0 {
        let anchor = reins_org_exec::q_checkpoint(c, org, after)?.ok_or(OrgError::NotFound)?;
        out.push(line(&ExportLine::Checkpoint(stored_envelope(&anchor)?)));
        Some((anchor.seq, anchor.hash))
    } else {
        None
    };
    let mut checkpoints = reins_org_exec::q_checkpoints(c, org, after)?.into_iter().peekable();
    let mut after = after;
    'pages: loop {
        let page = reins_org_event::q_events_from(c, org, after, PAGE)?;
        if page.is_empty() {
            break;
        }
        for e in &page {
            if upto.is_some_and(|u| e.seq > u) {
                break 'pages;
            }
            out.push(line(&ExportLine::Event(e.to_audit())));
            while let Some(cp) = checkpoints.next_if(|cp| cp.seq <= e.seq) {
                if cp.seq == e.seq {
                    out.push(line(&ExportLine::Checkpoint(stored_envelope(&cp)?)));
                }
            }
            after = e.seq;
            last = Some((e.seq, e.hash.clone()));
        }
    }
    Ok((out, last))
}

/// Where an export of the whole kept chain starts: the genesis (0), or the prune anchor (the checkpoint at the first
/// kept event).
fn kept_from(c: &mut DbConnInner, org: &str) -> Result<i64, OrgError> {
    Ok(match reins_org_event::q_first(c, org)? {
        Some((first, _)) if first > 1 => first,
        _ => 0,
    })
}

fn finish(mut out: Vec<String>, head: Envelope) -> String {
    out.push(line(&ExportLine::Head(head)));
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// `org`'s audit export as JSON Lines: from the genesis (or, once pruned, from the checkpoint pruning kept as the
/// anchor), or from the stored checkpoint at `from` (its first line); the stored checkpoints after the event they
/// cover; and a freshly signed head as the last line. Records `audit.exported` (by `actor`) after building it, so the
/// export's head is the chain just before that event.
pub fn export(
    c: &mut DbConnInner,
    org: &str,
    from: Option<i64>,
    actor: Option<&str>,
    now: i64,
) -> Result<String, OrgError> {
    kms::signing_key(c, org, KeyPurpose::Decision, now)?;
    let after = match from {
        Some(seq) => seq,
        None => kept_from(c, org)?,
    };
    let (out, last) = lines_between(c, org, after, None)?;
    let (seq, hash) = last.ok_or(OrgError::NotFound)?;
    let (_, head) = signed_head(c, org, EnvelopeKind::Head, seq, &hash, now)?;
    let text = finish(out, head);
    record(c, org, kinds::AUDIT_EXPORTED, actor, None, &serde_json::json!({"from": from, "to": seq}), now)?;
    Ok(text)
}

/// The export of `org`'s events after the stored checkpoint at `after` (the genesis when `None`) up to the stored
/// checkpoint at `to`, ending with a freshly signed head at `to`. It records nothing: audit streaming calls it every
/// few minutes, and each segment verifies alone with `reins-audit verify --anchor <the checkpoint at after>`.
pub fn segment(c: &mut DbConnInner, org: &str, after: Option<i64>, to: i64, now: i64) -> Result<String, OrgError> {
    let end = reins_org_exec::q_checkpoint(c, org, to)?.ok_or(OrgError::Invalid("A segment ends at a checkpoint."))?;
    let after = after.unwrap_or(0);
    if after == 0 && kept_from(c, org)? > 0 {
        return Err(OrgError::Invalid("The chain was pruned: start from its anchor."));
    }
    if to <= after {
        return Err(OrgError::Invalid("A segment ends after it starts."));
    }
    let (out, last) = lines_between(c, org, after, Some(to))?;
    if last.as_ref().map(|(s, h)| (*s, h.as_str())) != Some((end.seq, end.hash.as_str())) {
        return Err(OrgError::Internal(format!("checkpoint {to} of {org} is not on the stored chain")));
    }
    let (_, head) = signed_head(c, org, EnvelopeKind::Head, end.seq, &end.hash, now)?;
    Ok(finish(out, head))
}

/// How long an org's audit events are kept, in days (05 §11, 06 §4): Team 30, Business a year, Enterprise seven.
#[must_use]
pub fn audit_retention_days(tier: Tier) -> i64 {
    match tier {
        Tier::Team => 30,
        Tier::Business => 365,
        Tier::Enterprise => 7 * 365,
    }
}

/// Deletes `org`'s events older than its tier keeps, down to (not including) the newest checkpoint signed before the
/// cut-off, which stays as the chain's anchor: exports then start from it and still verify. Refuses a chain that
/// doesn't verify as stored (the evidence of an edit must stay). Returns the anchor's sequence number when it pruned.
pub fn prune(c: &mut DbConnInner, org: &str, now: i64) -> Result<Option<i64>, OrgError> {
    let tenant = reins_org::q_org(c, org)?.ok_or(OrgError::NotFound)?;
    let days = audit_retention_days(tenant.tier());
    let Some((first, _)) = reins_org_event::q_first(c, org)? else {
        return Ok(None);
    };
    let Some(anchor) = reins_org_exec::q_checkpoint_at_or_before(c, org, now - days * 86_400)? else {
        return Ok(None);
    };
    if anchor.seq <= first {
        return Ok(None);
    }
    if !intact(c, org)? {
        return Err(OrgError::Internal(format!("the audit chain of {org} does not verify: not pruned")));
    }
    c.transaction(|c| {
        let deleted = reins_org_event::q_delete_before(c, org, anchor.seq)?;
        reins_org_exec::q_delete_checkpoints_before(c, org, anchor.seq)?;
        reins_org_compliance::q_delete_witness_before(c, org, anchor.seq)?;
        record(
            c,
            org,
            kinds::AUDIT_PRUNED,
            None,
            None,
            &serde_json::json!({"anchor": anchor.seq, "hash": anchor.hash, "deleted": deleted, "days": days}),
            now,
        )?;
        Ok(Some(anchor.seq))
    })
}

static LAST_PRUNE: LazyLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> = LazyLock::new(Default::default);

/// Prunes every org, each at most once a day.
pub fn prune_due(c: &mut DbConnInner, now: i64) -> Result<usize, OrgError> {
    let orgs: Vec<String> = reins_orgs::table.select(reins_orgs::org_uuid).load(c)?;
    let mut pruned = 0;
    for org in orgs {
        {
            let mut last = LAST_PRUNE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if last.get(&org).is_some_and(|t| now - t < 86_400) {
                continue;
            }
            last.insert(org.clone(), now);
        }
        match prune(c, &org, now) {
            Ok(Some(_)) => pruned += 1,
            Ok(None) => {}
            Err(e) => warn!("Audit pruning of {org}: {e:?}"),
        }
    }
    Ok(pruned)
}

/// Whether `org`'s kept chain verifies as stored (the console's "Chain intact"): from the genesis, or from the first
/// kept event, which must be the prune anchor's.
pub fn intact(c: &mut DbConnInner, org: &str) -> Result<bool, OrgError> {
    let Some((first, first_hash)) = reins_org_event::q_first(c, org)? else {
        return Ok(true);
    };
    let mut after = first - 1;
    let mut prev: Option<String> = if first == 1 {
        Some(reins_proto::org::GENESIS_HASH.to_owned())
    } else {
        match reins_org_exec::q_checkpoint(c, org, first)? {
            Some(cp) if cp.hash == first_hash => None,
            _ => return Ok(false),
        }
    };
    loop {
        let page = reins_org_event::q_events_from(c, org, after, PAGE)?;
        if page.is_empty() {
            return Ok(true);
        }
        for e in page {
            let a = e.to_audit();
            let linked = prev.as_ref().is_none_or(|p| *p == e.prev_hash);
            if e.seq != after + 1 || !linked || a.expected_hash() != e.hash {
                return Ok(false);
            }
            after = e.seq;
            prev = Some(e.hash);
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

    fn tenant(c: &mut DbConnInner, org: &str) {
        reins_org::tests::add_org(c, org);
    }

    const DAY: i64 = 86_400;

    #[test]
    fn a_pruned_chain_exports_from_its_anchor_and_verifies() {
        use_test_kms();
        let mut c = super::super::testing::db();
        tenant(&mut c, "o1");
        some_events(&mut c, "o1", 5);
        let anchor = checkpoint(&mut c, "o1", 100).unwrap().unwrap();
        some_events(&mut c, "o1", 4);
        checkpoint(&mut c, "o1", 200).unwrap().unwrap();
        // Business keeps a year: a year and 50 s after it, the checkpoint signed at 100 is old enough, the one at 200 not.
        let now = 150 + 365 * DAY;
        assert_eq!(prune(&mut c, "o1", now).unwrap(), Some(anchor.seq));
        let (first, _) = reins_org_event::q_first(&mut c, "o1").unwrap().unwrap();
        assert_eq!(first, anchor.seq, "the anchor event stays");
        let text = export(&mut c, "o1", None, None, now).unwrap();
        let first_line: ExportLine = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert!(matches!(first_line, ExportLine::Checkpoint(_)), "{first_line:?}");
        let k = keys(&mut c, "o1");
        let r = verify(text.as_bytes(), &k, Some(&anchor)).unwrap();
        assert_eq!(r.first_seq, anchor.seq + 1);
        assert!(intact(&mut c, "o1").unwrap());
        // The prune is itself on the chain, and the chain still grows.
        let kinds: Vec<String> =
            reins_org_event::q_events(&mut c, "o1", None, 3).unwrap().into_iter().map(|e| e.kind).collect();
        assert!(kinds.iter().any(|k| k == kinds::AUDIT_PRUNED), "{kinds:?}");
        record(&mut c, "o1", "x.y", None, None, &json!({}), now + 1).unwrap();
        assert!(intact(&mut c, "o1").unwrap());
        // A second prune at the same time finds nothing new.
        assert_eq!(prune(&mut c, "o1", now).unwrap(), None);
    }

    #[test]
    fn nothing_is_pruned_without_an_old_enough_checkpoint() {
        use_test_kms();
        let mut c = super::super::testing::db();
        tenant(&mut c, "o1");
        some_events(&mut c, "o1", 5);
        checkpoint(&mut c, "o1", 100).unwrap().unwrap();
        assert_eq!(prune(&mut c, "o1", 100 + 364 * DAY).unwrap(), None);
        let (first, _) = reins_org_event::q_first(&mut c, "o1").unwrap().unwrap();
        assert_eq!(first, 1);
        // Without any checkpoint, nothing either.
        tenant(&mut c, "o2");
        some_events(&mut c, "o2", 3);
        assert_eq!(prune(&mut c, "o2", 10 * 365 * DAY).unwrap(), None);
    }

    #[test]
    fn retention_follows_the_tier() {
        assert_eq!(audit_retention_days(Tier::Team), 30);
        assert_eq!(audit_retention_days(Tier::Business), 365);
        assert_eq!(audit_retention_days(Tier::Enterprise), 7 * 365);
    }

    #[test]
    fn pruning_never_crosses_into_another_tenant() {
        use_test_kms();
        let mut c = super::super::testing::db();
        tenant(&mut c, "o1");
        tenant(&mut c, "o2");
        some_events(&mut c, "o1", 4);
        some_events(&mut c, "o2", 4);
        checkpoint(&mut c, "o1", 100).unwrap().unwrap();
        checkpoint(&mut c, "o2", 100 + 400 * DAY).unwrap().unwrap();
        assert!(prune(&mut c, "o1", 101 + 366 * DAY).unwrap().is_some());
        assert_eq!(prune(&mut c, "o2", 101 + 366 * DAY).unwrap(), None);
        assert_eq!(reins_org_event::q_first(&mut c, "o2").unwrap().unwrap().0, 1);
        assert!(intact(&mut c, "o2").unwrap());
    }

    #[test]
    fn a_tampered_chain_is_never_pruned() {
        use crate::db::schema::reins_org_events;
        use_test_kms();
        let mut c = super::super::testing::db();
        tenant(&mut c, "o1");
        some_events(&mut c, "o1", 4);
        checkpoint(&mut c, "o1", 100).unwrap().unwrap();
        diesel::update(
            reins_org_events::table.filter(reins_org_events::org_uuid.eq("o1")).filter(reins_org_events::seq.eq(2)),
        )
        .set(reins_org_events::detail.eq("{\"i\":99}"))
        .execute(&mut c)
        .unwrap();
        assert!(prune(&mut c, "o1", 101 + 366 * DAY).is_err(), "the evidence of the edit must stay");
        assert_eq!(reins_org_event::q_first(&mut c, "o1").unwrap().unwrap().0, 1);
    }

    #[test]
    fn editing_the_first_kept_event_breaks_intact() {
        use crate::db::schema::reins_org_events;
        use_test_kms();
        let mut c = super::super::testing::db();
        tenant(&mut c, "o1");
        some_events(&mut c, "o1", 4);
        let anchor = checkpoint(&mut c, "o1", 100).unwrap().unwrap();
        some_events(&mut c, "o1", 2);
        prune(&mut c, "o1", 101 + 366 * DAY).unwrap().unwrap();
        diesel::update(
            reins_org_events::table
                .filter(reins_org_events::org_uuid.eq("o1"))
                .filter(reins_org_events::seq.eq(anchor.seq)),
        )
        .set(reins_org_events::detail.eq("{\"i\":99}"))
        .execute(&mut c)
        .unwrap();
        assert!(!intact(&mut c, "o1").unwrap());
    }

    #[test]
    fn a_segment_verifies_against_its_anchor_and_records_nothing() {
        use_test_kms();
        let mut c = super::super::testing::db();
        some_events(&mut c, "o1", 3);
        let first = checkpoint(&mut c, "o1", 20).unwrap().unwrap();
        some_events(&mut c, "o1", 3);
        let second = checkpoint(&mut c, "o1", 30).unwrap().unwrap();
        some_events(&mut c, "o1", 1);
        let head = reins_org_event::q_head(&mut c, "o1").unwrap().unwrap();
        let k = keys(&mut c, "o1");
        let whole = segment(&mut c, "o1", None, first.seq, 40).unwrap();
        let r = verify(whole.as_bytes(), &k, None).unwrap();
        assert_eq!((r.first_seq, r.last_seq), (1, first.seq));
        let next = segment(&mut c, "o1", Some(first.seq), second.seq, 41).unwrap();
        let r = verify(next.as_bytes(), &k, Some(&first)).unwrap();
        assert_eq!((r.first_seq, r.last_seq), (first.seq + 1, second.seq));
        assert_eq!(reins_org_event::q_head(&mut c, "o1").unwrap().unwrap(), head, "nothing recorded");
        // Only between stored checkpoints.
        assert!(segment(&mut c, "o1", Some(first.seq), head.0, 42).is_err());
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
