//! Guardian notices (Decisions v2 P7, 04 §4, 08 R7): what the server observes about a ward and tells each of its
//! guardians.
//!
//! Personal guardianship is cooperative: the ward's phone holds the credentials and the server is blind, so leaving
//! can't be prevented, only made visible. The server routes, so it does see a few things in clear, and writes a notice
//! to every guardian of the ward (a Guardian link where the ward is the owner and that hasn't ended) for each:
//! - the account was reset, or deleted (written **before** the rows go);
//! - the ward's roster changed (a handover, a recovery, a phone removed);
//! - the guardianship changed (a signed step, or the end its notice reached).
//!
//! The guardian's phone checks what it can (a signed change against its own copy of the terms, a roster change against
//! the chain it pinned) and labels the rest "Reported by the server". The server can drop or delay a notice; it can't
//! forge what a phone checks.
//!
//! A notice that needs the guardian (a reset, a deletion, a recovery, an end request) pushes an alert
//! (`PushKind::Circle`, `alert:<link>`); the rest wait for the next look. Pushes go out right away where a pool is at
//! hand, else on the Circle sweep.

use data_encoding::HEXLOWER;
use reins_proto::{
    circle::{self, GuardianChange, GuardianNotice, GuardianNoticeKind, Seen},
    roster::RosterReason,
};
use rocket::{Data, Route, http::Status, serde::json::Json};

use super::{
    HUB,
    circles::{Wake, wake},
    device_api::{DeviceKey, PhoneResult, bad_request, internal, read_body_limited, require_approval_device, user_key},
    now_unix,
};
use crate::{
    auth::Headers,
    db::{DbConn, DbPool, models::ReinsGuardianNotice},
};

/// The stored type of a notice.
#[must_use]
pub fn kind_name(kind: &GuardianNoticeKind) -> &'static str {
    match kind {
        GuardianNoticeKind::AccountReset => "account_reset",
        GuardianNoticeKind::AccountDeleted => "account_deleted",
        GuardianNoticeKind::RosterChanged {
            ..
        } => "roster_changed",
        GuardianNoticeKind::LinkChanged {
            ..
        } => "link_changed",
    }
}

/// Whether a notice needs the guardian now (an alert), or waits for their next look.
#[must_use]
pub fn alerts(kind: &GuardianNoticeKind) -> bool {
    match kind {
        GuardianNoticeKind::AccountReset | GuardianNoticeKind::AccountDeleted => true,
        GuardianNoticeKind::RosterChanged {
            reason,
            ..
        } => *reason == RosterReason::Recovery,
        GuardianNoticeKind::LinkChanged {
            change,
        } => change.change().is_ok_and(|c| matches!(c.guardian, Some(GuardianChange::EndRequested { .. }))),
    }
}

/// Writes one notice to `guardian` about link `link`.
pub async fn write(
    guardian: &str,
    link: &str,
    kind: &GuardianNoticeKind,
    conn: &DbConn,
) -> Option<ReinsGuardianNotice> {
    let row = ReinsGuardianNotice {
        id: crate::crypto::encode_random_bytes::<16>(&HEXLOWER),
        guardian_uuid: guardian.to_owned(),
        link_id: link.to_owned(),
        kind: kind_name(kind).to_owned(),
        detail: serde_json::to_string(kind).ok()?,
        at: now_unix(),
        pushed: false,
        fetched: false,
    };
    match row.insert(conn).await {
        Ok(()) => {
            HUB.circles.flag(guardian);
            Some(row)
        }
        Err(e) => {
            warn!("Reins: a guardian notice could not be written: {e:?}");
            None
        }
    }
}

/// One notice of `kind` to each guardian of `ward`; returns how many were written. Call it while the ward's links
/// still exist (before a reset ends them, before a deletion erases them). Its pushes go out with [`push_pending`].
pub async fn notify_guardians(ward: &str, kind: &GuardianNoticeKind, conn: &DbConn) -> usize {
    let Ok(detail) = serde_json::to_string(kind) else {
        return 0;
    };
    match ReinsGuardianNotice::notify_guardians(ward, kind_name(kind), &detail, now_unix(), conn).await {
        Ok(rows) => {
            for row in &rows {
                HUB.circles.flag(&row.guardian_uuid);
            }
            rows.len()
        }
        Err(e) => {
            warn!("Reins: the guardians of {ward} could not be told: {e:?}");
            0
        }
    }
}

/// Sends the pushes notices still owe: an alert for one that needs the guardian, news for the rest.
pub async fn push_pending(conn: &DbConn, pool: &DbPool) {
    for notice in ReinsGuardianNotice::unpushed(conn).await {
        if ReinsGuardianNotice::mark_pushed(&notice.id, conn).await.is_err() {
            continue;
        }
        let how = match serde_json::from_str::<GuardianNoticeKind>(&notice.detail) {
            Ok(kind) if alerts(&kind) => Wake::Alert,
            _ => Wake::News,
        };
        wake(&notice.guardian_uuid, &notice.link_id, how, conn, pool).await;
    }
}

/// The calling guardian's notices its phone hasn't acknowledged (its approval phone only): returned until
/// `POST …/notices/seen` names them, so a notice lost in transit comes again.
#[get("/reins/api/guardian/notices")]
async fn get_notices(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Vec<GuardianNotice>>> {
    require_approval_device(&headers, &key, &conn).await?;
    let rows = ReinsGuardianNotice::pending(&user_key(&headers), &conn).await;
    Ok(Json(
        rows.into_iter()
            .filter_map(|r| {
                Some(GuardianNotice {
                    kind: serde_json::from_str(&r.detail).ok()?,
                    id: r.id,
                    link: r.link_id,
                    at: r.at,
                })
            })
            .collect(),
    ))
}

/// The guardian's phone kept these notices.
#[post("/reins/api/guardian/notices/seen", data = "<data>")]
async fn post_seen(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let seen: Seen = serde_json::from_slice(&read_body_limited(data, 64 * 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    if seen.ids.len() > 500 || seen.ids.iter().any(|id| !circle::valid_id(id)) {
        return Err(bad_request("ids: at most 500 notice ids"));
    }
    ReinsGuardianNotice::mark_fetched(&user_key(&headers), seen.ids, &conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

pub fn routes() -> Vec<Route> {
    routes![get_notices, post_seen]
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::circle::{LinkChange, LinkState, Side, SignedChange};

    use super::*;
    use crate::db::{
        DbConnInner,
        models::ReinsCircleLink,
        models::reins_circle::{
            self, ReinsCircleChange, q_changes, q_find_link, q_insert_link, q_mark_notices_fetched, q_notify_guardians,
            q_pending_notices, q_update_link, q_update_with_change,
        },
    };

    fn link(id: &str, owner: &str, helper: &str, kind: &str) -> ReinsCircleLink {
        ReinsCircleLink {
            id: id.to_owned(),
            owner_uuid: owner.to_owned(),
            helper_uuid: helper.to_owned(),
            kind: kind.to_owned(),
            state: "active".to_owned(),
            owner_copy: "o".to_owned(),
            helper_copy: "h".to_owned(),
            last_change: None,
            created_at: 1,
            updated_at: 1,
            effective_at: None,
        }
    }

    fn notify(c: &mut DbConnInner, ward: &str, kind: &GuardianNoticeKind) -> usize {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let detail = serde_json::to_string(kind).unwrap();
        q_notify_guardians(c, ward, kind_name(kind), &detail, 100, &mut || {
            format!("{:032x}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
        })
        .unwrap()
        .len()
    }

    /// What the guardian's phone gets, then acknowledges.
    fn taken(c: &mut DbConnInner, guardian: &str) -> Vec<ReinsGuardianNotice> {
        let rows = q_pending_notices(c, guardian);
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        q_mark_notices_fetched(c, guardian, &ids).unwrap();
        rows
    }

    #[test]
    fn reset_notifies_every_guardian() {
        let c = &mut crate::db::models::test_db();
        q_insert_link(c, &link("l1", "sam", "mom", "guardian")).unwrap();
        q_insert_link(c, &link("l2", "sam", "dad", "guardian")).unwrap();
        // The co-signer is no guardian, and Sam guarding Kim tells Sam nothing about Sam.
        q_insert_link(c, &link("l3", "sam", "alex", "cosign")).unwrap();
        q_insert_link(c, &link("l4", "kim", "sam", "guardian")).unwrap();
        assert_eq!(notify(c, "sam", &GuardianNoticeKind::AccountReset), 2);
        let mom = taken(c, "mom");
        assert_eq!(mom.len(), 1);
        assert_eq!(mom[0].link_id, "l1");
        assert_eq!(mom[0].kind, "account_reset");
        assert!(!mom[0].pushed, "the push is still owed");
        assert_eq!(taken(c, "dad").len(), 1);
        assert!(taken(c, "alex").is_empty(), "notices never reach non-guardians");
        assert!(taken(c, "kim").is_empty());
        assert!(taken(c, "sam").is_empty());
        // Each comes until it is acknowledged, then never again; another account's ids change nothing.
        assert!(taken(c, "mom").is_empty());
        assert_eq!(notify(c, "sam", &GuardianNoticeKind::AccountReset), 2);
        let first = q_pending_notices(c, "dad");
        assert_eq!(first.len(), 1);
        assert_eq!(q_pending_notices(c, "dad").len(), 1, "lost in transit: it comes again");
        q_mark_notices_fetched(c, "mom", &[first[0].id.clone()]).unwrap();
        assert_eq!(q_pending_notices(c, "dad").len(), 1, "only its own phone acknowledges it");
        assert!(alerts(&GuardianNoticeKind::AccountReset));
    }

    #[test]
    fn a_deleted_account_notifies_before_erase() {
        let c = &mut crate::db::models::test_db();
        q_insert_link(c, &link("l1", "sam", "mom", "guardian")).unwrap();
        assert_eq!(notify(c, "sam", &GuardianNoticeKind::AccountDeleted), 1);
        // The erase takes Sam's links (and Sam's own notices as a guardian), never Mom's notice about Sam.
        reins_circle::q_delete_user(c, "sam").unwrap();
        assert!(q_find_link(c, "l1").is_none());
        let mom = taken(c, "mom");
        assert_eq!(mom.len(), 1);
        assert_eq!(mom[0].kind, "account_deleted");
        assert_eq!(mom[0].link_id, "l1", "the link id outlives the link");
        // After the erase there is nobody left to tell: it must come first.
        assert_eq!(notify(c, "sam", &GuardianNoticeKind::AccountDeleted), 0);
    }

    #[test]
    fn roster_recovery_notifies_the_guardian() {
        let c = &mut crate::db::models::test_db();
        q_insert_link(c, &link("l1", "sam", "mom", "guardian")).unwrap();
        let recovery = GuardianNoticeKind::RosterChanged {
            reason: RosterReason::Recovery,
            epoch: 3,
        };
        assert_eq!(notify(c, "sam", &recovery), 1);
        let mom = taken(c, "mom");
        assert_eq!(serde_json::from_str::<GuardianNoticeKind>(&mom[0].detail).unwrap(), recovery);
        assert!(alerts(&recovery), "a recovery suspends the guardianship: the guardian is told at once");
        assert!(!alerts(&GuardianNoticeKind::RosterChanged {
            reason: RosterReason::Handover,
            epoch: 4,
        }));
        // An ended guardianship hears nothing more.
        let mut l = q_find_link(c, "l1").unwrap();
        "ended".clone_into(&mut l.state);
        assert!(q_update_link(c, &l, "active").unwrap());
        assert_eq!(notify(c, "sam", &recovery), 0);
    }

    #[test]
    fn history_keeps_every_change_once() {
        let c = &mut crate::db::models::test_db();
        let row = link("l1", "sam", "mom", "guardian");
        q_insert_link(c, &row).unwrap();
        let change = |seq: i64| ReinsCircleChange {
            link_id: "l1".to_owned(),
            seq,
            change_body: format!("{{\"seq\":{seq}}}"),
            at: 10 + seq,
        };
        assert!(q_update_with_change(c, &row, "active", &change(1)).unwrap());
        assert!(q_update_with_change(c, &row, "active", &change(2)).unwrap());
        // The same number again, or an older one, is refused and logs nothing.
        assert!(!q_update_with_change(c, &row, "active", &change(2)).unwrap());
        assert!(!q_update_with_change(c, &row, "active", &change(1)).unwrap());
        // A link that moved meanwhile logs nothing either.
        assert!(!q_update_with_change(c, &row, "paused", &change(3)).unwrap());
        assert_eq!(q_changes(c, "l1").iter().map(|c| c.seq).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn only_an_end_request_alerts_among_link_changes() {
        let signed = |guardian| {
            let c = LinkChange {
                v: 1,
                link: "0123456789abcdef0123456789abcdef".to_owned(),
                arrangement: "0".repeat(64),
                seq: 1,
                state: LinkState::Active,
                by: Side::Owner,
                at: 1,
                guardian,
            };
            GuardianNoticeKind::LinkChanged {
                change: SignedChange {
                    body: serde_json::to_string(&c).unwrap(),
                    sig: String::new(),
                },
            }
        };
        assert!(alerts(&signed(Some(GuardianChange::EndRequested {
            effective_at: 5
        }))));
        assert!(!alerts(&signed(Some(GuardianChange::Digest {
            on: true
        }))));
    }
}
