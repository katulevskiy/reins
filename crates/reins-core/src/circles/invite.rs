//! Invitations (04 §3): commit-then-reveal over both accounts' owner keys, six digits on both phones, then the
//! helper's and the owner's signatures.

use crate::session::api_call;
use reins_proto::{
    canon::decode_payload,
    circle::{
        self, Arrangement, Class, InviteAccept, InviteActivate, InviteChallenge, InviteReveal, InviteState,
        InviteTranscript, NewInvite, OnExpiry, Party, PeerKeys, Redaction, Scope, SealedTerms, Side, SignedArrangement,
        Terms,
    },
    enroll::{encode_nonce, parse_nonce},
    roster::{RosterEntry, verify_chain},
};
use serde::{Deserialize, Serialize};

use super::{
    CircleDraft, CircleInviteProgress, CircleInviteStart, CircleInviteView, CircleJoinStage, CirclePerson,
    CircleScopePreset, Held, Me, Peer, Standing, new_id, not_available, sealed_to, verify,
};
use crate::{engine::Engine, error::CoreError, phone_api::ApiFailure, session::Session, store::unix_now};

/// The link a helper opens: `reins-circle://<server host>/<secret>`.
const LINK_SCHEME: &str = "reins-circle://";

/// An invitation this account made (the owner's side).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OwnerInvite {
    pub id: String,
    pub terms: Terms,
    /// The owner's nonce (base64url): revealed once, after the one challenge this phone pinned.
    pub nonce: String,
    pub owner: Party,
    pub created_at: i64,
    /// The first challenge seen; a different later one cancels the invitation.
    #[serde(default)]
    pub challenge: Option<InviteChallenge>,
    #[serde(default)]
    pub helper: Option<Party>,
    #[serde(default)]
    pub peer: Option<Peer>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub helper_sig: Option<String>,
}

/// An invitation this account answered (the helper's side).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HelperJoin {
    pub id: String,
    pub opened: circle::InviteOpened,
    pub nonce: String,
    pub me: Party,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub peer: Option<Peer>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub ended: Option<CircleJoinStage>,
}

/// The scope and its words for a preset.
fn scope_of(preset: &CircleScopePreset) -> (Scope, String) {
    let mut s = Scope::default();
    let line = match preset {
        CircleScopePreset::Everything => "Everything".to_owned(),
        CircleScopePreset::Money => {
            s.effects = vec!["money".to_owned()];
            "Money".to_owned()
        }
        CircleScopePreset::MessagesOutside => {
            s.effects = vec!["message".to_owned()];
            s.zones = vec!["work".to_owned(), "outside".to_owned()];
            "Messages to Work or Outside".to_owned()
        }
        CircleScopePreset::Risky => {
            s.min_level = Some(3);
            "What can't be undone".to_owned()
        }
        CircleScopePreset::Routine => {
            s.max_level = Some(1);
            "Routine requests".to_owned()
        }
        CircleScopePreset::Agent {
            connection_id,
            label,
        } => {
            s.connections = vec![connection_id.clone()];
            let label = circle::shown_text(label, 60, false);
            if label.is_empty() {
                "One AI".to_owned()
            } else {
                format!("Everything {label} does")
            }
        }
    };
    (s, line)
}

/// The terms a draft asks for.
pub(crate) fn terms_of(draft: &CircleDraft) -> Result<Terms, CoreError> {
    let kind = draft.role.into();
    let (scope, scope_line) = scope_of(&draft.preset);
    let class = if draft.deferred {
        Class::Deferred
    } else {
        Class::Instant
    };
    let terms = Terms {
        kind,
        scope,
        class,
        window_secs: circle::clamp_window(class, draft.window_secs),
        on_expiry: if draft.fall_back_to_me && kind == circle::CircleKind::Delegate {
            OnExpiry::Owner
        } else {
            OnExpiry::Deny
        },
        redaction: if draft.full_cards && kind != circle::CircleKind::Watcher {
            Redaction::Full
        } else {
            Redaction::Summary
        },
        scope_line,
        guardian: None,
    };
    terms.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
    Ok(terms)
}

/// The secret of a link (`reins-circle://host/secret`, or the bare secret). The host, when given, must be this
/// phone's server.
fn token_of(link: &str, server: &str) -> Result<String, CoreError> {
    let link = link.trim();
    let rest = link.strip_prefix(LINK_SCHEME).unwrap_or(link);
    let (host, token) = match rest.rsplit_once('/') {
        Some((host, token)) => (Some(host), token),
        None => (None, rest),
    };
    if let Some(host) = host {
        let ours = server.trim_end_matches('/').rsplit("://").next().unwrap_or(server);
        if !host.eq_ignore_ascii_case(ours) {
            return Err(CoreError::invalid("This invitation is for another Reins server."));
        }
    }
    if token.len() != 43 || decode_payload(token).is_none() {
        return Err(CoreError::invalid("This isn't a Reins invitation."));
    }
    Ok(token.to_owned())
}

/// A peer's roster, verified from its first entry, with the entry at `epoch` holding `owner_pub`: that entry is the
/// anchor (the code pins the key, the chain pins the entry). Returns the anchor's hash and the current approval phone.
fn anchored_peer(chain: &[RosterEntry], owner_pub: &str, epoch: u64) -> Result<(String, Peer), CoreError> {
    verify_chain(chain, None, verify)
        .map_err(|e| CoreError::invalid(format!("The other roster doesn't verify: {e}")))?;
    let anchor = chain
        .iter()
        .find(|e| e.epoch == epoch)
        .filter(|e| e.owner_pub == owner_pub)
        .ok_or_else(|| CoreError::invalid("The other roster doesn't hold the key it named."))?;
    let hash = anchor.hash_hex().map_err(|e| CoreError::invalid(e.to_string()))?;
    match circle::peer_keys(owner_pub, (epoch, &hash), epoch, chain, verify)
        .map_err(|e| CoreError::invalid(e.to_string()))?
    {
        PeerKeys::Current {
            device_id,
            sign_pub,
            box_pub,
            credential_pub,
            epoch,
            hash: head,
        } => Ok((
            hash,
            Peer {
                device_id,
                sign_pub,
                box_pub,
                credential_pub,
                epoch,
                hash: head,
            },
        )),
        PeerKeys::OwnerChanged {
            ..
        } => Err(CoreError::invalid("The other account was recovered meanwhile.")),
    }
}

fn nonce32(n: &str) -> Result<[u8; 32], CoreError> {
    parse_nonce(n).ok_or_else(|| CoreError::storage("corrupt Circle nonce"))
}

impl Engine {
    // ---- the owner --------------------------------------------------------------------------------------------------

    pub(crate) async fn circle_invite_create(&self, draft: CircleDraft) -> Result<CircleInviteStart, CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let terms = terms_of(&draft)?;
        let nonce = crate::crypto::random_bytes::<32>()?;
        let commitment = circle::invite_commitment(&me.account, &me.owner_pub, me.epoch, &nonce)
            .ok_or_else(|| CoreError::storage("bad owner key"))?;
        let new = NewInvite {
            v: circle::version(),
            kind: terms.kind,
            commitment,
            owner_pub: me.owner_pub.clone(),
            owner_epoch: me.epoch,
        };
        let created = api_call!(&session, |api| api.circle_invite(&new)).map_err(not_available)?;
        let invite = OwnerInvite {
            id: created.id.clone(),
            terms,
            nonce: encode_nonce(&nonce),
            owner: Party {
                account: me.account.clone(),
                label: circle::shown_text(&session.email(), circle::MAX_LABEL_CHARS, false),
                owner_pub: me.owner_pub.clone(),
                epoch: me.epoch,
                entry_hash: me.entry_hash.clone(),
            },
            created_at: unix_now(),
            challenge: None,
            helper: None,
            peer: None,
            body: None,
            code: None,
            helper_sig: None,
        };
        self.with_circles(&me.slot, |s| {
            s.invites.push(invite);
            Ok(())
        })?;
        let host = session.server.as_str().trim_end_matches('/').rsplit("://").next().unwrap_or_default().to_owned();
        Ok(CircleInviteStart {
            id: created.id,
            link: format!("{LINK_SCHEME}{host}/{}", created.token),
            expires_at: created.expires_at,
        })
    }

    /// The owner's invitation: reveals once someone answered (the first answer only), and checks the acceptance.
    pub(crate) async fn circle_invite_poll(&self, id: &str) -> Result<CircleInviteProgress, CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        self.circle_invite_step(&session, &me, id).await
    }

    async fn circle_invite_step(
        &self,
        session: &Session,
        me: &Me,
        id: &str,
    ) -> Result<CircleInviteProgress, CoreError> {
        let mut local =
            self.circle_state(&me.slot)?.invites.iter().find(|i| i.id == id).cloned().ok_or(CoreError::NotFound)?;
        let status = api_call!(session, |api| api.circle_invite_status(id)).map_err(not_available)?;
        let helper_label = status.helper.clone().unwrap_or_default();
        match status.state {
            InviteState::Open => return Ok(CircleInviteProgress::Waiting),
            InviteState::Declined => return self.circle_invite_closed(me, id, CircleInviteProgress::Declined),
            InviteState::Expired => return self.circle_invite_closed(me, id, CircleInviteProgress::Expired),
            InviteState::Cancelled => return self.circle_invite_closed(me, id, CircleInviteProgress::Cancelled),
            InviteState::Active => {
                return Ok(CircleInviteProgress::Active {
                    link_id: status.link.unwrap_or_default(),
                });
            }
            InviteState::Challenged | InviteState::Revealed | InviteState::Accepted => {}
        }
        let challenge = status.challenge.clone().ok_or_else(|| CoreError::invalid("no answer yet"))?;
        challenge.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
        if let Some(pinned) = &local.challenge
            && (pinned.helper_pub != challenge.helper_pub
                || pinned.nonce != challenge.nonce
                || pinned.helper_epoch != challenge.helper_epoch)
        {
            // Never reveal for a second answer: a server could otherwise choose keys after seeing our nonce.
            if let Err(e) = api_call!(session, |api| api.circle_invite_cancel(id)) {
                log::warn!("a Circle invitation whose answer changed could not be cancelled: {e:?}");
            }
            self.circle_invite_closed(me, id, CircleInviteProgress::Cancelled)?;
            return Err(CoreError::invalid("The answer to this invitation changed; it was cancelled. Invite again."));
        }
        let helper_account = status.helper_account.clone().ok_or_else(|| CoreError::invalid("no helper"))?;
        if helper_account == me.account || challenge.helper_pub == me.owner_pub {
            return Err(CoreError::invalid("You can't share control with yourself."));
        }
        if local.body.is_none() {
            // The helper's roster, from its first entry, with the challenged key at the challenged epoch.
            let roster = api_call!(session, |api| api.circle_peer_roster(id)).map_err(not_available)?;
            let (anchor_hash, peer) = anchored_peer(&roster.entries, &challenge.helper_pub, challenge.helper_epoch)?;
            let helper = Party {
                account: helper_account.clone(),
                label: circle::shown_text(&helper_label, circle::MAX_LABEL_CHARS, false),
                owner_pub: challenge.helper_pub.clone(),
                epoch: challenge.helper_epoch,
                entry_hash: anchor_hash,
            };
            let arrangement = Arrangement {
                v: circle::version(),
                id: new_id()?,
                revision: 1,
                owner: local.owner.clone(),
                helper: helper.clone(),
                terms: local.terms.clone(),
                created_at: unix_now(),
            };
            let body = circle::arrangement_body(&arrangement).map_err(|e| CoreError::invalid(e.to_string()))?;
            let sealed_terms = sealed_to(
                &peer.box_pub,
                &serde_json::to_vec(&SealedTerms {
                    v: circle::version(),
                    body: body.clone(),
                })
                .map_err(|e| CoreError::storage(e.to_string()))?,
            )?;
            let code = InviteTranscript {
                invite: id,
                owner_account: &local.owner.account,
                owner_pub: &local.owner.owner_pub,
                owner_epoch: local.owner.epoch,
                helper_account: &helper_account,
                helper_pub: &challenge.helper_pub,
                helper_epoch: challenge.helper_epoch,
                owner_nonce: nonce32(&local.nonce)?,
                helper_nonce: nonce32(&challenge.nonce)?,
            }
            .code()
            .map_err(|e| CoreError::invalid(e.to_string()))?;
            // Pinned before the reveal leaves: a retry reveals the same thing for the same answer.
            local.challenge = Some(challenge.clone());
            local.helper = Some(helper);
            local.peer = Some(peer);
            local.body = Some(body);
            local.code = Some(code);
            self.circle_put_invite(me, &local)?;
            if status.state == InviteState::Challenged {
                let reveal = InviteReveal {
                    v: circle::version(),
                    nonce: local.nonce.clone(),
                    sealed_terms,
                };
                match api_call!(session, |api| api.circle_reveal(id, &reveal)) {
                    Ok(())
                    | Err(ApiFailure::Status {
                        status: 409,
                        ..
                    }) => {}
                    Err(e) => return Err(e.into_core()),
                }
            }
        }
        let code = local.code.clone().unwrap_or_default();
        if status.state != InviteState::Accepted {
            return Ok(CircleInviteProgress::Compare {
                code,
                helper: helper_label,
            });
        }
        if local.helper_sig.is_none() {
            let accept = status.accept.as_ref().ok_or_else(|| CoreError::invalid("no acceptance"))?;
            let raw = decode_payload(&accept.sealed).ok_or_else(|| CoreError::invalid("bad acceptance"))?;
            let plain = self.box_key()?.open(&raw).map_err(|_| CoreError::invalid("The acceptance doesn't open."))?;
            let signed: SignedArrangement =
                serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("bad acceptance"))?;
            if Some(&signed.body) != local.body.as_ref() {
                return Err(CoreError::invalid("They accepted different terms; it can't be confirmed."));
            }
            circle::verify_arrangement(&signed, false, verify).map_err(|e| CoreError::invalid(e.to_string()))?;
            local.helper_sig = Some(signed.helper_sig);
            self.circle_put_invite(me, &local)?;
        }
        Ok(CircleInviteProgress::Accepted {
            code,
            helper: helper_label,
        })
    }

    fn circle_put_invite(&self, me: &Me, invite: &OwnerInvite) -> Result<(), CoreError> {
        self.with_circles(&me.slot, |s| {
            if let Some(i) = s.invites.iter_mut().find(|i| i.id == invite.id) {
                *i = invite.clone();
            }
            Ok(())
        })
    }

    fn circle_invite_closed(
        &self,
        me: &Me,
        id: &str,
        progress: CircleInviteProgress,
    ) -> Result<CircleInviteProgress, CoreError> {
        self.with_circles(&me.slot, |s| {
            s.invites.retain(|i| i.id != id);
            Ok(())
        })?;
        Ok(progress)
    }

    /// The owner confirms (after the code; the app asks for the hold and biometric first): signs and activates.
    pub(crate) async fn circle_invite_confirm(&self, id: &str) -> Result<CirclePerson, CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        match self.circle_invite_step(&session, &me, id).await? {
            CircleInviteProgress::Accepted {
                ..
            } => {}
            _ => return Err(CoreError::invalid("They haven't accepted yet.")),
        }
        let local =
            self.circle_state(&me.slot)?.invites.iter().find(|i| i.id == id).cloned().ok_or(CoreError::NotFound)?;
        let (Some(body), Some(helper_sig), Some(peer), Some(helper)) =
            (local.body.clone(), local.helper_sig.clone(), local.peer.clone(), local.helper.clone())
        else {
            return Err(CoreError::invalid("This invitation isn't ready to confirm."));
        };
        let owner_msg = circle::owner_message(&body, &helper_sig).map_err(|e| CoreError::invalid(e.to_string()))?;
        let owner_sig = self.circle_owner_key(&me.slot)?.sign_b64(&owner_msg);
        let signed = SignedArrangement {
            body,
            helper_sig,
            owner_sig,
        };
        let arrangement =
            circle::verify_arrangement(&signed, true, verify).map_err(|e| CoreError::invalid(e.to_string()))?;
        let copy = serde_json::to_vec(&signed).map_err(|e| CoreError::storage(e.to_string()))?;
        let activate = InviteActivate {
            v: circle::version(),
            link: arrangement.id.clone(),
            owner_copy: sealed_to(&me.credential_pub, &copy)?,
            helper_copy: sealed_to(&peer.credential_pub, &copy)?,
        };
        let held = Held {
            arrangement: arrangement.clone(),
            side: Side::Owner,
            standing: Standing::Active,
            anchor_epoch: peer.epoch,
            anchor_hash: peer.hash.clone(),
            seen_epoch: peer.epoch.max(helper.epoch),
            peer: Some(peer),
            last_seq: 0,
            since: unix_now(),
            signed,
        };
        // Kept before the server hears: a retry after a lost answer finds it.
        self.with_circles(&me.slot, |s| {
            s.held.retain(|h| h.id() != arrangement.id);
            s.held.push(held.clone());
            Ok(())
        })?;
        match api_call!(&session, |api| api.circle_activate(id, &activate)) {
            Ok(()) => {}
            Err(ApiFailure::Status {
                status: 409,
                ..
            }) if matches!(
                api_call!(&session, |api| api.circle_invite_status(id)).map(|s| s.state),
                Ok(InviteState::Active)
            ) => {}
            Err(e) => {
                self.with_circles(&me.slot, |s| {
                    s.held.retain(|h| h.id() != arrangement.id);
                    Ok(())
                })?;
                return Err(e.into_core());
            }
        }
        self.with_circles(&me.slot, |s| {
            s.invites.retain(|i| i.id != id);
            Ok(())
        })?;
        self.notifier.circle_changed(super::CircleEvent::Changed);
        Ok(held.person())
    }

    pub(crate) async fn circle_invite_cancel(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        match api_call!(&session, |api| api.circle_invite_cancel(id)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        self.circle_invite_closed(&me, id, CircleInviteProgress::Cancelled).map(drop)
    }

    // ---- the helper -------------------------------------------------------------------------------------------------

    /// Opens a link: shows who invites, and answers with this account's key and a fresh nonce.
    pub(crate) async fn circle_join_open(&self, link: &str) -> Result<CircleInviteView, CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let token = token_of(link, session.server.as_str())?;
        let opened = api_call!(&session, |api| api.circle_join(&token)).map_err(not_available)?;
        if opened.inviter_account == me.account || opened.owner_pub == me.owner_pub {
            return Err(CoreError::invalid("This is your own invitation."));
        }
        if let Some(existing) = self.circle_state(&me.slot)?.joins.iter().find(|j| j.id == opened.id).cloned() {
            return Ok(Self::circle_join_view(&existing));
        }
        let nonce = crate::crypto::random_bytes::<32>()?;
        let join = HelperJoin {
            id: opened.id.clone(),
            opened: opened.clone(),
            nonce: encode_nonce(&nonce),
            me: Party {
                account: me.account.clone(),
                label: circle::shown_text(&session.email(), circle::MAX_LABEL_CHARS, false),
                owner_pub: me.owner_pub.clone(),
                epoch: me.epoch,
                entry_hash: me.entry_hash.clone(),
            },
            body: None,
            code: None,
            peer: None,
            link: None,
            accepted: false,
            ended: None,
        };
        // Kept before the challenge leaves: the nonce must outlive a lost answer.
        self.with_circles(&me.slot, |s| {
            s.joins.retain(|j| j.id != join.id);
            s.joins.push(join.clone());
            Ok(())
        })?;
        let challenge = InviteChallenge {
            v: circle::version(),
            token,
            helper_pub: me.owner_pub.clone(),
            helper_epoch: me.epoch,
            nonce: join.nonce.clone(),
        };
        api_call!(&session, |api| api.circle_challenge(&opened.id, &challenge)).map_err(|e| match e {
            ApiFailure::Status {
                status: 409,
                ..
            } => CoreError::invalid("Someone else already answered this invitation."),
            other => not_available(other),
        })?;
        Ok(Self::circle_join_view(&join))
    }

    fn circle_join_view(j: &HelperJoin) -> CircleInviteView {
        let terms = j.body.as_deref().and_then(|b| serde_json::from_str::<Arrangement>(b).ok()).map(|a| a.terms);
        let stage = match (&j.ended, j.accepted, &j.code) {
            (Some(stage), ..) => *stage,
            (None, true, _) => CircleJoinStage::Accepted,
            (None, false, Some(_)) => CircleJoinStage::Compare,
            (None, false, None) => CircleJoinStage::Opened,
        };
        CircleInviteView {
            id: j.id.clone(),
            inviter: j.opened.inviter.clone(),
            role: j.opened.kind.into(),
            scope_line: terms.as_ref().map(|t| t.scope_line.clone()).unwrap_or_default(),
            deferred: terms.as_ref().is_some_and(|t| t.class == Class::Deferred),
            window_secs: terms.as_ref().map_or(0, |t| t.window_secs),
            full_cards: terms.as_ref().is_some_and(|t| t.redaction == Redaction::Full),
            code: j.code.clone(),
            stage,
        }
    }

    /// The helper's side: once the inviter revealed, checks its nonce against its commitment, opens and checks the
    /// terms, verifies its roster, and shows the code.
    pub(crate) async fn circle_join_poll(&self, id: &str) -> Result<CircleInviteView, CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        self.circle_join_step(&session, &me, id).await
    }

    async fn circle_join_step(&self, session: &Session, me: &Me, id: &str) -> Result<CircleInviteView, CoreError> {
        let mut join =
            self.circle_state(&me.slot)?.joins.iter().find(|j| j.id == id).cloned().ok_or(CoreError::NotFound)?;
        if join.ended.is_some() {
            return Ok(Self::circle_join_view(&join));
        }
        let status = api_call!(session, |api| api.circle_helper_status(id)).map_err(not_available)?;
        let ended = match status.state {
            InviteState::Declined => Some(CircleJoinStage::Declined),
            InviteState::Expired | InviteState::Cancelled => Some(CircleJoinStage::Expired),
            _ => None,
        };
        if let Some(stage) = ended {
            join.ended = Some(stage);
            self.circle_put_join(me, &join)?;
            return Ok(Self::circle_join_view(&join));
        }
        if status.state == InviteState::Active {
            self.circle_links_sync(session, me).await?;
            let active = join.link.as_deref().and_then(|l| {
                self.circle_state(&me.slot).ok()?.held(l).filter(|h| h.standing == Standing::Active).map(|_| ())
            });
            if active.is_some() {
                join.ended = Some(CircleJoinStage::Active);
                self.circle_put_join(me, &join)?;
            }
            return Ok(Self::circle_join_view(&join));
        }
        if join.body.is_none()
            && let Some(reveal) = status.reveal.as_ref()
        {
            reveal.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            let owner_nonce = nonce32(&reveal.nonce)?;
            let o = &join.opened;
            if !circle::check_invite_commitment(
                &o.commitment,
                &o.inviter_account,
                &o.owner_pub,
                o.owner_epoch,
                &owner_nonce,
            ) {
                return Err(CoreError::invalid("The inviter's answer doesn't match its invitation."));
            }
            let raw = decode_payload(&reveal.sealed_terms).ok_or_else(|| CoreError::invalid("bad terms"))?;
            let plain = self.box_key()?.open(&raw).map_err(|_| CoreError::invalid("The terms don't open here."))?;
            let sealed: SealedTerms = serde_json::from_slice(&plain).map_err(|_| CoreError::invalid("bad terms"))?;
            let a: Arrangement = serde_json::from_str(&sealed.body).map_err(|_| CoreError::invalid("bad terms"))?;
            a.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
            // The terms name exactly the two keys the code is about to pin, and this account as it answered.
            if a.owner.account != o.inviter_account
                || a.owner.owner_pub != o.owner_pub
                || a.owner.epoch != o.owner_epoch
                || a.helper != join.me
                || a.terms.kind != o.kind
            {
                return Err(CoreError::invalid("The terms don't match this invitation."));
            }
            let roster = api_call!(session, |api| api.circle_peer_roster(id)).map_err(not_available)?;
            let (anchor_hash, peer) = anchored_peer(&roster.entries, &o.owner_pub, o.owner_epoch)?;
            if anchor_hash != a.owner.entry_hash {
                return Err(CoreError::invalid("The inviter's roster doesn't match the terms."));
            }
            let code = InviteTranscript {
                invite: id,
                owner_account: &o.inviter_account,
                owner_pub: &o.owner_pub,
                owner_epoch: o.owner_epoch,
                helper_account: &join.me.account,
                helper_pub: &join.me.owner_pub,
                helper_epoch: join.me.epoch,
                owner_nonce,
                helper_nonce: nonce32(&join.nonce)?,
            }
            .code()
            .map_err(|e| CoreError::invalid(e.to_string()))?;
            join.link = Some(a.id.clone());
            join.body = Some(sealed.body);
            join.peer = Some(peer);
            join.code = Some(code);
            self.circle_put_join(me, &join)?;
        }
        Ok(Self::circle_join_view(&join))
    }

    fn circle_put_join(&self, me: &Me, join: &HelperJoin) -> Result<(), CoreError> {
        self.with_circles(&me.slot, |s| {
            if let Some(j) = s.joins.iter_mut().find(|j| j.id == join.id) {
                *j = join.clone();
            }
            Ok(())
        })
    }

    /// The helper accepts (after comparing the code): signs the terms with its owner key.
    pub(crate) async fn circle_join_accept(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        let view = self.circle_join_step(&session, &me, id).await?;
        if view.stage != CircleJoinStage::Compare && view.stage != CircleJoinStage::Accepted {
            return Err(CoreError::invalid("Wait for the code first."));
        }
        let mut join =
            self.circle_state(&me.slot)?.joins.iter().find(|j| j.id == id).cloned().ok_or(CoreError::NotFound)?;
        let (Some(body), Some(peer)) = (join.body.clone(), join.peer.clone()) else {
            return Err(CoreError::invalid("Wait for the code first."));
        };
        let arrangement: Arrangement = serde_json::from_str(&body).map_err(|_| CoreError::storage("bad terms"))?;
        let helper_sig = self.circle_owner_key(&me.slot)?.sign_b64(&circle::helper_message(&body));
        let signed = SignedArrangement {
            body,
            helper_sig,
            owner_sig: String::new(),
        };
        let accept = InviteAccept {
            v: circle::version(),
            sealed: sealed_to(
                &peer.box_pub,
                &serde_json::to_vec(&signed).map_err(|e| CoreError::storage(e.to_string()))?,
            )?,
        };
        let held = Held {
            arrangement: arrangement.clone(),
            side: Side::Helper,
            standing: Standing::Pending,
            anchor_epoch: peer.epoch,
            anchor_hash: peer.hash.clone(),
            seen_epoch: peer.epoch,
            peer: Some(peer),
            last_seq: 0,
            since: unix_now(),
            signed,
        };
        self.with_circles(&me.slot, |s| {
            s.held.retain(|h| h.id() != arrangement.id);
            s.held.push(held);
            Ok(())
        })?;
        match api_call!(&session, |api| api.circle_accept(id, &accept)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 409,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        join.accepted = true;
        self.circle_put_join(&me, &join)?;
        Ok(())
    }

    pub(crate) async fn circle_join_decline(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let me = self.circle_me(&session).await?;
        match api_call!(&session, |api| api.circle_decline(id)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404 | 409,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        let link = self.circle_state(&me.slot)?.joins.iter().find(|j| j.id == id).and_then(|j| j.link.clone());
        self.with_circles(&me.slot, |s| {
            s.joins.retain(|j| j.id != id);
            if let Some(link) = &link {
                s.held.retain(|h| !(h.id() == link && h.standing == Standing::Pending));
            }
            Ok(())
        })
    }

    /// On a sync: open invitations move on without the app open (a reveal is safe without the user: the code is
    /// what they compare).
    pub(crate) async fn circle_invites_poll(&self, session: &Session, me: &Me) {
        let state = match self.circle_state(&me.slot) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Circle state: {e}");
                return;
            }
        };
        let mut moved = false;
        for invite in &state.invites {
            if let Err(e) = self.circle_invite_step(session, me, &invite.id).await {
                log::info!("a Circle invitation did not move: {e}");
            } else {
                moved = true;
            }
        }
        for join in state.joins.iter().filter(|j| j.ended.is_none()) {
            if let Err(e) = self.circle_join_step(session, me, &join.id).await {
                log::info!("a Circle invitation did not move: {e}");
            } else {
                moved = true;
            }
        }
        if moved {
            self.notifier.circle_changed(super::CircleEvent::Changed);
        }
        // Finished joins are kept a day, then dropped.
        let now = unix_now();
        if let Err(e) = self.with_circles(&me.slot, |s| {
            s.joins.retain(|j| j.ended.is_none() || j.opened.expires_at > now - 86_400);
            Ok(())
        }) {
            log::warn!("Circle state: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circles::CircleRole;

    #[test]
    fn links_carry_their_server_and_a_secret() {
        let token = "A".repeat(43);
        assert_eq!(token_of(&format!("reins-circle://reins.site/{token}"), "https://reins.site").unwrap(), token);
        assert_eq!(token_of(&token, "https://reins.site").unwrap(), token);
        assert!(token_of(&format!("reins-circle://evil.example/{token}"), "https://reins.site").is_err());
        assert!(token_of("reins-circle://reins.site/short", "https://reins.site").is_err());
    }

    #[test]
    fn drafts_make_valid_terms() {
        let draft = |role, preset, deferred| CircleDraft {
            role,
            preset,
            deferred,
            window_secs: None,
            fall_back_to_me: true,
            full_cards: true,
        };
        let t = terms_of(&draft(CircleRole::Cosign, CircleScopePreset::Money, false)).unwrap();
        assert_eq!(t.window_secs, circle::INSTANT_DEFAULT_SECS);
        assert_eq!(t.on_expiry, OnExpiry::Deny, "only a delegate falls back to the owner");
        assert_eq!(t.scope.effects, ["money"]);
        let t = terms_of(&draft(CircleRole::Delegate, CircleScopePreset::Routine, true)).unwrap();
        assert_eq!(t.on_expiry, OnExpiry::Owner);
        assert_eq!(t.scope.max_level, Some(1));
        let t = terms_of(&draft(CircleRole::Watcher, CircleScopePreset::Everything, true)).unwrap();
        assert_eq!(t.redaction, Redaction::Summary, "a watcher sees summaries");
        let t = terms_of(&draft(
            CircleRole::Guardian,
            CircleScopePreset::Agent {
                connection_id: "c1".into(),
                label: "Chat\u{202e}GPT".into(),
            },
            true,
        ))
        .unwrap();
        assert_eq!(t.scope_line, "Everything ChatGPT does");
    }
}
