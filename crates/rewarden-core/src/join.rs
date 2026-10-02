//! "Add another phone" ([`rewarden_proto::join`]), both sides.
//!
//! The new phone (signed in, keys `Locked`): [`Engine::join_begin`] makes an X25519 key pair, kept in the store until
//! the request ends, and asks the server; the app shows [`JoinStart::code`] and calls [`Engine::join_poll`] every few
//! seconds until it is no longer `Waiting`. Approved, the phone opens the sealed secret, checks that it answers this
//! request and opens the account's keys, and keeps it like the phone that made the account.
//!
//! The approval device: the request arrives like a pairing (push `join`, or the pending long-poll) and is parked as a
//! [`PendingKind::Join`] item; [`Engine::join_view`] shows the device's name and the same code, computed here from the
//! key the server relayed; [`Engine::answer_join`] (after biometrics) seals the account secret to that key.

use data_encoding::BASE64URL_NOPAD;
use reqwest::Method;
use rewarden_proto::PROTOCOL_VERSION;
use rewarden_proto::join::{
    JoinAnswer, JoinCreated, JoinRequest, JoinState, JoinStatus, NewJoin, SealedSecret, join_code,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;
use crate::engine::Engine;
use crate::phone_api::{ApiFailure, PhoneApi, check_id};
use crate::session::api_call;
use crate::sso::AccountSecret;
use crate::store::unix_now;
use crate::types::{JoinProgress, JoinStart, JoinView, PendingItem, PendingKind};

/// Where the new phone keeps its open request (store secrets: service, account).
const JOIN_SERVICE: &str = "reins.join";
const JOIN_ACCOUNT: &str = "open";

#[derive(Serialize, Deserialize)]
struct OpenJoin {
    id: String,
    /// The X25519 secret key, base64url.
    key: String,
}

impl PhoneApi<'_> {
    async fn create_join(&self, new: &NewJoin) -> Result<JoinCreated, ApiFailure> {
        Self::json(self.request(Method::POST, "/joins").json(new)).await
    }

    async fn join_state(&self, id: &str) -> Result<JoinState, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::GET, &format!("/joins/{id}"))).await
    }

    async fn get_join(&self, id: &str) -> Result<JoinRequest, ApiFailure> {
        check_id(id)?;
        let join: JoinRequest = Self::json(self.request(Method::GET, &format!("/joins/{id}"))).await?;
        if join.id != id {
            return Err(CoreError::invalid("the server returned a different request").into());
        }
        Ok(join)
    }

    async fn answer_join(&self, id: &str, answer: &JoinAnswer) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/joins/{id}/response")).json(answer)).await.map(drop)
    }
}

pub(crate) fn join_item(j: &JoinRequest) -> PendingItem {
    PendingItem {
        kind: PendingKind::Join,
        id: j.id.clone(),
        title: format!("Add {} to your account?", j.device_name),
        subtitle: "Another phone asks for this account's keys".to_owned(),
        created_at: j.created_at,
        connection_id: String::new(),
        connection_label: j.device_name.clone(),
        action: "join".to_owned(),
        count: 1,
        service: String::new(),
        account: None,
        wait_until: None,
        op: String::new(),
        op_title: String::new(),
        suggestion: None,
    }
}

fn code_of(public_key: &str) -> Result<String, CoreError> {
    join_code(public_key).ok_or_else(|| CoreError::invalid("the request carries no valid key"))
}

impl Engine {
    // ---- the new phone ---------------------------------------------------------------

    /// Asks the account's approval device for the account secret. Show `code` and ask the user to compare it with
    /// the one the other phone shows; then call [`Engine::join_poll`].
    pub async fn join_begin(&self, device_name: &str) -> Result<JoinStart, CoreError> {
        let session = self.session()?;
        let secret = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let public_key = BASE64URL_NOPAD.encode(secret.public_key().as_bytes());
        let new = NewJoin {
            v: PROTOCOL_VERSION,
            device_name: device_name.trim().to_owned(),
            public_key: public_key.clone(),
        };
        let created = api_call!(&session, |api| api.create_join(&new)).map_err(|e| match e {
            ApiFailure::Status {
                status: 409,
                ..
            } => CoreError::invalid("No other phone approves for this account yet. Use the recovery code."),
            ApiFailure::Status {
                status: 400,
                message,
                ..
            } if message.contains("approval device") => {
                CoreError::invalid("This phone already approves for the account.")
            }
            other => other.into_core(),
        })?;
        let open = OpenJoin {
            id: created.id.clone(),
            key: BASE64URL_NOPAD.encode(&secret.to_bytes()),
        };
        let raw = Zeroizing::new(serde_json::to_vec(&open).map_err(|e| CoreError::storage(e.to_string()))?);
        self.store.secret_put(JOIN_SERVICE, JOIN_ACCOUNT, &raw)?;
        Ok(JoinStart {
            id: created.id,
            code: code_of(&public_key)?,
            expires_at: created.expires_at,
        })
    }

    /// Where the request stands. `Joined`: the account's keys are open on this phone now.
    pub async fn join_poll(&self) -> Result<JoinProgress, CoreError> {
        let session = self.session()?;
        let raw = self.store.secret_get(JOIN_SERVICE, JOIN_ACCOUNT)?.ok_or(CoreError::NotFound)?;
        let open: OpenJoin =
            serde_json::from_slice(&raw).map_err(|_| CoreError::storage("corrupt open join request"))?;
        let state = match api_call!(&session, |api| api.join_state(&open.id)) {
            Ok(state) => state,
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => JoinState {
                status: JoinStatus::Expired,
                sealed: None,
            },
            Err(e) => return Err(e.into_core()),
        };
        let progress = match state.status {
            JoinStatus::Waiting => return Ok(JoinProgress::Waiting),
            JoinStatus::Denied => JoinProgress::Denied,
            JoinStatus::Expired => JoinProgress::Expired,
            JoinStatus::Approved => {
                let sealed = state.sealed.ok_or_else(|| CoreError::invalid("the approval carries no secret"))?;
                let secret = open_sealed(&open, &sealed)?;
                self.adopt_secret(secret).await?;
                JoinProgress::Joined
            }
        };
        self.store.secret_delete(JOIN_SERVICE, JOIN_ACCOUNT)?;
        Ok(progress)
    }

    /// Gives up the open request (the other phone's prompt expires on its own).
    pub fn join_cancel(&self) -> Result<(), CoreError> {
        self.store.secret_delete(JOIN_SERVICE, JOIN_ACCOUNT)
    }

    // ---- the approval device ---------------------------------------------------------

    pub(crate) async fn fetch_and_park_join(&self, id: &str) -> Result<(), CoreError> {
        check_id(id)?;
        let session = self.session()?;
        let Some(_guard) = self.begin(id)? else {
            return Ok(());
        };
        match api_call!(&session, |api| api.get_join(id)) {
            Ok(join) => self.park_join(&join),
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into_core()),
        }
    }

    pub(crate) fn park_join(&self, join: &JoinRequest) -> Result<(), CoreError> {
        rewarden_proto::check_version(join.v).map_err(|e| CoreError::invalid(e.to_string()))?;
        code_of(&join.public_key)?;
        let payload = serde_json::to_vec(join).map_err(|e| CoreError::storage(e.to_string()))?;
        if self.store.park(&join.id, PendingKind::Join, join.created_at, unix_now(), &payload)? {
            self.notifier.item_pending(join_item(join));
        }
        Ok(())
    }

    fn parked_join(&self, id: &str) -> Result<JoinRequest, CoreError> {
        let row = self.store.pending_item(id, unix_now())?.ok_or(CoreError::NotFound)?;
        if row.kind != PendingKind::Join {
            return Err(CoreError::NotFound);
        }
        serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked request"))
    }

    pub fn join_view(&self, id: &str) -> Result<JoinView, CoreError> {
        let join = self.parked_join(id)?;
        Ok(JoinView {
            id: join.id.clone(),
            device_name: join.device_name.clone(),
            code: code_of(&join.public_key)?,
            created_at: join.created_at,
        })
    }

    /// Approves (seals the account secret to the new phone's key) or denies. Ask for biometrics before approving.
    pub async fn answer_join(&self, id: &str, approve: bool) -> Result<(), CoreError> {
        let session = self.session()?;
        let Some(_claim) = self.claim(id) else {
            return Err(CoreError::NotFound);
        };
        let join = self.parked_join(id)?;
        let sealed = if approve {
            let secret = self.signed_in_secret().await?.ok_or_else(|| {
                CoreError::invalid(
                    "This account was made with a master password: sign in on the new phone with it instead.",
                )
            })?;
            let key = rewarden_proto::desktop::decode_key(&join.public_key)
                .ok_or_else(|| CoreError::invalid("the request carries no valid key"))?;
            let plain = Zeroizing::new(
                serde_json::to_vec(&SealedSecret {
                    v: PROTOCOL_VERSION,
                    join_id: join.id.clone(),
                    secret: BASE64URL_NOPAD.encode(secret.as_bytes()),
                })
                .map_err(|e| CoreError::storage(e.to_string()))?,
            );
            let sealed = crypto_box::PublicKey::from(key)
                .seal(&mut crypto_box::aead::OsRng, &plain)
                .map_err(|_| CoreError::storage("the secret could not be sealed"))?;
            Some(BASE64URL_NOPAD.encode(&sealed))
        } else {
            None
        };
        let answer = JoinAnswer {
            v: PROTOCOL_VERSION,
            approve,
            sealed,
        };
        let result = api_call!(&session, |api| api.answer_join(id, &answer));
        if let Err(e) = result {
            if !matches!(
                &e,
                ApiFailure::Status {
                    status: 404 | 409,
                    ..
                }
            ) {
                return Err(e.into_core());
            }
        }
        self.store.remove_pending(id)?;
        self.store.mark_handled(id, unix_now())?;
        self.notifier.item_resolved(id.to_owned());
        Ok(())
    }
}

/// The secret in `sealed`, after checking that it answers `open`'s request.
fn open_sealed(open: &OpenJoin, sealed: &str) -> Result<AccountSecret, CoreError> {
    let bad = || CoreError::invalid("The other phone's answer could not be opened.");
    let key = Zeroizing::new(BASE64URL_NOPAD.decode(open.key.as_bytes()).map_err(|_| bad())?);
    let key: [u8; 32] = key.as_slice().try_into().map_err(|_| bad())?;
    let secret_key = crypto_box::SecretKey::from(key);
    let bytes = BASE64URL_NOPAD.decode(sealed.as_bytes()).map_err(|_| bad())?;
    let plain = Zeroizing::new(secret_key.unseal(&bytes).map_err(|_| bad())?);
    let answer: SealedSecret = serde_json::from_slice(&plain).map_err(|_| bad())?;
    if answer.join_id != open.id {
        return Err(bad());
    }
    let raw = Zeroizing::new(BASE64URL_NOPAD.decode(answer.secret.as_bytes()).map_err(|_| bad())?);
    AccountSecret::from_bytes(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed_for(public: &crypto_box::PublicKey, join_id: &str, secret: &[u8]) -> String {
        let plain = serde_json::to_vec(&SealedSecret {
            v: 1,
            join_id: join_id.to_owned(),
            secret: BASE64URL_NOPAD.encode(secret),
        })
        .unwrap();
        BASE64URL_NOPAD.encode(&public.seal(&mut crypto_box::aead::OsRng, &plain).unwrap())
    }

    #[test]
    fn sealed_secrets_open_for_their_request_only() {
        let key = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let open = OpenJoin {
            id: "j1".to_owned(),
            key: BASE64URL_NOPAD.encode(&key.to_bytes()),
        };
        let secret = [7u8; 32];
        let ok = sealed_for(&key.public_key(), "j1", &secret);
        assert_eq!(open_sealed(&open, &ok).unwrap().as_bytes(), &secret);
        // An answer to another request (a replay), or sealed to another key, is refused.
        assert!(open_sealed(&open, &sealed_for(&key.public_key(), "j2", &secret)).is_err());
        let other = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        assert!(open_sealed(&open, &sealed_for(&other.public_key(), "j1", &secret)).is_err());
        assert!(open_sealed(&open, "garbage").is_err());
    }
}
