//! Enrolling a phone as a viewer ([`reins_proto::enroll`]): commit, challenge, reveal, then six digits compared on
//! both phones before the primary signs the `enroll` entry and its answer.
//!
//! **The joining phone** ([`Engine::enroll_begin`], then [`Engine::enroll_poll`] every few seconds): it commits to a
//! fresh nonce with its box and signing keys. Once the primary's challenge arrives it checks it (fresh, strict keys),
//! keeps it (a second, different challenge is refused) and reveals its nonce, with a no-tap proof when it holds the
//! account secret already; then it shows the code. Approved, it opens the sealed grant only if the primary key the
//! code covered signed it for this join, and verifies the roster from the owner key the code covered.
//!
//! **The primary** ([`Engine::enroll_view`], then [`Engine::enroll_confirm`]): it answers the request with its
//! challenge (its own fresh nonce, signing key, the owner key and the epoch), then waits for the reveal, checks it
//! against the commitment in constant time and shows the code. The user compares; approving appends the `enroll`
//! entry and answers with a signed answer (kind `join`) whose sealed grant is itself signed (sign-then-seal).

use data_encoding::BASE64URL_NOPAD;
use reins_proto::PROTOCOL_VERSION;
use reins_proto::answer::AnswerKind;
use reins_proto::enroll::{
    EnrollChallenge, EnrollReveal, MAX_CHALLENGE_SECS, Transcript, check_commitment, check_proof, commitment,
    encode_nonce, make_proof, parse_nonce, proof_key,
};
use reins_proto::join::{JoinAnswer, JoinRequest, JoinState, JoinStatus, NewJoin};
use reins_proto::roster::{Member, RosterAppend, RosterReason};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::chain::{Standing, anchored_by_code};
use super::sealed::{self, EnrollGrant};
use super::{Account, member_name};
use crate::CoreError;
use crate::crypto::random_bytes;
use crate::engine::Engine;
use crate::phone_api::ApiFailure;
use crate::session::api_call;
use crate::sso::AccountSecret;
use crate::store::unix_now;
use crate::types::PendingKind;

/// The joining phone's open enrollment (device store, per account).
pub(super) const ENROLL: &str = "reins.enroll";
/// The primary's side of each enrollment it challenged (device store, `<account slot>:<join id>`).
pub(super) const ENROLL_PRIMARY: &str = "reins.enroll-primary";

/// This phone asked to view the account: poll with `enroll_poll` until it shows the code to compare.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrollStart {
    pub id: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum EnrollProgress {
    /// The primary has not answered yet: ask again in a few seconds.
    Waiting,
    /// Show `code` ("482 193") and ask the user to check that the primary shows the same; keep polling.
    Compare {
        code: String,
    },
    /// This phone is a viewer of the account now (with the vault open when the account has a recovery code).
    Enrolled,
    Denied,
    Expired,
}

/// A phone asking to view the account, as the primary sees it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrollView {
    pub id: String,
    /// The name the asking phone gave, as the server relayed it: not covered by the code, so show it as unverified.
    /// (The asking phone refuses to be listed under another name, so a renamed request never completes.)
    pub device_name: String,
    /// The six digits to compare, once the asking phone revealed its nonce (`None`: ask again in a moment).
    pub code: Option<String>,
    /// The asking phone proved that it held the account secret already (a phone of the account before the update):
    /// it may be accepted without comparing.
    pub proven: bool,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
struct OpenEnrollment {
    id: String,
    account: String,
    device_id: String,
    /// The phone's nonce, base64url.
    nonce: String,
    /// The challenge this phone answered (the only one it ever accepts for this join).
    #[serde(default)]
    challenge: Option<EnrollChallenge>,
    #[serde(default)]
    revealed: bool,
    /// The name this phone asked to be listed under.
    #[serde(default)]
    name: String,
}

#[derive(Serialize, Deserialize)]
struct PrimaryEnrollment {
    nonce: String,
    challenge: EnrollChallenge,
    /// The server took the challenge.
    #[serde(default)]
    posted: bool,
}

fn invalid(e: impl std::fmt::Display) -> CoreError {
    CoreError::invalid(e.to_string())
}

/// The transcript both phones compare (all keys as they travel).
#[expect(clippy::too_many_arguments, reason = "the transcript's fields, named at both call sites")]
fn transcript<'a>(
    account: &'a str,
    join_id: &'a str,
    device_id: &'a str,
    box_pub: &'a str,
    sign_pub: &'a str,
    challenge: &'a EnrollChallenge,
    joiner_nonce: [u8; 32],
    primary_nonce: [u8; 32],
) -> Transcript<'a> {
    Transcript {
        account,
        join_id,
        device_id,
        box_pub,
        sign_pub,
        primary_sign_pub: &challenge.primary_sign_pub,
        owner_pub: &challenge.owner_pub,
        epoch: challenge.epoch,
        expires_at: challenge.expires_at,
        joiner_nonce,
        primary_nonce,
    }
}

impl Engine {
    // ---- the joining phone --------------------------------------------------------------------------------------

    /// Asks the account's primary to enroll this phone as a viewer.
    pub async fn enroll_begin(&self, device_name: &str) -> Result<EnrollStart, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let device_id = self.store.device_id()?;
        let nonce = Zeroizing::new(random_bytes::<32>()?);
        let (box_pub, sign_pub) = (self.box_key()?.public_b64(), self.sign_key()?.public_b64());
        let committed = commitment(&account.id, &device_id, &box_pub, &sign_pub, &nonce)
            .ok_or_else(|| CoreError::storage("invalid key"))?;
        let new = NewJoin {
            v: PROTOCOL_VERSION,
            device_name: member_name(device_name),
            public_key: box_pub,
            sign_pub: Some(sign_pub),
            commitment: Some(committed),
        };
        let created = api_call!(&session, |api| api.create_join(&new)).map_err(|e| match e {
            ApiFailure::Status {
                status: 409,
                ..
            } => CoreError::invalid("No phone decides for this account yet. Use the recovery code."),
            ApiFailure::Status {
                status: 400,
                message,
                ..
            } if message.contains("approval device") => {
                CoreError::invalid("This phone decides for the account already.")
            }
            other => other.into_core(),
        })?;
        self.save(
            ENROLL,
            &account.slot,
            &OpenEnrollment {
                id: created.id.clone(),
                account: account.id,
                device_id,
                nonce: encode_nonce(&nonce),
                challenge: None,
                revealed: false,
                name: new.device_name.clone(),
            },
        )?;
        Ok(EnrollStart {
            id: created.id,
            expires_at: created.expires_at,
        })
    }

    /// Where the enrollment stands (see [`EnrollProgress`]).
    pub async fn enroll_poll(&self) -> Result<EnrollProgress, CoreError> {
        let session = self.session()?;
        // One poll at a time: two overlapping polls must never each accept a challenge.
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let mut open: OpenEnrollment = self.load(ENROLL, &account.slot)?.ok_or(CoreError::NotFound)?;
        if open.account != account.id {
            self.store.device_secret_delete(ENROLL, &account.slot)?;
            return Err(CoreError::NotFound);
        }
        let state = match api_call!(&session, |api| api.join_state(&open.id)) {
            Ok(state) => state,
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => JoinState {
                status: JoinStatus::Expired,
                sealed: None,
                challenge: None,
            },
            Err(e) => return Err(e.into_core()),
        };
        let progress = match self.enroll_step(&session, &account, &mut open, state).await {
            Ok(EnrollProgress::Waiting) => return Ok(EnrollProgress::Waiting),
            Ok(EnrollProgress::Compare {
                code,
            }) => {
                self.save(ENROLL, &account.slot, &open)?;
                return Ok(EnrollProgress::Compare {
                    code,
                });
            }
            Ok(done) => done,
            Err(e) => {
                // Anything the server could have made up ends the enrollment: the user starts again.
                self.store.device_secret_delete(ENROLL, &account.slot)?;
                return Err(e);
            }
        };
        self.store.device_secret_delete(ENROLL, &account.slot)?;
        Ok(progress)
    }

    async fn enroll_step(
        &self,
        session: &crate::session::Session,
        account: &Account,
        open: &mut OpenEnrollment,
        state: JoinState,
    ) -> Result<EnrollProgress, CoreError> {
        match state.status {
            JoinStatus::Denied => return Ok(EnrollProgress::Denied),
            JoinStatus::Expired => return Ok(EnrollProgress::Expired),
            JoinStatus::Waiting | JoinStatus::Approved => {}
        }
        let Some(challenge) = state.challenge else {
            if state.status == JoinStatus::Approved {
                return Err(CoreError::invalid("The other phone's answer skipped the code. Try again."));
            }
            return Ok(EnrollProgress::Waiting);
        };
        match &open.challenge {
            Some(kept) if *kept != challenge => {
                return Err(CoreError::invalid("The other phone's challenge changed. Try again."));
            }
            Some(_) => {}
            None => {
                challenge.validate(unix_now()).map_err(invalid)?;
                open.challenge = Some(challenge.clone());
                // Kept before the nonce is revealed: whatever happens to the reveal (a stalled answer, a cancelled
                // poll, the app killed), this phone never takes another challenge for this join.
                self.save(ENROLL, &account.slot, &*open)?;
            }
        }
        let nonce = parse_nonce(&open.nonce).ok_or_else(|| CoreError::storage("corrupt enrollment"))?;
        let primary_nonce = parse_nonce(&challenge.nonce).ok_or_else(|| CoreError::invalid("invalid challenge"))?;
        let (box_pub, sign_pub) = (self.box_key()?.public_b64(), self.sign_key()?.public_b64());
        let t =
            transcript(&account.id, &open.id, &open.device_id, &box_pub, &sign_pub, &challenge, nonce, primary_nonce);
        let code = t.code().map_err(invalid)?;
        if !open.revealed {
            // A phone that held the account secret before proves it, so the primary may enroll it without a tap.
            let proof = match self.secret_of(&account.id)? {
                Some(secret) => Some(make_proof(&proof_key(secret.as_bytes(), &account.id), &t).map_err(invalid)?),
                None => None,
            };
            let reveal = EnrollReveal {
                v: PROTOCOL_VERSION,
                nonce: open.nonce.clone(),
                proof,
            };
            api_call!(session, |api| api.enroll_reveal(&open.id, &reveal)).map_err(ApiFailure::into_core)?;
            open.revealed = true;
            self.save(ENROLL, &account.slot, &*open)?;
        }
        if state.status != JoinStatus::Approved {
            return Ok(EnrollProgress::Compare {
                code,
            });
        }
        let sealed_grant = state.sealed.ok_or_else(|| CoreError::invalid("The approval carries nothing."))?;
        let primary = reins_proto::desktop::decode_key(&challenge.primary_sign_pub)
            .ok_or_else(|| CoreError::invalid("invalid challenge"))?;
        let (epoch, grant) =
            sealed::open_enroll_grant(&self.box_key()?, &sealed_grant, &account.id, &open.id, &primary)?;
        let fetched = api_call!(session, |api| api.roster()).map_err(ApiFailure::into_core)?.entries;
        let roster = anchored_by_code(&fetched, challenge.epoch, &challenge.owner_pub, &challenge.primary_sign_pub)?;
        let listed = roster.at(epoch).ok_or_else(|| CoreError::invalid("The roster lacks the epoch of the answer."))?;
        if listed.hash_hex().map_err(invalid)? != grant.roster_hash
            || listed.primary.sign_pub != challenge.primary_sign_pub
        {
            return Err(CoreError::invalid("The answer names another roster entry."));
        }
        if self.standing_in(&roster)? != Standing::Viewer {
            return Err(CoreError::invalid("The roster does not list this phone."));
        }
        // The name the primary saw came through the server: the roster must name this phone as it asked.
        if !open.name.is_empty() && roster.member(&open.device_id).is_some_and(|m| m.name != open.name) {
            return Err(CoreError::invalid("The roster names this phone differently than it asked."));
        }
        let mut ring = self.keyring(&account.slot)?;
        ring.extend(&grant.view_keys)?;
        self.save_keyring(&account.slot, &ring)?;
        self.save_roster(&account.slot, &roster)?;
        if let Some(secret) = &grant.account_secret
            && self.secret_of(&account.id)?.is_none()
        {
            let raw = Zeroizing::new(
                BASE64URL_NOPAD
                    .decode(secret.as_bytes())
                    .map_err(|_| CoreError::invalid("The answer is malformed."))?,
            );
            self.adopt_secret(AccountSecret::from_bytes(&raw)?).await?;
        }
        Ok(EnrollProgress::Enrolled)
    }

    /// Gives up this phone's enrollment (the primary's prompt expires on its own).
    pub async fn enroll_cancel(&self) -> Result<(), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        self.store.device_secret_delete(ENROLL, &account.slot)
    }

    // ---- the primary --------------------------------------------------------------------------------------------

    fn parked_enrollment(&self, id: &str) -> Result<JoinRequest, CoreError> {
        let row = self.store.pending_item(id, unix_now())?.ok_or(CoreError::NotFound)?;
        if row.kind != PendingKind::Join {
            return Err(CoreError::NotFound);
        }
        let join: JoinRequest =
            serde_json::from_slice(&row.payload).map_err(|_| CoreError::storage("corrupt parked request"))?;
        if join.sign_pub.is_none() || join.commitment.is_none() || join.device_id.is_none() {
            return Err(CoreError::invalid("This phone asks for the account's keys, not to view it."));
        }
        Ok(join)
    }

    /// A phone asking to view the account: challenges it the first time, then shows the code once it revealed its
    /// nonce. Call again until `code` is there.
    pub async fn enroll_view(&self, id: &str) -> Result<EnrollView, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let mut join = self.parked_enrollment(id)?;
        let (roster, seed) = self.as_primary(&session, &account).await?;
        let key = format!("{}:{id}", account.slot);
        let mut mine = if let Some(mine) = self.load::<PrimaryEnrollment>(ENROLL_PRIMARY, &key)? {
            mine
        } else {
            let nonce = Zeroizing::new(random_bytes::<32>()?);
            let challenge = EnrollChallenge {
                v: PROTOCOL_VERSION,
                nonce: encode_nonce(&nonce),
                primary_sign_pub: self.sign_key()?.public_b64(),
                owner_pub: seed.owner_key()?.public_b64(),
                epoch: roster.epoch(),
                expires_at: unix_now() + MAX_CHALLENGE_SECS,
            };
            let mine = PrimaryEnrollment {
                nonce: challenge.nonce.clone(),
                challenge,
                posted: false,
            };
            // Kept first: a challenge the server took must never be one this phone forgot.
            self.save(ENROLL_PRIMARY, &key, &mine)?;
            mine
        };
        if !mine.posted {
            match api_call!(&session, |api| api.enroll_challenge(id, &mine.challenge)) {
                // 409: the server holds a challenge already (this one, whose answer was lost, or none of this phone's:
                // then the codes differ and the user declines).
                Ok(())
                | Err(ApiFailure::Status {
                    status: 409,
                    ..
                }) => {}
                Err(e) => return Err(e.into_core()),
            }
            mine.posted = true;
            self.save(ENROLL_PRIMARY, &key, &mine)?;
        }
        if join.reveal.is_none() {
            let fresh = api_call!(&session, |api| api.get_join(id)).map_err(ApiFailure::into_core)?;
            if fresh.sign_pub != join.sign_pub
                || fresh.commitment != join.commitment
                || fresh.public_key != join.public_key
                || fresh.device_id != join.device_id
            {
                return Err(CoreError::invalid("The request changed on the server."));
            }
            if fresh.reveal.is_some() {
                let payload = serde_json::to_vec(&fresh).map_err(|e| CoreError::storage(e.to_string()))?;
                self.store.update_parked(id, &payload)?;
                join = fresh;
            }
        }
        let (code, proven) = match self.revealed_transcript_check(&account, &join, &mine)? {
            Some((code, proven)) => (Some(code), proven),
            None => (None, false),
        };
        Ok(EnrollView {
            id: join.id.clone(),
            device_name: join.device_name.clone(),
            code,
            proven,
            created_at: join.created_at,
        })
    }

    /// With the reveal: the code and whether a no-tap proof came with it. `None` before the reveal. A reveal that
    /// does not open the commitment is refused.
    fn revealed_transcript_check(
        &self,
        account: &Account,
        join: &JoinRequest,
        mine: &PrimaryEnrollment,
    ) -> Result<Option<(String, bool)>, CoreError> {
        let Some(reveal) = &join.reveal else {
            return Ok(None);
        };
        let (Some(device_id), Some(sign_pub), Some(committed)) = (&join.device_id, &join.sign_pub, &join.commitment)
        else {
            return Err(CoreError::invalid("not an enrollment"));
        };
        let nonce = parse_nonce(&reveal.nonce).ok_or_else(|| CoreError::invalid("invalid reveal"))?;
        if !check_commitment(committed, &account.id, device_id, &join.public_key, sign_pub, &nonce) {
            return Err(CoreError::invalid("The other phone's answer does not match what it committed to."));
        }
        let primary_nonce = parse_nonce(&mine.nonce).ok_or_else(|| CoreError::storage("corrupt enrollment"))?;
        let t = transcript(
            &account.id,
            &join.id,
            device_id,
            &join.public_key,
            sign_pub,
            &mine.challenge,
            nonce,
            primary_nonce,
        );
        let code = t.code().map_err(invalid)?;
        let proven = match (&reveal.proof, self.secret_of(&account.id)?) {
            (Some(proof), Some(secret)) => {
                check_proof(&proof_key(secret.as_bytes(), &account.id), &t, proof, unix_now())
            }
            _ => false,
        };
        Ok(Some((code, proven)))
    }

    /// After the user compared the codes (and biometrics): enrolls the phone as a viewer, or declines.
    pub async fn enroll_confirm(&self, id: &str, approve: bool) -> Result<(), CoreError> {
        let session = self.session()?;
        let Some(_claim) = self.claim(id) else {
            return Err(CoreError::NotFound);
        };
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let join = self.parked_enrollment(id)?;
        let key = format!("{}:{id}", account.slot);
        let answer = if approve {
            let mine: PrimaryEnrollment =
                self.load(ENROLL_PRIMARY, &key)?.ok_or_else(|| CoreError::invalid("Compare the codes first."))?;
            if self.revealed_transcript_check(&account, &join, &mine)?.is_none() {
                return Err(CoreError::invalid("The other phone has not shown its code yet."));
            }
            // The comparison holds for the challenge's lifetime only.
            if unix_now() >= mine.challenge.expires_at {
                return Err(CoreError::invalid("The codes ran out. Ask the other phone to try again."));
            }
            self.enroll_member(&session, &account, &join).await?
        } else {
            JoinAnswer {
                v: PROTOCOL_VERSION,
                approve: false,
                sealed: None,
            }
        };
        let bytes = serde_json::to_vec(&answer).map_err(|e| CoreError::storage(e.to_string()))?;
        let signed = self
            .signed_answer(&session, AnswerKind::Join, id, &bytes)
            .await?
            .ok_or_else(|| CoreError::invalid("Only the primary phone answers."))?;
        let result = api_call!(&session, |api| api.answer_join_body(id, &signed));
        if let Err(e) = result
            && !matches!(
                &e,
                ApiFailure::Status {
                    status: 404 | 409,
                    ..
                }
            )
        {
            return Err(e.into_core());
        }
        self.store.device_secret_delete(ENROLL_PRIMARY, &key)?;
        self.store.remove_pending(id)?;
        self.store.mark_handled(id, unix_now())?;
        self.notifier.item_resolved(id.to_owned());
        Ok(())
    }

    /// Appends the `enroll` entry for `join`'s phone (after removing an older enrollment of the same device, a
    /// reinstall) and makes the approval it gets.
    async fn enroll_member(
        &self,
        session: &crate::session::Session,
        account: &Account,
        join: &JoinRequest,
    ) -> Result<JoinAnswer, CoreError> {
        let (mut roster, seed) = self.as_primary(session, account).await?;
        let (Some(device_id), Some(sign_pub)) = (&join.device_id, &join.sign_pub) else {
            return Err(CoreError::invalid("not an enrollment"));
        };
        let member = Member {
            device_id: device_id.clone(),
            name: member_name(&join.device_name),
            sign_pub: sign_pub.clone(),
            box_pub: join.public_key.clone(),
        };
        member.validate().map_err(invalid)?;
        if roster.current().primary.device_id == member.device_id {
            return Err(CoreError::invalid("That is this phone."));
        }
        if roster.current().viewers.iter().any(|v| v.device_id == member.device_id) {
            roster = self.remove_viewer(session, account, roster, &seed, &member.device_id).await?;
        }
        let current = roster.current().clone();
        let mut viewers = current.viewers.clone();
        viewers.push(member.clone());
        let mut entry =
            current.successor(RosterReason::Enroll, current.primary.clone(), viewers, unix_now()).map_err(invalid)?;
        entry.sig = seed.owner_key()?.sign_b64(&entry.signing_bytes().map_err(invalid)?);
        let append = RosterAppend {
            entry: entry.clone(),
            recovery_wrap: None,
            bundle: None,
            master_password_hash: None,
        };
        // Answered only once the server has the entry (it said so, or a fetch shows it).
        self.commit_entry(session, account, &roster, &append).await.map_err(super::Commit::into_error)?;
        let grant = EnrollGrant {
            v: PROTOCOL_VERSION,
            join_id: join.id.clone(),
            roster_hash: entry.hash_hex().map_err(invalid)?,
            view_keys: self.keyring(&account.slot)?.entries(),
            account_secret: self.secret_of(&account.id)?.map(|s| BASE64URL_NOPAD.encode(s.as_bytes())),
        };
        let sealed = sealed::seal_enroll_grant(&self.sign_key()?, &account.id, entry.epoch, &member.box_pub, &grant)?;
        Ok(JoinAnswer {
            v: PROTOCOL_VERSION,
            approve: true,
            sealed: Some(sealed),
        })
    }
}
