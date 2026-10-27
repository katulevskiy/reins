//! An org's registered executors and its signed audit checkpoints (Decisions v2 · O2).

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{reins_org_checkpoints, reins_org_executors},
};

/// An executor bound to one tenant, with its own Ed25519 key (base64url).
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_executors)]
#[diesel(primary_key(org_uuid, executor_id))]
pub struct ReinsOrgExecutor {
    pub org_uuid: String,
    pub executor_id: String,
    /// `cloud`, `dedicated` or `self_hosted`.
    pub mode: String,
    pub public_key: String,
    pub created_at: i64,
    pub last_seen: Option<i64>,
    pub revoked_at: Option<i64>,
}

pub fn q_executor(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsOrgExecutor>> {
    reins_org_executors::table
        .filter(reins_org_executors::org_uuid.eq(org))
        .filter(reins_org_executors::executor_id.eq(id))
        .first(c)
        .optional()
}

pub fn q_executors(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgExecutor>> {
    reins_org_executors::table
        .filter(reins_org_executors::org_uuid.eq(org))
        .order(reins_org_executors::created_at.asc())
        .load(c)
}

/// Registers an executor; false when the id is taken in this org (a removed id is never reused).
pub fn q_insert_executor(c: &mut DbConnInner, row: &ReinsOrgExecutor) -> QueryResult<bool> {
    if q_executor(c, &row.org_uuid, &row.executor_id)?.is_some() {
        return Ok(false);
    }
    diesel::insert_into(reins_org_executors::table).values(row).execute(c)?;
    Ok(true)
}

pub fn q_revoke_executor(c: &mut DbConnInner, org: &str, id: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_executors::table
            .filter(reins_org_executors::org_uuid.eq(org))
            .filter(reins_org_executors::executor_id.eq(id))
            .filter(reins_org_executors::revoked_at.is_null()),
    )
    .set(reins_org_executors::revoked_at.eq(Some(at)))
    .execute(c)?;
    Ok(n == 1)
}

pub fn q_touch_executor(c: &mut DbConnInner, org: &str, id: &str, at: i64) -> QueryResult<()> {
    diesel::update(
        reins_org_executors::table
            .filter(reins_org_executors::org_uuid.eq(org))
            .filter(reins_org_executors::executor_id.eq(id)),
    )
    .set(reins_org_executors::last_seen.eq(Some(at)))
    .execute(c)
    .map(drop)
}

/// A signed checkpoint of an org's audit chain (06 §5.2).
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_checkpoints)]
#[diesel(primary_key(org_uuid, seq))]
pub struct ReinsOrgCheckpoint {
    pub org_uuid: String,
    pub seq: i64,
    pub hash: String,
    pub at: i64,
    /// The signed envelope (JSON).
    pub envelope: String,
}

pub fn q_last_checkpoint(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrgCheckpoint>> {
    reins_org_checkpoints::table
        .filter(reins_org_checkpoints::org_uuid.eq(org))
        .order(reins_org_checkpoints::seq.desc())
        .first(c)
        .optional()
}

pub fn q_checkpoint(c: &mut DbConnInner, org: &str, seq: i64) -> QueryResult<Option<ReinsOrgCheckpoint>> {
    reins_org_checkpoints::table
        .filter(reins_org_checkpoints::org_uuid.eq(org))
        .filter(reins_org_checkpoints::seq.eq(seq))
        .first(c)
        .optional()
}

pub fn q_insert_checkpoint(c: &mut DbConnInner, row: &ReinsOrgCheckpoint) -> QueryResult<()> {
    diesel::insert_into(reins_org_checkpoints::table).values(row).execute(c).map(drop)
}

/// The newest checkpoint of `org` signed at or before `at`.
pub fn q_checkpoint_at_or_before(c: &mut DbConnInner, org: &str, at: i64) -> QueryResult<Option<ReinsOrgCheckpoint>> {
    reins_org_checkpoints::table
        .filter(reins_org_checkpoints::org_uuid.eq(org))
        .filter(reins_org_checkpoints::at.le(at))
        .order(reins_org_checkpoints::seq.desc())
        .first(c)
        .optional()
}

/// Deletes `org`'s checkpoints before `seq` (exclusive).
pub fn q_delete_checkpoints_before(c: &mut DbConnInner, org: &str, seq: i64) -> QueryResult<usize> {
    diesel::delete(
        reins_org_checkpoints::table
            .filter(reins_org_checkpoints::org_uuid.eq(org))
            .filter(reins_org_checkpoints::seq.lt(seq)),
    )
    .execute(c)
}

/// Oldest first, after `after` (exclusive).
pub fn q_checkpoints(c: &mut DbConnInner, org: &str, after: i64) -> QueryResult<Vec<ReinsOrgCheckpoint>> {
    reins_org_checkpoints::table
        .filter(reins_org_checkpoints::org_uuid.eq(org))
        .filter(reins_org_checkpoints::seq.gt(after))
        .order(reins_org_checkpoints::seq.asc())
        .load(c)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    #[test]
    fn checkpoints_are_per_org() {
        let mut c = test_db();
        let cp = |org: &str, seq: i64| ReinsOrgCheckpoint {
            org_uuid: org.to_owned(),
            seq,
            hash: "h".to_owned(),
            at: seq,
            envelope: "{}".to_owned(),
        };
        q_insert_checkpoint(&mut c, &cp("o1", 3)).unwrap();
        q_insert_checkpoint(&mut c, &cp("o1", 7)).unwrap();
        q_insert_checkpoint(&mut c, &cp("o2", 9)).unwrap();
        assert_eq!(q_last_checkpoint(&mut c, "o1").unwrap().unwrap().seq, 7);
        assert_eq!(q_checkpoints(&mut c, "o1", 3).unwrap().len(), 1);
        assert!(q_checkpoint(&mut c, "o2", 3).unwrap().is_none());
    }

    #[test]
    fn executors_are_per_org_and_ids_are_never_reused() {
        let mut c = test_db();
        let ex = |org: &str| ReinsOrgExecutor {
            org_uuid: org.to_owned(),
            executor_id: "e1".to_owned(),
            mode: "cloud".to_owned(),
            public_key: "k".to_owned(),
            created_at: 1,
            last_seen: None,
            revoked_at: None,
        };
        assert!(q_insert_executor(&mut c, &ex("o1")).unwrap());
        assert!(q_insert_executor(&mut c, &ex("o2")).unwrap());
        assert!(q_revoke_executor(&mut c, "o1", "e1", 5).unwrap());
        assert!(!q_insert_executor(&mut c, &ex("o1")).unwrap());
        assert!(q_executor(&mut c, "o2", "e1").unwrap().unwrap().revoked_at.is_none());
    }
}
