//! Circles (Decisions v2, 04): shared control between two Reins accounts ([`reins_proto::circle`]).
//!
//! The server relays and routes; it opens nothing. Every route is for an account's **approval device** only (08 R8:
//! viewers never decide for others), and every lookup is scoped to the caller being a party:
//!
//! - **Invitations** (`/reins/api/circles/invites*`, `/reins/api/circles/join`): the owner posts a commitment and gets
//!   a one-time link (only the SHA-256 of its secret is kept); the helper opens it and posts its challenge (which
//!   binds that helper, once); the owner reveals; the helper accepts; the owner activates, which creates the link with
//!   both sides' sealed copies. Each step once, in order (compare-and-swap on `state`).
//! - **Peers' rosters** (`GET /reins/api/circles/peers/<id>/roster`): the other party's roster chain, which each phone
//!   verifies from the entry it pinned.
//! - **Links** (`GET /reins/api/circles/links`, `POST …/links/<id>/change`): routing state; a signed change (pause,
//!   resume, end) is kept with its signature, checked here too (defence in depth: the phones verify it themselves).
//! - **Packets** (`POST /reins/api/circles/packets`, `DELETE …/packets/<id>`, `GET …/inbox`,
//!   `POST …/packets/<id>/answer`, `GET …/answers`, `POST …/answers/seen`): sealed reviews to the helper's approval
//!   phone and sealed answers back.
//!
//! Pushes carry only ids (`PushKind::Circle`); only a review waiting for a helper, and a guardianship ending, alert.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex, PoisonError},
};

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use reins_proto::{
    PROTOCOL_VERSION,
    canon::{hex32, parse_sig, sha256, strict_ed25519_key},
    circle::{
        self, AnswerItem, CircleKind, InboxItem, InviteAccept, InviteActivate, InviteChallenge, InviteCreated,
        InviteOpened, InviteReveal, InviteState, InviteStatus, JoinBody, LinkState, LinkView, NewAnswer, NewInvite,
        NewPacket, PacketKind, PacketState, Seen, Side, SignedChange,
    },
    device::codes,
    pairing::{PushKind, PushMessage},
    roster::Roster,
};
use rocket::{Data, Route, State, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};

use super::{
    HUB,
    device_api::{
        DeviceKey, PhoneResult, api_err, bad_request, internal, not_found, parse_versioned, rate_limited,
        read_body_limited, require_approval_device, user_key,
    },
    now_unix, push,
    relay::ItemSignal,
};
use crate::{
    auth::{ClientIp, Headers},
    db::{
        DbConn, DbPool,
        models::{ReinsCircleInvite, ReinsCircleLink, ReinsCirclePacket, ReinsDevice, User, UserId, reins_circle},
    },
};

/// Largest request body (a sealed arrangement copy is the largest field).
const MAX_BODY_BYTES: u64 = 2 * circle::MAX_SEALED_CHARS as u64 + 4096;
/// Open invitations per owner.
const MAX_OPEN_INVITES: i64 = 5;
/// Waiting packets per link.
const MAX_WAITING_PACKETS: i64 = 200;

/// Accounts whose phone has Circle news: ends their long poll with `Pending.circles`.
pub struct CircleNews {
    users: Mutex<HashSet<String>>,
    signal: Arc<ItemSignal>,
}

impl CircleNews {
    pub fn new(signal: Arc<ItemSignal>) -> Self {
        Self {
            users: Mutex::new(HashSet::new()),
            signal,
        }
    }

    pub fn flag(&self, user: &str) {
        self.users.lock().unwrap_or_else(PoisonError::into_inner).insert(user.to_owned());
        self.signal.notify();
    }

    /// Whether `user` had news (and clears it).
    pub fn take(&self, user: &str) -> bool {
        self.users.lock().unwrap_or_else(PoisonError::into_inner).remove(user)
    }

    pub fn forget(&self, user: &str) {
        self.users.lock().unwrap_or_else(PoisonError::into_inner).remove(user);
    }
}

/// Why a push: what the phone fetches, and whether it alerts (a review waiting, a guardianship ending).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A review waits for the helper: an alert.
    Review,
    /// The helper must know at once (a guardianship ended): an alert.
    Alert,
    /// Anything else (an answer, a step, a change): silent.
    News,
}

/// The push id: `review:<id>`, `alert:<id>` or `news:<id>` (the iOS extension and `apns` read the prefix).
#[must_use]
pub fn push_id(wake: Wake, id: &str) -> String {
    let prefix = match wake {
        Wake::Review => "review",
        Wake::Alert => "alert",
        Wake::News => "news",
    };
    format!("{prefix}:{id}")
}

/// Whether a Circle push alerts (else it is silent).
#[must_use]
pub fn alerts(id: &str) -> bool {
    id.starts_with("review:") || id.starts_with("alert:")
}

/// Flags `user`'s news and pushes its approval device.
async fn wake(user: &str, id: &str, wake: Wake, conn: &DbConn, pool: &DbPool) {
    HUB.circles.flag(user);
    let user_uuid = UserId::from(user.to_owned());
    if let Some(device) = ReinsDevice::find_by_user(&user_uuid, conn).await {
        push::spawn_push(
            pool.clone(),
            user_uuid,
            device.fcm_token,
            PushMessage {
                t: PushKind::Circle,
                id: push_id(wake, id),
                p: None,
            },
        );
    }
}

/// The protocol steps an invitation has been through, as they arrived (stored as JSON in `data`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InviteSteps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<InviteChallenge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<InviteReveal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<InviteAccept>,
}

/// The SHA-256 (hex) of a link secret, as stored.
#[must_use]
pub fn token_hash(token: &str) -> String {
    hex32(&sha256(token.as_bytes()))
}

/// Whether an invitation may move from `from` to `to` (the protocol's order, [`InviteState::can_go`]).
#[must_use]
pub fn may_move(from: &str, to: InviteState) -> bool {
    InviteState::parse(from).is_some_and(|f| f.can_go(to))
}

fn steps(invite: &ReinsCircleInvite) -> InviteSteps {
    serde_json::from_str(&invite.data).unwrap_or_default()
}

fn conflict(message: &str) -> rocket::response::status::Custom<Json<reins_proto::device::ApiError>> {
    api_err(Status::Conflict, codes::ALREADY_ANSWERED, message)
}

async fn email_of(user: &str, conn: &DbConn) -> String {
    User::find_by_uuid(&UserId::from(user.to_owned()), conn).await.map(|u| u.email).unwrap_or_default()
}

/// The current owner key and epoch of `user`'s roster; refused without one (Circles need phone roles).
async fn roster_head(user: &str, conn: &DbConn) -> PhoneResult<(String, u64)> {
    let chain = super::roles::chain(user, conn).await?;
    let head = chain.last().ok_or_else(|| {
        api_err(Status::Conflict, codes::NO_ROSTER, "Circles need phone roles on this account (a primary phone)")
    })?;
    Ok((head.owner_pub.clone(), head.epoch))
}

async fn status(invite: &ReinsCircleInvite, owner_view: bool, conn: &DbConn) -> InviteStatus {
    let mut s = steps(invite);
    if let Some(challenge) = s.challenge.as_mut() {
        // The link's secret is never shown back.
        challenge.token = String::new();
    }
    let helper = match &invite.helper_uuid {
        Some(h) => Some(email_of(h, conn).await),
        None => None,
    };
    InviteStatus {
        v: PROTOCOL_VERSION,
        id: invite.id.clone(),
        state: InviteState::parse(&invite.state).unwrap_or(InviteState::Expired),
        kind: CircleKind::parse(&invite.kind).unwrap_or(CircleKind::Watcher),
        owner_account: invite.owner_uuid.clone(),
        owner: email_of(&invite.owner_uuid, conn).await,
        owner_pub: invite.owner_pub.clone(),
        owner_epoch: u64::try_from(invite.owner_epoch).unwrap_or(0),
        commitment: invite.commitment.clone(),
        helper_account: invite.helper_uuid.clone(),
        helper,
        challenge: s.challenge,
        // The owner already holds what it revealed; the helper already holds what it accepted.
        reveal: if owner_view {
            None
        } else {
            s.reveal
        },
        accept: if owner_view {
            s.accept
        } else {
            None
        },
        link: invite.link_id.clone(),
        expires_at: invite.expires_at,
    }
}

/// `user`'s invitation `id` as its owner.
async fn owned_invite(user: &str, id: &str, conn: &DbConn) -> PhoneResult<ReinsCircleInvite> {
    ReinsCircleInvite::find(id, conn).await.filter(|i| i.owner_uuid == user).ok_or_else(not_found)
}

/// `user`'s invitation `id` as its bound helper.
async fn helped_invite(user: &str, id: &str, conn: &DbConn) -> PhoneResult<ReinsCircleInvite> {
    ReinsCircleInvite::find(id, conn).await.filter(|i| i.helper_uuid.as_deref() == Some(user)).ok_or_else(not_found)
}

/// Writes `invite` (already changed) if it is still in `from`; a stale step is a conflict.
async fn advance(invite: &ReinsCircleInvite, from: &str, conn: &DbConn) -> PhoneResult<()> {
    match invite.update(from, conn).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(conflict("This invitation moved on")),
        Err(e) => Err(internal(&e)),
    }
}

/// Owner, step 1.
#[post("/reins/api/circles/invites", data = "<data>")]
async fn post_invite(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<InviteCreated>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    if let Err(wait) = super::limits::CIRCLE_INVITES.check(&user) {
        return Err(rate_limited("too many invitations", wait));
    }
    let new: NewInvite = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    new.validate().map_err(|e| bad_request(e.to_string()))?;
    let (owner_pub, epoch) = roster_head(&user, &conn).await?;
    if owner_pub != new.owner_pub || epoch != new.owner_epoch {
        return Err(bad_request("The owner key is not this account's current one"));
    }
    let now = now_unix();
    if ReinsCircleInvite::open_count(&user, now, &conn).await >= MAX_OPEN_INVITES {
        return Err(api_err(Status::TooManyRequests, codes::RATE_LIMITED, "Too many open invitations"));
    }
    let token = crate::crypto::encode_random_bytes::<32>(&BASE64URL_NOPAD);
    let row = ReinsCircleInvite {
        id: crate::crypto::encode_random_bytes::<16>(&HEXLOWER),
        token_hash: token_hash(&token),
        owner_uuid: user,
        kind: new.kind.as_str().to_owned(),
        state: InviteState::Open.as_str().to_owned(),
        owner_pub: new.owner_pub,
        owner_epoch: i64::try_from(new.owner_epoch).unwrap_or(i64::MAX),
        commitment: new.commitment,
        helper_uuid: None,
        data: "{}".to_owned(),
        link_id: None,
        created_at: now,
        expires_at: now + circle::INVITE_TTL_SECS,
        updated_at: now,
    };
    row.insert(&conn).await.map_err(|e| internal(&e))?;
    Ok(Json(InviteCreated {
        id: row.id,
        token,
        expires_at: row.expires_at,
    }))
}

#[get("/reins/api/circles/invites/<id>")]
async fn get_invite(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<InviteStatus>> {
    require_approval_device(&headers, &key, &conn).await?;
    let invite = owned_invite(&user_key(&headers), id, &conn).await?;
    Ok(Json(status(&invite, true, &conn).await))
}

#[delete("/reins/api/circles/invites/<id>")]
async fn delete_invite(
    id: &str,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let mut invite = owned_invite(&user_key(&headers), id, &conn).await?;
    if !may_move(&invite.state, InviteState::Cancelled) {
        return Ok(Status::NoContent);
    }
    let from = invite.state.clone();
    InviteState::Cancelled.as_str().clone_into(&mut invite.state);
    invite.updated_at = now_unix();
    advance(&invite, &from, &conn).await?;
    if let Some(helper) = &invite.helper_uuid {
        wake(helper, &invite.id, Wake::News, &conn, pool.inner()).await;
    }
    Ok(Status::NoContent)
}

/// Helper: opening a link. Another account's link only; the owner can't join its own invitation.
#[post("/reins/api/circles/join", data = "<data>")]
async fn post_join(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    ip: ClientIp,
    conn: DbConn,
) -> PhoneResult<Json<InviteOpened>> {
    if let Err(wait) = super::limits::CIRCLE_OPENS.check(&ip.ip) {
        return Err(rate_limited("too many invitation links from this address", wait));
    }
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let body: JoinBody = serde_json::from_slice(&read_body_limited(data, 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let invite = ReinsCircleInvite::find_by_token_hash(&token_hash(&body.token), &conn)
        .await
        .filter(|i| i.is_open() && i.expires_at > now_unix())
        .ok_or_else(not_found)?;
    if invite.owner_uuid == user {
        return Err(bad_request("This is your own invitation"));
    }
    if invite.helper_uuid.as_ref().is_some_and(|h| h != &user) {
        return Err(not_found());
    }
    roster_head(&user, &conn).await?;
    Ok(Json(InviteOpened {
        v: PROTOCOL_VERSION,
        id: invite.id.clone(),
        kind: CircleKind::parse(&invite.kind).unwrap_or(CircleKind::Watcher),
        inviter: email_of(&invite.owner_uuid, &conn).await,
        inviter_account: invite.owner_uuid.clone(),
        commitment: invite.commitment.clone(),
        owner_pub: invite.owner_pub.clone(),
        owner_epoch: u64::try_from(invite.owner_epoch).unwrap_or(0),
        expires_at: invite.expires_at,
    }))
}

/// Helper, step 2: binds this helper (once).
#[post("/reins/api/circles/invites/<id>/challenge", data = "<data>")]
async fn post_challenge(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let challenge: InviteChallenge = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    challenge.validate().map_err(|e| bad_request(e.to_string()))?;
    let mut invite = ReinsCircleInvite::find(id, &conn).await.ok_or_else(not_found)?;
    if !reins_proto::enroll::ct_eq(token_hash(&challenge.token).as_bytes(), invite.token_hash.as_bytes())
        || invite.expires_at <= now_unix()
    {
        return Err(not_found());
    }
    if invite.owner_uuid == user {
        return Err(bad_request("This is your own invitation"));
    }
    if !may_move(&invite.state, InviteState::Challenged) {
        // The same helper sending the same challenge again (its first answer was lost): already taken.
        let again = invite.helper_uuid.as_deref() == Some(user.as_str())
            && steps(&invite).challenge.is_some_and(|c| {
                c.helper_pub == challenge.helper_pub
                    && c.helper_epoch == challenge.helper_epoch
                    && c.nonce == challenge.nonce
            });
        if again {
            return Ok(Status::NoContent);
        }
        return Err(conflict("Someone already answered this invitation"));
    }
    let (helper_pub, epoch) = roster_head(&user, &conn).await?;
    if helper_pub != challenge.helper_pub || epoch != challenge.helper_epoch {
        return Err(bad_request("The owner key is not this account's current one"));
    }
    let mut s = steps(&invite);
    // The link's secret did its job; it is never stored.
    s.challenge = Some(InviteChallenge {
        token: String::new(),
        ..challenge
    });
    invite.data = serde_json::to_string(&s).unwrap_or_default();
    invite.helper_uuid = Some(user);
    InviteState::Challenged.as_str().clone_into(&mut invite.state);
    invite.updated_at = now_unix();
    advance(&invite, InviteState::Open.as_str(), &conn).await?;
    wake(&invite.owner_uuid, &invite.id, Wake::News, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

#[get("/reins/api/circles/invites/<id>/helper")]
async fn get_helper_invite(
    id: &str,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<InviteStatus>> {
    require_approval_device(&headers, &key, &conn).await?;
    let invite = helped_invite(&user_key(&headers), id, &conn).await?;
    Ok(Json(status(&invite, false, &conn).await))
}

/// Owner, step 3.
#[post("/reins/api/circles/invites/<id>/reveal", data = "<data>")]
async fn post_reveal(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let reveal: InviteReveal = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    reveal.validate().map_err(|e| bad_request(e.to_string()))?;
    let mut invite = owned_invite(&user_key(&headers), id, &conn).await?;
    if !may_move(&invite.state, InviteState::Revealed) {
        return Err(conflict("This invitation is not waiting for your answer"));
    }
    let mut s = steps(&invite);
    s.reveal = Some(reveal);
    invite.data = serde_json::to_string(&s).unwrap_or_default();
    InviteState::Revealed.as_str().clone_into(&mut invite.state);
    invite.updated_at = now_unix();
    advance(&invite, InviteState::Challenged.as_str(), &conn).await?;
    if let Some(helper) = &invite.helper_uuid {
        wake(helper, &invite.id, Wake::News, &conn, pool.inner()).await;
    }
    Ok(Status::NoContent)
}

/// Helper, step 4.
#[post("/reins/api/circles/invites/<id>/accept", data = "<data>")]
async fn post_accept(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let accept: InviteAccept = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    accept.validate().map_err(|e| bad_request(e.to_string()))?;
    let mut invite = helped_invite(&user_key(&headers), id, &conn).await?;
    if !may_move(&invite.state, InviteState::Accepted) {
        return Err(conflict("This invitation is not waiting for your answer"));
    }
    let mut s = steps(&invite);
    s.accept = Some(accept);
    invite.data = serde_json::to_string(&s).unwrap_or_default();
    InviteState::Accepted.as_str().clone_into(&mut invite.state);
    invite.updated_at = now_unix();
    advance(&invite, InviteState::Revealed.as_str(), &conn).await?;
    wake(&invite.owner_uuid, &invite.id, Wake::News, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

#[post("/reins/api/circles/invites/<id>/decline")]
async fn post_decline(
    id: &str,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let mut invite = helped_invite(&user_key(&headers), id, &conn).await?;
    if !may_move(&invite.state, InviteState::Declined) {
        return Ok(Status::NoContent);
    }
    let from = invite.state.clone();
    InviteState::Declined.as_str().clone_into(&mut invite.state);
    invite.updated_at = now_unix();
    advance(&invite, &from, &conn).await?;
    wake(&invite.owner_uuid, &invite.id, Wake::News, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

/// Owner, step 5: the link exists from here on.
#[post("/reins/api/circles/invites/<id>/activate", data = "<data>")]
async fn post_activate(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let activate: InviteActivate = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    activate.validate().map_err(|e| bad_request(e.to_string()))?;
    let mut invite = owned_invite(&user_key(&headers), id, &conn).await?;
    let Some(helper) = invite.helper_uuid.clone() else {
        return Err(conflict("Nobody accepted this invitation"));
    };
    if !may_move(&invite.state, InviteState::Active) {
        return Err(conflict("This invitation is not accepted"));
    }
    if ReinsCircleLink::find(&activate.link, &conn).await.is_some() {
        return Err(bad_request("This arrangement exists already"));
    }
    let now = now_unix();
    let link = ReinsCircleLink {
        id: activate.link,
        owner_uuid: invite.owner_uuid.clone(),
        helper_uuid: helper.clone(),
        kind: invite.kind.clone(),
        state: LinkState::Active.as_str().to_owned(),
        owner_copy: activate.owner_copy,
        helper_copy: activate.helper_copy,
        last_change: None,
        created_at: now,
        updated_at: now,
    };
    // The link first, then the invitation: an invitation marked active always has its link (a failed insert leaves
    // it accepted, to activate again); a lost race to activate removes the link it made.
    link.insert(&conn).await.map_err(|e| internal(&e))?;
    InviteState::Active.as_str().clone_into(&mut invite.state);
    invite.link_id = Some(link.id.clone());
    invite.updated_at = now;
    if let Err(e) = advance(&invite, InviteState::Accepted.as_str(), &conn).await {
        if let Err(d) = ReinsCircleLink::delete(&link.id, &conn).await {
            warn!("Reins: a Circle link of a lost activation could not be removed: {d:?}");
        }
        return Err(e);
    }
    wake(&helper, &link.id, Wake::News, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

/// The other party's roster, for an invitation or a link the caller is a party to.
#[get("/reins/api/circles/peers/<id>/roster")]
async fn get_peer_roster(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Roster>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let peer = if let Some(link) = ReinsCircleLink::find(id, &conn).await {
        if link.owner_uuid == user {
            Some(link.helper_uuid)
        } else if link.helper_uuid == user {
            Some(link.owner_uuid)
        } else {
            None
        }
    } else if let Some(invite) = ReinsCircleInvite::find(id, &conn).await {
        if invite.owner_uuid == user {
            invite.helper_uuid
        } else if invite.helper_uuid.as_deref() == Some(user.as_str()) {
            Some(invite.owner_uuid)
        } else {
            None
        }
    } else {
        None
    };
    let peer = peer.ok_or_else(not_found)?;
    Ok(Json(Roster {
        entries: super::roles::chain(&peer, &conn).await?,
    }))
}

#[get("/reins/api/circles/links")]
async fn get_links(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Vec<LinkView>>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let mut out = Vec::new();
    for link in ReinsCircleLink::for_user(&user, &conn).await {
        let owner = link.owner_uuid == user;
        let peer_account = if owner {
            link.helper_uuid.clone()
        } else {
            link.owner_uuid.clone()
        };
        out.push(LinkView {
            id: link.id.clone(),
            side: if owner {
                Side::Owner
            } else {
                Side::Helper
            },
            kind: CircleKind::parse(&link.kind).unwrap_or(CircleKind::Watcher),
            state: LinkState::parse(&link.state).unwrap_or(LinkState::Ended),
            peer: email_of(&peer_account, &conn).await,
            peer_account,
            copy: if owner {
                link.owner_copy.clone()
            } else {
                link.helper_copy.clone()
            },
            change: link.last_change.as_deref().and_then(|c| serde_json::from_str(c).ok()),
            updated_at: link.updated_at,
        });
    }
    Ok(Json(out))
}

/// Whether `sig` over `msg` verifies under `key` (strict Ed25519).
fn ed25519_ok(key: &str, msg: &[u8], sig: &str) -> bool {
    let (Some(key), Some(sig)) = (reins_proto::desktop::decode_key(key), parse_sig(sig)) else {
        return false;
    };
    strict_ed25519_key(&key)
        && ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(msg, &sig).is_ok()
}

/// Whether a link may go from `from` to `to` when `by` asks: the owner pauses, resumes and ends; the helper only ends;
/// an ended link stays ended.
#[must_use]
pub fn change_allowed(from: LinkState, to: LinkState, by: Side) -> bool {
    match (from, to, by) {
        (LinkState::Ended, ..) => false,
        (_, LinkState::Ended, _)
        | (LinkState::Active, LinkState::Paused, Side::Owner)
        | (LinkState::Paused, LinkState::Active, Side::Owner) => true,
        _ => false,
    }
}

#[post("/reins/api/circles/links/<id>/change", data = "<data>")]
async fn post_change(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let signed: SignedChange = serde_json::from_slice(&read_body_limited(data, 16 * 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    let change = signed.change().map_err(|e| bad_request(e.to_string()))?;
    let mut link = ReinsCircleLink::find(id, &conn).await.ok_or_else(not_found)?;
    let side = if link.owner_uuid == user {
        Side::Owner
    } else if link.helper_uuid == user {
        Side::Helper
    } else {
        return Err(not_found());
    };
    if change.link != link.id || change.by != side {
        return Err(bad_request("This change is for another arrangement or side"));
    }
    let from = LinkState::parse(&link.state).unwrap_or(LinkState::Ended);
    if link.kind == CircleKind::Guardian.as_str() && change.state == LinkState::Paused {
        return Err(conflict("A guardianship can't be paused; end it instead"));
    }
    if !change_allowed(from, change.state, side) {
        return Err(conflict("This arrangement can't change that way"));
    }
    let last_seq = link
        .last_change
        .as_deref()
        .and_then(|c| serde_json::from_str::<SignedChange>(c).ok())
        .and_then(|c| c.change().ok())
        .map_or(0, |c| c.seq);
    if change.seq <= last_seq {
        return Err(conflict("An older change"));
    }
    // Signed by the changer's current owner key (the phones check it against the key they pinned).
    let (owner_pub, _) = roster_head(&user, &conn).await?;
    if !ed25519_ok(&owner_pub, &circle::change_message(&signed.body), &signed.sig) {
        return Err(bad_request("The change is not signed by this account's owner key"));
    }
    let now = now_unix();
    let from_state = link.state.clone();
    change.state.as_str().clone_into(&mut link.state);
    link.last_change = Some(serde_json::to_string(&signed).unwrap_or_default());
    link.updated_at = now;
    match link.update(&from_state, &conn).await {
        Ok(true) => {}
        Ok(false) => return Err(conflict("This arrangement changed meanwhile")),
        Err(e) => return Err(internal(&e)),
    }
    if change.state != LinkState::Active {
        ReinsCirclePacket::withdraw_for_link(&link.id, now, &conn).await.map_err(|e| internal(&e))?;
    }
    let peer = if side == Side::Owner {
        &link.helper_uuid
    } else {
        &link.owner_uuid
    };
    // A guardian learns at once that the ward ended the guardianship (08 R7: tamper-evident); everything else is
    // quiet news.
    let how = if link.kind == CircleKind::Guardian.as_str() && side == Side::Owner && change.state == LinkState::Ended {
        Wake::Alert
    } else {
        Wake::News
    };
    wake(peer, &link.id, how, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

#[post("/reins/api/circles/packets", data = "<data>")]
async fn post_packet(
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    if let Err(wait) = super::limits::CIRCLE_PACKETS.check(&user) {
        return Err(rate_limited("too many Circle reviews", wait));
    }
    let new: NewPacket = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    let now = now_unix();
    new.validate(now).map_err(|e| bad_request(e.to_string()))?;
    let link = ReinsCircleLink::find(&new.link, &conn).await.filter(|l| l.owner_uuid == user).ok_or_else(not_found)?;
    match LinkState::parse(&link.state) {
        Some(LinkState::Active) => {}
        Some(LinkState::Paused) => return Err(api_err(Status::Conflict, codes::ALREADY_ANSWERED, "paused")),
        _ => return Err(api_err(Status::Conflict, codes::ALREADY_ANSWERED, "ended")),
    }
    if ReinsCirclePacket::count_waiting(&link.id, &conn).await >= MAX_WAITING_PACKETS {
        return Err(api_err(Status::TooManyRequests, codes::RATE_LIMITED, "Too many reviews waiting for this person"));
    }
    if ReinsCirclePacket::find(&new.id, &conn).await.is_some() {
        return Ok(Status::NoContent);
    }
    let row = ReinsCirclePacket {
        id: new.id,
        link_id: link.id.clone(),
        owner_uuid: user,
        helper_uuid: link.helper_uuid.clone(),
        kind: new.kind.as_str().to_owned(),
        state: PacketState::Waiting.as_str().to_owned(),
        sealed: new.sealed,
        sealed_answer: None,
        deadline: new.deadline,
        created_at: now,
        answered_at: None,
        delivered: false,
        owner_seen: false,
        updated_at: now,
    };
    row.insert(&conn).await.map_err(|e| internal(&e))?;
    match new.kind {
        // A review needs the helper: the one notification Circles send.
        PacketKind::Review => wake(&link.helper_uuid, &row.id, Wake::Review, &conn, pool.inner()).await,
        // A notice waits for the helper's next look.
        PacketKind::Notice => HUB.circles.flag(&link.helper_uuid),
    }
    Ok(Status::NoContent)
}

#[delete("/reins/api/circles/packets/<id>")]
async fn delete_packet(id: &str, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let mut packet = ReinsCirclePacket::find(id, &conn).await.filter(|p| p.owner_uuid == user).ok_or_else(not_found)?;
    if packet.state != PacketState::Waiting.as_str() {
        return Ok(Status::NoContent);
    }
    PacketState::Withdrawn.as_str().clone_into(&mut packet.state);
    packet.owner_seen = true;
    packet.updated_at = now_unix();
    packet.update(PacketState::Waiting.as_str(), &conn).await.map_err(|e| internal(&e))?;
    HUB.circles.flag(&packet.helper_uuid);
    Ok(Status::NoContent)
}

/// One page of the helper's packets: waiting ones first (oldest first), then those that changed since `since` (a day by
/// default); `offset` pages on ([`reins_circle::INBOX_PAGE`] a page).
#[get("/reins/api/circles/inbox?<since>&<offset>")]
async fn get_inbox(
    since: Option<i64>,
    offset: Option<i64>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
) -> PhoneResult<Json<Vec<InboxItem>>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let since = since.unwrap_or_else(|| now_unix() - 86_400);
    let rows = ReinsCirclePacket::inbox(&user, since, offset.unwrap_or(0).max(0), &conn).await;
    let fresh: Vec<String> = rows.iter().filter(|p| !p.delivered).map(|p| p.id.clone()).collect();
    if !fresh.is_empty() {
        ReinsCirclePacket::mark_delivered(&user, fresh, &conn).await.map_err(|e| internal(&e))?;
    }
    Ok(Json(
        rows.into_iter()
            .map(|p| InboxItem {
                kind: PacketKind::parse(&p.kind).unwrap_or(PacketKind::Notice),
                state: PacketState::parse(&p.state).unwrap_or(PacketState::Expired),
                id: p.id,
                link: p.link_id,
                deadline: p.deadline,
                created_at: p.created_at,
                sealed: p.sealed,
            })
            .collect(),
    ))
}

#[post("/reins/api/circles/packets/<id>/answer", data = "<data>")]
async fn post_answer(
    id: &str,
    data: Data<'_>,
    headers: Headers,
    key: DeviceKey,
    conn: DbConn,
    pool: &State<DbPool>,
) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    let answer: NewAnswer = parse_versioned(&read_body_limited(data, MAX_BODY_BYTES).await?)?;
    answer.validate().map_err(|e| bad_request(e.to_string()))?;
    let mut packet =
        ReinsCirclePacket::find(id, &conn).await.filter(|p| p.helper_uuid == user).ok_or_else(not_found)?;
    if packet.kind != PacketKind::Review.as_str() {
        return Err(bad_request("A notice takes no answer"));
    }
    let link = ReinsCircleLink::find(&packet.link_id, &conn).await.ok_or_else(not_found)?;
    if link.state != LinkState::Active.as_str() {
        return Err(api_err(Status::Conflict, codes::ALREADY_ANSWERED, link.state.as_str()));
    }
    if packet.state != PacketState::Waiting.as_str() {
        return Err(super::device_api::already_answered());
    }
    let now = now_unix();
    PacketState::Answered.as_str().clone_into(&mut packet.state);
    packet.sealed_answer = Some(answer.sealed);
    packet.answered_at = Some(now);
    packet.updated_at = now;
    match packet.update(PacketState::Waiting.as_str(), &conn).await {
        Ok(true) => {}
        Ok(false) => return Err(super::device_api::already_answered()),
        Err(e) => return Err(internal(&e)),
    }
    wake(&packet.owner_uuid, &packet.id, Wake::News, &conn, pool.inner()).await;
    Ok(Status::NoContent)
}

#[get("/reins/api/circles/answers")]
async fn get_answers(headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Json<Vec<AnswerItem>>> {
    require_approval_device(&headers, &key, &conn).await?;
    let user = user_key(&headers);
    Ok(Json(
        ReinsCirclePacket::news_for_owner(&user, &conn)
            .await
            .into_iter()
            .map(|p| AnswerItem {
                state: PacketState::parse(&p.state).unwrap_or(PacketState::Expired),
                packet: p.id,
                link: p.link_id,
                sealed: p.sealed_answer,
                answered_at: p.answered_at,
            })
            .collect(),
    ))
}

#[post("/reins/api/circles/answers/seen", data = "<data>")]
async fn post_seen(data: Data<'_>, headers: Headers, key: DeviceKey, conn: DbConn) -> PhoneResult<Status> {
    require_approval_device(&headers, &key, &conn).await?;
    let seen: Seen = serde_json::from_slice(&read_body_limited(data, 64 * 1024).await?)
        .map_err(|e| bad_request(format!("invalid body: {e}")))?;
    if seen.ids.len() > 500 || seen.ids.iter().any(|id| !circle::valid_id(id)) {
        return Err(bad_request("ids: at most 500 packet ids"));
    }
    ReinsCirclePacket::mark_seen(&user_key(&headers), seen.ids, &conn).await.map_err(|e| internal(&e))?;
    Ok(Status::NoContent)
}

/// Packets and invitations whose time ran out; the owner's phone hears of an expired packet (it settles the request).
pub async fn sweep(pool: &DbPool, now: i64, purge: bool) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    for mut packet in ReinsCirclePacket::due(now, &conn).await {
        PacketState::Expired.as_str().clone_into(&mut packet.state);
        packet.updated_at = now;
        if matches!(packet.update(PacketState::Waiting.as_str(), &conn).await, Ok(true)) {
            HUB.circles.flag(&packet.helper_uuid);
            if packet.kind == PacketKind::Review.as_str() {
                wake(&packet.owner_uuid, &packet.id, Wake::News, &conn, pool).await;
            }
        }
    }
    for mut invite in ReinsCircleInvite::due(now, &conn).await {
        let from = invite.state.clone();
        InviteState::Expired.as_str().clone_into(&mut invite.state);
        invite.updated_at = now;
        if matches!(invite.update(&from, &conn).await, Ok(true)) {
            HUB.circles.flag(&invite.owner_uuid);
        }
    }
    if purge && let Err(e) = reins_circle::purge(now, &conn).await {
        warn!("Reins: could not purge Circle rows: {e:?}");
    }
}

/// Every 15 s; purges hourly.
pub fn spawn_sweeper(pool: DbPool) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            tick = tick.wrapping_add(1);
            sweep(&pool, now_unix(), tick.is_multiple_of(240)).await;
        }
    });
}

/// An account reset (its roster, so its keys, are gone): its links end server-side, without a signature, so that no
/// packet reaches keys that no longer exist. Phones never drop an obligation on such an end (only on a signed change);
/// they see the arrangement as suspended, and the peer is told (a guardian with an alert, 08 R7).
pub async fn end_all_for(user: &str, conn: &DbConn, pool: &DbPool) {
    let now = now_unix();
    for mut link in ReinsCircleLink::for_user(user, conn).await {
        if link.state == LinkState::Ended.as_str() {
            continue;
        }
        let from = link.state.clone();
        LinkState::Ended.as_str().clone_into(&mut link.state);
        link.updated_at = now;
        if matches!(link.update(&from, conn).await, Ok(true)) {
            if let Err(e) = ReinsCirclePacket::withdraw_for_link(&link.id, now, conn).await {
                warn!("Reins: could not withdraw the packets of an ended Circle link: {e:?}");
            }
            let (peer, how) = if link.owner_uuid == user {
                let how = if link.kind == CircleKind::Guardian.as_str() {
                    Wake::Alert
                } else {
                    Wake::News
                };
                (link.helper_uuid.clone(), how)
            } else {
                (link.owner_uuid.clone(), Wake::News)
            };
            wake(&peer, &link.id, how, conn, pool).await;
        }
    }
}

/// `user`'s owner key changed (a recovery): its peers are told, a guardian with an alert (08 R7). Their phones verify
/// the roster themselves and suspend the arrangement.
pub async fn owner_key_changed(user: &str, conn: &DbConn, pool: &DbPool) {
    for link in ReinsCircleLink::for_user(user, conn).await {
        if link.state == LinkState::Ended.as_str() {
            continue;
        }
        let (peer, how) = if link.owner_uuid == user {
            let how = if link.kind == CircleKind::Guardian.as_str() { Wake::Alert } else { Wake::News };
            (link.helper_uuid.clone(), how)
        } else {
            (link.owner_uuid.clone(), Wake::News)
        };
        wake(&peer, &link.id, how, conn, pool).await;
    }
}

pub fn routes() -> Vec<Route> {
    routes![
        post_invite,
        get_invite,
        delete_invite,
        post_join,
        post_challenge,
        get_helper_invite,
        post_reveal,
        post_accept,
        post_decline,
        post_activate,
        get_peer_roster,
        get_links,
        post_change,
        post_packet,
        delete_packet,
        get_inbox,
        post_answer,
        get_answers,
        post_seen,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_hashed() {
        let t = "abc";
        assert_eq!(token_hash(t).len(), 64);
        assert_ne!(token_hash(t), token_hash("abd"));
        assert_ne!(token_hash(t), t);
    }

    #[test]
    fn invite_transitions_are_ordered() {
        assert!(may_move("open", InviteState::Challenged));
        assert!(!may_move("challenged", InviteState::Challenged), "one challenge only");
        assert!(!may_move("open", InviteState::Revealed));
        assert!(!may_move("active", InviteState::Cancelled));
        assert!(!may_move("nonsense", InviteState::Challenged));
    }

    #[test]
    fn link_changes_follow_the_sides() {
        use LinkState::{Active, Ended, Paused};
        assert!(change_allowed(Active, Paused, Side::Owner));
        assert!(change_allowed(Paused, Active, Side::Owner));
        assert!(change_allowed(Active, Ended, Side::Helper));
        assert!(change_allowed(Paused, Ended, Side::Helper));
        assert!(!change_allowed(Active, Paused, Side::Helper));
        assert!(!change_allowed(Ended, Active, Side::Owner));
        assert!(!change_allowed(Ended, Ended, Side::Helper));
    }

    #[test]
    fn only_reviews_and_alerts_alert() {
        assert!(alerts(&push_id(Wake::Review, "p")));
        assert!(alerts(&push_id(Wake::Alert, "l")));
        assert!(!alerts(&push_id(Wake::News, "p")));
    }
}
