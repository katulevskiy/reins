//! The server's phone-roles endpoints (`/reins/api`, [`reins_proto::roster`], [`reins_proto::enroll`]).

use reins_proto::enroll::{EnrollChallenge, EnrollReveal};
use reins_proto::roster::{
    HandoverAnswer, HandoverBundle, HandoverCreated, HandoverRequest, HandoverState, MemberRegistration, NewHandover,
    RecoveryWrap, RecoveryWrapRequest, Roster, RosterAppend, RosterAppended,
};
use reqwest::Method;
use serde::Serialize;

use crate::phone_api::{ApiFailure, PhoneApi, check_id};

impl PhoneApi<'_> {
    /// `GET /roster`: the chain, oldest first (empty for an account without one).
    pub(crate) async fn roster(&self) -> Result<Roster, ApiFailure> {
        Self::json(self.request(Method::GET, "/roster")).await
    }

    /// `POST /roster`.
    pub(crate) async fn append_roster(&self, append: &RosterAppend) -> Result<RosterAppended, ApiFailure> {
        Self::json(self.request(Method::POST, "/roster").json(append)).await
    }

    /// `POST /roster/recovery-wrap`: the wrap the latest entry names, for the master password hash.
    pub(crate) async fn recovery_wrap(&self, request: &RecoveryWrapRequest) -> Result<RecoveryWrap, ApiFailure> {
        Self::json(self.request(Method::POST, "/roster/recovery-wrap").json(request)).await
    }

    /// `PUT /member`: a viewer's push token.
    pub(crate) async fn put_member(&self, registration: &MemberRegistration) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/member").json(registration)).await.map(drop)
    }

    /// `POST /handovers` (a viewer: "Decide on this phone").
    pub(crate) async fn create_handover(
        &self,
        signed: reins_proto::roster::HandoverSignature,
    ) -> Result<HandoverCreated, ApiFailure> {
        let new = NewHandover {
            v: reins_proto::PROTOCOL_VERSION,
            signed: Some(signed),
        };
        Self::json(self.request(Method::POST, "/handovers").json(&new)).await
    }

    /// `GET /handovers/<id>` as the viewer that asked.
    pub(crate) async fn handover_state(&self, id: &str) -> Result<HandoverState, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::GET, &format!("/handovers/{id}"))).await
    }

    /// `GET /handovers/<id>` as the primary.
    pub(crate) async fn handover_request(&self, id: &str) -> Result<HandoverRequest, ApiFailure> {
        check_id(id)?;
        let request: HandoverRequest = Self::json(self.request(Method::GET, &format!("/handovers/{id}"))).await?;
        if request.id != id {
            return Err(crate::CoreError::invalid("the server returned a different request").into());
        }
        Ok(request)
    }

    /// `POST /handovers/<id>/response` (the primary declines).
    pub(crate) async fn deny_handover(&self, id: &str) -> Result<(), ApiFailure> {
        check_id(id)?;
        let answer = HandoverAnswer {
            v: reins_proto::PROTOCOL_VERSION,
            approve: false,
        };
        Self::send(self.request(Method::POST, &format!("/handovers/{id}/response")).json(&answer)).await.map(drop)
    }

    /// `GET /handover-bundle` (the new primary).
    pub(crate) async fn handover_bundle(&self) -> Result<HandoverBundle, ApiFailure> {
        Self::json(self.request(Method::GET, "/handover-bundle")).await
    }

    /// `DELETE /handover-bundle` (the new primary, once it kept what it carried).
    pub(crate) async fn drop_handover_bundle(&self) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::DELETE, "/handover-bundle")).await.map(drop)
    }

    /// `POST /joins/<id>/challenge` (the primary, enrollment step 2).
    pub(crate) async fn enroll_challenge(&self, id: &str, challenge: &EnrollChallenge) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/joins/{id}/challenge")).json(challenge)).await.map(drop)
    }

    /// `POST /joins/<id>/reveal` (the joining phone, enrollment step 3).
    pub(crate) async fn enroll_reveal(&self, id: &str, reveal: &EnrollReveal) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/joins/{id}/reveal")).json(reveal)).await.map(drop)
    }

    /// `POST /joins/<id>/response` with any body (a signed answer, on an account with a roster).
    pub(crate) async fn answer_join_body(&self, id: &str, body: &impl Serialize) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/joins/{id}/response")).json(body)).await.map(drop)
    }

    /// A4 `POST /requests/<id>/response` with any body (a signed answer).
    pub(crate) async fn respond_body(&self, id: &str, body: &impl Serialize) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/requests/{id}/response")).json(body)).await.map(drop)
    }

    /// A6 `POST /pairings/<id>/response` with any body (a signed answer).
    pub(crate) async fn answer_pairing_body(
        &self,
        id: &str,
        body: &impl Serialize,
    ) -> Result<reins_proto::device::PairingResult, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::POST, &format!("/pairings/{id}/response")).json(body)).await
    }
}
