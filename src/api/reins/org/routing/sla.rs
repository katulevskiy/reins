//! SLA reports (05 §6: "SLA reports show time to decision per group"; §10: time to decision, expired reviews).
//!
//! Time to decision is `decided_at − created_at` of a reviewed request. Each obligation counts toward its group (the
//! group's name), else its kind ("Manager", "Code owners", "On-call: payments"); the overall line counts each request
//! once.

use std::collections::BTreeMap;

use reins_policy::org::{Obligation, Reviewers};
use reins_proto::org::Capability;
use rocket::{Route, serde::json::Json};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    api::reins::{
        now_unix,
        org::{OrgError, enabled, require},
    },
    auth::Headers,
    db::{
        DbConn, DbConnInner,
        models::{
            reins_org_request::{self, ReinsOrgRequest, request_state as st},
            reins_org_routing,
        },
    },
};

/// One line of the report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GroupSla {
    pub group: String,
    pub decided: u32,
    pub median_secs: u64,
    pub p90_secs: u64,
    pub expired: u32,
    pub escalated: u32,
}

/// The `q`-th percentile (nearest rank) of sorted `xs`.
fn percentile(xs: &[u64], q: u64) -> u64 {
    if xs.is_empty() {
        return 0;
    }
    let n = u64::try_from(xs.len()).unwrap_or(u64::MAX);
    let rank = (q * n).div_ceil(100).max(1);
    xs[usize::try_from(rank - 1).unwrap_or(0).min(xs.len() - 1)]
}

fn decided(row: &ReinsOrgRequest) -> Option<u64> {
    let ended = !matches!(row.state.as_str(), st::REVIEWING | st::EXPIRED | st::CANCELLED);
    let at = row.decided_at.or(if row.state == st::DENIED {
        row.finished_at
    } else {
        None
    })?;
    (ended && at >= row.created_at).then(|| u64::try_from(at - row.created_at).unwrap_or(0))
}

#[derive(Default)]
struct Acc {
    times: Vec<u64>,
    expired: u32,
    escalated: u32,
}

impl Acc {
    fn line(mut self, group: String) -> GroupSla {
        self.times.sort_unstable();
        GroupSla {
            group,
            decided: u32::try_from(self.times.len()).unwrap_or(u32::MAX),
            median_secs: percentile(&self.times, 50),
            p90_secs: percentile(&self.times, 90),
            expired: self.expired,
            escalated: self.escalated,
        }
    }
}

fn group_of(c: &mut DbConnInner, org: &str, o: &Obligation) -> Result<String, OrgError> {
    match &o.spec.reviewers {
        Reviewers::Group(g) => super::group_name(c, org, g),
        Reviewers::Requester => Ok("The requester".to_owned()),
        other => super::label(c, org, other),
    }
}

/// The report for `[since, until)`: overall, and per group (by name).
pub fn sla(c: &mut DbConnInner, org: &str, since: i64, until: i64) -> Result<(GroupSla, Vec<GroupSla>), OrgError> {
    let mut all = Acc::default();
    let mut groups: BTreeMap<String, Acc> = BTreeMap::new();
    for row in reins_org_request::q_finished_between(c, org, since, until)? {
        let obligations: Vec<Obligation> = serde_json::from_str(&row.obligations).unwrap_or_default();
        let steps = reins_org_routing::q_steps(c, &row.uuid)?;
        let time = decided(&row);
        let expired = row.state == st::EXPIRED;
        let any_escalated = steps.iter().any(|s| s.step > 0);
        if let Some(t) = time {
            all.times.push(t);
        }
        all.expired += u32::from(expired);
        all.escalated += u32::from(any_escalated);
        for o in &obligations {
            let g = group_of(c, org, o)?;
            let acc = groups.entry(g).or_default();
            if let Some(t) = time {
                acc.times.push(t);
            }
            acc.expired += u32::from(expired);
            acc.escalated += u32::from(steps.iter().any(|s| s.obligation == o.id && s.step > 0));
        }
    }
    Ok((all.line("All".to_owned()), groups.into_iter().map(|(g, a)| a.line(g)).collect()))
}

/// The median time to decision of reviewed requests by `members` (everyone when `None`), for O4's rollout guard.
#[allow(dead_code, reason = "O4's staged rollout reads it")]
pub fn median_ttd(
    c: &mut DbConnInner,
    org: &str,
    members: Option<&[String]>,
    since: i64,
    until: i64,
) -> Result<Option<u64>, OrgError> {
    let mut times: Vec<u64> = reins_org_request::q_finished_between(c, org, since, until)?
        .iter()
        .filter(|r| members.is_none_or(|m| m.contains(&r.user_uuid)))
        .filter_map(decided)
        .collect();
    times.sort_unstable();
    Ok((!times.is_empty()).then(|| percentile(&times, 50)))
}

/// The report as JSON, for a viewer who may see admin events.
pub fn report(c: &mut DbConnInner, org: &str, viewer: &str, since: i64, until: i64) -> Result<Value, OrgError> {
    require(c, org, viewer, Capability::ViewAdminEvents)?;
    let (overall, groups) = sla(c, org, since, until)?;
    Ok(json!({"since": since, "until": until, "overall": overall, "groups": groups}))
}

pub fn routes() -> Vec<Route> {
    routes![get_sla]
}

/// `GET /reins/api/org/reports/sla?since&until` (Unix seconds; the last 7 days by default).
#[get("/reins/api/org/reports/sla?<since>&<until>")]
async fn get_sla(
    since: Option<i64>,
    until: Option<i64>,
    headers: Headers,
    conn: DbConn,
) -> Result<Json<Value>, rocket::http::Status> {
    use rocket::http::Status;
    if !enabled() {
        return Err(Status::NotFound);
    }
    let user = headers.user.uuid.to_string();
    let now = now_unix();
    let (since, until) = (since.unwrap_or(now - 7 * 86_400), until.unwrap_or(now));
    conn.run(move |c| {
        let org = crate::api::reins::org::leases::managed_org(c, &user)?.ok_or(OrgError::NotFound)?;
        report(c, &org, &user, since, until)
    })
    .await
    .map(Json)
    .map_err(|e| match e {
        OrgError::Forbidden | OrgError::NotFound => Status::NotFound,
        OrgError::Invalid(_) | OrgError::Taken(_) => Status::BadRequest,
        OrgError::Internal(_) => Status::InternalServerError,
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::api::reins::org::routing::{
        Dynamic,
        tests::{approve, ask_with, routed_org},
    };
    use crate::api::reins::org::testing::db;

    #[test]
    fn sla_medians_and_p90_per_group() {
        assert_eq!(percentile(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100], 50), 50);
        assert_eq!(percentile(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100], 90), 90);
        assert_eq!(percentile(&[7], 90), 7);
        let mut c = db();
        let phones = routed_org(&mut c);
        // Three Security reviews decided after 60, 120 and 600 s; one Finance review left to expire.
        for (i, after) in [60, 120, 600].into_iter().enumerate() {
            let at = 10_000 + i64::try_from(i).unwrap() * 1_000;
            let s = ask_with(&mut c, "acme/prod", &Dynamic::new(), at);
            approve(&mut c, &phones, "kim", &s.id, at + after).unwrap();
        }
        let f = ask_with(&mut c, "acme/fin", &Dynamic::new(), 20_000);
        let row = reins_org_request::q_request(&mut c, "o1", &f.id).unwrap().unwrap();
        crate::api::reins::org::decisions::expire(&mut c, "o1", &row, "window", 30_000).unwrap();
        let (all, groups) = sla(&mut c, "o1", 0, 100_000).unwrap();
        assert_eq!((all.decided, all.median_secs, all.p90_secs, all.expired), (3, 120, 600, 1));
        let sec = groups.iter().find(|g| g.group == "Group sec").unwrap();
        assert_eq!((sec.decided, sec.median_secs), (3, 120));
        let fin = groups.iter().find(|g| g.group == "Group fin").unwrap();
        assert_eq!((fin.decided, fin.expired), (0, 1));
        assert_eq!(median_ttd(&mut c, "o1", None, 0, 100_000).unwrap(), Some(120));
        assert_eq!(median_ttd(&mut c, "o1", Some(&["mia".to_owned()]), 0, 100_000).unwrap(), None);
        // Outside the window: nothing.
        assert_eq!(sla(&mut c, "o1", 50_000, 60_000).unwrap().0.decided, 0);
    }

    #[test]
    fn members_cannot_read_sla() {
        let mut c = db();
        routed_org(&mut c);
        assert_eq!(report(&mut c, "o1", "lee", 0, 1), Err(OrgError::Forbidden));
        assert!(report(&mut c, "o1", "dana", 0, 1).is_ok());
    }
}
