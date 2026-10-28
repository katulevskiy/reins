//! Held email sends (Decisions v2, 01 §5, 08 R10): an approved email waits out its undo window here, sealed to the
//! approval device's own key. The server stores the ciphertext as it got it and never opens it (it has no key).
//!
//! - **Undo** deletes the row ([`ReinsSendHold::cancel_for_user`], [`ReinsSendHold::cancel_for_connection`]).
//! - **Release** is one claim: the row is deleted by a compare-and-swap on its id, and only the claim whose delete took
//!   gets the ciphertext ([`ReinsSendHold::claim`]). A second claim, a replayed one, or one after an undo finds nothing.
//! - Bounds per account: [`reins_proto::hold::MAX_PER_ACCOUNT`] holds and
//!   [`reins_proto::hold::MAX_BYTES_PER_ACCOUNT`] of ciphertext.

use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{DbConn, DbConnInner, schema::reins_send_holds},
    error::MapResult,
};

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_send_holds)]
pub struct ReinsSendHold {
    pub id: String,
    pub user_uuid: String,
    /// The device that sealed it: the only one that may claim it.
    pub device_uuid: String,
    /// The connection whose request it answers: it may cancel it.
    pub connection_uuid: String,
    pub request_id: String,
    /// Base64url ciphertext, opaque here.
    pub sealed: String,
    pub release_at: i64,
    pub created_at: i64,
    /// The release push went out.
    pub woken: bool,
}

/// What [`ReinsSendHold::insert`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inserted {
    Created,
    /// The account has [`reins_proto::hold::MAX_PER_ACCOUNT`] holds waiting already.
    TooMany,
    /// It would take the account past [`reins_proto::hold::MAX_BYTES_PER_ACCOUNT`].
    TooLarge,
    /// This request is held already.
    Exists,
}

/// What [`ReinsSendHold::claim`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Deleted by this claim: the only one that gets the ciphertext.
    Claimed(ReinsSendHold),
    /// Not released yet (its release time).
    NotYet(i64),
    /// No such hold for this device: undone, claimed already, another account's or another device's.
    NotFound,
}

impl ReinsSendHold {
    pub async fn insert(&self, conn: &DbConn) -> Result<Inserted, crate::Error> {
        conn.run(move |c| q_insert(c, self)).await.map_res("Error saving a Reins send hold")
    }

    pub async fn find(id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| reins_send_holds::table.find(id).first::<Self>(c).ok()).await
    }

    /// `user`'s waiting holds, soonest first (never another account's).
    pub async fn list_for_user(user: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_list_for_user(c, user)).await
    }

    /// Undo from the approval device: `true` when it was waiting and is gone now.
    pub async fn cancel_for_user(id: &str, user: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_cancel_for_user(c, id, user)).await.map_res("Error deleting a Reins send hold")
    }

    /// Undo from the connection that asked (the desktop app, or the agent's `reins_cancel`), by hold id or by request
    /// id: the user and the id of the hold, if it was waiting and is gone now.
    pub async fn cancel_for_connection(
        id: Option<&str>,
        request_id: Option<&str>,
        connection: &str,
        conn: &DbConn,
    ) -> Result<Option<(String, String)>, crate::Error> {
        conn.run(move |c| q_cancel_for_connection(c, id, request_id, connection))
            .await
            .map_res("Error deleting a Reins send hold")
    }

    /// The release: `device` of `user` claims hold `id` at `now`.
    pub async fn claim(id: &str, user: &str, device: &str, now: i64, conn: &DbConn) -> Result<Claim, crate::Error> {
        conn.run(move |c| q_claim(c, id, user, device, now)).await.map_res("Error claiming a Reins send hold")
    }

    /// Holds due at `now` whose release push has not gone out.
    pub async fn due(now: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_due(c, now)).await
    }

    /// Notes the release push of `id`: `true` for the one caller that set it.
    pub async fn mark_woken(id: &str, conn: &DbConn) -> bool {
        conn.run(move |c| {
            diesel::update(
                reins_send_holds::table.filter(reins_send_holds::id.eq(id)).filter(reins_send_holds::woken.eq(false)),
            )
            .set(reins_send_holds::woken.eq(true))
            .execute(c)
            .is_ok_and(|n| n == 1)
        })
        .await
    }

    /// Drops holds created before `before` that nobody claimed (the phone never woke): they are never sent.
    pub async fn purge(before: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::delete(reins_send_holds::table.filter(reins_send_holds::created_at.lt(before)))
                .execute(c)
                .map(|_| ())
        })
        .await
        .map_res("Error purging Reins send holds")
    }

    /// Every hold of `user` (an account reset: they were sealed to a key that opens nothing now).
    pub async fn delete_all(user: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::delete(reins_send_holds::table.filter(reins_send_holds::user_uuid.eq(user))).execute(c).map(|_| ())
        })
        .await
        .map_res("Error deleting Reins send holds")
    }
}

fn q_insert(c: &mut DbConnInner, row: &ReinsSendHold) -> QueryResult<Inserted> {
    c.transaction(|c| {
        let exists: i64 = reins_send_holds::table
            .filter(reins_send_holds::user_uuid.eq(&row.user_uuid))
            .filter(reins_send_holds::request_id.eq(&row.request_id))
            .count()
            .get_result(c)?;
        if exists > 0 {
            return Ok(Inserted::Exists);
        }
        let sizes: Vec<String> = reins_send_holds::table
            .filter(reins_send_holds::user_uuid.eq(&row.user_uuid))
            .select(reins_send_holds::sealed)
            .limit(reins_proto::hold::MAX_PER_ACCOUNT)
            .load(c)?;
        if i64::try_from(sizes.len()).unwrap_or(i64::MAX) >= reins_proto::hold::MAX_PER_ACCOUNT {
            return Ok(Inserted::TooMany);
        }
        let used: usize = sizes.iter().map(String::len).sum();
        let total = i64::try_from(used.saturating_add(row.sealed.len())).unwrap_or(i64::MAX);
        if total > reins_proto::hold::MAX_BYTES_PER_ACCOUNT {
            return Ok(Inserted::TooLarge);
        }
        match diesel::insert_into(reins_send_holds::table).values(row).execute(c) {
            Ok(_) => Ok(Inserted::Created),
            // Two holds of one request racing: the unique (user, request) keeps one.
            Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)) => {
                Ok(Inserted::Exists)
            }
            Err(e) => Err(e),
        }
    })
}

fn q_cancel_for_user(c: &mut DbConnInner, id: &str, user: &str) -> QueryResult<bool> {
    diesel::delete(
        reins_send_holds::table.filter(reins_send_holds::id.eq(id)).filter(reins_send_holds::user_uuid.eq(user)),
    )
    .execute(c)
    .map(|n| n == 1)
}

fn q_list_for_user(c: &mut DbConnInner, user: &str) -> Vec<ReinsSendHold> {
    reins_send_holds::table
        .filter(reins_send_holds::user_uuid.eq(user))
        .order(reins_send_holds::release_at.asc())
        .limit(reins_proto::hold::MAX_PER_ACCOUNT)
        .load::<ReinsSendHold>(c)
        .unwrap_or_default()
}

fn q_cancel_for_connection(
    c: &mut DbConnInner,
    id: Option<&str>,
    request_id: Option<&str>,
    connection: &str,
) -> QueryResult<Option<(String, String)>> {
    c.transaction(|c| {
        let mut query = reins_send_holds::table.filter(reins_send_holds::connection_uuid.eq(connection)).into_boxed();
        match (id, request_id) {
            (Some(id), _) => query = query.filter(reins_send_holds::id.eq(id.to_owned())),
            (None, Some(request)) => query = query.filter(reins_send_holds::request_id.eq(request.to_owned())),
            (None, None) => return Ok(None),
        }
        let Some(row) = query.first::<ReinsSendHold>(c).optional()? else {
            return Ok(None);
        };
        let n = diesel::delete(reins_send_holds::table.filter(reins_send_holds::id.eq(&row.id))).execute(c)?;
        Ok((n == 1).then_some((row.user_uuid, row.id)))
    })
}

fn q_claim(c: &mut DbConnInner, id: &str, user: &str, device: &str, now: i64) -> QueryResult<Claim> {
    let Some(row) = reins_send_holds::table
        .filter(reins_send_holds::id.eq(id))
        .filter(reins_send_holds::user_uuid.eq(user))
        .filter(reins_send_holds::device_uuid.eq(device))
        .first::<ReinsSendHold>(c)
        .optional()?
    else {
        return Ok(Claim::NotFound);
    };
    if row.release_at > now {
        return Ok(Claim::NotYet(row.release_at));
    }
    // The compare-and-swap: of claims racing for one hold, only the one whose delete took gets it.
    let n = diesel::delete(
        reins_send_holds::table
            .filter(reins_send_holds::id.eq(id))
            .filter(reins_send_holds::user_uuid.eq(user))
            .filter(reins_send_holds::device_uuid.eq(device)),
    )
    .execute(c)?;
    Ok(if n == 1 {
        Claim::Claimed(row)
    } else {
        Claim::NotFound
    })
}

fn q_due(c: &mut DbConnInner, now: i64) -> Vec<ReinsSendHold> {
    reins_send_holds::table
        .filter(reins_send_holds::woken.eq(false))
        .filter(reins_send_holds::release_at.le(now))
        .order(reins_send_holds::release_at.asc())
        .limit(500)
        .load::<ReinsSendHold>(c)
        .unwrap_or_default()
}

#[cfg(all(test, sqlite))]
pub(crate) mod tests {
    use super::*;
    use crate::db::models::reins_device::test_db;

    pub fn hold(id: &str, user: &str, request: &str, release_at: i64) -> ReinsSendHold {
        ReinsSendHold {
            id: id.to_owned(),
            user_uuid: user.to_owned(),
            device_uuid: "d1".to_owned(),
            connection_uuid: "c1".to_owned(),
            request_id: request.to_owned(),
            sealed: format!("sealed-{id}"),
            release_at,
            created_at: release_at - 30,
            woken: false,
        }
    }

    fn rows(c: &mut DbConnInner) -> Vec<ReinsSendHold> {
        reins_send_holds::table.load(c).unwrap()
    }

    #[test]
    fn the_ciphertext_is_stored_as_given_and_deleted_by_the_claim() {
        let mut c = test_db();
        // Whatever the phone sealed is kept byte for byte: nothing here parses or opens it.
        let mut h = hold("h1", "u1", "r1", 100);
        h.sealed = "AAEC_-opaque~not-json".to_owned();
        assert_eq!(q_insert(&mut c, &h).unwrap(), Inserted::Created);
        assert_eq!(rows(&mut c)[0].sealed, "AAEC_-opaque~not-json");
        assert_eq!(q_claim(&mut c, "h1", "u1", "d1", 99).unwrap(), Claim::NotYet(100), "never before its time");
        assert_eq!(q_claim(&mut c, "h1", "u1", "d1", 100).unwrap(), Claim::Claimed(h));
        assert!(rows(&mut c).is_empty(), "the claim deleted the ciphertext");
        assert_eq!(q_claim(&mut c, "h1", "u1", "d1", 100).unwrap(), Claim::NotFound, "a replayed claim finds nothing");
    }

    #[test]
    fn undo_deletes_and_a_later_claim_finds_nothing() {
        let mut c = test_db();
        q_insert(&mut c, &hold("h1", "u1", "r1", 100)).unwrap();
        q_insert(&mut c, &hold("h2", "u1", "r2", 100)).unwrap();
        assert!(!q_cancel_for_user(&mut c, "h1", "u2").unwrap(), "another account's undo deletes nothing");
        assert!(q_cancel_for_user(&mut c, "h1", "u1").unwrap());
        assert_eq!(q_claim(&mut c, "h1", "u1", "d1", 200).unwrap(), Claim::NotFound);
        // The connection that asked may undo its own, by request id; another connection may not.
        assert_eq!(q_cancel_for_connection(&mut c, None, Some("r2"), "c2").unwrap(), None);
        assert_eq!(
            q_cancel_for_connection(&mut c, None, Some("r2"), "c1").unwrap(),
            Some(("u1".to_owned(), "h2".to_owned()))
        );
        assert_eq!(q_cancel_for_connection(&mut c, None, Some("r2"), "c1").unwrap(), None, "gone already");
        assert!(rows(&mut c).is_empty());
    }

    #[test]
    fn another_account_or_device_never_claims() {
        let mut c = test_db();
        q_insert(&mut c, &hold("h1", "u1", "r1", 100)).unwrap();
        assert_eq!(q_claim(&mut c, "h1", "u2", "d1", 200).unwrap(), Claim::NotFound, "another account");
        assert_eq!(q_claim(&mut c, "h1", "u1", "d2", 200).unwrap(), Claim::NotFound, "another device");
        assert!(q_list_for_user(&mut c, "u2").is_empty());
        assert_eq!(rows(&mut c).len(), 1, "still waiting for its own device");
    }

    #[test]
    fn bounds_per_account() {
        let mut c = test_db();
        for i in 0..reins_proto::hold::MAX_PER_ACCOUNT {
            assert_eq!(
                q_insert(&mut c, &hold(&format!("h{i}"), "u1", &format!("r{i}"), 100)).unwrap(),
                Inserted::Created
            );
        }
        assert_eq!(q_insert(&mut c, &hold("hx", "u1", "rx", 100)).unwrap(), Inserted::TooMany);
        assert_eq!(q_insert(&mut c, &hold("hy", "u2", "rx", 100)).unwrap(), Inserted::Created, "per account");
        assert_eq!(q_insert(&mut c, &hold("hz", "u2", "rx", 100)).unwrap(), Inserted::Exists, "one per request");

        let mut c = test_db();
        let big = |id: &str| {
            let mut h = hold(id, "u1", id, 100);
            h.sealed = "a".repeat(reins_proto::hold::MAX_SEALED_CHARS);
            h
        };
        for id in ["b1", "b2", "b3", "b4"] {
            assert_eq!(q_insert(&mut c, &big(id)).unwrap(), Inserted::Created);
        }
        assert_eq!(q_insert(&mut c, &big("b5")).unwrap(), Inserted::TooLarge);
    }

    #[test]
    fn due_holds_wake_once() {
        let mut c = test_db();
        q_insert(&mut c, &hold("h1", "u1", "r1", 100)).unwrap();
        q_insert(&mut c, &hold("h2", "u1", "r2", 300)).unwrap();
        let due: Vec<String> = q_due(&mut c, 200).into_iter().map(|h| h.id).collect();
        assert_eq!(due, ["h1"]);
        let n = diesel::update(reins_send_holds::table.filter(reins_send_holds::id.eq("h1")))
            .set(reins_send_holds::woken.eq(true))
            .execute(&mut c)
            .unwrap();
        assert_eq!(n, 1);
        assert!(q_due(&mut c, 200).is_empty(), "woken already");
    }
}
