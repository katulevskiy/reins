//! Escalation chains (05 §6: "Each obligation has a window; on expiry it escalates (backup group, manager, then
//! Security admin)").
//!
//! [`escalate_due`] runs first in the decision service's sweep. For each unmet obligation whose step passed its
//! deadline it takes the next step of `escalate` (or the org default chain), skipping steps that add nobody:
//! - it records the step with a new deadline (the step's group window, else the obligation's) and moves the request's
//!   deadline to cover it;
//! - it **adds** the step's members to the resolution (never removes anyone);
//! - it tells only the new members, and records `review.escalated {obligation, step, to}`.
//!
//! With no step left the request expires (`review.expired`): "Expired: not approved in time." It never auto-approves
//! and never falls to the requester.

use std::collections::{BTreeMap, BTreeSet};

use reins_policy::org::{Met, Obligation, Reviewers};
use serde_json::{Value, json};

use super::{Ctx, Directory, Dynamic, Touches, dynamic_key, record_step, resolve, stored};
use crate::{
    api::reins::org::{OrgError, audit, kms, policy, reviews},
    db::{
        DbConnInner,
        models::{
            reins_org_request::{self, ReinsOrgRequest},
            reins_org_routing,
        },
    },
};

/// One escalation: whom to tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escalated {
    pub org: String,
    pub request: String,
    pub obligation: String,
    pub step: usize,
    pub to: Vec<String>,
    /// The step's reviewers, in words ("Security admin").
    pub label: String,
}

/// What the sweep did with one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Swept {
    Escalated(Escalated),
    /// No step left: expired.
    Expired {
        org: String,
        request: String,
    },
}

/// The request's call, from its sealed body: (tool, args).
fn call_of(c: &mut DbConnInner, row: &ReinsOrgRequest) -> Result<(String, Value), OrgError> {
    let body = reins_org_request::q_body(c, &row.org_uuid, &row.uuid, "request")?.ok_or(OrgError::NotFound)?;
    let plain = kms::open_data(c, &row.org_uuid, &body.key_id, &format!("{}/request", row.uuid), &body.sealed)?;
    let v: Value = serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(e.to_string()))?;
    Ok((v["tool"].as_str().unwrap_or_default().to_owned(), v["args"].clone()))
}

/// The obligations of `row` waiting past their current step's deadline, with that step's number.
fn due(c: &mut DbConnInner, row: &ReinsOrgRequest, now: i64) -> Result<Vec<(Obligation, usize)>, OrgError> {
    let steps = reins_org_routing::q_steps(c, &row.uuid)?;
    if steps.is_empty() {
        return Ok(Vec::new());
    }
    let Met::Waiting(waiting) = reviews::met(c, &row.org_uuid, row)? else {
        return Ok(Vec::new());
    };
    let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
    let mut out = Vec::new();
    for o in obligations.into_iter().filter(|o| waiting.contains(&o.id)) {
        let last = steps.iter().filter(|s| s.obligation == o.id).max_by_key(|s| s.step);
        if let Some(last) = last
            && last.deadline <= now
        {
            out.push((o, usize::try_from(last.step).unwrap_or(usize::MAX)));
        }
    }
    Ok(out)
}

/// What the dynamic sources must answer before a sweep: org → (tool, args, dynamic reviewer sets) per due request.
pub fn due_sets(c: &mut DbConnInner, now: i64) -> Result<Vec<(String, String, Value, Vec<Reviewers>)>, OrgError> {
    let mut out = Vec::new();
    for row in reins_org_request::q_reviewing(c)? {
        let mut sets = Vec::new();
        for (o, step) in due(c, &row, now)? {
            for r in o.spec.chain().iter().skip(step).flat_map(Reviewers::parts) {
                if matches!(r, Reviewers::CodeOwners | Reviewers::OnCall { .. }) && !sets.contains(r) {
                    sets.push(r.clone());
                }
            }
        }
        if !sets.is_empty() {
            let (tool, args) = call_of(c, &row)?;
            out.push((row.org_uuid.clone(), tool, args, sets));
        }
    }
    Ok(out)
}

/// Escalates every obligation past its deadline (see the module docs). `dynamic`: org → what code owners and on-call
/// said for this sweep.
pub fn escalate_due(
    c: &mut DbConnInner,
    now: i64,
    dynamic: &BTreeMap<String, Dynamic>,
) -> Result<Vec<Swept>, OrgError> {
    let mut out = Vec::new();
    let none = Dynamic::new();
    for row in reins_org_request::q_reviewing(c)? {
        let due = due(c, &row, now)?;
        if due.is_empty() {
            continue;
        }
        let org = row.org_uuid.clone();
        let Some((policy, _)) = policy::active(c, &org)? else {
            continue;
        };
        let (tool, args) = call_of(c, &row)?;
        let touches = Touches {
            repo: super::repo_of(&tool, &args),
            paths: None,
        };
        let req = reins_policy::org::ManagedRequest {
            org: &org,
            member: &row.user_uuid,
            connection: &row.connection_uuid,
            tool: &tool,
            args: &args,
            now,
            reason: None,
        };
        let resources = super::resources_of(&policy, &req);
        let dynamic = dynamic.get(&org).unwrap_or(&none);
        // Code owners asked for a PR merge carry their paths in the key: fall back to the repository's.
        let dynamic: Dynamic = dynamic
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain(
                dynamic
                    .iter()
                    .filter(|(k, _)| k.starts_with("codeowners:"))
                    .map(|(_, v)| (dynamic_key(&Reviewers::CodeOwners, &touches).unwrap_or_default(), v.clone())),
            )
            .collect();
        let ctx = Ctx {
            org: &org,
            requester: &row.user_uuid,
            policy: &policy,
            resources: &resources,
            touches: &touches,
            dynamic: &dynamic,
        };
        let dir = Directory::load(c, &org, now)?;
        let mut resolved = stored(row.resolved.as_deref());
        let mut deadline = row.expires_at;
        let mut exhausted = false;
        for (o, step) in due {
            let chain = o.spec.chain();
            let have: BTreeSet<String> = resolved.get(&o.id).cloned().unwrap_or_default();
            let mut taken = None;
            for (i, r) in chain.iter().enumerate().skip(step) {
                let next = resolve(&dir, &ctx, r, true);
                let eligible_before = dir.eligible(&o, &row.user_uuid, Some(&have));
                let new: BTreeSet<String> = next
                    .members
                    .iter()
                    .filter(|m| !have.contains(*m) && !eligible_before.contains(*m))
                    .cloned()
                    .collect();
                if !new.is_empty() {
                    taken = Some((i + 1, r.clone(), next, new));
                    break;
                }
            }
            let Some((n, r, next, new)) = taken else {
                exhausted = true;
                break;
            };
            let until = now + dir.window(&r, &o.spec);
            record_step(c, &org, &row.uuid, &o.id, n, &r, &next, now, until)?;
            audit::record(
                c,
                &org,
                audit::kinds::REVIEW_ESCALATED,
                None,
                Some(&row.uuid),
                &json!({"obligation": o.id, "step": n, "to": new}),
                now,
            )?;
            resolved.entry(o.id.clone()).or_default().extend(new.iter().cloned());
            deadline = deadline.max(until);
            out.push(Swept::Escalated(Escalated {
                org: org.clone(),
                request: row.uuid.clone(),
                obligation: o.id.clone(),
                step: n,
                to: new.into_iter().collect(),
                label: super::label(c, &org, &r)?,
            }));
        }
        if exhausted {
            if crate::api::reins::org::decisions::expire(c, &org, &row, "escalation exhausted", now)? {
                out.retain(|s| !matches!(s, Swept::Escalated(e) if e.request == row.uuid));
                out.push(Swept::Expired {
                    org,
                    request: row.uuid.clone(),
                });
            }
            continue;
        }
        reins_org_request::q_set_resolved(c, &org, &row.uuid, &serde_json::to_string(&resolved).unwrap_or_default())?;
        if deadline != row.expires_at {
            reins_org_request::q_set_expiry(c, &org, &row.uuid, deadline)?;
        }
    }
    Ok(out)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::api::reins::org::{
        decisions,
        routing::{
            Resolution,
            tests::{approve, ask_with, resolved_of, routed_org, set_manager},
        },
        testing::db,
    };

    use crate::db::models::reins_org_request::request_state;

    fn state(c: &mut DbConnInner, id: &str) -> String {
        reins_org_request::q_request(c, "o1", id).unwrap().unwrap().state
    }

    fn backup(c: &mut DbConnInner, group: &str, to: &str, window: Option<i64>) {
        reins_org_routing::q_save_group_setting(
            c,
            &reins_org_routing::ReinsOrgGroupSetting {
                org_uuid: "o1".into(),
                group_uuid: group.into(),
                backup_group: Some(to.into()),
                window_secs: window,
            },
        )
        .unwrap();
    }

    const H4: i64 = 4 * 3600;

    #[test]
    fn deadlines_walk_the_chain_then_expire() {
        let mut c = db();
        routed_org(&mut c);
        backup(&mut c, "sec", "sec-backup", Some(600));
        set_manager(&mut c, "lee", "mia");
        // `prod` asks Security (Kim). Kim doesn't answer within Security's 10 minutes: the backup (Ola, the backup
        // group's window: the obligation's 4 h), then Lee's manager (Mia), then Security admins (only Kim, who
        // already could): nothing new, so it expires.
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 1_000);
        assert_eq!(s.expires_at, 1_600);
        let swept = escalate_due(&mut c, 1_599, &BTreeMap::new()).unwrap();
        assert!(swept.is_empty(), "not yet");
        let swept = escalate_due(&mut c, 1_600, &BTreeMap::new()).unwrap();
        assert!(matches!(&swept[..], [Swept::Escalated(e)] if e.to == ["ola"] && e.step == 1), "{swept:?}");
        assert!(escalate_due(&mut c, 1_600 + 600, &BTreeMap::new()).unwrap().is_empty(), "the backup's own window");
        let swept = escalate_due(&mut c, 1_600 + H4, &BTreeMap::new()).unwrap();
        assert!(matches!(&swept[..], [Swept::Escalated(e)] if e.to == ["mia"] && e.step == 2), "{swept:?}");
        let row = reins_org_request::q_request(&mut c, "o1", &s.id).unwrap().unwrap();
        assert_eq!(row.expires_at, 1_600 + 2 * H4);
        let swept = escalate_due(&mut c, 1_600 + 2 * H4, &BTreeMap::new()).unwrap();
        assert!(matches!(&swept[..], [Swept::Expired { .. }]), "{swept:?}");
        assert_eq!(state(&mut c, &s.id), request_state::EXPIRED);
        let st = decisions::status(&mut c, "o1", "conn-lee", &s.id, 99_999).unwrap();
        assert_eq!(st["reason"], "Expired: not approved in time.");
    }

    #[test]
    fn empty_steps_are_skipped() {
        let mut c = db();
        routed_org(&mut c);
        // No backup, no manager: Security's window passes, and the next step that adds anyone is Security admins
        // (Dana has no phone; Kim already could) — nobody new, so it expires at once.
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 1_000);
        let swept = escalate_due(&mut c, 1_000 + H4, &BTreeMap::new()).unwrap();
        assert!(matches!(&swept[..], [Swept::Expired { .. }]), "{swept:?}");
        // With a manager, the empty backup step is skipped and the manager's is taken.
        set_manager(&mut c, "lee", "mia");
        let s2 = ask_with(&mut c, "acme/prod", &Dynamic::new(), 2_000);
        let swept = escalate_due(&mut c, 2_000 + H4, &BTreeMap::new()).unwrap();
        assert!(matches!(&swept[..], [Swept::Escalated(e)] if e.to == ["mia"] && e.step == 2), "{swept:?}");
        assert_ne!(s.id, s2.id);
    }

    #[test]
    fn escalation_notifies_only_new_reviewers() {
        let mut c = db();
        routed_org(&mut c);
        set_manager(&mut c, "lee", "mia");
        backup(&mut c, "sec", "sec-backup", None);
        let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), 1_000);
        assert_eq!(s.notify, ["kim"]);
        let swept = escalate_due(&mut c, 1_000 + H4, &BTreeMap::new()).unwrap();
        let Swept::Escalated(e) = &swept[0] else {
            panic!("{swept:?}")
        };
        assert_eq!(e.to, ["ola"], "Kim isn't told again");
        assert_eq!(e.label, "Backup of Group sec");
    }

    #[test]
    fn earlier_reviewers_still_count() {
        let mut c = db();
        let phones = routed_org(&mut c);
        set_manager(&mut c, "lee", "mia");
        // On-call (Ola) is slow; it escalates to Mia. Ola still counts, and so does Mia.
        let mut d = Dynamic::new();
        d.insert("oncall:payments".into(), Resolution::from(["ola".to_owned()].into(), "oncall:pagerduty"));
        let s = ask_with(&mut c, "acme/pay", &d, 1_000);
        escalate_due(&mut c, 1_000 + H4, &BTreeMap::new()).unwrap();
        let r = resolved_of(&mut c, &s.id);
        assert_eq!(r["oncall"], ["mia".to_owned(), "ola".to_owned()].into());
        assert!(matches!(approve(&mut c, &phones, "ola", &s.id, 1_000 + H4 + 1).unwrap(), reviews::Answered::Approved));
    }
}
