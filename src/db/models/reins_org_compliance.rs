//! Organizations, O5: audit streaming sinks (`api::reins::org::sinks`) and RFC 3161 timestamps of audit checkpoints
//! (`api::reins::org::witness`).

#![allow(dead_code, reason = "TEMP: the audit sinks and the console use the rest")]

use diesel::prelude::*;

use super::reins_org_exec::ReinsOrgCheckpoint;
use crate::db::{
    DbConnInner,
    schema::{reins_org_audit_sinks, reins_org_checkpoints, reins_org_witness},
};

/// One org's audit sink: its configuration sealed under the tenant data key, and how far it was delivered.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_org_audit_sinks)]
#[diesel(primary_key(org_uuid, kind))]
#[diesel(treat_none_as_null = true)]
pub struct ReinsOrgAuditSink {
    pub org_uuid: String,
    pub kind: String,
    pub key_id: String,
    pub sealed: String,
    /// SHA-256 (hex) of the canonical configuration: what a two-person enable names.
    pub digest: String,
    pub enabled: bool,
    /// The last event sequence number delivered (for object storage: always a checkpoint's).
    pub sent_seq: i64,
    pub last_ok_at: Option<i64>,
    /// Why the last delivery failed (host and status only).
    pub last_error: Option<String>,
    pub failing: bool,
    pub updated_at: i64,
}

pub fn q_sink(c: &mut DbConnInner, org: &str, kind: &str) -> QueryResult<Option<ReinsOrgAuditSink>> {
    reins_org_audit_sinks::table
        .filter(reins_org_audit_sinks::org_uuid.eq(org))
        .filter(reins_org_audit_sinks::kind.eq(kind))
        .first(c)
        .optional()
}

pub fn q_sinks_of(c: &mut DbConnInner, org: &str) -> QueryResult<Vec<ReinsOrgAuditSink>> {
    reins_org_audit_sinks::table
        .filter(reins_org_audit_sinks::org_uuid.eq(org))
        .order(reins_org_audit_sinks::kind.asc())
        .load(c)
}

/// Every org's sinks that are on.
pub fn q_enabled_sinks(c: &mut DbConnInner) -> QueryResult<Vec<ReinsOrgAuditSink>> {
    reins_org_audit_sinks::table
        .filter(reins_org_audit_sinks::enabled.eq(true))
        .order((reins_org_audit_sinks::org_uuid.asc(), reins_org_audit_sinks::kind.asc()))
        .load(c)
}

/// Inserts or replaces a sink row.
pub fn q_save_sink(c: &mut DbConnInner, row: &ReinsOrgAuditSink) -> QueryResult<()> {
    let n = diesel::update(
        reins_org_audit_sinks::table
            .filter(reins_org_audit_sinks::org_uuid.eq(&row.org_uuid))
            .filter(reins_org_audit_sinks::kind.eq(&row.kind)),
    )
    .set(row)
    .execute(c)?;
    if n == 0 {
        diesel::insert_into(reins_org_audit_sinks::table).values(row).execute(c)?;
    }
    Ok(())
}

/// A checkpoint's timestamp from an RFC 3161 authority.
#[derive(Clone, Debug, PartialEq, Eq, Identifiable, Queryable, Insertable)]
#[diesel(table_name = reins_org_witness)]
#[diesel(primary_key(org_uuid, seq))]
pub struct ReinsOrgWitness {
    pub org_uuid: String,
    pub seq: i64,
    /// The authority's URL.
    pub tsa: String,
    /// The DER `TimeStampResp`, base64.
    pub response: String,
    pub at: i64,
}

pub fn q_witness(c: &mut DbConnInner, org: &str, seq: i64) -> QueryResult<Option<ReinsOrgWitness>> {
    reins_org_witness::table
        .filter(reins_org_witness::org_uuid.eq(org))
        .filter(reins_org_witness::seq.eq(seq))
        .first(c)
        .optional()
}

pub fn q_insert_witness(c: &mut DbConnInner, row: &ReinsOrgWitness) -> QueryResult<()> {
    diesel::insert_into(reins_org_witness::table).values(row).execute(c).map(drop)
}

/// The oldest `limit` checkpoints (any org) without a timestamp.
pub fn q_unwitnessed(c: &mut DbConnInner, limit: i64) -> QueryResult<Vec<ReinsOrgCheckpoint>> {
    reins_org_checkpoints::table
        .filter(diesel::dsl::not(diesel::dsl::exists(
            reins_org_witness::table
                .filter(reins_org_witness::org_uuid.eq(reins_org_checkpoints::org_uuid))
                .filter(reins_org_witness::seq.eq(reins_org_checkpoints::seq)),
        )))
        .order((reins_org_checkpoints::at.asc(), reins_org_checkpoints::seq.asc()))
        .limit(limit)
        .load(c)
}

/// Deletes `org`'s timestamps of checkpoints before `seq` (pruning).
pub fn q_delete_witness_before(c: &mut DbConnInner, org: &str, seq: i64) -> QueryResult<usize> {
    diesel::delete(
        reins_org_witness::table.filter(reins_org_witness::org_uuid.eq(org)).filter(reins_org_witness::seq.lt(seq)),
    )
    .execute(c)
}
