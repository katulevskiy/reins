//! Policy as code, simulation and staged rollout (Decisions v2 · O4): the org's GitHub reader and policy repo, stored
//! simulation reports, rollouts, and the webhook deliveries already acted on.

use diesel::prelude::*;

use crate::db::{
    DbConnInner,
    schema::{
        reins_org_github_reader, reins_org_policies, reins_org_policy_repo, reins_org_requests, reins_org_rollouts,
        reins_org_simulations, reins_org_webhook_deliveries,
    },
};

use super::{reins_org_keys::ReinsOrgPolicy, reins_org_request::ReinsOrgRequest};

/// The org's GitHub reader App. `sealed` / `webhook_sealed`: the private key and webhook secret, sealed under the org's
/// data key (`kms::seal_data`), once posted.
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

/// The org's policy repo binding.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_policy_repo)]
#[diesel(primary_key(org_uuid))]
pub struct ReinsOrgPolicyRepo {
    pub org_uuid: String,
    /// `owner/name`, lowercase.
    pub repo: String,
    pub default_branch: String,
    pub bound_by: String,
    pub bound_at: i64,
}

pub fn q_policy_repo(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrgPolicyRepo>> {
    reins_org_policy_repo::table.filter(reins_org_policy_repo::org_uuid.eq(org)).first(c).optional()
}

pub fn q_policy_repo_by_name(c: &mut DbConnInner, repo: &str) -> QueryResult<Option<ReinsOrgPolicyRepo>> {
    reins_org_policy_repo::table.filter(reins_org_policy_repo::repo.eq(repo)).first(c).optional()
}

/// Binds `row.repo` to its org, replacing the org's previous binding. `false` when another org has the repo.
pub fn q_bind_policy_repo(c: &mut DbConnInner, row: &ReinsOrgPolicyRepo) -> QueryResult<bool> {
    if q_policy_repo_by_name(c, &row.repo)?.is_some_and(|r| r.org_uuid != row.org_uuid) {
        return Ok(false);
    }
    q_unbind_policy_repo(c, &row.org_uuid)?;
    diesel::insert_into(reins_org_policy_repo::table).values(row).execute(c).map(|_| true)
}

pub fn q_unbind_policy_repo(c: &mut DbConnInner, org: &str) -> QueryResult<()> {
    diesel::delete(reins_org_policy_repo::table.filter(reins_org_policy_repo::org_uuid.eq(org))).execute(c).map(drop)
}

/// A stored simulation report (`reins_policy::simulate::SimulationReport` JSON).
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_simulations)]
#[diesel(primary_key(org_uuid, report_hash))]
pub struct ReinsOrgSimulation {
    pub org_uuid: String,
    pub report_hash: String,
    pub revision: i64,
    pub draft_hash: String,
    pub base_hash: String,
    pub source_ref: Option<String>,
    pub report: String,
    pub ran_at: i64,
}

/// Stores a report (once: the same report hash is the same report).
pub fn q_insert_simulation(c: &mut DbConnInner, row: &ReinsOrgSimulation) -> QueryResult<()> {
    if q_simulation(c, &row.org_uuid, &row.report_hash)?.is_some() {
        return Ok(());
    }
    diesel::insert_into(reins_org_simulations::table).values(row).execute(c).map(drop)
}

pub fn q_simulation(c: &mut DbConnInner, org: &str, report: &str) -> QueryResult<Option<ReinsOrgSimulation>> {
    reins_org_simulations::table
        .filter(reins_org_simulations::org_uuid.eq(org))
        .filter(reins_org_simulations::report_hash.eq(report))
        .first(c)
        .optional()
}

/// The newest report for a draft hash.
pub fn q_last_simulation(c: &mut DbConnInner, org: &str, draft_hash: &str) -> QueryResult<Option<ReinsOrgSimulation>> {
    reins_org_simulations::table
        .filter(reins_org_simulations::org_uuid.eq(org))
        .filter(reins_org_simulations::draft_hash.eq(draft_hash))
        .order(reins_org_simulations::ran_at.desc())
        .first(c)
        .optional()
}

/// [`ReinsOrgRollout::state`] values.
pub mod rollout_state {
    pub const RUNNING: &str = "running";
    pub const PAUSED: &str = "paused";
    pub const COMPLETE: &str = "complete";
    pub const ROLLED_BACK: &str = "rolled_back";
}

/// A staged rollout: `envelopes` holds every step's issuer-signed envelope (JSON array, seq 1..n+1); the first
/// `published_seq` stages are published (the rollback, seq n+1, once rolled back).
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_rollouts)]
#[diesel(primary_key(rollout_id))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrgRollout {
    pub rollout_id: String,
    pub org_uuid: String,
    /// The candidate's revision.
    pub revision: i64,
    pub epoch: i64,
    pub state: String,
    pub stable: String,
    pub candidate: String,
    /// `reins_proto::rollout::RolloutPlan` JSON (pilot members resolved).
    pub plan: String,
    pub envelopes: String,
    pub published_seq: i32,
    pub stage_started_at: i64,
    /// Why it was rolled back (the numbers), when it was.
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn q_insert_rollout(c: &mut DbConnInner, row: &ReinsOrgRollout) -> QueryResult<()> {
    diesel::insert_into(reins_org_rollouts::table).values(row).execute(c).map(drop)
}

pub fn q_rollout(c: &mut DbConnInner, org: &str, id: &str) -> QueryResult<Option<ReinsOrgRollout>> {
    reins_org_rollouts::table
        .filter(reins_org_rollouts::org_uuid.eq(org))
        .filter(reins_org_rollouts::rollout_id.eq(id))
        .first(c)
        .optional()
}

/// The org's rollout in progress (running or paused), if any.
pub fn q_open_rollout(c: &mut DbConnInner, org: &str) -> QueryResult<Option<ReinsOrgRollout>> {
    reins_org_rollouts::table
        .filter(reins_org_rollouts::org_uuid.eq(org))
        .filter(reins_org_rollouts::state.eq_any([rollout_state::RUNNING, rollout_state::PAUSED]))
        .order(reins_org_rollouts::epoch.desc())
        .first(c)
        .optional()
}

/// The org's rollouts updated since `since` or still open, newest first.
pub fn q_recent_rollouts(c: &mut DbConnInner, org: &str, since: i64) -> QueryResult<Vec<ReinsOrgRollout>> {
    reins_org_rollouts::table
        .filter(reins_org_rollouts::org_uuid.eq(org))
        .filter(
            reins_org_rollouts::updated_at
                .ge(since)
                .or(reins_org_rollouts::state.eq_any([rollout_state::RUNNING, rollout_state::PAUSED])),
        )
        .order(reins_org_rollouts::epoch.desc())
        .load(c)
}

/// Every running rollout, of every org (the controller's tick).
pub fn q_running_rollouts(c: &mut DbConnInner) -> QueryResult<Vec<ReinsOrgRollout>> {
    reins_org_rollouts::table.filter(reins_org_rollouts::state.eq(rollout_state::RUNNING)).load(c)
}

/// The org's highest rollout epoch (0 before the first).
pub fn q_last_epoch(c: &mut DbConnInner, org: &str) -> QueryResult<i64> {
    let max: Option<i64> = reins_org_rollouts::table
        .filter(reins_org_rollouts::org_uuid.eq(org))
        .select(diesel::dsl::max(reins_org_rollouts::epoch))
        .first(c)?;
    Ok(max.unwrap_or(0))
}

/// Saves `row` if its state is still `from` (one writer moves a rollout).
pub fn q_save_rollout(c: &mut DbConnInner, row: &ReinsOrgRollout, from: &str) -> QueryResult<bool> {
    diesel::update(
        reins_org_rollouts::table
            .filter(reins_org_rollouts::rollout_id.eq(&row.rollout_id))
            .filter(reins_org_rollouts::state.eq(from)),
    )
    .set(row)
    .execute(c)
    .map(|n| n == 1)
}

/// Records a webhook delivery; `false` when it was already seen. Deliveries older than a day are forgotten.
pub fn q_first_delivery(c: &mut DbConnInner, delivery: &str, now: i64) -> QueryResult<bool> {
    diesel::delete(
        reins_org_webhook_deliveries::table.filter(reins_org_webhook_deliveries::received_at.lt(now - 86_400)),
    )
    .execute(c)?;
    let seen: i64 = reins_org_webhook_deliveries::table
        .filter(reins_org_webhook_deliveries::delivery_id.eq(delivery))
        .count()
        .get_result(c)?;
    if seen > 0 {
        return Ok(false);
    }
    diesel::insert_into(reins_org_webhook_deliveries::table)
        .values((
            reins_org_webhook_deliveries::delivery_id.eq(delivery),
            reins_org_webhook_deliveries::received_at.eq(now),
        ))
        .execute(c)
        .map(|_| true)
}

/// Decided requests of `org` created in `[since, until)`, oldest first, at most `limit` (simulation's history).
pub fn q_history(
    c: &mut DbConnInner,
    org: &str,
    since: i64,
    until: i64,
    limit: i64,
) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::created_at.ge(since))
        .filter(reins_org_requests::created_at.lt(until))
        .order((reins_org_requests::created_at.asc(), reins_org_requests::uuid.asc()))
        .limit(limit)
        .load(c)
}

/// Requests of `org` decided under rollout `rollout` since `since` (the guards' measurements).
pub fn q_rollout_requests(
    c: &mut DbConnInner,
    org: &str,
    rollout: &str,
    since: i64,
) -> QueryResult<Vec<ReinsOrgRequest>> {
    reins_org_requests::table
        .filter(reins_org_requests::org_uuid.eq(org))
        .filter(reins_org_requests::rollout_id.eq(rollout))
        .filter(reins_org_requests::created_at.ge(since))
        .load(c)
}

/// Moves a policy revision from one state to another, with its envelope (activation, candidacy, rollback).
pub fn q_set_policy_state(
    c: &mut DbConnInner,
    org: &str,
    revision: i64,
    from: &[&str],
    to: &str,
    envelope: Option<&str>,
    at: Option<i64>,
) -> QueryResult<bool> {
    let target = reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(reins_org_policies::revision.eq(revision))
        .filter(reins_org_policies::state.eq_any(from));
    let n = match envelope {
        Some(env) => diesel::update(target)
            .set((
                reins_org_policies::state.eq(to),
                reins_org_policies::envelope.eq(Some(env)),
                reins_org_policies::activated_at.eq(at),
            ))
            .execute(c)?,
        None => diesel::update(target).set(reins_org_policies::state.eq(to)).execute(c)?,
    };
    Ok(n == 1)
}

/// Retires the org's active policy (when a candidate becomes active).
pub fn q_retire_active(c: &mut DbConnInner, org: &str) -> QueryResult<usize> {
    diesel::update(
        reins_org_policies::table
            .filter(reins_org_policies::org_uuid.eq(org))
            .filter(reins_org_policies::state.eq(super::reins_org_keys::policy_state::ACTIVE)),
    )
    .set(reins_org_policies::state.eq(super::reins_org_keys::policy_state::RETIRED))
    .execute(c)
}

/// The org's drafts and candidates, newest first.
pub fn q_drafts(c: &mut DbConnInner, org: &str, limit: i64) -> QueryResult<Vec<ReinsOrgPolicy>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(
            reins_org_policies::state
                .eq_any([super::reins_org_keys::policy_state::DRAFT, super::reins_org_keys::policy_state::CANDIDATE]),
        )
        .order(reins_org_policies::revision.desc())
        .limit(limit)
        .load(c)
}

/// The newest revision drafted from `source_prefix` (`pr#12@`): a newer push to the same PR supersedes older ones.
pub fn q_newest_from_source(c: &mut DbConnInner, org: &str, source_prefix: &str) -> QueryResult<Option<i64>> {
    reins_org_policies::table
        .filter(reins_org_policies::org_uuid.eq(org))
        .filter(reins_org_policies::source_ref.like(format!("{source_prefix}%")))
        .select(diesel::dsl::max(reins_org_policies::revision))
        .first(c)
}
