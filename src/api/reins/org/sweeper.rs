//! The org decision service's housekeeping, every [`EVERY_SECS`] while organizations are on: expiring reviews and
//! approvals nobody acted on, marking executions that never reported back, expiring leases (so the executor revokes
//! them upstream) and settling revocations whose token died upstream on its own, purging bodies past retention,
//! checkpointing the audit chains, and moving staged policy rollouts ([`rollout::tick_all`]).

use std::time::Duration;

use super::{OrgError, audit, decisions, enabled, reviews, rollout};
use crate::{
    api::reins::now_unix,
    db::{DbConnInner, DbPool, models::reins_org_request},
};

pub const EVERY_SECS: u64 = 15;

/// One pass; returns whom to tell about a rollout rolled back (its Security admins: they act on it).
pub fn sweep_once(c: &mut DbConnInner, now: i64) -> Result<Vec<String>, OrgError> {
    decisions::sweep(c, now)?;
    reins_org_request::q_expire_leases(c, now)?;
    // Revocations nobody could do (the minting executor stayed offline, or was removed): once the token's own lifetime
    // is over, it is dead upstream and leaves the queue, recorded as such.
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
    reins_org_request::q_purge_bodies(c, now)?;
    audit::checkpoint_due(c, now)?;
    rollout::tick_all(c, now)
}

pub fn spawn(pool: DbPool) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        loop {
            match pool.get().await {
                Ok(conn) => {
                    let now = now_unix();
                    match conn.run(move |c| sweep_once(c, now)).await {
                        Ok(notify) if !notify.is_empty() => reviews::notify(&pool, &notify, "rollout"),
                        Ok(_) => {}
                        Err(e) => warn!("Org sweeper: {e:?}"),
                    }
                }
                Err(e) => warn!("Org sweeper: no database connection: {e:?}"),
            }
            tokio::time::sleep(Duration::from_secs(EVERY_SECS)).await;
        }
    });
}
