//! Circles on the phone (`docs/design/decisions-v2/04-shared-control.md`, 08 R8): shared control with other Reins
//! accounts.
//!
//! - **Invitations** ([`invite`]): a safety code pins each side's roster owner key; the arrangement is signed by both
//!   owner keys and kept here (the server holds only sealed copies).
//! - **The owner's side** ([`review`]): a parked request that an arrangement covers gets a **hold**; review packets go
//!   to each helper's approval phone; the owner's tap and the helpers' signed answers are **settled**
//!   (`reins_policy::circles::settle`) before anything runs; either deny wins.
//! - **The helper's side** ([`review`]): packets are opened, checked against the owner's current approval phone, kept
//!   as reviews ("For others"), and answered with a signed, sealed answer.
//!
//! Everything a Circle signs or seals uses this account's keys: the owner key (arrangements, changes), this phone's
//! signing key (packets, answers) and box key (what others seal to it), and the roster credential key (the sealed
//! copies). Only the approval phone (the roster's primary) does any of it.

mod invite;
mod net;
mod review;
pub mod types;

use crate::session::api_call;
use std::sync::PoisonError;

use reins_proto::{
    canon::encode_payload,
    circle::{self, Arrangement, LinkState, PeerKeys, Side, SignedArrangement},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use types::*;

use crate::{
    engine::Engine,
    error::CoreError,
    phone_api::ApiFailure,
    roles::keys::{ed25519_verify, seal_to},
    session::Session,
};

/// The device-store service of this account's Circle state (one JSON document per account slot).
const STATE: &str = "reins.circles";

/// How an arrangement stands on this phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Standing {
    /// The helper signed; the owner hasn't confirmed yet (helper side only).
    Pending,
    Active,
    Paused,
    Ended,
    /// The other side's owner key changed: nothing counts until a new invitation.
    Suspended,
}

/// The other side's approval phone, as last verified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Peer {
    pub device_id: String,
    pub sign_pub: String,
    pub box_pub: String,
    pub credential_pub: String,
    pub epoch: u64,
    pub hash: String,
}

/// An arrangement this account is in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Held {
    pub signed: SignedArrangement,
    pub arrangement: Arrangement,
    pub side: Side,
    pub standing: Standing,
    /// The other side's roster entry this phone last verified (starts at the one the code pinned).
    pub anchor_epoch: u64,
    pub anchor_hash: String,
    /// The newest epoch of the other side this phone ever verified (a server can't roll it back).
    pub seen_epoch: u64,
    pub peer: Option<Peer>,
    /// The last signed change this phone accepted for the link.
    pub last_seq: u64,
    pub since: i64,
}

impl Held {
    pub fn id(&self) -> &str {
        &self.arrangement.id
    }

    /// The other party.
    pub fn other(&self) -> &circle::Party {
        match self.side {
            Side::Owner => &self.arrangement.helper,
            Side::Helper => &self.arrangement.owner,
        }
    }

    pub fn person(&self) -> CirclePerson {
        let terms = &self.arrangement.terms;
        CirclePerson {
            link_id: self.arrangement.id.clone(),
            peer: self.other().label.clone(),
            role: terms.kind.into(),
            mine: self.side == Side::Owner,
            scope_line: terms.scope_line.clone(),
            deferred: terms.class == circle::Class::Deferred,
            window_secs: terms.window_secs,
            state: match self.standing {
                Standing::Active => CircleLinkState::Active,
                Standing::Paused | Standing::Pending => CircleLinkState::Paused,
                Standing::Ended => CircleLinkState::Ended,
                Standing::Suspended => CircleLinkState::Suspended,
            },
            since: self.since,
        }
    }
}

/// Everything Circles keep on this phone for one account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CircleState {
    pub invites: Vec<invite::OwnerInvite>,
    pub joins: Vec<invite::HelperJoin>,
    pub held: Vec<Held>,
    pub holds: Vec<review::Hold>,
    pub reviews: Vec<review::Review>,
    pub notices: Vec<review::NoticeRow>,
}

impl CircleState {
    pub fn held(&self, id: &str) -> Option<&Held> {
        self.held.iter().find(|h| h.id() == id)
    }

    pub fn held_mut(&mut self, id: &str) -> Option<&mut Held> {
        self.held.iter_mut().find(|h| h.id() == id)
    }
}

/// Who this account is now, as its own roster says.
pub(crate) struct Me {
    pub account: String,
    pub slot: String,
    pub device_id: String,
    pub owner_pub: String,
    pub epoch: u64,
    pub entry_hash: String,
    pub credential_pub: String,
}

/// Strict Ed25519, for `reins_proto::circle`.
pub(crate) fn verify(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    ed25519_verify(key, msg, sig)
}

/// `plain` sealed to `to` (X25519, base64url) as it travels.
pub(crate) fn sealed_to(to: &str, plain: &[u8]) -> Result<String, CoreError> {
    Ok(encode_payload(&seal_to(to, plain)?))
}

fn json<T: Serialize>(v: &T) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    serde_json::to_vec(v).map(Zeroizing::new).map_err(|e| CoreError::storage(e.to_string()))
}

/// 16 random bytes as 32 hex characters (an arrangement's or a packet's id).
pub(crate) fn new_id() -> Result<String, CoreError> {
    Ok(data_encoding::HEXLOWER.encode(&crate::crypto::random_bytes::<16>()?))
}

fn not_available(e: ApiFailure) -> CoreError {
    match e {
        ApiFailure::Status {
            status: 404,
            ..
        } => CoreError::NotFound,
        ApiFailure::Status {
            status: 409,
            message,
            ..
        } if message.contains("phone roles") => {
            CoreError::invalid("Circles need phone roles on both accounts (Settings › Phones).")
        }
        other => other.into_core(),
    }
}

impl Engine {
    // ---- state ------------------------------------------------------------------------------------------------------

    pub(crate) fn circle_state(&self, slot: &str) -> Result<CircleState, CoreError> {
        match self.store.device_secret_get(STATE, slot)? {
            Some(raw) => serde_json::from_slice(&raw).map_err(|_| CoreError::storage("corrupt Circle state")),
            None => Ok(CircleState::default()),
        }
    }

    pub(crate) fn save_circle_state(&self, slot: &str, state: &CircleState) -> Result<(), CoreError> {
        self.store.device_secret_put(STATE, slot, &json(state)?)
    }

    /// Reads, changes and writes this account's Circle state under one lock.
    pub(crate) fn with_circles<T>(
        &self,
        slot: &str,
        change: impl FnOnce(&mut CircleState) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let _one = self.circles_lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.circle_state(slot)?;
        let out = change(&mut state)?;
        self.save_circle_state(slot, &state)?;
        Ok(out)
    }

    /// The account slot without the network, if this phone holds Circle state for it.
    pub(crate) fn circle_slot_now(&self) -> Option<String> {
        self.account_slot_now().ok().flatten()
    }

    /// This account as its roster says now (refreshed), on its approval phone. Circles need phone roles.
    pub(crate) async fn circle_me(&self, session: &Session) -> Result<Me, CoreError> {
        self.require_decider()?;
        let account = self.roles_account(session).await?;
        let roster = match self.refresh_roster(session, &account).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Err(CoreError::invalid("Circles need phone roles on this account (Settings › Phones)."));
            }
            Err(e) => {
                log::info!("the roster could not be refreshed for a Circle step: {e}");
                self.local_roster(&account.slot)?.ok_or_else(|| {
                    CoreError::invalid("Circles need phone roles on this account (Settings › Phones).")
                })?
            }
        };
        let current = roster.current();
        let device_id = self.store.device_id()?;
        if current.primary.device_id != device_id || current.primary.sign_pub != self.sign_key()?.public_b64() {
            return Err(CoreError::ViewerOnly);
        }
        Ok(Me {
            account: account.id,
            slot: account.slot,
            device_id,
            owner_pub: current.owner_pub.clone(),
            epoch: current.epoch,
            entry_hash: current.hash_hex().map_err(|e| CoreError::invalid(e.to_string()))?,
            credential_pub: current.credential_pub.clone(),
        })
    }

    /// This account's owner key (the primary holds its seed).
    pub(crate) fn circle_owner_key(&self, slot: &str) -> Result<crate::roles::keys::SigningKey, CoreError> {
        self.owner_seed(slot)?
            .ok_or_else(|| CoreError::invalid("This phone doesn't hold the account's owner key."))?
            .owner_key()
    }

    /// Fetches and verifies the other side's roster for arrangement `id` (or an invitation), from the anchor this
    /// phone keeps; advances the anchor. `OwnerChanged` suspends the arrangement.
    pub(crate) async fn circle_peer(&self, session: &Session, slot: &str, id: &str) -> Result<Peer, CoreError> {
        let held = self.circle_state(slot)?.held(id).cloned().ok_or(CoreError::NotFound)?;
        let roster = api_call!(session, |api| api.circle_peer_roster(id)).map_err(not_available)?;
        let keys = circle::peer_keys(
            &held.other().owner_pub,
            (held.anchor_epoch, &held.anchor_hash),
            held.seen_epoch,
            &roster.entries,
            verify,
        )
        .map_err(|e| CoreError::invalid(format!("The other person's phones could not be verified: {e}")))?;
        match keys {
            PeerKeys::Current {
                device_id,
                sign_pub,
                box_pub,
                credential_pub,
                epoch,
                hash,
            } => {
                let peer = Peer {
                    device_id,
                    sign_pub,
                    box_pub,
                    credential_pub,
                    epoch,
                    hash: hash.clone(),
                };
                let stored = peer.clone();
                self.with_circles(slot, |s| {
                    if let Some(h) = s.held_mut(id) {
                        h.anchor_epoch = epoch;
                        h.anchor_hash = hash;
                        h.seen_epoch = h.seen_epoch.max(epoch);
                        h.peer = Some(stored);
                    }
                    Ok(())
                })?;
                Ok(peer)
            }
            PeerKeys::OwnerChanged {
                epoch,
            } => {
                self.with_circles(slot, |s| {
                    if let Some(h) = s.held_mut(id)
                        && h.standing != Standing::Ended
                    {
                        log::warn!("a Circle peer's owner key changed at epoch {epoch}: the arrangement is suspended");
                        h.standing = Standing::Suspended;
                    }
                    Ok(())
                })?;
                self.notifier.circle_changed(CircleEvent::Changed);
                Err(CoreError::invalid("The other person's account was recovered: invite them again."))
            }
        }
    }

    // ---- links ------------------------------------------------------------------------------------------------------

    /// Applies what the server says of this account's links: a copy we never had is accepted only when it carries our
    /// own owner key's signature (or, as the helper, our own helper signature on an arrangement we accepted); a
    /// signed change is applied when the other side's pinned key signed it and it is newer. An ended link stays ended.
    pub(crate) async fn circle_links_sync(&self, session: &Session, me: &Me) -> Result<(), CoreError> {
        let links = api_call!(session, |api| api.circle_links()).map_err(not_available)?;
        let credential = self.owner_seed(&me.slot)?.map(|s| s.credential_key());
        let mut changed = false;
        for link in links {
            let state = self.circle_state(&me.slot)?;
            let known = state.held(&link.id).cloned();
            if known.as_ref().is_none_or(|h| h.standing == Standing::Pending) {
                let Some(credential) = &credential else {
                    continue;
                };
                let Some(raw) = reins_proto::canon::decode_payload(&link.copy) else {
                    continue;
                };
                let Ok(plain) = credential.open(&raw) else {
                    log::warn!("a Circle link's copy does not open with this account's key");
                    continue;
                };
                let Ok(signed) = serde_json::from_slice::<SignedArrangement>(&plain) else {
                    continue;
                };
                match self.circle_adopt(me, &link, signed, known.as_ref()) {
                    Ok(()) => changed = true,
                    Err(e) => log::warn!("a Circle link was not accepted: {e}"),
                }
            }
            if let Some(change) = &link.change
                && let Some(h) = self.circle_state(&me.slot)?.held(&link.id).cloned()
                && h.standing != Standing::Ended
            {
                let other_side = match h.side {
                    Side::Owner => Side::Helper,
                    Side::Helper => Side::Owner,
                };
                let hash = circle::arrangement_hash(&h.signed.body);
                match change.verify(&h.other().owner_pub, &hash, h.last_seq, verify) {
                    Ok(c) if c.by == other_side => {
                        self.with_circles(&me.slot, |s| {
                            if let Some(h) = s.held_mut(&link.id) {
                                h.last_seq = c.seq;
                                h.standing = match c.state {
                                    LinkState::Active => Standing::Active,
                                    LinkState::Paused => Standing::Paused,
                                    LinkState::Ended => Standing::Ended,
                                };
                            }
                            Ok(())
                        })?;
                        changed = true;
                    }
                    Ok(_) | Err(circle::CircleError::Replayed) => {}
                    Err(e) => log::warn!("a Circle change was refused: {e}"),
                }
            }
            // The server ended it without a signature (the other account was reset): suspended here, never dropped
            // (an owner's obligations stay until the owner ends them).
            if link.state == LinkState::Ended
                && link.change.is_none()
                && let Some(h) = self.circle_state(&me.slot)?.held(&link.id)
                && matches!(h.standing, Standing::Active | Standing::Paused)
            {
                self.with_circles(&me.slot, |s| {
                    if let Some(h) = s.held_mut(&link.id) {
                        h.standing = Standing::Suspended;
                    }
                    Ok(())
                })?;
                changed = true;
            }
        }
        if changed {
            self.notifier.circle_changed(CircleEvent::Changed);
        }
        Ok(())
    }

    /// A link's copy, checked: both signatures, our own side's key is ours now, and (as the helper) it is the
    /// arrangement we signed.
    fn circle_adopt(
        &self,
        me: &Me,
        link: &circle::LinkView,
        signed: SignedArrangement,
        pending: Option<&Held>,
    ) -> Result<(), CoreError> {
        let a = circle::verify_arrangement(&signed, true, verify).map_err(|e| CoreError::invalid(e.to_string()))?;
        if a.id != link.id {
            return Err(CoreError::invalid("the copy is of another arrangement"));
        }
        let (mine, other) = match link.side {
            Side::Owner => (&a.owner, &a.helper),
            Side::Helper => (&a.helper, &a.owner),
        };
        // A server can't plant an arrangement this account never signed: our side must be us, with our owner key.
        if mine.account != me.account || mine.owner_pub != me.owner_pub || other.account != link.peer_account {
            return Err(CoreError::invalid("the arrangement doesn't name this account as it is now"));
        }
        if let Some(p) = pending
            && p.signed.body != signed.body
        {
            return Err(CoreError::invalid("the confirmed arrangement isn't the one this phone accepted"));
        }
        let held = Held {
            arrangement: a.clone(),
            side: link.side,
            standing: Standing::Active,
            anchor_epoch: other.epoch,
            anchor_hash: other.entry_hash.clone(),
            seen_epoch: other.epoch,
            peer: None,
            last_seq: 0,
            since: a.created_at,
            signed,
        };
        self.with_circles(&me.slot, |s| {
            s.held.retain(|h| h.id() != a.id);
            s.held.push(held);
            s.joins.retain(|j| j.link.as_deref() != Some(a.id.as_str()));
            Ok(())
        })
    }

    /// Everything Circles do on a sync: links, the owner's answers, the helper's inbox, expiries. Quiet when this
    /// account has no Circle state and the server had no news.
    pub(crate) async fn circles_sync(&self, session: &Session, news: bool) -> Result<(), CoreError> {
        let Some(slot) = self.circle_slot_now() else {
            return Ok(());
        };
        let state = self.circle_state(&slot)?;
        if !news && state.held.is_empty() && state.invites.is_empty() && state.joins.is_empty() {
            return Ok(());
        }
        if !self.decides() {
            return Ok(());
        }
        let me = self.circle_me(session).await?;
        self.circle_invites_poll(session, &me).await;
        self.circle_links_sync(session, &me).await?;
        if let Err(e) = Box::pin(self.circle_take_answers(session, &me)).await {
            log::warn!("Circle answers: {e}");
        }
        if let Err(e) = self.circle_take_inbox(session, &me).await {
            log::warn!("Circle reviews: {e}");
        }
        Box::pin(self.circle_expire(session, &me)).await;
        Ok(())
    }

    // ---- people -----------------------------------------------------------------------------------------------------

    pub(crate) async fn circle_people(&self, refresh: bool) -> Result<Vec<CirclePerson>, CoreError> {
        if refresh {
            let session = self.session()?;
            self.circles_sync(&session, true).await?;
        }
        let Some(slot) = self.circle_slot_now() else {
            return Ok(Vec::new());
        };
        Ok(self
            .circle_state(&slot)?
            .held
            .iter()
            .filter(|h| h.standing != Standing::Pending)
            .map(Held::person)
            .collect())
    }

    /// Pause or resume (the owner), as a signed change.
    pub(crate) async fn circle_pause(&self, link_id: &str, paused: bool) -> Result<(), CoreError> {
        let to = if paused {
            LinkState::Paused
        } else {
            LinkState::Active
        };
        self.circle_change(link_id, to).await
    }

    /// End (either side; the helper leaving).
    pub(crate) async fn circle_end(&self, link_id: &str) -> Result<(), CoreError> {
        self.circle_change(link_id, LinkState::Ended).await
    }

    async fn circle_change(&self, link_id: &str, to: LinkState) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let held = self.circle_state(&me.slot)?.held(link_id).cloned().ok_or(CoreError::NotFound)?;
        if held.side == Side::Helper && to != LinkState::Ended {
            return Err(CoreError::invalid("Only the person who invited you can pause this."));
        }
        if held.standing == Standing::Ended {
            return Ok(());
        }
        // Ended here first: whatever the server does, this phone stops counting it (a late answer is refused).
        if to != LinkState::Active {
            self.with_circles(&me.slot, |s| {
                if let Some(h) = s.held_mut(link_id) {
                    h.standing = if to == LinkState::Ended {
                        Standing::Ended
                    } else {
                        Standing::Paused
                    };
                }
                Ok(())
            })?;
        }
        let change = circle::LinkChange {
            v: circle::version(),
            link: link_id.to_owned(),
            arrangement: circle::arrangement_hash(&held.signed.body),
            seq: held.last_seq + 1,
            state: to,
            by: held.side,
            at: crate::store::unix_now(),
        };
        let body = serde_json::to_string(&change).map_err(|e| CoreError::storage(e.to_string()))?;
        let sig = self.circle_owner_key(&me.slot)?.sign_b64(&circle::change_message(&body));
        let signed = circle::SignedChange {
            body,
            sig,
        };
        let sent = api_call!(&session, |api| api.circle_change(link_id, &signed));
        match sent {
            Ok(()) => {}
            // Already ended on the server: ours stays ended.
            Err(ApiFailure::Status {
                status: 409,
                ..
            }) if to == LinkState::Ended => {}
            Err(e) => return Err(e.into_core()),
        }
        self.with_circles(&me.slot, |s| {
            if let Some(h) = s.held_mut(link_id) {
                h.last_seq = change.seq;
                h.standing = match to {
                    LinkState::Active => Standing::Active,
                    LinkState::Paused => Standing::Paused,
                    LinkState::Ended => Standing::Ended,
                };
            }
            Ok(())
        })?;
        // Requests this arrangement held: its packets are gone (withdrawn by the server); settle what waits on them.
        if to != LinkState::Active {
            Box::pin(self.circle_release(&session, &me, link_id)).await;
        }
        self.notifier.circle_changed(CircleEvent::Changed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_32_hex() {
        let id = new_id().unwrap();
        assert!(circle::valid_id(&id));
        assert_ne!(id, new_id().unwrap());
    }
}
