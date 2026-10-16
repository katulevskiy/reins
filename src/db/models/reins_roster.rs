//! Phone roles and live sync (`reins_proto::roster`, `reins_proto::oplog`): the roster chain, its members, the op log
//! and the checkpoints. The server stores what the phones signed and sealed; nothing here opens or changes it.
//!
//! Atomicity without row locks (the same code runs on SQLite, MySQL and PostgreSQL): the roster's primary key
//! (account, epoch) is its compare-and-swap, an author's counter moves with a conditional update, and an op's primary
//! key (account, seq) makes two concurrent appends collide instead of sharing a number.

use diesel::{
    prelude::*,
    result::{DatabaseErrorKind, Error as DieselError},
};

use crate::{
    api::EmptyResult,
    db::{
        DbConn, DbConnInner,
        schema::{reins_checkpoints, reins_devices, reins_members, reins_ops, reins_rosters},
    },
    error::MapResult,
};

use super::{DeviceId, ReinsDevice, UserId};

pub const ROLE_PRIMARY: &str = "primary";
pub const ROLE_VIEWER: &str = "viewer";
/// Listed before, not now: kept so that its counter cannot start over.
pub const ROLE_REMOVED: &str = "removed";

/// A device a roster lists or listed.
#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_members)]
pub struct ReinsMember {
    pub user_uuid: String,
    pub device_uuid: String,
    pub role: String,
    pub sign_pub: String,
    /// SHA-256 (hex) of the device key it enrolled with.
    pub key_hash: Option<String>,
    /// Viewers' push token (`sync` pushes); the primary's stays in `reins_devices`.
    pub fcm_token: Option<String>,
    /// The highest op counter stored for it.
    pub last_counter: i64,
    /// The highest `seq` it acknowledged.
    pub acked_seq: i64,
    /// The primary's latest signed heartbeat (JSON).
    pub heartbeat: Option<String>,
    /// When the server took that heartbeat (Unix seconds).
    pub heartbeat_at: Option<i64>,
    pub updated_at: i64,
}

impl ReinsMember {
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.role == ROLE_PRIMARY || self.role == ROLE_VIEWER
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_rosters)]
pub struct ReinsRosterRow {
    pub user_uuid: String,
    pub epoch: i64,
    /// The signed `RosterEntry`, JSON.
    pub entry: String,
    pub entry_hash: String,
    pub recovery_wrap: Option<String>,
    pub bundle: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_ops)]
pub struct ReinsOpRow {
    pub user_uuid: String,
    pub seq: i64,
    pub author: String,
    /// The author's signing key when it wrote the op (a counter is unique per key).
    pub sign_pub: String,
    pub counter: i64,
    /// The signed `Op`, JSON.
    pub op: String,
    /// Bytes of `op`.
    pub size: i32,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Queryable, Insertable)]
#[diesel(table_name = reins_checkpoints)]
pub struct ReinsCheckpointRow {
    pub user_uuid: String,
    pub seq: i64,
    /// The signed `Checkpoint`, JSON.
    pub checkpoint: String,
    pub created_at: i64,
}

/// Why a write did not happen.
#[derive(Debug)]
pub enum WriteError {
    /// Someone else wrote first (an epoch, a counter or a checkpoint `seq` that is no longer the next): reload.
    Conflict,
    /// The account's log is as large as it may be.
    Full,
    /// Two appends took the same `seq` at once: try again.
    Retry,
    Db(Box<crate::Error>),
}

impl From<DieselError> for WriteError {
    fn from(e: DieselError) -> Self {
        match e {
            DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => Self::Retry,
            other => Self::Db(Box::new(crate::Error::from(other))),
        }
    }
}

/// One member as a roster entry lists it.
#[derive(Clone, Debug)]
pub struct Listed {
    pub device_uuid: String,
    pub role: &'static str,
    pub sign_pub: String,
    /// The device key to bind; `None` keeps the one bound (a member listed again).
    pub key_hash: Option<String>,
}

/// Bounds of one account's log.
#[derive(Clone, Copy, Debug)]
pub struct LogLimits {
    pub max_ops: i64,
    pub max_bytes: i64,
}

impl LogLimits {
    /// What authors other than the primary may fill: all but a tenth, so the primary's ops always fit.
    #[must_use]
    pub const fn without_reserve(self) -> Self {
        Self {
            max_ops: self.max_ops - self.max_ops / 10,
            max_bytes: self.max_bytes - self.max_bytes / 10,
        }
    }
}

/// What one append may use: the account's log, and for an author other than the primary its own share.
#[derive(Clone, Copy, Debug)]
pub struct OpsQuota {
    pub log: LogLimits,
    /// `None` for the primary; else the most ops and bytes of this author the log may hold.
    pub author: Option<LogLimits>,
}

pub struct ReinsRoster;

impl ReinsRoster {
    /// The chain, oldest first.
    pub async fn entries(user: &str, conn: &DbConn) -> Result<Vec<ReinsRosterRow>, crate::Error> {
        conn.run(move |c| q_entries(c, user)).await.map_res("Error reading the Reins roster")
    }

    pub async fn latest(user: &str, conn: &DbConn) -> Result<Option<ReinsRosterRow>, crate::Error> {
        conn.run(move |c| q_latest(c, user)).await.map_res("Error reading the Reins roster")
    }

    /// Appends `row` if its epoch is the next one, with the members it lists (every other member becomes
    /// [`ROLE_REMOVED`]). Without a recovery wrap of its own, `row` keeps the previous one; older rows lose theirs and
    /// any bundle. `move_role`: the approval role (`reins_devices`) moves to this new primary in the same transaction,
    /// with its member row's device key and push token; the approval device it replaced is returned.
    pub async fn append(
        row: ReinsRosterRow,
        listed: Vec<Listed>,
        move_role: Option<String>,
        now: i64,
        conn: &DbConn,
    ) -> Result<Option<ReinsDevice>, WriteError> {
        conn.run(move |c| q_append(c, row, &listed, move_role.as_deref(), now)).await
    }

    pub async fn clear_bundle(user: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(reins_rosters::table.filter(reins_rosters::user_uuid.eq(user)))
                .set(reins_rosters::bundle.eq(None::<String>))
                .execute(c)
        })
        .await
        .map_res("Error dropping the Reins handover bundle")
    }

    /// Everything phone roles keep for `user` (a vault reset).
    pub async fn delete_all(user: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| q_delete_all(c, user)).await.map_res("Error deleting the Reins roster")
    }
}

impl ReinsMember {
    pub async fn find(user: &str, device: &str, conn: &DbConn) -> Result<Option<Self>, crate::Error> {
        conn.run(move |c| {
            reins_members::table
                .filter(reins_members::user_uuid.eq(user))
                .filter(reins_members::device_uuid.eq(device))
                .first::<Self>(c)
                .optional()
        })
        .await
        .map_res("Error reading a Reins member")
    }

    /// Every member, current or removed.
    pub async fn all(user: &str, conn: &DbConn) -> Result<Vec<Self>, crate::Error> {
        conn.run(move |c| reins_members::table.filter(reins_members::user_uuid.eq(user)).load::<Self>(c))
            .await
            .map_res("Error reading Reins members")
    }

    pub async fn set_token(user: &str, device: &str, token: Option<String>, now: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(
                reins_members::table
                    .filter(reins_members::user_uuid.eq(user))
                    .filter(reins_members::device_uuid.eq(device)),
            )
            .set((reins_members::fcm_token.eq(token), reins_members::updated_at.eq(now)))
            .execute(c)
        })
        .await
        .map_res("Error saving a Reins member's push token")
    }

    /// Forgets `token` wherever a member of `user` still has it (the push service said it is gone).
    pub async fn clear_token(user: &str, token: &str, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(
                reins_members::table
                    .filter(reins_members::user_uuid.eq(user))
                    .filter(reins_members::fcm_token.eq(token)),
            )
            .set(reins_members::fcm_token.eq(None::<String>))
            .execute(c)
        })
        .await
        .map_res("Error clearing a Reins member's push token")
    }

    /// Raises the member's acknowledged `seq` to `seq` (never lowers it).
    pub async fn ack(user: &str, device: &str, seq: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(
                reins_members::table
                    .filter(reins_members::user_uuid.eq(user))
                    .filter(reins_members::device_uuid.eq(device))
                    .filter(reins_members::acked_seq.lt(seq)),
            )
            .set(reins_members::acked_seq.eq(seq))
            .execute(c)
        })
        .await
        .map_res("Error saving a Reins acknowledgement")
    }

    pub async fn set_heartbeat(user: &str, device: &str, heartbeat: &str, at: i64, conn: &DbConn) -> EmptyResult {
        conn.run(move |c| {
            diesel::update(
                reins_members::table
                    .filter(reins_members::user_uuid.eq(user))
                    .filter(reins_members::device_uuid.eq(device)),
            )
            .set((reins_members::heartbeat.eq(Some(heartbeat)), reins_members::heartbeat_at.eq(Some(at))))
            .execute(c)
        })
        .await
        .map_res("Error saving a Reins heartbeat")
    }
}

/// Where an account's log stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogState {
    /// The highest `seq` assigned (ops or checkpoints).
    pub head: i64,
    /// Ops up to here are gone.
    pub floor: i64,
    /// The latest checkpoint's `seq`.
    pub checkpoint: Option<i64>,
}

pub struct ReinsOps;

impl ReinsOps {
    pub async fn state(user: &str, conn: &DbConn) -> Result<LogState, crate::Error> {
        conn.run(move |c| q_state(c, user)).await.map_res("Error reading the Reins log")
    }

    /// The op `author` stored with `counter` under the signing key `sign_pub`, if it is still kept.
    pub async fn find(
        user: &str,
        author: &str,
        sign_pub: &str,
        counter: i64,
        conn: &DbConn,
    ) -> Result<Option<ReinsOpRow>, crate::Error> {
        conn.run(move |c| {
            reins_ops::table
                .filter(reins_ops::user_uuid.eq(user))
                .filter(reins_ops::author.eq(author))
                .filter(reins_ops::sign_pub.eq(sign_pub))
                .filter(reins_ops::counter.eq(counter))
                .first::<ReinsOpRow>(c)
                .optional()
        })
        .await
        .map_res("Error reading a Reins op")
    }

    /// Ops after `after`, ascending, at most `limit`.
    pub async fn after(user: &str, after: i64, limit: i64, conn: &DbConn) -> Result<Vec<ReinsOpRow>, crate::Error> {
        conn.run(move |c| {
            reins_ops::table
                .filter(reins_ops::user_uuid.eq(user))
                .filter(reins_ops::seq.gt(after))
                .order(reins_ops::seq.asc())
                .limit(limit)
                .load::<ReinsOpRow>(c)
        })
        .await
        .map_res("Error reading Reins ops")
    }

    /// Stores `ops` of `author` (counters ascending, the first above `expected_last`) under the next `seq`s.
    /// [`WriteError::Conflict`] when the author's stored counter is no longer `expected_last`.
    #[allow(clippy::too_many_arguments, reason = "an append is described by this many independent facts")]
    pub async fn append(
        user: &str,
        author: &str,
        sign_pub: &str,
        expected_last: i64,
        ops: Vec<(i64, String)>,
        quota: OpsQuota,
        now: i64,
        conn: &DbConn,
    ) -> Result<Vec<i64>, WriteError> {
        conn.run(move |c| q_append_ops(c, user, (author, sign_pub), expected_last, &ops, quota, now)).await
    }

    /// Stores a checkpoint at a `seq` above the latest one, keeps the latest two and drops the ops up to the older.
    pub async fn put_checkpoint(row: ReinsCheckpointRow, conn: &DbConn) -> Result<(), WriteError> {
        conn.run(move |c| q_put_checkpoint(c, &row)).await
    }

    pub async fn latest_checkpoint(user: &str, conn: &DbConn) -> Result<Option<ReinsCheckpointRow>, crate::Error> {
        conn.run(move |c| {
            reins_checkpoints::table
                .filter(reins_checkpoints::user_uuid.eq(user))
                .order(reins_checkpoints::seq.desc())
                .first::<ReinsCheckpointRow>(c)
                .optional()
        })
        .await
        .map_res("Error reading a Reins checkpoint")
    }
}

fn q_entries(c: &mut DbConnInner, user: &str) -> QueryResult<Vec<ReinsRosterRow>> {
    reins_rosters::table.filter(reins_rosters::user_uuid.eq(user)).order(reins_rosters::epoch.asc()).load(c)
}

fn q_latest(c: &mut DbConnInner, user: &str) -> QueryResult<Option<ReinsRosterRow>> {
    reins_rosters::table
        .filter(reins_rosters::user_uuid.eq(user))
        .order(reins_rosters::epoch.desc())
        .first::<ReinsRosterRow>(c)
        .optional()
}

fn q_append(
    c: &mut DbConnInner,
    mut row: ReinsRosterRow,
    listed: &[Listed],
    move_role: Option<&str>,
    now: i64,
) -> Result<Option<ReinsDevice>, WriteError> {
    c.transaction::<_, WriteError, _>(|c| {
        let previous = q_latest(c, &row.user_uuid)?;
        if row.epoch != previous.as_ref().map_or(1, |p| p.epoch + 1) {
            return Err(WriteError::Conflict);
        }
        if row.recovery_wrap.is_none() {
            row.recovery_wrap = previous.and_then(|p| p.recovery_wrap);
        }
        let user = row.user_uuid.clone();
        match diesel::insert_into(reins_rosters::table).values(&row).execute(c) {
            Err(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => return Err(WriteError::Conflict),
            other => other?,
        };
        diesel::update(
            reins_rosters::table.filter(reins_rosters::user_uuid.eq(&user)).filter(reins_rosters::epoch.lt(row.epoch)),
        )
        .set((reins_rosters::recovery_wrap.eq(None::<String>), reins_rosters::bundle.eq(None::<String>)))
        .execute(c)?;
        let existing: Vec<ReinsMember> = reins_members::table.filter(reins_members::user_uuid.eq(&user)).load(c)?;
        for member in &existing {
            if member.is_current() && !listed.iter().any(|l| l.device_uuid == member.device_uuid) {
                diesel::update(
                    reins_members::table
                        .filter(reins_members::user_uuid.eq(&user))
                        .filter(reins_members::device_uuid.eq(&member.device_uuid)),
                )
                .set((
                    reins_members::role.eq(ROLE_REMOVED),
                    reins_members::fcm_token.eq(None::<String>),
                    reins_members::updated_at.eq(now),
                ))
                .execute(c)?;
            }
        }
        for l in listed {
            match existing.iter().find(|m| m.device_uuid == l.device_uuid) {
                Some(m) => {
                    // Another signing key is another identity: its counters and acknowledgements start over.
                    let rekeyed = m.sign_pub != l.sign_pub;
                    diesel::update(
                        reins_members::table
                            .filter(reins_members::user_uuid.eq(&user))
                            .filter(reins_members::device_uuid.eq(&l.device_uuid)),
                    )
                    .set((
                        reins_members::role.eq(l.role),
                        reins_members::sign_pub.eq(&l.sign_pub),
                        reins_members::key_hash.eq(l.key_hash.clone().or_else(|| m.key_hash.clone())),
                        reins_members::last_counter.eq(if rekeyed {
                            0
                        } else {
                            m.last_counter
                        }),
                        reins_members::acked_seq.eq(if rekeyed {
                            0
                        } else {
                            m.acked_seq
                        }),
                        // A heartbeat signed with the old keys is not this member's any more.
                        reins_members::heartbeat.eq(if rekeyed {
                            None
                        } else {
                            m.heartbeat.clone()
                        }),
                        reins_members::heartbeat_at.eq(if rekeyed {
                            None
                        } else {
                            m.heartbeat_at
                        }),
                        reins_members::updated_at.eq(now),
                    ))
                    .execute(c)?;
                }
                None => {
                    diesel::insert_into(reins_members::table)
                        .values(ReinsMember {
                            user_uuid: user.clone(),
                            device_uuid: l.device_uuid.clone(),
                            role: l.role.to_owned(),
                            sign_pub: l.sign_pub.clone(),
                            key_hash: l.key_hash.clone(),
                            fcm_token: None,
                            last_counter: 0,
                            acked_seq: 0,
                            heartbeat: None,
                            heartbeat_at: None,
                            updated_at: now,
                        })
                        .execute(c)?;
                }
            }
        }
        let Some(device) = move_role else {
            return Ok(None);
        };
        let member: ReinsMember = reins_members::table
            .filter(reins_members::user_uuid.eq(&user))
            .filter(reins_members::device_uuid.eq(device))
            .first(c)?;
        let user_id = UserId::from(user.clone());
        let previous =
            reins_devices::table.filter(reins_devices::user_uuid.eq(&user_id)).first::<ReinsDevice>(c).optional()?;
        diesel::delete(reins_devices::table.filter(reins_devices::user_uuid.eq(&user_id))).execute(c)?;
        diesel::insert_into(reins_devices::table)
            .values(ReinsDevice {
                user_uuid: user_id,
                device_uuid: DeviceId::from(device.to_owned()),
                fcm_token: member.fcm_token,
                updated_at: now,
                key_hash: member.key_hash,
            })
            .execute(c)?;
        Ok(previous)
    })
}

pub(super) fn q_delete_all(c: &mut DbConnInner, user: &str) -> QueryResult<()> {
    c.transaction(|c| {
        diesel::delete(reins_ops::table.filter(reins_ops::user_uuid.eq(user))).execute(c)?;
        diesel::delete(reins_checkpoints::table.filter(reins_checkpoints::user_uuid.eq(user))).execute(c)?;
        diesel::delete(reins_members::table.filter(reins_members::user_uuid.eq(user))).execute(c)?;
        diesel::delete(reins_rosters::table.filter(reins_rosters::user_uuid.eq(user))).execute(c)?;
        Ok(())
    })
}

fn q_state(c: &mut DbConnInner, user: &str) -> QueryResult<LogState> {
    let op_head: Option<i64> =
        reins_ops::table.filter(reins_ops::user_uuid.eq(user)).select(diesel::dsl::max(reins_ops::seq)).first(c)?;
    let checkpoints: Vec<i64> = reins_checkpoints::table
        .filter(reins_checkpoints::user_uuid.eq(user))
        .select(reins_checkpoints::seq)
        .order(reins_checkpoints::seq.desc())
        .load(c)?;
    let checkpoint = checkpoints.first().copied();
    Ok(LogState {
        head: op_head.unwrap_or(0).max(checkpoint.unwrap_or(0)),
        floor: if checkpoints.len() >= 2 {
            checkpoints[checkpoints.len() - 1]
        } else {
            0
        },
        checkpoint,
    })
}

fn q_append_ops(
    c: &mut DbConnInner,
    user: &str,
    (author, sign_pub): (&str, &str),
    expected_last: i64,
    ops: &[(i64, String)],
    quota: OpsQuota,
    now: i64,
) -> Result<Vec<i64>, WriteError> {
    let Some(last) = ops.last().map(|(counter, _)| *counter) else {
        return Ok(Vec::new());
    };
    c.transaction::<_, WriteError, _>(|c| {
        let moved = diesel::update(
            reins_members::table
                .filter(reins_members::user_uuid.eq(user))
                .filter(reins_members::device_uuid.eq(author))
                .filter(reins_members::last_counter.eq(expected_last)),
        )
        .set(reins_members::last_counter.eq(last))
        .execute(c)?;
        if moved != 1 {
            return Err(WriteError::Conflict);
        }
        let stored: Vec<(String, i32)> = reins_ops::table
            .filter(reins_ops::user_uuid.eq(user))
            .select((reins_ops::author, reins_ops::size))
            .load(c)?;
        let adding: i64 = ops.iter().map(|(_, op)| i64::try_from(op.len()).unwrap_or(i64::MAX)).sum();
        let added = i64::try_from(ops.len()).unwrap_or(i64::MAX);
        let fits = |limits: LogLimits, mine: bool| {
            let rows = stored.iter().filter(|(a, _)| !mine || a == author);
            let (count, bytes) = rows.fold((0i64, 0i64), |(n, b), (_, size)| (n + 1, b + i64::from(*size)));
            count.saturating_add(added) <= limits.max_ops && bytes.saturating_add(adding) <= limits.max_bytes
        };
        // The primary may fill the whole log; anyone else only its own share, and never the primary's reserve.
        let log = if quota.author.is_some() {
            quota.log.without_reserve()
        } else {
            quota.log
        };
        if !fits(log, false) || quota.author.is_some_and(|share| !fits(share, true)) {
            return Err(WriteError::Full);
        }
        let head = q_state(c, user)?.head;
        let mut seqs = Vec::with_capacity(ops.len());
        for (i, (counter, op)) in ops.iter().enumerate() {
            let seq = head + 1 + i64::try_from(i).unwrap_or(i64::MAX);
            diesel::insert_into(reins_ops::table)
                .values(ReinsOpRow {
                    user_uuid: user.to_owned(),
                    seq,
                    author: author.to_owned(),
                    sign_pub: sign_pub.to_owned(),
                    counter: *counter,
                    op: op.clone(),
                    size: i32::try_from(op.len()).unwrap_or(i32::MAX),
                    created_at: now,
                })
                .execute(c)?;
            seqs.push(seq);
        }
        Ok(seqs)
    })
}

fn q_put_checkpoint(c: &mut DbConnInner, row: &ReinsCheckpointRow) -> Result<(), WriteError> {
    c.transaction::<_, WriteError, _>(|c| {
        let latest: Option<i64> = reins_checkpoints::table
            .filter(reins_checkpoints::user_uuid.eq(&row.user_uuid))
            .select(diesel::dsl::max(reins_checkpoints::seq))
            .first(c)?;
        if latest.is_some_and(|l| row.seq <= l) {
            return Err(WriteError::Conflict);
        }
        diesel::insert_into(reins_checkpoints::table).values(row).execute(c)?;
        let seqs: Vec<i64> = reins_checkpoints::table
            .filter(reins_checkpoints::user_uuid.eq(&row.user_uuid))
            .select(reins_checkpoints::seq)
            .order(reins_checkpoints::seq.desc())
            .load(c)?;
        if let Some(&older) = seqs.get(1) {
            diesel::delete(
                reins_checkpoints::table
                    .filter(reins_checkpoints::user_uuid.eq(&row.user_uuid))
                    .filter(reins_checkpoints::seq.lt(older)),
            )
            .execute(c)?;
            diesel::delete(
                reins_ops::table.filter(reins_ops::user_uuid.eq(&row.user_uuid)).filter(reins_ops::seq.le(older)),
            )
            .execute(c)?;
        }
        Ok(())
    })
}

#[cfg(all(test, sqlite))]
mod tests {
    use super::*;

    fn roster_row(user: &str, epoch: i64) -> ReinsRosterRow {
        ReinsRosterRow {
            user_uuid: user.to_owned(),
            epoch,
            entry: format!("entry-{epoch}"),
            entry_hash: format!("hash-{epoch}"),
            recovery_wrap: (epoch == 1).then(|| "wrap".to_owned()),
            bundle: Some("bundle".to_owned()),
            created_at: epoch,
        }
    }

    fn listed(device: &str, role: &'static str, key: &str) -> Listed {
        Listed {
            device_uuid: device.to_owned(),
            role,
            sign_pub: key.to_owned(),
            key_hash: Some(format!("kh-{device}")),
        }
    }

    const LIMITS: OpsQuota = OpsQuota {
        log: LogLimits {
            max_ops: 5,
            max_bytes: 1000,
        },
        author: None,
    };

    fn q_append_t(c: &mut DbConnInner, row: ReinsRosterRow, listed: &[Listed], now: i64) -> Result<(), WriteError> {
        q_append(c, row, listed, None, now).map(drop)
    }

    #[test]
    fn the_roster_appends_only_the_next_epoch_and_tracks_members() {
        let mut c = super::super::reins_device::test_db();
        assert!(matches!(q_append_t(&mut c, roster_row("u1", 2), &[], 1), Err(WriteError::Conflict)));
        q_append_t(&mut c, roster_row("u1", 1), &[listed("p", ROLE_PRIMARY, "kp")], 1).unwrap();
        assert!(matches!(q_append_t(&mut c, roster_row("u1", 1), &[], 2), Err(WriteError::Conflict)), "a replay");
        q_append_t(&mut c, roster_row("u1", 2), &[listed("p", ROLE_PRIMARY, "kp"), listed("v", ROLE_VIEWER, "kv")], 2)
            .unwrap();
        let rows = q_entries(&mut c, "u1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].recovery_wrap.as_deref(), rows[0].bundle.as_deref()), (None, None), "older rows forget");
        assert_eq!(rows[1].recovery_wrap.as_deref(), Some("wrap"), "the wrap carries over");
        // A handover: v primary, p viewer; then a removal of p.
        q_append_t(&mut c, roster_row("u1", 3), &[listed("v", ROLE_PRIMARY, "kv"), listed("p", ROLE_VIEWER, "kp")], 3)
            .unwrap();
        q_append_t(&mut c, roster_row("u1", 4), &[listed("v", ROLE_PRIMARY, "kv")], 4).unwrap();
        let members: Vec<ReinsMember> = reins_members::table.order(reins_members::device_uuid).load(&mut c).unwrap();
        assert_eq!(
            members.iter().map(|m| (m.device_uuid.as_str(), m.role.as_str())).collect::<Vec<_>>(),
            [("p", ROLE_REMOVED), ("v", ROLE_PRIMARY)]
        );
        q_delete_all(&mut c, "u1").unwrap();
        assert!(q_latest(&mut c, "u1").unwrap().is_none());
    }

    #[test]
    fn ops_take_the_next_seqs_counters_move_once_and_checkpoints_drop_old_ops() {
        let mut c = super::super::reins_device::test_db();
        q_append_t(&mut c, roster_row("u1", 1), &[listed("p", ROLE_PRIMARY, "kp")], 1).unwrap();
        let ops = |from: i64, n: i64| (from..from + n).map(|i| (i, format!("op{i}"))).collect::<Vec<_>>();
        assert_eq!(q_append_ops(&mut c, "u1", ("p", "kp"), 0, &ops(1, 2), LIMITS, 1).unwrap(), [1, 2]);
        assert!(matches!(q_append_ops(&mut c, "u1", ("p", "kp"), 0, &ops(1, 1), LIMITS, 1), Err(WriteError::Conflict)));
        assert!(matches!(q_append_ops(&mut c, "u1", ("x", "kx"), 0, &ops(1, 1), LIMITS, 1), Err(WriteError::Conflict)));
        assert_eq!(q_append_ops(&mut c, "u1", ("p", "kp"), 2, &ops(3, 2), LIMITS, 1).unwrap(), [3, 4]);
        assert!(matches!(q_append_ops(&mut c, "u1", ("p", "kp"), 4, &ops(5, 2), LIMITS, 1), Err(WriteError::Full)));
        let cp = |seq: i64| ReinsCheckpointRow {
            user_uuid: "u1".to_owned(),
            seq,
            checkpoint: format!("cp{seq}"),
            created_at: 1,
        };
        q_put_checkpoint(&mut c, &cp(2)).unwrap();
        assert!(matches!(q_put_checkpoint(&mut c, &cp(2)), Err(WriteError::Conflict)));
        assert_eq!(
            q_state(&mut c, "u1").unwrap(),
            LogState {
                head: 4,
                floor: 0,
                checkpoint: Some(2)
            }
        );
        q_put_checkpoint(&mut c, &cp(4)).unwrap();
        assert_eq!(
            q_state(&mut c, "u1").unwrap(),
            LogState {
                head: 4,
                floor: 2,
                checkpoint: Some(4)
            }
        );
        let left: Vec<i64> = reins_ops::table.select(reins_ops::seq).order(reins_ops::seq).load(&mut c).unwrap();
        assert_eq!(left, [3, 4], "ops up to the older checkpoint are gone");
        // The space freed lets the author write again, numbered after the head.
        assert_eq!(q_append_ops(&mut c, "u1", ("p", "kp"), 4, &ops(5, 1), LIMITS, 1).unwrap(), [5]);
    }

    #[test]
    fn the_role_moves_with_the_roster_in_one_transaction() {
        let mut c = super::super::reins_device::test_db();
        q_append_t(&mut c, roster_row("u1", 1), &[listed("p", ROLE_PRIMARY, "kp")], 1).unwrap();
        q_append_t(&mut c, roster_row("u1", 2), &[listed("p", ROLE_PRIMARY, "kp"), listed("v", ROLE_VIEWER, "kv")], 2)
            .unwrap();
        let handover = [listed("v", ROLE_PRIMARY, "kv"), listed("p", ROLE_VIEWER, "kp")];
        assert!(q_append(&mut c, roster_row("u1", 3), &handover, Some("v"), 3).unwrap().is_none());
        let approval: ReinsDevice = reins_devices::table.first(&mut c).unwrap();
        assert_eq!((approval.device_uuid.to_string(), approval.key_hash.as_deref()), ("v".to_owned(), Some("kh-v")));
        // A losing compare-and-swap moves nothing.
        let back = [listed("p", ROLE_PRIMARY, "kp"), listed("v", ROLE_VIEWER, "kv")];
        assert!(matches!(q_append(&mut c, roster_row("u1", 3), &back, Some("p"), 4), Err(WriteError::Conflict)));
        let approval: ReinsDevice = reins_devices::table.first(&mut c).unwrap();
        assert_eq!(approval.device_uuid.to_string(), "v");
        let previous = q_append(&mut c, roster_row("u1", 4), &back, Some("p"), 5).unwrap().unwrap();
        assert_eq!(previous.device_uuid.to_string(), "v");
    }

    #[test]
    fn viewers_get_a_share_of_the_log_and_never_the_primarys_reserve() {
        let mut c = super::super::reins_device::test_db();
        q_append_t(&mut c, roster_row("u1", 1), &[listed("p", ROLE_PRIMARY, "kp")], 1).unwrap();
        q_append_t(&mut c, roster_row("u1", 2), &[listed("p", ROLE_PRIMARY, "kp"), listed("v", ROLE_VIEWER, "kv")], 2)
            .unwrap();
        q_append_t(
            &mut c,
            roster_row("u1", 3),
            &[listed("p", ROLE_PRIMARY, "kp"), listed("v", ROLE_VIEWER, "kv"), listed("w", ROLE_VIEWER, "kw")],
            3,
        )
        .unwrap();
        let quota = |author: Option<i64>| OpsQuota {
            log: LogLimits {
                max_ops: 10,
                max_bytes: 10_000,
            },
            author: author.map(|n| LogLimits {
                max_ops: n,
                max_bytes: 10_000,
            }),
        };
        let one = |i: i64| vec![(i, format!("op{i}"))];
        // v's share is 3 ops.
        for i in 1..=3 {
            q_append_ops(&mut c, "u1", ("v", "kv"), i - 1, &one(i), quota(Some(3)), 1).unwrap();
        }
        assert!(matches!(
            q_append_ops(&mut c, "u1", ("v", "kv"), 3, &one(4), quota(Some(3)), 1),
            Err(WriteError::Full)
        ));
        // w fills up to 9 of 10 (a tenth is the primary's), then stops.
        for i in 1..=6 {
            q_append_ops(&mut c, "u1", ("w", "kw"), i - 1, &one(i), quota(Some(100)), 1).unwrap();
        }
        assert!(matches!(
            q_append_ops(&mut c, "u1", ("w", "kw"), 6, &one(7), quota(Some(100)), 1),
            Err(WriteError::Full)
        ));
        // The primary still writes.
        q_append_ops(&mut c, "u1", ("p", "kp"), 0, &one(1), quota(None), 1).unwrap();
    }

    #[test]
    fn a_counter_is_unique_per_signing_key() {
        let mut c = super::super::reins_device::test_db();
        q_append_t(&mut c, roster_row("u1", 1), &[listed("p", ROLE_PRIMARY, "kp")], 1).unwrap();
        let one = vec![(1, "op1".to_owned())];
        q_append_ops(&mut c, "u1", ("p", "kp"), 0, &one, LIMITS, 1).unwrap();
        // The phone enrolls again with a new key: its counter starts over, and the old op stays the old key's.
        q_append_t(&mut c, roster_row("u1", 2), &[listed("p", ROLE_PRIMARY, "kp2")], 2).unwrap();
        q_append_ops(&mut c, "u1", ("p", "kp2"), 0, &one, LIMITS, 2).unwrap();
        let rows: Vec<(String, i64)> = reins_ops::table
            .filter(reins_ops::author.eq("p"))
            .filter(reins_ops::counter.eq(1))
            .select((reins_ops::sign_pub, reins_ops::seq))
            .order(reins_ops::seq)
            .load(&mut c)
            .unwrap();
        assert_eq!(rows, [("kp".to_owned(), 1), ("kp2".to_owned(), 2)]);
        // The same (author, key, counter) twice is refused by the index itself.
        let dup = diesel::insert_into(reins_ops::table)
            .values(ReinsOpRow {
                user_uuid: "u1".to_owned(),
                seq: 9,
                author: "p".to_owned(),
                sign_pub: "kp2".to_owned(),
                counter: 1,
                op: "x".to_owned(),
                size: 1,
                created_at: 3,
            })
            .execute(&mut c);
        assert!(matches!(dup, Err(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _))));
    }
}
