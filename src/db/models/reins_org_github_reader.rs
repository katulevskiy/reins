//! The org's GitHub reader App (Decisions v2 · O3 and O4, shared: whichever lands first creates it). `sealed` /
//! `webhook_sealed`: the private key and webhook secret, sealed under the org's data key (`kms::seal_data`), once
//! posted.

use diesel::prelude::*;

use crate::db::{DbConnInner, schema::reins_org_github_reader};

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_github_reader)]
#[diesel(primary_key(org_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrgGithubReader {
    pub org_uuid: String,
    pub app_id: i64,
    pub installation_id: i64,
    pub key_id: Option<String>,
    pub sealed: Option<String>,
    pub webhook_key_id: Option<String>,
    pub webhook_sealed: Option<String>,
    pub set_at: i64,
}

pub fn q_github_reader(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrgGithubReader>> {
    reins_org_github_reader::table.filter(reins_org_github_reader::org_uuid.eq(org)).first(c).optional()
}

pub fn q_github_reader_by_installation(
    c: &mut DbConnInner,
    installation: i64,
) -> QueryResult<Option<ReinsOrgGithubReader>> {
    reins_org_github_reader::table.filter(reins_org_github_reader::installation_id.eq(installation)).first(c).optional()
}

/// Sets the app and installation ids (an approved admin change); a different App drops the secrets it had.
pub fn q_set_github_reader(
    c: &mut DbConnInner,
    org: &str,
    app_id: i64,
    installation_id: i64,
    at: i64,
) -> QueryResult<()> {
    let row = match q_github_reader(c, org)? {
        Some(r) if r.app_id == app_id && r.installation_id == installation_id => return Ok(()),
        Some(r) if r.app_id == app_id => ReinsOrgGithubReader {
            installation_id,
            set_at: at,
            ..r
        },
        _ => ReinsOrgGithubReader {
            org_uuid: org.to_owned(),
            app_id,
            installation_id,
            key_id: None,
            sealed: None,
            webhook_key_id: None,
            webhook_sealed: None,
            set_at: at,
        },
    };
    diesel::delete(reins_org_github_reader::table.filter(reins_org_github_reader::org_uuid.eq(org))).execute(c)?;
    diesel::insert_into(reins_org_github_reader::table).values(&row).execute(c).map(drop)
}

/// Stores the sealed private key and webhook secret of the reader already set up.
pub fn q_set_github_reader_secrets(
    c: &mut DbConnInner,
    org: &str,
    key: (&str, &str),
    webhook: (&str, &str),
) -> QueryResult<bool> {
    diesel::update(reins_org_github_reader::table.filter(reins_org_github_reader::org_uuid.eq(org)))
        .set((
            reins_org_github_reader::key_id.eq(Some(key.0)),
            reins_org_github_reader::sealed.eq(Some(key.1)),
            reins_org_github_reader::webhook_key_id.eq(Some(webhook.0)),
            reins_org_github_reader::webhook_sealed.eq(Some(webhook.1)),
        ))
        .execute(c)
        .map(|n| n == 1)
}
