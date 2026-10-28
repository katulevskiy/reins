//! Family connections (Decisions v2 · P7, 08 R7): a guardian connects a member's Google or GitHub account, and only the
//! family's **executor** ever holds its token.
//!
//! 1. **Begin** (a guardian with `ManageConnections`, in the console): the server queues a `begin` job; the executor
//!    makes a PKCE verifier and a `state`, keeps them, and answers with the provider's authorization URL, where the
//!    console sends the guardian's browser.
//! 2. **Finish**: the provider sends the guardian back to [`CALLBACK_PATH`] with a one-time `code`. The server can't
//!    redeem it (it has neither the verifier nor the client secret): it queues a `finish` job holding the code sealed
//!    at rest, and clears it as soon as the executor claims the job. The executor redeems it and keeps the refresh
//!    token; its answer names the account, never a token.
//! 3. **Revoke** (a guardian, or the member's deprovisioning): the executor revokes the grant upstream and forgets it.
//!
//! The callback is bound to the guardian who began it (their console session), so a link opened by anyone else
//! connects nothing. A family member's agents see only the tools of their connected providers ([`member_tools`]).

use reins_proto::{
    managed::{ConnectDone, ConnectJob, ConnectPhase, OAuthProvider},
    org::{Capability, Tier},
};
use rocket::{Data, Route, http::Status, serde::json::Json};
use serde_json::json;

use super::{
    OrgError,
    executor_api::{ExecHeaders, authenticate},
    kms, member_of, require,
};
use crate::{
    CONFIG,
    api::reins::{device_api::read_body_limited, now_unix},
    db::{
        DbConn, DbConnInner,
        models::{
            reins_family_conn::{self, ReinsFamilyConnectJob, ReinsFamilyConnection, connection_state, job_state},
            reins_org,
        },
    },
};

/// Where the provider sends the guardian back.
pub const CALLBACK_PATH: &str = "/reins/org/family/oauth/callback";
/// How long a begin or finish job lives (the provider's code is short-lived too).
pub const CONNECT_TTL_SECS: i64 = 600;
/// How long a revocation waits for the executor.
pub const REVOKE_TTL_SECS: i64 = 7 * 86_400;
/// Jobs per claim.
const CLAIM: i64 = 10;

pub fn routes() -> Vec<Route> {
    routes![claim, done]
}

/// The server's callback URL (what the executor puts in the authorization request).
#[must_use]
pub fn redirect_uri() -> String {
    format!("{}{CALLBACK_PATH}", CONFIG.domain().trim_end_matches('/'))
}

fn family_tenant(c: &mut DbConnInner, org: &str) -> Result<(), OrgError> {
    reins_org::q_org(c, org)?.filter(|t| t.tier() == Tier::Family).map(drop).ok_or(OrgError::NotFound)
}

fn new_job(
    org: &str,
    member: &str,
    provider: OAuthProvider,
    phase: &str,
    by: &str,
    now: i64,
    ttl: i64,
) -> ReinsFamilyConnectJob {
    ReinsFamilyConnectJob {
        id: crate::crypto::encode_random_bytes::<16>(&data_encoding::HEXLOWER),
        org_uuid: org.to_owned(),
        user_uuid: member.to_owned(),
        provider: provider.as_str().to_owned(),
        phase: phase.to_owned(),
        oauth_state: None,
        code_key_id: None,
        sealed_code: None,
        auth_url: None,
        state: job_state::QUEUED.to_owned(),
        claimed_by: None,
        requested_by: by.to_owned(),
        created_at: now,
        expires_at: now + ttl,
        done_at: None,
    }
}

fn connection(org: &str, member: &str, provider: OAuthProvider, state: &str, now: i64) -> ReinsFamilyConnection {
    ReinsFamilyConnection {
        org_uuid: org.to_owned(),
        user_uuid: member.to_owned(),
        provider: provider.as_str().to_owned(),
        account: None,
        state: state.to_owned(),
        connected_at: None,
        updated_at: now,
    }
}

/// A guardian starts connecting `member`'s `provider` account; returns the job the console waits on.
pub fn begin(
    c: &mut DbConnInner,
    org: &str,
    guardian: &str,
    member: &str,
    provider: OAuthProvider,
    now: i64,
) -> Result<String, OrgError> {
    family_tenant(c, org)?;
    require(c, org, guardian, Capability::ManageConnections)?;
    member_of(c, org, member)?.ok_or(OrgError::NotFound)?;
    let existing =
        reins_family_conn::q_connections(c, org, Some(member))?.into_iter().find(|r| r.provider == provider.as_str());
    let mut row = existing.unwrap_or_else(|| connection(org, member, provider, connection_state::PENDING, now));
    if row.state != connection_state::CONNECTED {
        connection_state::PENDING.clone_into(&mut row.state);
    }
    row.updated_at = now;
    reins_family_conn::q_save_connection(c, &row)?;
    let job = new_job(org, member, provider, "begin", guardian, now, CONNECT_TTL_SECS);
    reins_family_conn::q_insert_job(c, &job)?;
    super::event(
        c,
        org,
        "family.connect_started",
        Some(guardian),
        None,
        &json!({"member": member, "provider": provider}),
        now,
    )?;
    Ok(job.id)
}

/// The authorization URL of a begin job, once the executor answered (for the guardian who began it only).
pub fn auth_url(c: &mut DbConnInner, org: &str, guardian: &str, job: &str) -> Result<Option<String>, OrgError> {
    let row = reins_family_conn::q_job(c, org, job)?.ok_or(OrgError::NotFound)?;
    if row.requested_by != guardian || row.phase != "begin" {
        return Err(OrgError::NotFound);
    }
    if row.state == job_state::FAILED {
        return Err(OrgError::Invalid("The family's executor couldn't start the connection."));
    }
    Ok(row.auth_url)
}

/// The provider's redirect: queues the finish job with the code sealed at rest. Only for the guardian who began it,
/// within its time. Returns the family.
pub fn callback(c: &mut DbConnInner, guardian: &str, state: &str, code: &str, now: i64) -> Result<String, OrgError> {
    if code.is_empty() || code.len() > 2048 || state.is_empty() || state.len() > 128 {
        return Err(OrgError::Invalid("That sign-in didn't come back as expected."));
    }
    let mut begin = reins_family_conn::q_job_by_oauth_state(c, state)?
        .filter(|j| j.phase == "begin" && j.state == job_state::DONE && j.expires_at > now)
        .ok_or(OrgError::Invalid("That sign-in expired. Start again."))?;
    if !crate::crypto::ct_eq(&begin.requested_by, guardian) {
        return Err(OrgError::Invalid("That sign-in was started by someone else."));
    }
    let provider = OAuthProvider::parse(&begin.provider).ok_or(OrgError::NotFound)?;
    let org = begin.org_uuid.clone();
    // The state moves to the finish job: a second redirect with it finds nothing.
    begin.oauth_state = None;
    reins_family_conn::q_save_job(c, &begin)?;
    let mut finish = new_job(&org, &begin.user_uuid, provider, "finish", guardian, now, CONNECT_TTL_SECS);
    let (key_id, sealed) = kms::seal_data(c, &org, &format!("{}/code", finish.id), code.as_bytes(), now)?;
    finish.oauth_state = Some(state.to_owned());
    finish.code_key_id = Some(key_id);
    finish.sealed_code = Some(sealed);
    reins_family_conn::q_insert_job(c, &finish)?;
    Ok(org)
}

/// A guardian revokes `member`'s `provider` connection.
pub fn revoke(
    c: &mut DbConnInner,
    org: &str,
    guardian: &str,
    member: &str,
    provider: OAuthProvider,
    now: i64,
) -> Result<(), OrgError> {
    family_tenant(c, org)?;
    require(c, org, guardian, Capability::ManageConnections)?;
    q_revoke(c, org, member, provider, guardian, now)?;
    super::event(
        c,
        org,
        "family.connect_revoked",
        Some(guardian),
        None,
        &json!({"member": member, "provider": provider}),
        now,
    )
}

fn q_revoke(
    c: &mut DbConnInner,
    org: &str,
    member: &str,
    provider: OAuthProvider,
    by: &str,
    now: i64,
) -> Result<(), OrgError> {
    let Some(mut row) =
        reins_family_conn::q_connections(c, org, Some(member))?.into_iter().find(|r| r.provider == provider.as_str())
    else {
        return Err(OrgError::NotFound);
    };
    if row.state == connection_state::REVOKED {
        return Ok(());
    }
    connection_state::REVOKED.clone_into(&mut row.state);
    row.updated_at = now;
    reins_family_conn::q_save_connection(c, &row)?;
    reins_family_conn::q_insert_job(c, &new_job(org, member, provider, "revoke", by, now, REVOKE_TTL_SECS))?;
    Ok(())
}

/// Deprovisioning (any reason): every connection of the member is revoked upstream by the executor.
pub fn q_revoke_all(c: &mut DbConnInner, org: &str, member: &str, now: i64) -> Result<usize, OrgError> {
    let mut n = 0;
    for row in reins_family_conn::q_connections(c, org, Some(member))? {
        if row.state != connection_state::REVOKED
            && let Some(p) = OAuthProvider::parse(&row.provider)
        {
            q_revoke(c, org, member, p, member, now)?;
            n += 1;
        }
    }
    Ok(n)
}

/// The tools a family member's agents may call: those of their connected providers (and `reins_get_result`).
/// `None` for anyone who isn't a family's member (an ordinary Managed member: the org's tools).
pub fn member_tools(c: &mut DbConnInner, org: &str, member: &str) -> Result<Option<Vec<&'static str>>, OrgError> {
    if reins_org::q_org(c, org)?.is_none_or(|t| t.tier() != Tier::Family) {
        return Ok(None);
    }
    let mut tools = vec!["reins_get_result"];
    for row in reins_family_conn::q_connections(c, org, Some(member))? {
        if row.state == connection_state::CONNECTED
            && let Some(p) = OAuthProvider::parse(&row.provider)
        {
            tools.extend_from_slice(p.tools());
        }
    }
    Ok(Some(tools))
}

/// Hands the executor of `org` its queued connect jobs. A finishing job's code leaves the server here, once.
pub fn q_claim(c: &mut DbConnInner, org: &str, executor: &str, now: i64) -> Result<Vec<ConnectJob>, OrgError> {
    if family_tenant(c, org).is_err() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for mut row in reins_family_conn::q_queued_jobs(c, org, now, CLAIM)? {
        let phase = match row.phase.as_str() {
            "begin" => ConnectPhase::Begin,
            "revoke" => ConnectPhase::Revoke,
            "finish" => {
                let (Some(key_id), Some(sealed), Some(state)) =
                    (row.code_key_id.as_deref(), row.sealed_code.as_deref(), row.oauth_state.clone())
                else {
                    continue;
                };
                let code = kms::open_data(c, org, key_id, &format!("{}/code", row.id), sealed)?;
                ConnectPhase::Finish {
                    code: String::from_utf8_lossy(&code).into_owned(),
                    state,
                }
            }
            _ => continue,
        };
        let Some(provider) = OAuthProvider::parse(&row.provider) else {
            continue;
        };
        job_state::CLAIMED.clone_into(&mut row.state);
        row.claimed_by = Some(executor.to_owned());
        row.sealed_code = None;
        row.code_key_id = None;
        reins_family_conn::q_save_job(c, &row)?;
        out.push(ConnectJob {
            member: super::console::email_of(c, &row.user_uuid)?,
            member_id: row.user_uuid.clone(),
            id: row.id,
            provider,
            phase,
            redirect_uri: redirect_uri(),
        });
    }
    Ok(out)
}

/// What the executor did with a job it claimed.
pub fn q_done(c: &mut DbConnInner, org: &str, executor: &str, d: &ConnectDone, now: i64) -> Result<(), OrgError> {
    let mut row = reins_family_conn::q_job(c, org, &d.job)?.ok_or(OrgError::NotFound)?;
    if row.state != job_state::CLAIMED || row.claimed_by.as_deref() != Some(executor) {
        return Err(OrgError::Invalid("That job isn't this executor's to finish."));
    }
    let provider = OAuthProvider::parse(&row.provider).ok_or(OrgError::NotFound)?;
    row.done_at = Some(now);
    if !d.ok {
        job_state::FAILED.clone_into(&mut row.state);
        reins_family_conn::q_save_job(c, &row)?;
        info!("Family {org}: a {} job failed: {}", row.phase, d.reason);
        return Ok(());
    }
    match row.phase.as_str() {
        "begin" => {
            let (Some(url), Some(state)) = (d.auth_url.as_deref(), d.state.as_deref()) else {
                return Err(OrgError::Invalid("A begin answers with its URL and state."));
            };
            if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1"))
                || url.len() > 4096
                || state.len() > 128
            {
                return Err(OrgError::Invalid("That authorization URL isn't one to send a guardian to."));
            }
            row.auth_url = Some(url.to_owned());
            row.oauth_state = Some(state.to_owned());
        }
        "finish" => {
            let mut conn = reins_family_conn::q_connections(c, org, Some(&row.user_uuid))?
                .into_iter()
                .find(|r| r.provider == provider.as_str())
                .unwrap_or_else(|| connection(org, &row.user_uuid, provider, connection_state::PENDING, now));
            connection_state::CONNECTED.clone_into(&mut conn.state);
            conn.account = d.account.as_deref().map(|a| a.chars().take(254).collect());
            conn.connected_at = Some(now);
            conn.updated_at = now;
            reins_family_conn::q_save_connection(c, &conn)?;
            row.oauth_state = None;
            super::audit::record(
                c,
                org,
                "family.connected",
                Some(&row.requested_by),
                None,
                &json!({"member": row.user_uuid, "provider": provider, "account": conn.account}),
                now,
            )?;
        }
        _ => {}
    }
    job_state::DONE.clone_into(&mut row.state);
    reins_family_conn::q_save_job(c, &row).map_err(OrgError::from)
}

#[post("/reins/org-exec/connect/claim", data = "<data>")]
async fn claim(h: ExecHeaders, data: Data<'_>, conn: DbConn) -> Result<Json<Vec<ConnectJob>>, Status> {
    let body = read_body_limited(data, 4096).await.map_err(|_| Status::PayloadTooLarge)?;
    let ex = authenticate(&h, "POST", &body, &conn).await?;
    let now = now_unix();
    conn.run(move |c| q_claim(c, &ex.org, &ex.id, now)).await.map(Json).map_err(|e| {
        warn!("Family connect claim: {e:?}");
        Status::InternalServerError
    })
}

#[post("/reins/org-exec/connect/done", data = "<data>")]
async fn done(h: ExecHeaders, data: Data<'_>, conn: DbConn) -> Status {
    let Ok(body) = read_body_limited(data, 16 * 1024).await else {
        return Status::PayloadTooLarge;
    };
    let ex = match authenticate(&h, "POST", &body, &conn).await {
        Ok(ex) => ex,
        Err(s) => return s,
    };
    let Ok(d) = serde_json::from_slice::<ConnectDone>(&body) else {
        return Status::BadRequest;
    };
    let now = now_unix();
    match conn.run(move |c| q_done(c, &ex.org, &ex.id, &d, now)).await {
        Ok(()) => Status::NoContent,
        Err(OrgError::NotFound) => Status::NotFound,
        Err(OrgError::Invalid(m)) => {
            info!("Family connect answer refused: {m}");
            Status::Conflict
        }
        Err(e) => {
            warn!("Family connect answer: {e:?}");
            Status::InternalServerError
        }
    }
}

/// Drops connect jobs whose time ran out (with any code they still held).
pub fn sweep(c: &mut DbConnInner, now: i64) -> Result<(), OrgError> {
    reins_family_conn::q_expire_jobs(c, now)?;
    Ok(())
}

#[cfg(all(test, sqlite))]
mod tests {
    use diesel::prelude::*;

    use super::super::{
        family::{self, tests::fam, tests::joined},
        kms::tests::use_test_kms,
        testing::db,
    };
    use super::*;
    use crate::db::schema::{reins_family_connect_jobs, reins_family_connections, reins_org_events};

    /// Every text column the family's connection tables and events hold.
    fn everything(c: &mut DbConnInner) -> String {
        let jobs: Vec<ReinsFamilyConnectJob> = reins_family_connect_jobs::table.load(c).unwrap();
        let conns: Vec<ReinsFamilyConnection> = reins_family_connections::table.load(c).unwrap();
        let events: Vec<String> = reins_org_events::table.select(reins_org_events::detail).load(c).unwrap();
        format!("{jobs:?}{conns:?}{events:?}")
    }

    fn done_ok(job: &str) -> ConnectDone {
        ConnectDone {
            job: job.to_owned(),
            ok: true,
            auth_url: None,
            state: None,
            account: None,
            reason: String::new(),
        }
    }

    #[test]
    fn the_server_never_holds_a_family_token() {
        use_test_kms();
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "sam", true, now);
        let job = begin(&mut c, &org, "mom", "sam", OAuthProvider::Google, now).unwrap();
        // The executor answers the begin with its URL and state.
        let jobs = q_claim(&mut c, &org, "e1", now).unwrap();
        assert_eq!((jobs.len(), &jobs[0].phase), (1, &ConnectPhase::Begin));
        assert_eq!(jobs[0].member, "sam@example.com");
        let mut d = done_ok(&job);
        d.auth_url = Some("https://accounts.google.com/o/oauth2/v2/auth?state=st-1".into());
        d.state = Some("st-1".into());
        q_done(&mut c, &org, "e1", &d, now).unwrap();
        assert!(auth_url(&mut c, &org, "mom", &job).unwrap().is_some());
        assert!(auth_url(&mut c, &org, "sam", &job).is_err(), "only the guardian who began it");
        // Someone else's browser coming back with the state connects nothing.
        assert!(callback(&mut c, "sam", "st-1", "CODE-123", now).is_err());
        assert_eq!(callback(&mut c, "mom", "st-1", "CODE-123", now).unwrap(), org);
        assert!(callback(&mut c, "mom", "st-1", "CODE-123", now).is_err(), "once");
        // At rest the code is sealed; it leaves once, to the executor, and is then gone.
        assert!(!everything(&mut c).contains("CODE-123"));
        let jobs = q_claim(&mut c, &org, "e1", now).unwrap();
        assert_eq!(
            jobs[0].phase,
            ConnectPhase::Finish {
                code: "CODE-123".into(),
                state: "st-1".into()
            }
        );
        assert!(!everything(&mut c).contains("CODE-123"));
        assert!(q_claim(&mut c, &org, "e1", now).unwrap().is_empty());
        // The executor's answer names the account, never a token; the server has nowhere to keep one.
        let mut d = done_ok(&jobs[0].id);
        d.account = Some("sam.kid@gmail.com".into());
        q_done(&mut c, &org, "e1", &d, now).unwrap();
        let conns = reins_family_conn::q_connections(&mut c, &org, Some("sam")).unwrap();
        assert_eq!((conns[0].state.as_str(), conns[0].account.as_deref()), ("connected", Some("sam.kid@gmail.com")));
        // Another executor can't finish someone's job.
        assert!(q_done(&mut c, &org, "e2", &done_ok(&jobs[0].id), now).is_err());
    }

    #[test]
    fn family_tools_follow_connected_providers() {
        use_test_kms();
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "sam", true, now);
        assert_eq!(member_tools(&mut c, &org, "sam").unwrap(), Some(vec!["reins_get_result"]));
        reins_family_conn::q_save_connection(
            &mut c,
            &connection(&org, "sam", OAuthProvider::Github, connection_state::CONNECTED, now),
        )
        .unwrap();
        let tools = member_tools(&mut c, &org, "sam").unwrap().unwrap();
        assert!(tools.contains(&"github_create_issue") && !tools.contains(&"gmail_send"), "{tools:?}");
        // Revoked: gone, and the executor is told to revoke upstream.
        revoke(&mut c, &org, "mom", "sam", OAuthProvider::Github, now).unwrap();
        assert_eq!(member_tools(&mut c, &org, "sam").unwrap(), Some(vec!["reins_get_result"]));
        let jobs = q_claim(&mut c, &org, "e1", now).unwrap();
        assert_eq!(jobs[0].phase, ConnectPhase::Revoke);
        // Only guardians connect or revoke.
        assert_eq!(begin(&mut c, &org, "sam", "sam", OAuthProvider::Google, now), Err(OrgError::Forbidden));
        // An ordinary org's members are unaffected.
        reins_org::tests::add_org(&mut c, "acme");
        assert_eq!(member_tools(&mut c, "acme", "x").unwrap(), None);
    }

    #[test]
    fn a_child_agent_acts_only_through_family_connections() {
        use_test_kms();
        let mut c = db();
        let now = now_unix();
        let org = fam(&mut c, now);
        joined(&mut c, &org, "sam", true, now);
        reins_family_conn::q_save_connection(
            &mut c,
            &connection(&org, "sam", OAuthProvider::Google, connection_state::CONNECTED, now),
        )
        .unwrap();
        // Leaving the family revokes what the guardian connected: the executor revokes it upstream.
        family::consent_leave(&mut c, &org, "mom", "sam", now).unwrap_err();
        family::request_leave(&mut c, &org, "sam", now).unwrap();
        family::consent_leave(&mut c, &org, "mom", "sam", now).unwrap();
        family::sweep(&mut c, now).unwrap();
        let conns = reins_family_conn::q_connections(&mut c, &org, Some("sam")).unwrap();
        assert_eq!(conns[0].state, "revoked");
        let jobs = q_claim(&mut c, &org, "e1", now).unwrap();
        assert!(jobs.iter().any(|j| j.phase == ConnectPhase::Revoke && j.member_id == "sam"));
        // And they're no longer the family's member: no tool, no Managed request.
        assert!(member_of(&mut c, &org, "sam").unwrap().is_none());
    }
}
