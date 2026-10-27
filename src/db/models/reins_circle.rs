//! Circles (Decisions v2, 04): invitations between two accounts, arrangements' routing, and review packets.
//!
//! In clear: who is in an arrangement with whom, its kind and state, deadlines, timestamps. Sealed (and never opened
//! here): each side's copy of the signed arrangement, packets, answers, an invitation's proposed terms. State changes
//! are compare-and-swap on `state`, so they are atomic on SQLite, MySQL and PostgreSQL without row locks.

use diesel::prelude::*;

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{
            reins_circle_changes, reins_circle_invites, reins_circle_links, reins_circle_packets,
            reins_guardian_notices,
        },
    },
    error::MapResult,
};

/// Invitation states an invitation can still move from (`reins_proto::circle::InviteState::is_open`).
pub const OPEN_INVITE_STATES: [&str; 4] = ["open", "challenged", "revealed", "accepted"];

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_circle_invites)]
#[diesel(treat_none_as_null = true)]
pub struct ReinsCircleInvite {
    pub id: String,
    /// SHA-256 (hex) of the link's secret.
    pub token_hash: String,
    pub owner_uuid: String,
    pub kind: String,
    pub state: String,
    pub owner_pub: String,
    pub owner_epoch: i64,
    pub commitment: String,
    /// The account that answered the link (bound by the first challenge).
    pub helper_uuid: Option<String>,
    /// JSON: the challenge, the reveal and the acceptance as they arrived (`InviteSteps` in `api::reins::circles`).
    pub data: String,
    pub link_id: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_circle_links)]
#[diesel(treat_none_as_null = true)]
pub struct ReinsCircleLink {
    pub id: String,
    pub owner_uuid: String,
    pub helper_uuid: String,
    pub kind: String,
    pub state: String,
    pub owner_copy: String,
    pub helper_copy: String,
    /// The last `SignedChange` (JSON).
    pub last_change: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// A guardianship the ward asked to end: when the end takes effect (`i64::MAX`: a minor's, waiting for the
    /// guardian's consent). Until then it stays as it is.
    pub effective_at: Option<i64>,
}

/// One signed change of a link, as it arrived (the history a guardian's phone checks step by step).
#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_circle_changes)]
pub struct ReinsCircleChange {
    pub link_id: String,
    /// The change's own `seq` (unique per link: a second change with the same number is refused).
    pub seq: i64,
    /// The `SignedChange` (JSON).
    pub change_body: String,
    /// When the server received it.
    pub at: i64,
}

/// What the server observed about a ward, for one of its guardians (08 R7).
#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_guardian_notices)]
pub struct ReinsGuardianNotice {
    pub id: String,
    pub guardian_uuid: String,
    pub link_id: String,
    /// The notice's type (`account_reset`, `account_deleted`, `roster_changed`, `link_changed`).
    pub kind: String,
    /// The `GuardianNoticeKind` (JSON).
    pub detail: String,
    pub at: i64,
    /// Its push went out (or isn't owed).
    pub pushed: bool,
    /// The guardian's phone took it.
    pub fetched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable, AsChangeset)]
#[diesel(table_name = reins_circle_packets)]
#[diesel(treat_none_as_null = true)]
pub struct ReinsCirclePacket {
    pub id: String,
    pub link_id: String,
    pub owner_uuid: String,
    pub helper_uuid: String,
    /// `review` or `notice`.
    pub kind: String,
    /// `waiting`, `answered`, `withdrawn` or `expired`.
    pub state: String,
    pub sealed: String,
    pub sealed_answer: Option<String>,
    pub deadline: i64,
    pub created_at: i64,
    pub answered_at: Option<i64>,
    /// The helper's phone fetched it.
    pub delivered: bool,
    /// The owner's phone took the answer (or the final state).
    pub owner_seen: bool,
    pub updated_at: i64,
}

impl ReinsCircleInvite {
    #[must_use]
    pub fn is_open(&self) -> bool {
        OPEN_INVITE_STATES.contains(&self.state.as_str())
    }

    pub async fn insert(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_insert_invite(c, self)).await.map_res("Error saving a Circle invitation")
    }

    pub async fn find(id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_invite(c, id)).await
    }

    pub async fn find_by_token_hash(hash: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_invite_by_token(c, hash)).await
    }

    /// Writes `self` if the stored state is still `from`; `false` when someone moved it first.
    pub async fn update(&self, from: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_update_invite(c, self, from)).await.map_res("Error updating a Circle invitation")
    }

    /// `owner`'s invitations still open at `now`.
    pub async fn open_count(owner: &str, now: i64, conn: &DbConn) -> i64 {
        conn.run(move |c| q_open_invites(c, owner, now)).await
    }

    /// Open invitations whose time ran out.
    pub async fn due(now: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_due_invites(c, now)).await
    }
}

impl ReinsCircleLink {
    pub async fn insert(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_insert_link(c, self)).await.map_res("Error saving a Circle link")
    }

    pub async fn find(id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_link(c, id)).await
    }

    /// Links `user` is on, either side.
    pub async fn for_user(user: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_links_for(c, user)).await
    }

    /// Links `user` is on, or the error: what a phone reads, which must never take a failed load for "no links".
    pub async fn try_for_user(user: &str, conn: &DbConn) -> Result<Vec<Self>, crate::Error> {
        conn.run(move |c| q_try_links_for(c, user)).await.map_res("Error reading Circle links")
    }

    /// Writes `self` if the stored state is still `from`.
    pub async fn update(&self, from: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_update_link(c, self, from)).await.map_res("Error updating a Circle link")
    }

    /// `ward`'s guardianships (as the owner) that aren't ended.
    pub async fn guardians_of(ward: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_guardians_of(c, ward)).await
    }

    /// Guardianships whose end took effect at `now` and aren't ended yet.
    pub async fn ending_due(now: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_ending_due(c, now)).await
    }

    /// Logs `change` and writes `self` if the stored state is still `from`, at once: `false` when someone moved the
    /// link first or a change with the same `seq` is logged already.
    pub async fn update_with_change(
        &self,
        from: &str,
        change: &ReinsCircleChange,
        conn: &DbConn,
    ) -> Result<bool, crate::Error> {
        conn.run(move |c| q_update_with_change(c, self, from, change)).await.map_res("Error updating a Circle link")
    }

    /// Every logged change of `link`, oldest first.
    pub async fn changes(link: &str, conn: &DbConn) -> Vec<ReinsCircleChange> {
        conn.run(move |c| q_changes(c, link)).await
    }

    /// Whether `a` and `b` share a link (either way round) that isn't ended.
    pub async fn linked(a: &str, b: &str, conn: &DbConn) -> bool {
        conn.run(move |c| q_linked(c, a, b)).await
    }
}

impl ReinsCirclePacket {
    pub async fn insert(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_insert_packet(c, self)).await.map_res("Error saving a Circle packet")
    }

    pub async fn find(id: &str, conn: &DbConn) -> Option<Self> {
        conn.run(move |c| q_find_packet(c, id)).await
    }

    /// `helper`'s packets: waiting ones, and those changed at `since` or later.
    pub async fn inbox(helper: &str, since: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_inbox(c, helper, since)).await
    }

    /// `owner`'s packets whose news (an answer, an expiry) its phone hasn't taken.
    pub async fn news_for_owner(owner: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_owner_news(c, owner)).await
    }

    pub async fn update(&self, from: &str, conn: &DbConn) -> Result<bool, crate::Error> {
        conn.run(move |c| q_update_packet(c, self, from)).await.map_res("Error updating a Circle packet")
    }

    /// Waiting packets past their deadline.
    pub async fn due(now: i64, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_due_packets(c, now)).await
    }

    pub async fn count_waiting(link: &str, conn: &DbConn) -> i64 {
        conn.run(move |c| q_count_waiting(c, link)).await
    }

    /// Withdraws every waiting packet of `link` (it ended or paused).
    pub async fn withdraw_for_link(link: &str, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_withdraw_link(c, link, now)).await.map_res("Error withdrawing Circle packets")
    }

    /// The owner's phone took these (only its own packets are touched).
    pub async fn mark_seen(owner: &str, ids: Vec<String>, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_mark_seen(c, owner, &ids)).await.map_res("Error updating Circle packets")
    }

    pub async fn mark_delivered(helper: &str, ids: Vec<String>, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_mark_delivered(c, helper, &ids)).await.map_res("Error updating Circle packets")
    }
}

impl ReinsGuardianNotice {
    /// One notice to each guardian of `ward` (its Guardian links as the owner that haven't ended): `detail` is the
    /// notice's JSON, `kind` its type. Returns what was written.
    pub async fn notify_guardians(
        ward: &str,
        kind: &str,
        detail: &str,
        now: i64,
        conn: &DbConn,
    ) -> Result<Vec<Self>, crate::Error> {
        conn.run(move |c| q_notify_guardians(c, ward, kind, detail, now, &mut new_notice_id))
            .await
            .map_res("Error saving guardian notices")
    }

    pub async fn insert(&self, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_insert_notice(c, self)).await.map_res("Error saving a guardian notice")
    }

    /// `guardian`'s notices its phone hasn't acknowledged, oldest first (they stay until it does).
    pub async fn pending(guardian: &str, conn: &DbConn) -> Vec<Self> {
        conn.run(move |c| q_pending_notices(c, guardian)).await
    }

    /// The guardian's phone kept these (only its own notices are touched).
    pub async fn mark_fetched(guardian: &str, ids: Vec<String>, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_mark_notices_fetched(c, guardian, &ids)).await.map_res("Error updating guardian notices")
    }

    /// Notices whose push hasn't gone out.
    pub async fn unpushed(conn: &DbConn) -> Vec<Self> {
        conn.run(q_unpushed_notices).await
    }

    pub async fn mark_pushed(id: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(reins_guardian_notices::table.find(id))
                .set(reins_guardian_notices::pushed.eq(true))
                .execute(c)
                .map(|_| ())
        })
        .await
        .map_res("Error updating a guardian notice")
    }
}

/// Drops invitations a day past their expiry and packets closed for 7 days.
pub async fn purge(now: i64, conn: &DbConn) -> EmptyResult {
    conn.run(move |c| q_purge(c, now)).await.map_res("Error purging Circle rows")
}

// ---- queries ------------------------------------------------------------------------------------------------------

fn q_insert_invite(c: &mut DbConnInner, row: &ReinsCircleInvite) -> QueryResult<()> {
    diesel::insert_into(reins_circle_invites::table).values(row).execute(c).map(|_| ())
}

fn q_find_invite(c: &mut DbConnInner, id: &str) -> Option<ReinsCircleInvite> {
    reins_circle_invites::table.find(id).first(c).ok()
}

fn q_invite_by_token(c: &mut DbConnInner, hash: &str) -> Option<ReinsCircleInvite> {
    reins_circle_invites::table.filter(reins_circle_invites::token_hash.eq(hash)).first(c).ok()
}

fn q_update_invite(c: &mut DbConnInner, row: &ReinsCircleInvite, from: &str) -> QueryResult<bool> {
    diesel::update(
        reins_circle_invites::table
            .filter(reins_circle_invites::id.eq(&row.id))
            .filter(reins_circle_invites::state.eq(from)),
    )
    .set(row)
    .execute(c)
    .map(|n| n == 1)
}

fn q_open_invites(c: &mut DbConnInner, owner: &str, now: i64) -> i64 {
    reins_circle_invites::table
        .filter(reins_circle_invites::owner_uuid.eq(owner))
        .filter(reins_circle_invites::state.eq_any(OPEN_INVITE_STATES))
        .filter(reins_circle_invites::expires_at.gt(now))
        .count()
        .get_result(c)
        .unwrap_or(i64::MAX)
}

fn q_due_invites(c: &mut DbConnInner, now: i64) -> Vec<ReinsCircleInvite> {
    reins_circle_invites::table
        .filter(reins_circle_invites::state.eq_any(OPEN_INVITE_STATES))
        .filter(reins_circle_invites::expires_at.le(now))
        .load(c)
        .unwrap_or_default()
}

pub(crate) fn q_insert_link(c: &mut DbConnInner, row: &ReinsCircleLink) -> QueryResult<()> {
    diesel::insert_into(reins_circle_links::table).values(row).execute(c).map(|_| ())
}

pub(crate) fn q_find_link(c: &mut DbConnInner, id: &str) -> Option<ReinsCircleLink> {
    reins_circle_links::table.find(id).first(c).ok()
}

fn q_links_for(c: &mut DbConnInner, user: &str) -> Vec<ReinsCircleLink> {
    q_try_links_for(c, user).unwrap_or_default()
}

fn q_try_links_for(c: &mut DbConnInner, user: &str) -> QueryResult<Vec<ReinsCircleLink>> {
    reins_circle_links::table
        .filter(reins_circle_links::owner_uuid.eq(user).or(reins_circle_links::helper_uuid.eq(user)))
        .order(reins_circle_links::created_at.asc())
        .load(c)
}

pub(crate) fn q_update_link(c: &mut DbConnInner, row: &ReinsCircleLink, from: &str) -> QueryResult<bool> {
    diesel::update(
        reins_circle_links::table.filter(reins_circle_links::id.eq(&row.id)).filter(reins_circle_links::state.eq(from)),
    )
    .set(row)
    .execute(c)
    .map(|n| n == 1)
}

pub(crate) fn q_guardians_of(c: &mut DbConnInner, ward: &str) -> Vec<ReinsCircleLink> {
    reins_circle_links::table
        .filter(reins_circle_links::owner_uuid.eq(ward))
        .filter(reins_circle_links::kind.eq("guardian"))
        .filter(reins_circle_links::state.ne("ended"))
        .order(reins_circle_links::created_at.asc())
        .load(c)
        .unwrap_or_default()
}

fn q_ending_due(c: &mut DbConnInner, now: i64) -> Vec<ReinsCircleLink> {
    reins_circle_links::table
        .filter(reins_circle_links::kind.eq("guardian"))
        .filter(reins_circle_links::state.ne("ended"))
        .filter(reins_circle_links::effective_at.le(now))
        .load(c)
        .unwrap_or_default()
}

pub(crate) fn q_update_with_change(
    c: &mut DbConnInner,
    row: &ReinsCircleLink,
    from: &str,
    change: &ReinsCircleChange,
) -> QueryResult<bool> {
    c.transaction(|c| {
        let logged = reins_circle_changes::table
            .filter(reins_circle_changes::link_id.eq(&change.link_id))
            .filter(reins_circle_changes::seq.ge(change.seq))
            .count()
            .get_result::<i64>(c)?;
        if logged > 0 {
            return Ok(false);
        }
        diesel::insert_into(reins_circle_changes::table).values(change).execute(c)?;
        if q_update_link(c, row, from)? {
            Ok(true)
        } else {
            // Moved meanwhile: the change isn't logged either.
            Err(diesel::result::Error::RollbackTransaction)
        }
    })
    .or_else(|e| match e {
        // Moved meanwhile, or two changes with one `seq` at once (the second finds the first's row).
        diesel::result::Error::RollbackTransaction
        | diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => Ok(false),
        e => Err(e),
    })
}

pub(crate) fn q_changes(c: &mut DbConnInner, link: &str) -> Vec<ReinsCircleChange> {
    reins_circle_changes::table
        .filter(reins_circle_changes::link_id.eq(link))
        .order(reins_circle_changes::seq.asc())
        .limit(1_000)
        .load(c)
        .unwrap_or_default()
}

fn new_notice_id() -> String {
    crate::crypto::encode_random_bytes::<16>(&data_encoding::HEXLOWER)
}

pub(crate) fn q_insert_notice(c: &mut DbConnInner, row: &ReinsGuardianNotice) -> QueryResult<()> {
    diesel::insert_into(reins_guardian_notices::table).values(row).execute(c).map(|_| ())
}

pub(crate) fn q_notify_guardians(
    c: &mut DbConnInner,
    ward: &str,
    kind: &str,
    detail: &str,
    now: i64,
    new_id: &mut dyn FnMut() -> String,
) -> QueryResult<Vec<ReinsGuardianNotice>> {
    c.transaction(|c| {
        let mut out = Vec::new();
        for link in q_guardians_of(c, ward) {
            let row = ReinsGuardianNotice {
                id: new_id(),
                guardian_uuid: link.helper_uuid,
                link_id: link.id,
                kind: kind.to_owned(),
                detail: detail.to_owned(),
                at: now,
                pushed: false,
                fetched: false,
            };
            q_insert_notice(c, &row)?;
            out.push(row);
        }
        Ok(out)
    })
}

pub(crate) fn q_pending_notices(c: &mut DbConnInner, guardian: &str) -> Vec<ReinsGuardianNotice> {
    reins_guardian_notices::table
        .filter(reins_guardian_notices::guardian_uuid.eq(guardian))
        .filter(reins_guardian_notices::fetched.eq(false))
        .order(reins_guardian_notices::at.asc())
        .limit(200)
        .load(c)
        .unwrap_or_default()
}

pub(crate) fn q_mark_notices_fetched(c: &mut DbConnInner, guardian: &str, ids: &[String]) -> QueryResult<()> {
    diesel::update(
        reins_guardian_notices::table
            .filter(reins_guardian_notices::guardian_uuid.eq(guardian))
            .filter(reins_guardian_notices::id.eq_any(ids)),
    )
    .set(reins_guardian_notices::fetched.eq(true))
    .execute(c)
    .map(|_| ())
}

fn q_unpushed_notices(c: &mut DbConnInner) -> Vec<ReinsGuardianNotice> {
    reins_guardian_notices::table
        .filter(reins_guardian_notices::pushed.eq(false))
        .order(reins_guardian_notices::at.asc())
        .limit(200)
        .load(c)
        .unwrap_or_default()
}

fn q_linked(c: &mut DbConnInner, a: &str, b: &str) -> bool {
    reins_circle_links::table
        .filter(
            (reins_circle_links::owner_uuid.eq(a).and(reins_circle_links::helper_uuid.eq(b)))
                .or(reins_circle_links::owner_uuid.eq(b).and(reins_circle_links::helper_uuid.eq(a))),
        )
        .filter(reins_circle_links::state.ne("ended"))
        .count()
        .get_result::<i64>(c)
        .is_ok_and(|n| n > 0)
}

fn q_insert_packet(c: &mut DbConnInner, row: &ReinsCirclePacket) -> QueryResult<()> {
    diesel::insert_into(reins_circle_packets::table).values(row).execute(c).map(|_| ())
}

fn q_find_packet(c: &mut DbConnInner, id: &str) -> Option<ReinsCirclePacket> {
    reins_circle_packets::table.find(id).first(c).ok()
}

fn q_inbox(c: &mut DbConnInner, helper: &str, since: i64) -> Vec<ReinsCirclePacket> {
    reins_circle_packets::table
        .filter(reins_circle_packets::helper_uuid.eq(helper))
        .filter(reins_circle_packets::state.eq("waiting").or(reins_circle_packets::updated_at.ge(since)))
        .order(reins_circle_packets::created_at.asc())
        .limit(500)
        .load(c)
        .unwrap_or_default()
}

fn q_owner_news(c: &mut DbConnInner, owner: &str) -> Vec<ReinsCirclePacket> {
    reins_circle_packets::table
        .filter(reins_circle_packets::owner_uuid.eq(owner))
        .filter(reins_circle_packets::owner_seen.eq(false))
        .filter(reins_circle_packets::state.ne("waiting"))
        .order(reins_circle_packets::updated_at.asc())
        .limit(500)
        .load(c)
        .unwrap_or_default()
}

fn q_update_packet(c: &mut DbConnInner, row: &ReinsCirclePacket, from: &str) -> QueryResult<bool> {
    diesel::update(
        reins_circle_packets::table
            .filter(reins_circle_packets::id.eq(&row.id))
            .filter(reins_circle_packets::state.eq(from)),
    )
    .set(row)
    .execute(c)
    .map(|n| n == 1)
}

fn q_due_packets(c: &mut DbConnInner, now: i64) -> Vec<ReinsCirclePacket> {
    reins_circle_packets::table
        .filter(reins_circle_packets::state.eq("waiting"))
        .filter(reins_circle_packets::deadline.le(now))
        .load(c)
        .unwrap_or_default()
}

fn q_count_waiting(c: &mut DbConnInner, link: &str) -> i64 {
    reins_circle_packets::table
        .filter(reins_circle_packets::link_id.eq(link))
        .filter(reins_circle_packets::state.eq("waiting"))
        .count()
        .get_result(c)
        .unwrap_or(i64::MAX)
}

fn q_withdraw_link(c: &mut DbConnInner, link: &str, now: i64) -> QueryResult<()> {
    diesel::update(
        reins_circle_packets::table
            .filter(reins_circle_packets::link_id.eq(link))
            .filter(reins_circle_packets::state.eq("waiting")),
    )
    .set((
        reins_circle_packets::state.eq("withdrawn"),
        reins_circle_packets::owner_seen.eq(true),
        reins_circle_packets::updated_at.eq(now),
    ))
    .execute(c)
    .map(|_| ())
}

fn q_mark_seen(c: &mut DbConnInner, owner: &str, ids: &[String]) -> QueryResult<()> {
    diesel::update(
        reins_circle_packets::table
            .filter(reins_circle_packets::owner_uuid.eq(owner))
            .filter(reins_circle_packets::id.eq_any(ids)),
    )
    .set(reins_circle_packets::owner_seen.eq(true))
    .execute(c)
    .map(|_| ())
}

fn q_mark_delivered(c: &mut DbConnInner, helper: &str, ids: &[String]) -> QueryResult<()> {
    diesel::update(
        reins_circle_packets::table
            .filter(reins_circle_packets::helper_uuid.eq(helper))
            .filter(reins_circle_packets::id.eq_any(ids)),
    )
    .set(reins_circle_packets::delivered.eq(true))
    .execute(c)
    .map(|_| ())
}

fn q_purge(c: &mut DbConnInner, now: i64) -> QueryResult<()> {
    diesel::delete(reins_circle_invites::table.filter(reins_circle_invites::expires_at.lt(now - 86_400))).execute(c)?;
    diesel::delete(
        reins_circle_packets::table
            .filter(reins_circle_packets::state.ne("waiting"))
            .filter(reins_circle_packets::owner_seen.eq(true))
            .filter(reins_circle_packets::updated_at.lt(now - 7 * 86_400)),
    )
    .execute(c)?;
    diesel::delete(
        reins_guardian_notices::table
            .filter(reins_guardian_notices::fetched.eq(true))
            .filter(reins_guardian_notices::at.lt(now - 30 * 86_400)),
    )
    .execute(c)
    .map(|_| ())
}

/// Every Circle row of `user`, either side (account deletion or reset), and the notices it holds as a guardian. Notices
/// about it (as a ward) stay with its guardians.
pub(crate) fn q_delete_user(c: &mut DbConnInner, user: &str) -> QueryResult<()> {
    diesel::delete(reins_guardian_notices::table.filter(reins_guardian_notices::guardian_uuid.eq(user))).execute(c)?;
    diesel::delete(
        reins_circle_packets::table
            .filter(reins_circle_packets::owner_uuid.eq(user).or(reins_circle_packets::helper_uuid.eq(user))),
    )
    .execute(c)?;
    diesel::delete(
        reins_circle_links::table
            .filter(reins_circle_links::owner_uuid.eq(user).or(reins_circle_links::helper_uuid.eq(user))),
    )
    .execute(c)?;
    diesel::delete(
        reins_circle_invites::table
            .filter(reins_circle_invites::owner_uuid.eq(user).or(reins_circle_invites::helper_uuid.eq(user))),
    )
    .execute(c)
    .map(|_| ())
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;
    use crate::db::models::reins_device::test_db;

    pub fn invite(id: &str, owner: &str, state: &str) -> ReinsCircleInvite {
        ReinsCircleInvite {
            id: id.to_owned(),
            token_hash: format!("{id:0>64}"),
            owner_uuid: owner.to_owned(),
            kind: "cosign".to_owned(),
            state: state.to_owned(),
            owner_pub: "k".to_owned(),
            owner_epoch: 1,
            commitment: "c".repeat(64),
            helper_uuid: None,
            data: "{}".to_owned(),
            link_id: None,
            created_at: 1,
            expires_at: 100,
            updated_at: 1,
        }
    }

    pub fn link(id: &str, owner: &str, helper: &str) -> ReinsCircleLink {
        ReinsCircleLink {
            id: id.to_owned(),
            owner_uuid: owner.to_owned(),
            helper_uuid: helper.to_owned(),
            kind: "cosign".to_owned(),
            state: "active".to_owned(),
            owner_copy: "o".to_owned(),
            helper_copy: "h".to_owned(),
            last_change: None,
            created_at: 1,
            updated_at: 1,
            effective_at: None,
        }
    }

    pub fn packet(id: &str, link: &str, owner: &str, helper: &str, deadline: i64) -> ReinsCirclePacket {
        ReinsCirclePacket {
            id: id.to_owned(),
            link_id: link.to_owned(),
            owner_uuid: owner.to_owned(),
            helper_uuid: helper.to_owned(),
            kind: "review".to_owned(),
            state: "waiting".to_owned(),
            sealed: "s".to_owned(),
            sealed_answer: None,
            deadline,
            created_at: 1,
            answered_at: None,
            delivered: false,
            owner_seen: false,
            updated_at: 1,
        }
    }

    #[test]
    fn invites_move_forward_only() {
        let mut c = test_db();
        let mut i = invite("i1", "dana", "open");
        q_insert_invite(&mut c, &i).unwrap();
        assert_eq!(q_invite_by_token(&mut c, &i.token_hash).unwrap().id, "i1");
        i.state = "challenged".to_owned();
        i.helper_uuid = Some("alex".to_owned());
        assert!(q_update_invite(&mut c, &i, "open").unwrap());
        // A second challenge, made from the state the first one left, finds nothing to move.
        let mut again = i.clone();
        again.helper_uuid = Some("eve".to_owned());
        assert!(!q_update_invite(&mut c, &again, "open").unwrap());
        assert_eq!(q_find_invite(&mut c, "i1").unwrap().helper_uuid.as_deref(), Some("alex"));
        assert_eq!(q_open_invites(&mut c, "dana", 50), 1);
        assert_eq!(q_open_invites(&mut c, "dana", 100), 0, "expired ones don't count");
        assert_eq!(q_due_invites(&mut c, 100).len(), 1);
    }

    #[test]
    fn links_list_for_both_parties() {
        let mut c = test_db();
        q_insert_link(&mut c, &link("l1", "dana", "alex")).unwrap();
        q_insert_link(&mut c, &link("l2", "sam", "dana")).unwrap();
        q_insert_link(&mut c, &link("l3", "sam", "pat")).unwrap();
        let ids = |v: Vec<ReinsCircleLink>| v.into_iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(ids(q_links_for(&mut c, "dana")), ["l1", "l2"]);
        assert_eq!(ids(q_links_for(&mut c, "pat")), ["l3"]);
        assert!(q_linked(&mut c, "alex", "dana"));
        assert!(!q_linked(&mut c, "alex", "sam"));
        let mut ended = link("l1", "dana", "alex");
        ended.state = "ended".to_owned();
        assert!(q_update_link(&mut c, &ended, "active").unwrap());
        assert!(!q_linked(&mut c, "alex", "dana"));
        assert!(!q_update_link(&mut c, &ended, "active").unwrap(), "already ended");
    }

    #[test]
    fn packets_are_listed_for_their_helper_and_answers_for_their_owner() {
        let mut c = test_db();
        q_insert_packet(&mut c, &packet("p1", "l1", "dana", "alex", 100)).unwrap();
        q_insert_packet(&mut c, &packet("p2", "l1", "dana", "alex", 50)).unwrap();
        q_insert_packet(&mut c, &packet("p3", "l2", "sam", "pat", 100)).unwrap();
        assert_eq!(q_inbox(&mut c, "alex", 1_000).len(), 2);
        assert!(q_inbox(&mut c, "dana", 0).is_empty(), "the owner has no inbox of its own packets");
        assert!(q_owner_news(&mut c, "dana").is_empty(), "nothing answered yet");
        let mut p = q_find_packet(&mut c, "p1").unwrap();
        p.state = "answered".to_owned();
        p.sealed_answer = Some("a".to_owned());
        assert!(q_update_packet(&mut c, &p, "waiting").unwrap());
        assert!(!q_update_packet(&mut c, &p, "waiting").unwrap(), "answered once");
        assert_eq!(q_owner_news(&mut c, "dana").len(), 1);
        assert!(q_owner_news(&mut c, "sam").is_empty());
        // Marking seen touches only the owner's own packets.
        q_mark_seen(&mut c, "sam", &["p1".to_owned()]).unwrap();
        assert_eq!(q_owner_news(&mut c, "dana").len(), 1);
        q_mark_seen(&mut c, "dana", &["p1".to_owned()]).unwrap();
        assert!(q_owner_news(&mut c, "dana").is_empty());
        assert_eq!(q_due_packets(&mut c, 60).len(), 1);
        assert_eq!(q_count_waiting(&mut c, "l1"), 1);
        q_withdraw_link(&mut c, "l1", 70).unwrap();
        assert_eq!(q_count_waiting(&mut c, "l1"), 0);
    }

    #[test]
    fn account_deletion_removes_circle_rows() {
        let mut c = test_db();
        let mut i = invite("i1", "sam", "challenged");
        i.helper_uuid = Some("dana".to_owned());
        q_insert_invite(&mut c, &i).unwrap();
        q_insert_link(&mut c, &link("l1", "dana", "alex")).unwrap();
        q_insert_link(&mut c, &link("l2", "sam", "pat")).unwrap();
        q_insert_packet(&mut c, &packet("p1", "l1", "dana", "alex", 100)).unwrap();
        q_insert_packet(&mut c, &packet("p2", "l2", "sam", "pat", 100)).unwrap();
        q_delete_user(&mut c, "dana").unwrap();
        assert!(q_find_invite(&mut c, "i1").is_none());
        assert!(q_find_link(&mut c, "l1").is_none());
        assert!(q_find_packet(&mut c, "p1").is_none());
        assert!(q_find_link(&mut c, "l2").is_some());
        assert!(q_find_packet(&mut c, "p2").is_some());
    }
}
