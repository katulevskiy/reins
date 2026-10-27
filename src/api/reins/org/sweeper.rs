//! The org decision service's housekeeping, every [`EVERY_SECS`] while organizations are on: expiring reviews and
//! approvals nobody acted on, marking executions that never reported back, expiring leases (so the executor revokes
//! them upstream), purging bodies past retention, and checkpointing the audit chains.

use std::time::Duration;

use super::{OrgError, audit, decisions, enabled};
use crate::{
    api::reins::now_unix,
    db::{DbConnInner, DbPool, models::reins_org_request},
};

pub const EVERY_SECS: u64 = 15;

/// One pass.
pub fn sweep_once(c: &mut DbConnInner, now: i64) -> Result<(), OrgError> {
    decisions::sweep(c, now)?;
    reins_org_request::q_expire_leases(c, now)?;
    reins_org_request::q_purge_bodies(c, now)?;
    audit::checkpoint_due(c, now)?;
    Ok(())
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
                    if let Err(e) = conn.run(move |c| sweep_once(c, now)).await {
                        warn!("Org sweeper: {e:?}");
                    }
                    // Families: members whose leave took effect go (P7).
                    super::family::sweep_now(&conn, &pool).await;
                }
                Err(e) => warn!("Org sweeper: no database connection: {e:?}"),
            }
            tokio::time::sleep(Duration::from_secs(EVERY_SECS)).await;
        }
    });
}
