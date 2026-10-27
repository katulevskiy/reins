//! Short-lived managed credentials (`docs/design/decisions-v2/05-organizations.md` §5: "Git credentials come from the
//! executor as short leases … issued by the org instead of a phone").
//!
//! A Managed desktop daemon asks with its connection's token for read or push access to one repository and an X25519
//! key to seal the credential to. The request is decided like any other (`git_lease`, judged by the repository's zone
//! and the org's rules); the executor mints a credential narrowed to that repository and seals it to the key, so the
//! server never holds it in clear. A lease lives [`decisions::LEASE_SECS`] at most; deprovisioning revokes every lease
//! of the member, and the executor then revokes the credential upstream.

use reins_proto::managed::{LEASE_TOOL, LeaseRequest, LeaseView, lease_args};
use rocket::{Route, http::Status, serde::json::Json};
use serde_json::json;

use super::{OrgError, audit, decisions, enabled};
use crate::{
    api::reins::{
        mcp_routes::{McpRequest, authenticate},
        now_unix,
    },
    db::{
        DbConn, DbConnInner, DbPool,
        models::{
            ReinsConnection, reins_org,
            reins_org_request::{self, ReinsOrgLease, lease_state},
        },
    },
};

pub fn routes() -> Vec<Route> {
    routes![post_lease, get_lease]
}

/// `owner/name` with GitHub's characters.
#[must_use]
pub fn repo_ok(repo: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    matches!(repo.split_once('/'), Some((o, n)) if ok(o) && ok(n) && !n.contains('/'))
}

/// An X25519 public key, base64url without padding.
#[must_use]
pub fn box_key_ok(key: &str) -> bool {
    reins_proto::canon::decode_payload(key).is_some_and(|k| k.len() == 32)
}

/// The org a connection's account is an active member of.
pub fn managed_org(c: &mut DbConnInner, user: &str) -> Result<Option<String>, OrgError> {
    for o in reins_org::q_orgs_of_user(c, user)? {
        if super::member_of(c, &o.org_uuid, user)?.is_some() {
            return Ok(Some(o.org_uuid));
        }
    }
    Ok(None)
}

fn view(l: &ReinsOrgLease, explain: String) -> LeaseView {
    LeaseView {
        id: l.uuid.clone(),
        state: l.state.clone(),
        sealed: l.sealed.clone().filter(|_| l.state == lease_state::ISSUED),
        expires_at: l.expires_at,
        explain,
    }
}

/// Asks for a lease; returns its view and whom to notify for a review.
pub fn q_request_lease(
    c: &mut DbConnInner,
    connection: &ReinsConnection,
    ask: &LeaseRequest,
    now: i64,
) -> Result<(LeaseView, Vec<String>, String), OrgError> {
    if !repo_ok(&ask.repo) || !box_key_ok(&ask.box_key) {
        return Err(OrgError::Invalid("Name the repository as owner/name and give an X25519 key."));
    }
    let user = connection.user_uuid.to_string();
    let org = managed_org(c, &user)?.ok_or(OrgError::NotFound)?;
    let lease = decisions::LeaseAsk {
        id: format!("ol_{}", crate::util::get_uuid()),
        repo: ask.repo.to_ascii_lowercase(),
        access: ask.access,
        box_key: ask.box_key.clone(),
    };
    let args = lease_args(&lease.repo, lease.access, &lease.box_key);
    let s =
        decisions::submit(c, &org, &user, &connection.uuid, &connection.label, LEASE_TOOL, &args, Some(&lease), now)?;
    let row = reins_org_request::q_lease(c, &org, &lease.id)?.ok_or(OrgError::NotFound)?;
    Ok((view(&row, s.explain), s.notify, row.request_uuid))
}

/// A lease, for the connection that asked for it only.
pub fn q_lease_view(c: &mut DbConnInner, connection: &ReinsConnection, id: &str) -> Result<LeaseView, OrgError> {
    let user = connection.user_uuid.to_string();
    let org = managed_org(c, &user)?.ok_or(OrgError::NotFound)?;
    let l = reins_org_request::q_lease(c, &org, id)?.ok_or(OrgError::NotFound)?;
    if !crate::crypto::ct_eq(&l.connection_uuid, &connection.uuid) {
        return Err(OrgError::NotFound);
    }
    Ok(view(&l, String::new()))
}

/// Deprovisioning: every lease of `user` in `org` is revoked (the executor revokes the issued ones upstream).
pub fn q_revoke_for_deprovision(c: &mut DbConnInner, org: &str, user: &str, now: i64) -> Result<usize, OrgError> {
    let revoked = reins_org_request::q_revoke_leases_of_user(c, org, user, now)?;
    for l in &revoked {
        audit::record(
            c,
            org,
            audit::kinds::LEASE_REVOKED,
            None,
            Some(&l.uuid),
            &json!({"reason": "member_removed", "repo": l.repo, "was": l.state}),
            now,
        )?;
    }
    Ok(revoked.len())
}

fn status_of(e: &OrgError) -> Status {
    match e {
        OrgError::NotFound | OrgError::Forbidden => Status::NotFound,
        OrgError::Invalid(_) | OrgError::Taken(_) => Status::BadRequest,
        OrgError::Internal(m) => {
            warn!("Lease: {m}");
            Status::InternalServerError
        }
    }
}

async fn connection_of(request: &McpRequest, conn: &DbConn) -> Result<ReinsConnection, Status> {
    if !enabled() || request.origin.is_some() {
        return Err(Status::NotFound);
    }
    authenticate(request.authorization.as_deref(), conn).await.map_err(|_| Status::Unauthorized)
}

#[post("/reins/api/org/leases", data = "<body>")]
async fn post_lease(
    request: McpRequest,
    body: Json<LeaseRequest>,
    conn: DbConn,
    pool: &rocket::State<DbPool>,
) -> Result<Json<LeaseView>, (Status, Json<serde_json::Value>)> {
    let connection = connection_of(&request, &conn).await.map_err(|s| (s, Json(json!({}))))?;
    let ask = body.into_inner();
    let now = now_unix();
    let (view, notify, request_id) = conn
        .run(move |c| q_request_lease(c, &connection, &ask, now))
        .await
        .map_err(|e| (status_of(&e), Json(json!({"error": e.message()}))))?;
    super::reviews::notify(pool.inner(), &notify, &request_id);
    Ok(Json(view))
}

#[get("/reins/api/org/leases/<id>")]
async fn get_lease(id: &str, request: McpRequest, conn: DbConn) -> Result<Json<LeaseView>, Status> {
    let connection = connection_of(&request, &conn).await?;
    let id = id.to_owned();
    conn.run(move |c| q_lease_view(c, &connection, &id)).await.map(Json).map_err(|e| status_of(&e))
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::managed::LeaseAccess;

    use super::*;
    use crate::api::reins::org::{decisions::tests::pilot, testing::db};
    use crate::db::models::UserId;

    fn conn_of(user: &str, id: &str) -> ReinsConnection {
        ReinsConnection {
            uuid: id.into(),
            user_uuid: UserId::from(user.to_owned()),
            client_id: "c".into(),
            client_name: "Reins desktop".into(),
            client_host: "h".into(),
            label: "Laptop".into(),
            created_at: 1,
            last_used_at: None,
            sealed_name: None,
        }
    }

    fn ask(repo: &str) -> LeaseRequest {
        LeaseRequest {
            repo: repo.into(),
            access: LeaseAccess::Push,
            box_key: reins_proto::canon::encode_payload(&[9u8; 32]),
        }
    }

    #[test]
    fn requests_are_checked_and_decided() {
        let mut c = db();
        pilot(&mut c);
        let lee = conn_of("lee", "conn-lee");
        assert!(q_request_lease(&mut c, &lee, &ask("acme"), 100).is_err());
        let mut bad_key = ask("acme/api");
        bad_key.box_key = "short".into();
        assert!(q_request_lease(&mut c, &lee, &bad_key, 100).is_err());
        let (v, _, _) = q_request_lease(&mut c, &lee, &ask("acme/api"), 100).unwrap();
        assert_eq!((v.state.as_str(), v.sealed), (lease_state::PENDING, None));
        assert!(q_lease_view(&mut c, &conn_of("lee", "conn-other"), &v.id).is_err(), "another connection");
        assert!(q_request_lease(&mut c, &conn_of("bob", "conn-bob"), &ask("acme/api"), 100).is_err(), "not a member");
    }

    #[test]
    fn deprovision_revokes_every_lease_of_the_member() {
        let mut c = db();
        pilot(&mut c);
        let lee = conn_of("lee", "conn-lee");
        let (pending, _, _) = q_request_lease(&mut c, &lee, &ask("acme/api"), 100).unwrap();
        let (issued, _, _) = q_request_lease(&mut c, &lee, &ask("acme/web"), 100).unwrap();
        // The executor issues one.
        let l = reins_org_request::q_lease(&mut c, "o1", &issued.id).unwrap().unwrap();
        reins_org_request::q_set_lease_issued(&mut c, "o1", &l.uuid, "sealed", l.expires_at, 101).unwrap();
        assert_eq!(q_lease_view(&mut c, &lee, &issued.id).unwrap().sealed.as_deref(), Some("sealed"));
        assert_eq!(q_revoke_for_deprovision(&mut c, "o1", "lee", 110).unwrap(), 2);
        let after = q_lease_view(&mut c, &lee, &issued.id).unwrap();
        assert_eq!((after.state.as_str(), after.sealed), (lease_state::REVOKED, None));
        assert_eq!(q_lease_view(&mut c, &lee, &pending.id).unwrap().state, lease_state::REVOKED);
        // The issued one goes to the executor for upstream revocation.
        let todo: Vec<String> =
            reins_org_request::q_leases_to_revoke(&mut c, "o1").unwrap().into_iter().map(|l| l.uuid).collect();
        assert_eq!(todo, [issued.id]);
    }

    #[test]
    fn no_lease_is_issued_after_removal() {
        let mut c = db();
        pilot(&mut c);
        let lee = conn_of("lee", "conn-lee");
        let (v, _, _) = q_request_lease(&mut c, &lee, &ask("acme/api"), 100).unwrap();
        q_revoke_for_deprovision(&mut c, "o1", "lee", 110).unwrap();
        assert!(!reins_org_request::q_set_lease_issued(&mut c, "o1", &v.id, "sealed", 900, 111).unwrap());
        // A credential minted in the race is still queued for revocation upstream.
        reins_org_request::q_mark_issued_after_revocation(&mut c, "o1", &v.id, 111).unwrap();
        assert_eq!(reins_org_request::q_leases_to_revoke(&mut c, "o1").unwrap().len(), 1);
        assert_eq!(q_lease_view(&mut c, &lee, &v.id).unwrap().sealed, None);
    }
}
