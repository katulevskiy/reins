//! The server's Circle endpoints (`/reins/api/circles/*`).

use reins_proto::{
    circle::{
        AnswerItem, GuardianNotice, InboxItem, InviteAccept, InviteActivate, InviteChallenge, InviteCreated,
        InviteOpened, InviteReveal, InviteStatus, JoinBody, LinkView, NewAnswer, NewInvite, NewPacket, Seen,
        SignedChange,
    },
    roster::Roster,
};
use reqwest::Method;

use crate::phone_api::{ApiFailure, PhoneApi, check_id};

impl PhoneApi<'_> {
    pub(crate) async fn circle_invite(&self, new: &NewInvite) -> Result<InviteCreated, ApiFailure> {
        Self::json(self.request(Method::POST, "/circles/invites").json(new)).await
    }

    pub(crate) async fn circle_invite_status(&self, id: &str) -> Result<InviteStatus, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::GET, &format!("/circles/invites/{id}"))).await
    }

    pub(crate) async fn circle_invite_cancel(&self, id: &str) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::DELETE, &format!("/circles/invites/{id}"))).await.map(drop)
    }

    pub(crate) async fn circle_join(&self, token: &str) -> Result<InviteOpened, ApiFailure> {
        Self::json(self.request(Method::POST, "/circles/join").json(&JoinBody {
            token: token.to_owned(),
        }))
        .await
    }

    pub(crate) async fn circle_challenge(&self, id: &str, challenge: &InviteChallenge) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/invites/{id}/challenge")).json(challenge))
            .await
            .map(drop)
    }

    pub(crate) async fn circle_helper_status(&self, id: &str) -> Result<InviteStatus, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::GET, &format!("/circles/invites/{id}/helper"))).await
    }

    pub(crate) async fn circle_reveal(&self, id: &str, reveal: &InviteReveal) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/invites/{id}/reveal")).json(reveal)).await.map(drop)
    }

    pub(crate) async fn circle_accept(&self, id: &str, accept: &InviteAccept) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/invites/{id}/accept")).json(accept)).await.map(drop)
    }

    pub(crate) async fn circle_decline(&self, id: &str) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/invites/{id}/decline"))).await.map(drop)
    }

    pub(crate) async fn circle_activate(&self, id: &str, activate: &InviteActivate) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/invites/{id}/activate")).json(activate))
            .await
            .map(drop)
    }

    pub(crate) async fn circle_peer_roster(&self, id: &str) -> Result<Roster, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::GET, &format!("/circles/peers/{id}/roster"))).await
    }

    pub(crate) async fn circle_links(&self) -> Result<Vec<LinkView>, ApiFailure> {
        Self::json(self.request(Method::GET, "/circles/links")).await
    }

    /// This account's new notices as a guardian (each returned once).
    pub(crate) async fn guardian_notices(&self) -> Result<Vec<GuardianNotice>, ApiFailure> {
        Self::json(self.request(Method::GET, "/guardian/notices")).await
    }

    pub(crate) async fn circle_change(&self, id: &str, change: &SignedChange) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/links/{id}/change")).json(change)).await.map(drop)
    }

    pub(crate) async fn circle_packet(&self, packet: &NewPacket) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/circles/packets").json(packet)).await.map(drop)
    }

    pub(crate) async fn circle_withdraw(&self, id: &str) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::DELETE, &format!("/circles/packets/{id}"))).await.map(drop)
    }

    pub(crate) async fn circle_inbox(&self) -> Result<Vec<InboxItem>, ApiFailure> {
        Self::json(self.request(Method::GET, "/circles/inbox")).await
    }

    pub(crate) async fn circle_answer(&self, id: &str, answer: &NewAnswer) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/circles/packets/{id}/answer")).json(answer)).await.map(drop)
    }

    pub(crate) async fn circle_answers(&self) -> Result<Vec<AnswerItem>, ApiFailure> {
        Self::json(self.request(Method::GET, "/circles/answers")).await
    }

    pub(crate) async fn circle_seen(&self, ids: Vec<String>) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/circles/answers/seen").json(&Seen {
            ids,
        }))
        .await
        .map(drop)
    }
}
