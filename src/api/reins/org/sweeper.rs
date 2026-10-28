//! The org decision service's housekeeping, every [`EVERY_SECS`] while organizations are on: expiring reviews and
//! approvals nobody acted on, marking executions that never reported back, expiring leases (so the executor revokes
//! them upstream) and settling revocations whose token died upstream on its own, purging bodies past retention,
//! checkpointing the audit chains, and moving staged policy rollouts ([`rollout::tick_all`]).

use std::{collections::BTreeMap, time::Duration};

use super::{OrgError, audit, decisions, enabled, reviews, rollout, routing};
use crate::{
    api::reins::now_unix,
    db::{DbConnInner, DbPool, models::reins_org_request},
};

pub const EVERY_SECS: u64 = 15;

/// [`EVERY_SECS`], or (tests only, with `REINS_TEST_ALLOW_LOOPBACK`) `REINS_TEST_ORG_SWEEP_SECS`.
fn every_secs() -> u64 {
    if crate::api::reins::outbound::allow_loopback()
        && let Some(s) = std::env::var("REINS_TEST_ORG_SWEEP_SECS").ok().and_then(|v| v.parse::<u64>().ok())
    {
        return s.max(1);
    }
    EVERY_SECS
}

/// One pass without dynamic routing answers (tests).
#[cfg(test)]
pub fn sweep_once(c: &mut DbConnInner, now: i64) -> Result<(), OrgError> {
    sweep_routed(c, now, &BTreeMap::new());
    Ok(())
}

/// One pass, with the dynamic routing sources' answers; returns the escalations to notify. Each part runs whatever
/// the others did (a failing one is logged): lease expiry and checkpoints never wait on escalation.
pub fn sweep_routed(
    c: &mut DbConnInner,
    now: i64,
    prefetched: &routing::escalation::Prefetched,
) -> Vec<routing::escalation::Swept> {
    let swept = match decisions::sweep_routed(c, now, prefetched) {
        Ok((_, swept)) => swept,
        Err(e) => {
            warn!("Org sweeper: decisions: {e:?}");
            Vec::new()
        }
    };
    if let Err(e) = reins_org_request::q_expire_leases(c, now) {
        warn!("Org sweeper: leases: {e:?}");
    }
    // Revocations nobody could do (the minting executor stayed offline, or was removed): once the token's own lifetime
    // is over, it is dead upstream and leaves the queue, recorded as such.
    if let Err(e) = settle_lapsed_tokens(c, now) {
        warn!("Org sweeper: lapsed tokens: {e:?}");
    }
    if let Err(e) = reins_org_request::q_purge_bodies(c, now) {
        warn!("Org sweeper: purge: {e:?}");
    }
    if let Err(e) = audit::checkpoint_due(c, now) {
        warn!("Org sweeper: checkpoints: {e:?}");
    }
    // Families (P7): connect jobs whose time ran out, with any code they still held.
    if let Err(e) = super::family_connect::sweep(c, now) {
        warn!("Org sweeper: family connect jobs: {e:?}");
    }
    swept
}

/// Asks code owners and on-call what the escalations due now need, before the sweep's transaction.
async fn prefetch(pool: &DbPool, now: i64) -> routing::escalation::Prefetched {
    let Ok(conn) = pool.get().await else {
        return BTreeMap::new();
    };
    let due = conn.run(move |c| routing::escalation::due_sets(c, now)).await.unwrap_or_default();
    drop(conn);
    let mut out = routing::escalation::Prefetched::new();
    for d in due {
        let touches = routing::touches(pool, &d.org, &d.tool, &d.args).await;
        let answers = routing::fetch(pool, &d.org, &d.sets, &touches, now).await;
        out.insert(d.request, (touches, answers));
    }
    out
}

fn settle_lapsed_tokens(c: &mut DbConnInner, now: i64) -> Result<(), OrgError> {
    for l in reins_org_request::q_settle_lapsed_tokens(c, now)? {
        audit::record(
            c,
            &l.org_uuid,
            audit::kinds::LEASE_TOKEN_LAPSED,
            None,
            Some(&l.uuid),
            &serde_json::json!({"issued_at": l.issued_at, "token_max_secs": reins_org_request::TOKEN_MAX_SECS}),
            now,
        )?;
    }
    Ok(())
}

pub fn spawn(pool: DbPool) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        let mut relinked = 0_i64;
        loop {
            // O3: members' GitHub logins re-checked against the GitHub organization's SAML identities, daily.
            if now_unix() - relinked >= 86_400 {
                relinked = now_unix();
                routing::codeowners::relink_all(&pool, relinked).await;
            }
            let now = now_unix();
            let dynamic = prefetch(&pool, now).await;
            match pool.get().await {
                Ok(conn) => {
                    let swept = conn.run(move |c| sweep_routed(c, now, &dynamic)).await;
                    routing::after_sweep(&pool, swept).await;
                    // O4: staged policy rollouts advance or roll back; a rollback tells its Security admins.
                    match conn.run(move |c| rollout::tick_all(c, now)).await {
                        Ok(notify) if !notify.is_empty() => reviews::notify(&pool, &notify, "rollout"),
                        Ok(_) => {}
                        Err(e) => warn!("Org sweeper: rollouts: {e:?}"),
                    }
                    // Families: members whose leave took effect go (P7).
                    super::family::sweep_now(&conn, &pool).await;
                }
                Err(e) => warn!("Org sweeper: no database connection: {e:?}"),
            }
            tokio::time::sleep(Duration::from_secs(every_secs())).await;
        }
    });
}
