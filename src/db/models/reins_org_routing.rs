//! Approval routing (Decisions v2 · O3): who a member's manager is and which GitHub, Slack and Teams identities are
//! theirs; group backups and windows; on-call sources, approval channels; each
//! obligation's escalation steps; web passkeys, one-time web approval tokens and the channel messages posted.
//!
//! Secrets (an on-call API key, a channel's tokens) are stored sealed under the tenant's data key
//! and only ever read back by the one query that opens them for use; listings never carry them. A web approval token
//! is stored as its SHA-256 and used once (a guarded update).

#![allow(dead_code, reason = "the routing, channel and web approval code that follows uses them")]

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{
        reins_org_channels, reins_org_group_settings, reins_org_oncall, reins_org_people, reins_org_posted,
        reins_org_steps, reins_org_web_passkeys, reins_org_web_tokens,
    },
};

/// Where a manager link came from.
pub mod manager_source {
    /// SCIM's `manager_email`, through WorkOS Directory Sync.
    pub const DIRECTORY: &str = "directory";
    /// A two-person admin change (Team tier, which has no directory).
    pub const ADMIN: &str = "admin";
}

/// How a GitHub login was proven to be a member's.
pub mod github_via {
    /// The GitHub organization's SAML identities name the member's email.
    pub const SAML: &str = "saml";
    /// The member authorized the org's reader App themselves.
    pub const OAUTH: &str = "oauth";
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_people)]
#[diesel(primary_key(org_uuid, user_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrgPerson {
    pub org_uuid: String,
    pub user_uuid: String,
    pub manager_user: Option<String>,
    pub manager_source: Option<String>,
    pub github_login: Option<String>,
    pub github_verified_via: Option<String>,
    pub slack_user: Option<String>,
    pub teams_aad: Option<String>,
    pub teams_conversation: Option<String>,
    pub updated_at: i64,
}

impl ReinsOrgPerson {
    #[must_use]
    pub fn empty(org: &str, user: &str, now: i64) -> Self {
        Self {
            org_uuid: org.to_owned(),
            user_uuid: user.to_owned(),
            manager_user: None,
            manager_source: None,
            github_login: None,
            github_verified_via: None,
            slack_user: None,
            teams_aad: None,
            teams_conversation: None,
            updated_at: now,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_group_settings)]
#[diesel(primary_key(org_uuid, group_uuid))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrgGroupSetting {
    pub org_uuid: String,
    pub group_uuid: String,
    pub backup_group: Option<String>,
    pub window_secs: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_oncall)]
#[diesel(primary_key(org_uuid, alias))]
pub struct ReinsOrgOnCall {
    pub org_uuid: String,
    pub alias: String,
    /// `pagerduty`, `opsgenie` or `incident_io`.
    pub provider: String,
    pub schedule_id: String,
    pub key_id: Option<String>,
    /// The API key, sealed.
    pub sealed: Option<String>,
}

/// An on-call source as listings show it: never its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnCallSummary {
    pub alias: String,
    pub provider: String,
    pub schedule_id: String,
    pub has_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_channels)]
#[diesel(primary_key(org_uuid, kind))]
pub struct ReinsOrgChannel {
    pub org_uuid: String,
    /// `slack` or `teams`.
    pub kind: String,
    pub key_id: Option<String>,
    /// The channel's configuration (tokens, signing secret), sealed.
    pub sealed: Option<String>,
    pub enabled: bool,
    pub installed_at: Option<i64>,
}

/// A channel as listings show it: never its secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSummary {
    pub kind: String,
    pub enabled: bool,
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_steps)]
#[diesel(primary_key(request_uuid, obligation, step))]
pub struct ReinsOrgStep {
    pub request_uuid: String,
    pub obligation: String,
    /// 0 is the obligation's own reviewers; 1.. its escalations.
    pub step: i32,
    /// The step's `Reviewers` as JSON.
    pub reviewers: String,
    /// The members it added (JSON array of user ids).
    pub resolved: String,
    /// Where they came from (`roster`, `directory`, `codeowners`, `oncall:<provider>`, …).
    pub source: String,
    pub started_at: i64,
    pub deadline: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_web_passkeys)]
#[diesel(primary_key(credential_id))]
pub struct ReinsOrgWebPasskey {
    /// base64url.
    pub credential_id: String,
    pub org_uuid: String,
    pub user_uuid: String,
    /// The public key: P-256 SubjectPublicKeyInfo DER, base64.
    pub passkey: String,
    pub created_at: i64,
    /// Confirmed on the member's enrolled phone; only then usable.
    pub confirmed_at: Option<i64>,
    pub confirmed_device: Option<String>,
    pub revoked_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_web_tokens)]
#[diesel(primary_key(token_hash))]
pub struct ReinsOrgWebToken {
    /// Hex SHA-256 of the token.
    pub token_hash: String,
    pub org_uuid: String,
    pub request_uuid: String,
    pub user_uuid: String,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_posted)]
#[diesel(primary_key(request_uuid, user_uuid, channel))]
pub struct ReinsOrgPosted {
    pub request_uuid: String,
    pub user_uuid: String,
    pub channel: String,
    /// The channel's own reference (Slack `channel:ts`, Teams `conversation:activity`).
    pub external_ref: String,
    pub posted_at: i64,
}

// ---- People

pub fn q_person(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Option<ReinsOrgPerson>> {
    reins_org_people::table
        .filter(reins_org_people::org_uuid.eq(org))
        .filter(reins_org_people::user_uuid.eq(user))
        .first(c)
        .optional()
}

pub fn q_people(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgPerson>> {
    reins_org_people::table.filter(reins_org_people::org_uuid.eq(org)).order(reins_org_people::user_uuid.asc()).load(c)
}

/// Writes `row` (an insert, or an update of the member's row).
pub fn q_save_person(c: &mut DbConnInner, row: &ReinsOrgPerson) -> QueryResult<()> {
    let n = diesel::update(
        reins_org_people::table
            .filter(reins_org_people::org_uuid.eq(&row.org_uuid))
            .filter(reins_org_people::user_uuid.eq(&row.user_uuid)),
    )
    .set(row)
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_org_people::table).values(row).execute(c)?;
    }
    Ok(())
}

/// Changes one member's row with `edit`, creating it when there is none.
pub fn q_edit_person(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    now: i64,
    edit: impl FnOnce(&mut ReinsOrgPerson),
) -> QueryResult<()> {
    let mut row = q_person(c, org, user)?.unwrap_or_else(|| ReinsOrgPerson::empty(org, user, now));
    edit(&mut row);
    row.updated_at = now;
    q_save_person(c, &row)
}

/// The member whose verified GitHub login is `login` (case-insensitive).
pub fn q_by_github_login(c: &mut DbConnInner, org: &str, login: &str) -> QueryResult<Option<String>> {
    let lower = login.to_ascii_lowercase();
    Ok(q_people(c, org)?
        .into_iter()
        .find(|p| {
            p.github_verified_via.is_some()
                && p.github_login.as_deref().map(str::to_ascii_lowercase) == Some(lower.clone())
        })
        .map(|p| p.user_uuid))
}

// ---- Groups

pub fn q_group_settings(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgGroupSetting>> {
    reins_org_group_settings::table.filter(reins_org_group_settings::org_uuid.eq(org)).load(c)
}

pub fn q_group_setting(c: &mut DbConnInner, org: &str, group: &str) -> QueryResult<Option<ReinsOrgGroupSetting>> {
    reins_org_group_settings::table
        .filter(reins_org_group_settings::org_uuid.eq(org))
        .filter(reins_org_group_settings::group_uuid.eq(group))
        .first(c)
        .optional()
}

pub fn q_save_group_setting(c: &mut DbConnInner, row: &ReinsOrgGroupSetting) -> QueryResult<()> {
    let n = diesel::update(
        reins_org_group_settings::table
            .filter(reins_org_group_settings::org_uuid.eq(&row.org_uuid))
            .filter(reins_org_group_settings::group_uuid.eq(&row.group_uuid)),
    )
    .set(row)
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_org_group_settings::table).values(row).execute(c)?;
    }
    Ok(())
}

// ---- On-call

/// Sets an alias's provider and schedule; its key stays (or stays absent) until posted.
pub fn q_set_oncall_source(
    c: &mut DbConnInner,
    org: &str,
    alias: &str,
    provider: &str,
    schedule_id: &str,
) -> QueryResult<()> {
    let n = diesel::update(
        reins_org_oncall::table.filter(reins_org_oncall::org_uuid.eq(org)).filter(reins_org_oncall::alias.eq(alias)),
    )
    .set((reins_org_oncall::provider.eq(provider), reins_org_oncall::schedule_id.eq(schedule_id)))
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_org_oncall::table)
            .values(ReinsOrgOnCall {
                org_uuid: org.to_owned(),
                alias: alias.to_owned(),
                provider: provider.to_owned(),
                schedule_id: schedule_id.to_owned(),
                key_id: None,
                sealed: None,
            })
            .execute(c)?;
    }
    Ok(())
}

pub fn q_set_oncall_key(c: &mut DbConnInner, org: &str, alias: &str, key_id: &str, sealed: &str) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_oncall::table.filter(reins_org_oncall::org_uuid.eq(org)).filter(reins_org_oncall::alias.eq(alias)),
    )
    .set((reins_org_oncall::key_id.eq(Some(key_id)), reins_org_oncall::sealed.eq(Some(sealed))))
    .execute(c)?;
    Ok(n == 1)
}

/// One source with its sealed key, to open and use it.
pub fn q_oncall_for_use(c: &mut DbConnInner, org: &str, alias: &str) -> QueryResult<Option<ReinsOrgOnCall>> {
    reins_org_oncall::table
        .filter(reins_org_oncall::org_uuid.eq(org))
        .filter(reins_org_oncall::alias.eq(alias))
        .first(c)
        .optional()
}

pub fn q_oncalls(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<OnCallSummary>> {
    let rows: Vec<(String, String, String, Option<String>)> = reins_org_oncall::table
        .filter(reins_org_oncall::org_uuid.eq(org))
        .order(reins_org_oncall::alias.asc())
        .select((
            reins_org_oncall::alias,
            reins_org_oncall::provider,
            reins_org_oncall::schedule_id,
            reins_org_oncall::key_id,
        ))
        .load(c)?;
    Ok(rows
        .into_iter()
        .map(|(alias, provider, schedule_id, key)| OnCallSummary {
            alias,
            provider,
            schedule_id,
            has_key: key.is_some(),
        })
        .collect())
}

// ---- Channels

pub fn q_set_channel_enabled(c: &mut DbConnInner, org: &str, kind: &str, enabled: bool) -> QueryResult<()> {
    let n = diesel::update(
        reins_org_channels::table
            .filter(reins_org_channels::org_uuid.eq(org))
            .filter(reins_org_channels::kind.eq(kind)),
    )
    .set(reins_org_channels::enabled.eq(enabled))
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_org_channels::table)
            .values(ReinsOrgChannel {
                org_uuid: org.to_owned(),
                kind: kind.to_owned(),
                key_id: None,
                sealed: None,
                enabled,
                installed_at: None,
            })
            .execute(c)?;
    }
    Ok(())
}

/// Stores a channel's sealed configuration (an install).
pub fn q_set_channel_secret(
    c: &mut DbConnInner,
    org: &str,
    kind: &str,
    key_id: &str,
    sealed: &str,
    now: i64,
) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_channels::table
            .filter(reins_org_channels::org_uuid.eq(org))
            .filter(reins_org_channels::kind.eq(kind)),
    )
    .set((
        reins_org_channels::key_id.eq(Some(key_id)),
        reins_org_channels::sealed.eq(Some(sealed)),
        reins_org_channels::installed_at.eq(Some(now)),
    ))
    .execute(c)?;
    Ok(n == 1)
}

pub fn q_channel_for_use(c: &mut DbConnInner, org: &str, kind: &str) -> QueryResult<Option<ReinsOrgChannel>> {
    reins_org_channels::table
        .filter(reins_org_channels::org_uuid.eq(org))
        .filter(reins_org_channels::kind.eq(kind))
        .first(c)
        .optional()
}

/// Every org with an enabled, installed channel of `kind` (inbound webhooks find their org by trying each).
pub fn q_channels_of_kind(c: &mut DbConnInner, kind: &str) -> QueryResult<Vec<ReinsOrgChannel>> {
    reins_org_channels::table
        .filter(reins_org_channels::kind.eq(kind))
        .filter(reins_org_channels::enabled.eq(true))
        .filter(reins_org_channels::sealed.is_not_null())
        .load(c)
}

/// Every org with an installed channel of `kind`, on or off (an app id may be one org's only).
pub fn q_installed_of_kind(c: &mut DbConnInner, kind: &str) -> QueryResult<Vec<ReinsOrgChannel>> {
    reins_org_channels::table
        .filter(reins_org_channels::kind.eq(kind))
        .filter(reins_org_channels::sealed.is_not_null())
        .load(c)
}

pub fn q_channels(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ChannelSummary>> {
    let rows: Vec<(String, bool, Option<i64>)> = reins_org_channels::table
        .filter(reins_org_channels::org_uuid.eq(org))
        .order(reins_org_channels::kind.asc())
        .select((reins_org_channels::kind, reins_org_channels::enabled, reins_org_channels::installed_at))
        .load(c)?;
    Ok(rows
        .into_iter()
        .map(|(kind, enabled, installed)| ChannelSummary {
            kind,
            enabled,
            installed: installed.is_some(),
        })
        .collect())
}

// ---- Escalation steps

pub fn q_insert_step(c: &mut DbConnInner, row: &ReinsOrgStep) -> QueryResult<bool> {
    let exists: i64 = reins_org_steps::table
        .filter(reins_org_steps::request_uuid.eq(&row.request_uuid))
        .filter(reins_org_steps::obligation.eq(&row.obligation))
        .filter(reins_org_steps::step.eq(row.step))
        .count()
        .get_result(c)?;
    if exists > 0 {
        return Ok(false);
    }
    diesel::insert_into(reins_org_steps::table).values(row).execute(c)?;
    Ok(true)
}

/// A request's steps, per obligation in step order.
pub fn q_steps(c: &mut DbConnInner, request: &str) -> QueryResult<Vec<ReinsOrgStep>> {
    reins_org_steps::table
        .filter(reins_org_steps::request_uuid.eq(request))
        .order((reins_org_steps::obligation.asc(), reins_org_steps::step.asc()))
        .load(c)
}

// ---- Web passkeys

pub fn q_insert_passkey(c: &mut DbConnInner, row: &ReinsOrgWebPasskey) -> QueryResult<bool> {
    let exists: i64 = reins_org_web_passkeys::table
        .filter(reins_org_web_passkeys::credential_id.eq(&row.credential_id))
        .count()
        .get_result(c)?;
    if exists > 0 {
        return Ok(false);
    }
    diesel::insert_into(reins_org_web_passkeys::table).values(row).execute(c)?;
    Ok(true)
}

pub fn q_passkey(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsOrgWebPasskey>> {
    reins_org_web_passkeys::table
        .filter(reins_org_web_passkeys::org_uuid.eq(org))
        .filter(reins_org_web_passkeys::credential_id.eq(id))
        .first(c)
        .optional()
}

/// A member's passkeys that aren't revoked (confirmed or not).
pub fn q_passkeys_of(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Vec<ReinsOrgWebPasskey>> {
    reins_org_web_passkeys::table
        .filter(reins_org_web_passkeys::org_uuid.eq(org))
        .filter(reins_org_web_passkeys::user_uuid.eq(user))
        .filter(reins_org_web_passkeys::revoked_at.is_null())
        .order(reins_org_web_passkeys::created_at.asc())
        .load(c)
}

/// The passkeys a member may approve with: confirmed on their phone and not revoked.
pub fn q_usable_passkeys(c: &mut DbConnInner, org: &str, user: &str) -> QueryResult<Vec<ReinsOrgWebPasskey>> {
    reins_org_web_passkeys::table
        .filter(reins_org_web_passkeys::org_uuid.eq(org))
        .filter(reins_org_web_passkeys::user_uuid.eq(user))
        .filter(reins_org_web_passkeys::revoked_at.is_null())
        .filter(reins_org_web_passkeys::confirmed_at.is_not_null())
        .order(reins_org_web_passkeys::credential_id.asc())
        .load(c)
}

/// Confirms an unconfirmed, unrevoked passkey of `user`.
pub fn q_confirm_passkey(
    c: &mut DbConnInner,
    org: &str,
    user: &str,
    id: &str,
    device: &str,
    at: i64,
) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_web_passkeys::table
            .filter(reins_org_web_passkeys::org_uuid.eq(org))
            .filter(reins_org_web_passkeys::user_uuid.eq(user))
            .filter(reins_org_web_passkeys::credential_id.eq(id))
            .filter(reins_org_web_passkeys::confirmed_at.is_null())
            .filter(reins_org_web_passkeys::revoked_at.is_null()),
    )
    .set((reins_org_web_passkeys::confirmed_at.eq(Some(at)), reins_org_web_passkeys::confirmed_device.eq(Some(device))))
    .execute(c)?;
    Ok(n == 1)
}

pub fn q_revoke_passkey(c: &mut DbConnInner, org: &str, user: &str, id: &str, at: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_web_passkeys::table
            .filter(reins_org_web_passkeys::org_uuid.eq(org))
            .filter(reins_org_web_passkeys::user_uuid.eq(user))
            .filter(reins_org_web_passkeys::credential_id.eq(id))
            .filter(reins_org_web_passkeys::revoked_at.is_null()),
    )
    .set(reins_org_web_passkeys::revoked_at.eq(Some(at)))
    .execute(c)?;
    Ok(n == 1)
}

/// Every passkey of a member goes (deprovisioning).
pub fn q_revoke_passkeys_of(c: &mut DbConnInner, org: &str, user: &str, at: i64) -> QueryResult<usize> {
    diesel::update(
        reins_org_web_passkeys::table
            .filter(reins_org_web_passkeys::org_uuid.eq(org))
            .filter(reins_org_web_passkeys::user_uuid.eq(user))
            .filter(reins_org_web_passkeys::revoked_at.is_null()),
    )
    .set(reins_org_web_passkeys::revoked_at.eq(Some(at)))
    .execute(c)
}

// ---- Web approval tokens

pub fn q_insert_token(c: &mut DbConnInner, row: &ReinsOrgWebToken) -> QueryResult<()> {
    diesel::insert_into(reins_org_web_tokens::table).values(row).execute(c)?;
    Ok(())
}

pub fn q_token(c: &mut DbConnInner, hash: &str) -> QueryResult<Option<ReinsOrgWebToken>> {
    reins_org_web_tokens::table.filter(reins_org_web_tokens::token_hash.eq(hash)).first(c).optional()
}

/// Uses a token once: only while unused and unexpired.
pub fn q_use_token(c: &mut DbConnInner, hash: &str, now: i64) -> QueryResult<bool> {
    let n = diesel::update(
        reins_org_web_tokens::table
            .filter(reins_org_web_tokens::token_hash.eq(hash))
            .filter(reins_org_web_tokens::used_at.is_null())
            .filter(reins_org_web_tokens::expires_at.gt(now)),
    )
    .set(reins_org_web_tokens::used_at.eq(Some(now)))
    .execute(c)?;
    Ok(n == 1)
}

// ---- Posted channel messages

pub fn q_insert_posted(c: &mut DbConnInner, row: &ReinsOrgPosted) -> QueryResult<bool> {
    let exists: i64 = reins_org_posted::table
        .filter(reins_org_posted::request_uuid.eq(&row.request_uuid))
        .filter(reins_org_posted::user_uuid.eq(&row.user_uuid))
        .filter(reins_org_posted::channel.eq(&row.channel))
        .count()
        .get_result(c)?;
    if exists > 0 {
        return Ok(false);
    }
    diesel::insert_into(reins_org_posted::table).values(row).execute(c)?;
    Ok(true)
}

pub fn q_posted_of(c: &mut DbConnInner, request: &str) -> QueryResult<Vec<ReinsOrgPosted>> {
    reins_org_posted::table.filter(reins_org_posted::request_uuid.eq(request)).load(c)
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::db::models::reins_org::tests::add_org;

    fn db() -> DbConnInner {
        crate::db::models::reins_device_test_db()
    }

    #[test]
    fn people_links_are_per_org() {
        let mut c = db();
        add_org(&mut c, "o1");
        add_org(&mut c, "o2");
        q_edit_person(&mut c, "o1", "u1", 5, |p| {
            p.manager_user = Some("u2".into());
            p.manager_source = Some(manager_source::DIRECTORY.into());
            p.github_login = Some("Lee-GH".into());
            p.github_verified_via = Some(github_via::SAML.into());
        })
        .unwrap();
        q_edit_person(&mut c, "o2", "u1", 5, |p| p.github_login = Some("lee-gh".into())).unwrap();
        assert_eq!(q_person(&mut c, "o1", "u1").unwrap().unwrap().manager_user.as_deref(), Some("u2"));
        assert!(q_person(&mut c, "o2", "u1").unwrap().unwrap().manager_user.is_none());
        assert_eq!(q_by_github_login(&mut c, "o1", "lee-gh").unwrap().as_deref(), Some("u1"));
        // Unverified in o2: no match there.
        assert_eq!(q_by_github_login(&mut c, "o2", "lee-gh").unwrap(), None);
        // An edit keeps the rest of the row and can clear a field.
        q_edit_person(&mut c, "o1", "u1", 6, |p| p.manager_user = None).unwrap();
        let p = q_person(&mut c, "o1", "u1").unwrap().unwrap();
        assert_eq!((p.manager_user, p.github_login.as_deref(), p.updated_at), (None, Some("Lee-GH"), 6));
    }

    #[test]
    fn a_web_token_is_used_once() {
        let mut c = db();
        add_org(&mut c, "o1");
        q_insert_token(
            &mut c,
            &ReinsOrgWebToken {
                token_hash: "h".into(),
                org_uuid: "o1".into(),
                request_uuid: "or_1".into(),
                user_uuid: "u1".into(),
                expires_at: 100,
                used_at: None,
            },
        )
        .unwrap();
        assert!(!q_use_token(&mut c, "h", 100).unwrap(), "expired at 100");
        assert!(q_use_token(&mut c, "h", 50).unwrap());
        assert!(!q_use_token(&mut c, "h", 51).unwrap(), "used");
        assert!(!q_use_token(&mut c, "other", 51).unwrap());
        assert_eq!(q_token(&mut c, "h").unwrap().unwrap().used_at, Some(50));
    }

    fn passkey(id: &str, user: &str) -> ReinsOrgWebPasskey {
        ReinsOrgWebPasskey {
            credential_id: id.into(),
            org_uuid: "o1".into(),
            user_uuid: user.into(),
            passkey: "spki".into(),
            created_at: 1,
            confirmed_at: None,
            confirmed_device: None,
            revoked_at: None,
        }
    }

    #[test]
    fn only_confirmed_unrevoked_passkeys_are_usable() {
        let mut c = db();
        add_org(&mut c, "o1");
        assert!(q_insert_passkey(&mut c, &passkey("p1", "u1")).unwrap());
        assert!(q_insert_passkey(&mut c, &passkey("p2", "u1")).unwrap());
        assert!(!q_insert_passkey(&mut c, &passkey("p1", "u2")).unwrap(), "a credential id is one member's");
        assert!(q_usable_passkeys(&mut c, "o1", "u1").unwrap().is_empty());
        assert!(!q_confirm_passkey(&mut c, "o1", "u2", "p1", "d", 5).unwrap(), "someone else's");
        assert!(q_confirm_passkey(&mut c, "o1", "u1", "p1", "d", 5).unwrap());
        assert!(!q_confirm_passkey(&mut c, "o1", "u1", "p1", "d", 6).unwrap(), "once");
        assert!(q_confirm_passkey(&mut c, "o1", "u1", "p2", "d", 5).unwrap());
        assert_eq!(q_usable_passkeys(&mut c, "o1", "u1").unwrap().len(), 2);
        assert!(q_revoke_passkey(&mut c, "o1", "u1", "p2", 7).unwrap());
        let usable = q_usable_passkeys(&mut c, "o1", "u1").unwrap();
        assert_eq!(usable.iter().map(|p| p.credential_id.as_str()).collect::<Vec<_>>(), ["p1"]);
        // A revoked passkey can't be confirmed again.
        assert!(!q_confirm_passkey(&mut c, "o1", "u1", "p2", "d", 8).unwrap());
        assert_eq!(q_revoke_passkeys_of(&mut c, "o1", "u1", 9).unwrap(), 1);
        assert!(q_usable_passkeys(&mut c, "o1", "u1").unwrap().is_empty());
    }

    #[test]
    fn steps_append_in_order() {
        let mut c = db();
        add_org(&mut c, "o1");
        crate::db::models::reins_org_request::q_insert_request(
            &mut c,
            &crate::db::models::reins_org_request::tests::request("o1", "or_1", "reviewing"),
        )
        .unwrap();
        let step = |n: i32, ob: &str| ReinsOrgStep {
            request_uuid: "or_1".into(),
            obligation: ob.into(),
            step: n,
            reviewers: "\"manager\"".into(),
            resolved: "[]".into(),
            source: "directory".into(),
            started_at: i64::from(n),
            deadline: i64::from(n) + 10,
        };
        assert!(q_insert_step(&mut c, &step(1, "b")).unwrap());
        assert!(q_insert_step(&mut c, &step(0, "b")).unwrap());
        assert!(q_insert_step(&mut c, &step(0, "a")).unwrap());
        assert!(!q_insert_step(&mut c, &step(0, "a")).unwrap(), "once per step");
        let got: Vec<(String, i32)> =
            q_steps(&mut c, "or_1").unwrap().into_iter().map(|s| (s.obligation, s.step)).collect();
        assert_eq!(got, [("a".into(), 0), ("b".into(), 0), ("b".into(), 1)]);
    }

    #[test]
    fn sealed_secrets_never_read_back_in_clear_through_queries() {
        let mut c = db();
        add_org(&mut c, "o1");
        q_set_oncall_source(&mut c, "o1", "payments", "pagerduty", "P1").unwrap();
        assert!(!q_oncalls(&mut c, "o1").unwrap()[0].has_key);
        assert!(q_set_oncall_key(&mut c, "o1", "payments", "k1", "SEALED-KEY").unwrap());
        // Moving the schedule keeps the key; listings say only that there is one.
        q_set_oncall_source(&mut c, "o1", "payments", "pagerduty", "P2").unwrap();
        let list = q_oncalls(&mut c, "o1").unwrap();
        assert_eq!((list[0].schedule_id.as_str(), list[0].has_key), ("P2", true));
        assert!(!format!("{list:?}").contains("SEALED"));
        q_set_channel_enabled(&mut c, "o1", "slack", false).unwrap();
        assert!(q_set_channel_secret(&mut c, "o1", "slack", "k1", "SEALED-TOKEN", 5).unwrap());
        let channels = q_channels(&mut c, "o1").unwrap();
        assert!(!format!("{channels:?}").contains("SEALED") && channels[0].installed && !channels[0].enabled);
        assert!(q_channels_of_kind(&mut c, "slack").unwrap().is_empty(), "disabled");
        // Only the use queries carry what's sealed (still sealed).
        assert_eq!(q_oncall_for_use(&mut c, "o1", "payments").unwrap().unwrap().sealed.as_deref(), Some("SEALED-KEY"));
    }
}
