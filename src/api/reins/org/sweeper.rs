//! The org decision service's housekeeping, every [`EVERY_SECS`] while organizations are on: expiring reviews and
//! approvals nobody acted on, marking executions that never reported back, expiring leases (so the executor revokes
//! them upstream), purging bodies past retention, and checkpointing the audit chains.

use std::{collections::BTreeMap, time::Duration};

use super::{OrgError, audit, decisions, enabled, routing};
use crate::{
    api::reins::now_unix,
    db::{DbConnInner, DbPool, models::reins_org_request},
};

pub const EVERY_SECS: u64 = 15;

/// One pass, with the dynamic routing sources' answers; returns the escalations to notify.
pub fn sweep_routed(
    c: &mut DbConnInner,
    now: i64,
    dynamic: &BTreeMap<String, routing::Dynamic>,
) -> Result<Vec<routing::escalation::Swept>, OrgError> {
    let (_, swept) = decisions::sweep_routed(c, now, dynamic)?;
    reins_org_request::q_expire_leases(c, now)?;
    reins_org_request::q_purge_bodies(c, now)?;
    audit::checkpoint_due(c, now)?;
    Ok(swept)
}

/// Asks code owners and on-call what the escalations due now need, before the sweep's transaction.
async fn prefetch(pool: &DbPool, now: i64) -> BTreeMap<String, routing::Dynamic> {
    let Ok(conn) = pool.get().await else {
        return BTreeMap::new();
    };
    let due = conn.run(move |c| routing::escalation::due_sets(c, now)).await.unwrap_or_default();
    drop(conn);
    let mut out: BTreeMap<String, routing::Dynamic> = BTreeMap::new();
    for (org, tool, args, sets) in due {
        let touches = routing::touches(pool, &org, &tool, &args).await;
        let answers = routing::fetch(pool, &org, &sets, &touches, now).await;
        out.entry(org).or_default().extend(answers);
    }
    out
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
                Ok(conn) => match conn.run(move |c| sweep_routed(c, now, &dynamic)).await {
                    Ok(swept) => routing::after_sweep(&pool, swept).await,
                    Err(e) => warn!("Org sweeper: {e:?}"),
                },
                Err(e) => warn!("Org sweeper: no database connection: {e:?}"),
            }
            tokio::time::sleep(Duration::from_secs(EVERY_SECS)).await;
        }
    });
}
