//! An org's keys (sealed by the KMS), its policy snapshots and its members' own policies (Decisions v2 · O2).

#![allow(dead_code, reason = "O2-WIP: wired by the decision service tasks")]

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{reins_org_keys, reins_org_member_policies, reins_org_policies},
};

/// One tenant key. `sealed` is the private key (or the data key) sealed by the KMS; `public_key` is empty for a data
/// key.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_keys)]
#[diesel(primary_key(org_uuid, key_id))]
pub struct ReinsOrgKey {
    pub org_uuid: String,
    pub key_id: String,
    /// `issuer`, `decision` or `data`.
    pub purpose: String,
    pub public_key: String,
    pub sealed: String,
    pub created_at: i64,
    pub retired_at: Option<i64>,
}

/// The newest key of `org` for `purpose` that is not retired.
pub fn q_key(c: &mut DbConnInner, org: &str, purpose: &str) -> QueryResult<Option<ReinsOrgKey>> {
    reins_org_keys::table
        .filter(reins_org_keys::org_uuid.eq(org))
        .filter(reins_org_keys::purpose.eq(purpose))
        .filter(reins_org_keys::retired_at.is_null())
        .order(reins_org_keys::created_at.desc())
        .first(c)
        .optional()
}

/// A key of `org` by id (retired ones too: old data stays readable).
pub fn q_key_by_id(c: &mut DbConnInner, org: &str, key_id: &str) -> QueryResult<Option<ReinsOrgKey>> {
    reins_org_keys::table
        .filter(reins_org_keys::org_uuid.eq(org))
        .filter(reins_org_keys::key_id.eq(key_id))
        .first(c)
        .optional()
}

pub fn q_insert_key(c: &mut DbConnInner, row: &ReinsOrgKey) -> QueryResult<()> {
    diesel::insert_into(reins_org_keys::table).values(row).execute(c).map(drop)
}

/// [`ReinsOrgPolicy::state`] values.
pub mod policy_state {
    pub const DRAFT: &str = "draft";
    pub const ACTIVE: &str = "active";
    pub const RETIRED: &str = "retired";
}

/// One revision of an org's policy. `envelope`: the issuer-signed snapshot, once activated.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_policies)]
#[diesel(primary_key(org_uuid, revision))]
pub struct ReinsOrgPolicy {
    pub org_uuid: String,
    pub revision: i64,
    pub hash: String,
    pub body: String,
    pub envelope: Option<String>,
    pub state: String,
    pub created_by: String,
    pub created_at: i64,
    pub activated_at: Option<i64>,
}

pub fn q_policy(c: &mut DbConnInner, org: &str, revision: i64) -> QueryResult<Option<ReinsOrgPolicy>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(reins_org_policies::revision.eq(revision))
        .first(c)
        .optional()
}

pub fn q_active_policy(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrgPolicy>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(reins_org_policies::state.eq(policy_state::ACTIVE))
        .first(c)
        .optional()
}

/// A signed (active or retired) policy of `org` by its hash.
pub fn q_policy_by_hash(c: &mut DbConnInner, org: &str, hash: &str) -> QueryResult<Option<ReinsOrgPolicy>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(reins_org_policies::hash.eq(hash))
        .filter(reins_org_policies::envelope.is_not_null())
        .order(reins_org_policies::revision.desc())
        .first(c)
        .optional()
}

pub fn q_last_revision(c: &mut DbConnInner, org: &str) -> QueryResult<i64> {
    let max: Option<i64> = reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .select(diesel::dsl::max(reins_org_policies::revision))
        .first(c)?;
    Ok(max.unwrap_or(0))
}

pub fn q_policies(c: &mut DbConnInner, org: &str, limit: i64) -> QueryResult<Vec<ReinsOrgPolicy>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .order(reins_org_policies::revision.desc())
        .limit(limit)
        .load(c)
}

pub fn q_insert_policy(c: &mut DbConnInner, row: &ReinsOrgPolicy) -> QueryResult<()> {
    diesel::insert_into(reins_org_policies::table).values(row).execute(c).map(drop)
}

/// Makes `revision` the active policy, with its signed `envelope`; the previous active one is retired. One
/// transaction, and the previous one is retired only once `revision` is confirmed to be a draft and promoted: there is
/// never more than one active policy, and a failed activation leaves the current one active.
pub fn q_activate(c: &mut DbConnInner, org: &str, revision: i64, envelope: &str, at: i64) -> QueryResult<bool> {
    let done = c.transaction(|c| {
        if !q_policy(c, org, revision)?.is_some_and(|p| p.state == policy_state::DRAFT) {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        diesel::update(
            reins_org_policies::table
                .filter(reins_org_policies::org_uuid.eq(org))
                .filter(reins_org_policies::state.eq(policy_state::ACTIVE)),
        )
        .set(reins_org_policies::state.eq(policy_state::RETIRED))
        .execute(c)?;
        let n = diesel::update(
            reins_org_policies::table
                .filter(reins_org_policies::org_uuid.eq(org))
                .filter(reins_org_policies::revision.eq(revision))
                .filter(reins_org_policies::state.eq(policy_state::DRAFT)),
        )
        .set((
            reins_org_policies::state.eq(policy_state::ACTIVE),
            reins_org_policies::envelope.eq(Some(envelope)),
            reins_org_policies::activated_at.eq(Some(at)),
        ))
        .execute(c)?;
        if n != 1 {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        Ok(())
    });
    match done {
        Ok(()) => Ok(true),
        Err(diesel::result::Error::RollbackTransaction) => Ok(false),
        Err(e) => Err(e),
    }
}

/// A member's own policy, signed by their device.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_member_policies)]
#[diesel(primary_key(org_uuid, user_uuid))]
pub struct ReinsOrgMemberPolicy {
    pub org_uuid: String,
    pub user_uuid: String,
    pub body: String,
    pub hash: String,
    pub device_uuid: String,
    pub sig: String,
    pub updated_at: i64,
}

pub fn q_member_policy(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Option<ReinsOrgMemberPolicy>> {
    reins_org_member_policies::table
        .filter(reins_org_member_policies::org_uuid.eq(org))
        .filter(reins_org_member_policies::user_uuid.eq(user))
        .first(c)
        .optional()
}

pub fn q_set_member_policy(c: &mut DbConnInner, row: &ReinsOrgMemberPolicy) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(
            reins_org_member_policies::table
                .filter(reins_org_member_policies::org_uuid.eq(&row.org_uuid))
                .filter(reins_org_member_policies::user_uuid.eq(&row.user_uuid)),
        )
        .execute(c)?;
        diesel::insert_into(reins_org_member_policies::table).values(row).execute(c).map(drop)
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::super::reins_device::test_db;
    use super::*;

    fn draft(org: &str, revision: i64) -> ReinsOrgPolicy {
        ReinsOrgPolicy {
            org_uuid: org.to_owned(),
            revision,
            hash: format!("h{revision}"),
            body: "{}".to_owned(),
            envelope: None,
            state: policy_state::DRAFT.to_owned(),
            created_by: "u1".to_owned(),
            created_at: 1,
            activated_at: None,
        }
    }

    #[test]
    fn activating_retires_the_previous_policy() {
        let mut c = test_db();
        q_insert_policy(&mut c, &draft("o1", 1)).unwrap();
        q_insert_policy(&mut c, &draft("o1", 2)).unwrap();
        q_insert_policy(&mut c, &draft("o2", 1)).unwrap();
        assert!(q_activate(&mut c, "o1", 1, "env1", 5).unwrap());
        assert_eq!(q_active_policy(&mut c, "o1").unwrap().unwrap().revision, 1);
        assert!(q_activate(&mut c, "o1", 2, "env2", 6).unwrap());
        assert_eq!(q_active_policy(&mut c, "o1").unwrap().unwrap().revision, 2);
        assert_eq!(q_policy(&mut c, "o1", 1).unwrap().unwrap().state, policy_state::RETIRED);
        // A retired revision can't be activated again by its old draft path, and another org's is untouched.
        assert!(!q_activate(&mut c, "o1", 1, "env1", 7).unwrap());
        // A failed activation leaves the current policy active.
        assert_eq!(q_active_policy(&mut c, "o1").unwrap().unwrap().revision, 2);
        assert!(!q_activate(&mut c, "o1", 99, "env", 8).unwrap(), "no such draft");
        assert_eq!(q_active_policy(&mut c, "o1").unwrap().unwrap().revision, 2);
        assert!(!q_activate(&mut c, "o1", 2, "env", 9).unwrap(), "already active");
        assert_eq!(q_active_policy(&mut c, "o1").unwrap().unwrap().envelope.as_deref(), Some("env2"));
        assert!(q_active_policy(&mut c, "o2").unwrap().is_none());
        assert_eq!(q_policy_by_hash(&mut c, "o1", "h1").unwrap().unwrap().envelope.as_deref(), Some("env1"));
        assert!(q_policy_by_hash(&mut c, "o2", "h1").unwrap().is_none(), "drafts have no envelope");
        assert_eq!(q_last_revision(&mut c, "o1").unwrap(), 2);
        assert_eq!(q_last_revision(&mut c, "o3").unwrap(), 0);
    }

    #[test]
    fn keys_are_per_org_and_purpose() {
        let mut c = test_db();
        let key = |org: &str, id: &str, purpose: &str, at: i64| ReinsOrgKey {
            org_uuid: org.to_owned(),
            key_id: id.to_owned(),
            purpose: purpose.to_owned(),
            public_key: String::new(),
            sealed: "s".to_owned(),
            created_at: at,
            retired_at: None,
        };
        q_insert_key(&mut c, &key("o1", "k1", "issuer", 1)).unwrap();
        q_insert_key(&mut c, &key("o1", "k2", "issuer", 2)).unwrap();
        q_insert_key(&mut c, &key("o2", "k3", "issuer", 1)).unwrap();
        assert_eq!(q_key(&mut c, "o1", "issuer").unwrap().unwrap().key_id, "k2");
        assert!(q_key(&mut c, "o1", "decision").unwrap().is_none());
        assert!(q_key_by_id(&mut c, "o2", "k1").unwrap().is_none());
    }
}
