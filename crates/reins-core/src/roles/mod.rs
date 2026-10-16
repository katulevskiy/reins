//! Phone roles (`phone-roles-and-live-sync.md`, approved with Q1 = B, Q2 = allow, Q3 = off): one phone of an account
//! is the **primary** and decides; every other enrolled phone is a **viewer**. This module is the phone's side of the
//! roster ([`reins_proto::roster`]): its keys ([`keys`]), the chain it verified ([`chain`]), what it seals
//! ([`sealed`]), and the changes it makes:
//!
//! - **create** (the approval device): a new owner seed, roster epoch 1, the recovery wrap;
//! - **enroll** ([`enroll`]): the commit-then-reveal exchange of [`reins_proto::enroll`], six digits compared on both
//!   phones, then an `enroll` entry and a signed, sealed answer with the view keys (and, Q1 = B, the account secret);
//! - **remove**: an entry without the viewer, and a new view key for the others;
//! - **handover**: a viewer asks, the primary signs the swap and seals the bundle (owner seed, view keys, the account
//!   secret, the card provider's key) to it, the viewer verifies and keeps it;
//! - **recovery**: the recovery wrap (fetched only with the master password hash), opened with the master key; a new
//!   owner seed signed in by the old owner key, epoch + 1, and a new view key;
//! - **rewrap**: the wrap sealed again after the master password changed.
//!
//! On an account with a roster the primary signs its answers to requests (A4), pairings (A6) and joins
//! ([`reins_proto::answer::SignedAnswer`]). An account without one (the server answers an empty roster, or has no
//! phone roles) keeps working exactly as before.
//!
//! Nothing here trusts the server: every entry is verified from the anchor this phone pinned ([`chain`]), and what it
//! opens is signed by a key that chain (or the enrollment code) vouches for.

pub mod chain;
mod enroll;
pub mod keys;
mod net;
pub mod sealed;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use data_encoding::BASE64URL_NOPAD;
use reins_proto::PROTOCOL_VERSION;
use reins_proto::answer::{AnswerKind, SignedAnswer};
use reins_proto::roster::{
    HANDOVER_REQUEST_SKEW_SECS, HandoverRequest, HandoverSignature, HandoverStatus, Member, RecoveryWrapRequest,
    RosterAppend, RosterEntry, RosterReason, handover_request_message, recovery_wrap_hash, verify_chain,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

pub use chain::{LocalRoster, Standing};
pub use enroll::{EnrollProgress, EnrollStart, EnrollView};
use keys::{BOX_KEY, BoxKey, OWNER_SEED, OwnerSeed, SIGN_KEY, SigningKey, VIEW_KEYS, ViewKeyring, ed25519_verify};
use sealed::{BundlePayload, CarriedSecret, WrapPlace};

use crate::CoreError;
use crate::crypto::MasterKey;
use crate::engine::Engine;
use crate::phone_api::ApiFailure;
use crate::session::{Session, api_call};
use crate::sso::AccountSecret;
use crate::store::{Owner, unix_now};

/// Where this phone keeps its verified roster, its open requests and the view-key grants it owes (device store,
/// per account).
const ROSTER: &str = "reins.roster";
const VIEW_GRANTS: &str = "reins.view-grants";
const HANDOVER: &str = "reins.handover";
/// The view-key ids whose grants the primary made already.
const GRANTS_MADE: &str = "reins.view-grants-made";
/// Handover requests the primary answered (by nonce), so a replay is not shown again.
const HANDOVER_NONCES: &str = "reins.handover-nonces";
/// The key the primary seals recovery wraps with (derived from the master key).
const WRAP_KEY: &str = "reins.wrap-key";
/// The device secrets that move with the primary role (the card provider's API key).
const MOVING_SECRETS: &[&str] = &["payments.provider"];

/// What this phone is on its account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PhoneRole {
    /// The account has no roster (or this phone was never enrolled in it): everything works as before phone roles.
    Legacy,
    /// This phone decides.
    Primary,
    /// This phone watches; decisions are on the primary.
    Viewer,
    /// This phone was listed and is not now (removed, or replaced by a recovery).
    Removed,
}

impl From<Standing> for PhoneRole {
    fn from(s: Standing) -> Self {
        match s {
            Standing::Primary => Self::Primary,
            Standing::Viewer => Self::Viewer,
            Standing::Removed => Self::Removed,
        }
    }
}

/// One phone of the roster.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RosterMemberView {
    pub device_id: String,
    pub name: String,
    pub primary: bool,
    pub this_phone: bool,
}

/// The roster as this phone verified it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RosterSummary {
    pub epoch: u64,
    pub role: PhoneRole,
    /// The primary first, then the viewers.
    pub members: Vec<RosterMemberView>,
    /// A short fingerprint of the owner key ("a1b2 c3d4").
    pub owner_fingerprint: String,
}

/// What a recovery would make of the account's phones, for the user to check before it is signed.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RecoveryPreview {
    /// The roster entry the recovery starts from (hex); hand it back to `roster_recover`.
    pub tip: String,
    /// The epoch the recovery makes.
    pub epoch: u64,
    /// The roster's epoch now, and its owner key's fingerprint: compare them with another phone's.
    pub roster_epoch: u64,
    pub owner_fingerprint: String,
    /// The primary the recovery removes.
    pub removed_primary: String,
    /// The viewers that stay (this phone becomes the primary).
    pub viewers: Vec<RosterMemberView>,
}

/// A viewer asking to become primary, as the primary sees it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HandoverView {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at: i64,
}

/// Where this phone's "Decide on this phone" request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HandoverProgress {
    Waiting,
    /// This phone is the primary now, with the owner seed and the keys that move with the role.
    Done,
    Denied,
    Expired,
}

/// Phone-roles state an engine keeps in memory.
#[derive(Default)]
pub(crate) struct State {
    /// One roster change at a time from this phone.
    changing: tokio::sync::Mutex<()>,
    /// Handover requests the last pending poll carried (the primary's).
    handovers: Mutex<Vec<HandoverRequest>>,
}

/// The account phone-roles calls are about: the server's id of it, and the slot its keys are kept under.
pub(crate) struct Account {
    pub id: String,
    pub slot: String,
}

/// A view-key grant this phone owes a member (after a removal or a recovery), for the sync to deliver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingGrant {
    pub device_id: String,
    pub kid: u64,
    /// [`sealed::seal_view_grant`], base64url.
    pub sealed: String,
}

/// A roster change the server did not take.
enum Commit {
    /// It said no (a 4xx): nothing of the change is on the server.
    Refused(CoreError),
    /// Its answer could hide either outcome, and a fetch did not show the entry yet.
    Unconfirmed(CoreError),
}

impl Commit {
    fn into_error(self) -> CoreError {
        match self {
            Self::Refused(e) | Self::Unconfirmed(e) => e,
        }
    }
}

/// Whether the server's answer says for sure that it did not take the change (a refusal), rather than leaving it
/// open (a timeout, a lost answer, a 5xx).
fn is_definite(e: &ApiFailure) -> bool {
    matches!(
        e,
        ApiFailure::Unauthorized
            | ApiFailure::Status {
                status: 400..=499,
                ..
            }
    )
}

/// What a recovery starts from (see `prepare_recovery`).
struct Recovering {
    master: Master,
    fetched: Vec<RosterEntry>,
    base: LocalRoster,
    wrap_key: Zeroizing<[u8; 32]>,
    old_seed: OwnerSeed,
    carried: ViewKeyring,
}

#[derive(Serialize, Deserialize)]
struct OpenHandover {
    id: String,
}

/// The master key of what the user typed (or of the account secret this phone keeps), with its master password
/// hash (the proof the server checks).
struct Master {
    key: MasterKey,
    hash: Zeroizing<String>,
    /// The account secret, when it was a recovery code (or the one kept here).
    secret: Option<AccountSecret>,
}

fn interrupted(_: tokio::task::JoinError) -> CoreError {
    CoreError::storage("key derivation was interrupted")
}

/// A member name as rosters sign it: one line, no control or invisible formatting characters, at most 64 characters.
pub(crate) fn member_name(name: &str) -> String {
    // As the server cleans a join's name: whitespace as spaces, invisible characters out, runs of spaces as one.
    let visible: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control() && !reins_proto::payments::is_format_char(*c))
        .collect();
    let clean: String = visible
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(reins_proto::roster::MAX_MEMBER_NAME_CHARS)
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() {
        "Phone".to_owned()
    } else {
        clean
    }
}

/// Whether `typed` is shaped like a recovery code (groups of a few letters and digits, dashes or spaces between), as
/// opposed to a master password: then it is checked as a code only, and a bad one is never sent.
pub(crate) fn looks_like_recovery_code(typed: &str) -> bool {
    let groups: Vec<&str> =
        typed.trim().split(|c: char| c == '-' || c.is_whitespace()).filter(|g| !g.is_empty()).collect();
    groups.len() >= 8 && groups.iter().all(|g| g.len() <= 6 && g.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// What the server's refusal of a roster change means for the user.
fn roster_failure(e: ApiFailure) -> CoreError {
    match e {
        ApiFailure::Status {
            status: 409,
            ..
        } => CoreError::invalid("The account's phones changed meanwhile. Try again."),
        ApiFailure::Status {
            status: 400 | 403,
            message,
            ..
        } => CoreError::invalid(format!("The server refused the change: {message}")),
        other => other.into_core(),
    }
}

/// A handover request as the primary may show and approve it: signed by the asking viewer's key in `roster` (this
/// phone's verified one), at the roster's current epoch, recently, for this account and that phone. Its name comes from
/// the roster, never from the server.
pub(crate) fn verified_handover(
    roster: &LocalRoster,
    account: &str,
    request: &HandoverRequest,
    now: i64,
) -> Result<HandoverView, CoreError> {
    let refuse = |why: &str| CoreError::invalid(format!("A request to decide was refused: {why}."));
    let signed = request.signed.as_ref().ok_or_else(|| refuse("it is not signed by the phone"))?;
    signed.validate().map_err(|e| refuse(&e.to_string()))?;
    let current = roster.current();
    let member = current
        .viewers
        .iter()
        .find(|v| v.device_id == request.device_id)
        .ok_or_else(|| refuse("the phone is not a viewer of the account"))?;
    if signed.epoch != current.epoch {
        return Err(refuse("the account's phones changed since it was made"));
    }
    if (now - signed.at).abs() > HANDOVER_REQUEST_SKEW_SECS {
        return Err(refuse("it is too old"));
    }
    let message = handover_request_message(account, signed.epoch, &member.device_id, &signed.nonce, signed.at)
        .ok_or_else(|| refuse("it is malformed"))?;
    let key = member.sign_key().map_err(|e| refuse(&e.to_string()))?;
    let sig = reins_proto::canon::parse_sig(&signed.sig).ok_or_else(|| refuse("it is malformed"))?;
    if !ed25519_verify(&key, &message, &sig) {
        return Err(refuse("it is not signed by the phone"));
    }
    Ok(HandoverView {
        id: request.id.clone(),
        device_id: member.device_id.clone(),
        device_name: member.name.clone(),
        created_at: request.created_at,
    })
}

/// Forgets everything phone roles keep for the account `slot` on this phone: the roster, the owner seed, the view keys,
/// the recovery wrap's key, the grants owed, and every open enrollment or handover. Only for a reset or a deletion the
/// user made here (after a fresh sign-in); never because a server answered an empty roster.
pub(crate) fn forget(device_store: &crate::store::Store, slot: &str) -> Result<(), CoreError> {
    for service in
        [ROSTER, OWNER_SEED, VIEW_KEYS, WRAP_KEY, VIEW_GRANTS, GRANTS_MADE, HANDOVER, HANDOVER_NONCES, enroll::ENROLL]
    {
        device_store.device_secret_delete(service, slot)?;
    }
    device_store.device_secret_delete_prefix(enroll::ENROLL_PRIMARY, &format!("{slot}:"))
}

impl State {
    pub(crate) fn note_handovers(&self, requests: Vec<HandoverRequest>) {
        *self.handovers.lock().unwrap_or_else(PoisonError::into_inner) = requests;
    }
}

impl Engine {
    // ---- keys and stored state ----------------------------------------------------------------------------------

    pub(crate) fn sign_key(&self) -> Result<SigningKey, CoreError> {
        SigningKey::from_seed(&*self.store.device_seed(SIGN_KEY.0, SIGN_KEY.1)?)
    }

    pub(crate) fn box_key(&self) -> Result<BoxKey, CoreError> {
        Ok(BoxKey::from_bytes(*self.store.device_seed(BOX_KEY.0, BOX_KEY.1)?))
    }

    fn load<T: DeserializeOwned>(&self, service: &str, slot: &str) -> Result<Option<T>, CoreError> {
        self.store
            .device_secret_get(service, slot)?
            .map(|raw| serde_json::from_slice(&raw).map_err(|_| CoreError::storage(format!("corrupt {service}"))))
            .transpose()
    }

    fn save<T: Serialize>(&self, service: &str, slot: &str, value: &T) -> Result<(), CoreError> {
        let raw = Zeroizing::new(serde_json::to_vec(value).map_err(|e| CoreError::storage(e.to_string()))?);
        self.store.device_secret_put(service, slot, &raw)
    }

    pub(crate) fn local_roster(&self, slot: &str) -> Result<Option<LocalRoster>, CoreError> {
        self.load(ROSTER, slot)
    }

    fn save_roster(&self, slot: &str, roster: &LocalRoster) -> Result<(), CoreError> {
        self.save(ROSTER, slot, roster)
    }

    pub(crate) fn keyring(&self, slot: &str) -> Result<ViewKeyring, CoreError> {
        self.store
            .device_secret_get(VIEW_KEYS, slot)?
            .map_or_else(|| Ok(ViewKeyring::default()), |raw| ViewKeyring::from_bytes(&raw))
    }

    fn save_keyring(&self, slot: &str, ring: &ViewKeyring) -> Result<(), CoreError> {
        self.store.device_secret_put(VIEW_KEYS, slot, &ring.to_bytes()?)
    }

    pub(crate) fn owner_seed(&self, slot: &str) -> Result<Option<OwnerSeed>, CoreError> {
        self.store.device_secret_get(OWNER_SEED, slot)?.map(|raw| OwnerSeed::from_bytes(&raw)).transpose()
    }

    fn keep_owner_seed(&self, slot: &str, seed: &OwnerSeed) -> Result<(), CoreError> {
        self.store.device_secret_put(OWNER_SEED, slot, seed.as_bytes())
    }

    /// The view-key grants this phone owes members (after a removal or a recovery); the sync delivers them.
    pub(crate) fn outgoing_grants(&self, slot: &str) -> Result<Vec<OutgoingGrant>, CoreError> {
        Ok(self.load(VIEW_GRANTS, slot)?.unwrap_or_default())
    }

    fn owe_grants(&self, slot: &str, grants: Vec<OutgoingGrant>) -> Result<(), CoreError> {
        let mut owed = self.outgoing_grants(slot)?;
        owed.extend(grants);
        self.save(VIEW_GRANTS, slot, &owed)
    }

    /// The signed-in account.
    pub(crate) async fn roles_account(&self, session: &Session) -> Result<Account, CoreError> {
        let id = session.account_user_id().await?;
        let slot = Owner {
            server: session.server.as_str().to_owned(),
            user_id: id.clone(),
        }
        .id();
        Ok(Account {
            id,
            slot,
        })
    }

    /// The signed-in account's slot without the network: from the unlocked store, else from the account id this phone
    /// cached (every phone-roles call caches it first, so a phone without it holds no roster).
    pub(crate) fn account_slot_now(&self) -> Result<Option<String>, CoreError> {
        if let Some(owner) = self.store.bound_owner() {
            return Ok(Some(owner.id()));
        }
        let Ok(session) = self.session() else {
            return Ok(None);
        };
        let key = crate::session::account_id_key(&session.server, &session.email());
        Ok(self.store.meta_get(&key)?.map(|user_id| {
            Owner {
                server: session.server.as_str().to_owned(),
                user_id,
            }
            .id()
        }))
    }

    /// Whether this phone holds a roster of the signed-in account (it is or was a member), without the network.
    pub(crate) fn holds_roster_now(&self) -> Result<bool, CoreError> {
        match self.account_slot_now()? {
            Some(slot) => Ok(self.local_roster(&slot)?.is_some()),
            None => Ok(false),
        }
    }

    /// This phone's role from the roster it verified last, without the network: `Legacy` when it holds none for the
    /// signed-in account.
    pub(crate) fn role_now(&self) -> Result<PhoneRole, CoreError> {
        let Some(slot) = self.account_slot_now()? else {
            return Ok(PhoneRole::Legacy);
        };
        match self.local_roster(&slot)? {
            Some(roster) => Ok(self.standing_in(&roster)?.into()),
            None => Ok(PhoneRole::Legacy),
        }
    }

    /// Whether this phone may decide: the primary, or any phone of an account without phone roles. A viewer, or a
    /// phone the roster removed, never answers a request, pairing or upload: whatever its local Autopilot modes or
    /// grants say, the item waits for the primary. When the role cannot be read, it does not decide either.
    pub(crate) fn decides(&self) -> bool {
        match self.role_now() {
            Ok(PhoneRole::Primary | PhoneRole::Legacy) => true,
            Ok(PhoneRole::Viewer | PhoneRole::Removed) => false,
            Err(e) => {
                log::warn!("this phone's role could not be read; it decides nothing: {e}");
                false
            }
        }
    }

    /// Refuses an answer or a change on a phone that does not decide ([`Engine::decides`]): [`CoreError::ViewerOnly`].
    pub(crate) fn require_decider(&self) -> Result<(), CoreError> {
        if self.decides() {
            Ok(())
        } else {
            Err(CoreError::ViewerOnly)
        }
    }

    /// Whether setting `mode` for `connection_id` (the global mode when `None`) only pulls the brake: Lockdown, or
    /// stopping a running bypass (the mode stays what it is under the bypass).
    pub(crate) fn is_brake(
        &self,
        connection_id: Option<&str>,
        mode: Option<crate::autopilot::AutopilotMode>,
    ) -> Result<bool, CoreError> {
        use crate::autopilot::AutopilotMode;
        if mode == Some(AutopilotMode::Lockdown) {
            return Ok(true);
        }
        if mode == Some(AutopilotMode::Bypass) {
            return Ok(false);
        }
        let row = self.store.ap_mode_row(connection_id.unwrap_or_default())?;
        Ok(row.bypass_until.is_some() && row.mode == mode)
    }

    /// This phone in a roster entry, named `name`.
    fn me(&self, name: &str) -> Result<Member, CoreError> {
        Ok(Member {
            device_id: self.store.device_id()?,
            name: member_name(name),
            sign_pub: self.sign_key()?.public_b64(),
            box_pub: self.box_key()?.public_b64(),
        })
    }

    fn standing_in(&self, roster: &LocalRoster) -> Result<Standing, CoreError> {
        Ok(roster.standing(&self.store.device_id()?, &self.sign_key()?.public_b64()))
    }

    // ---- reading the roster -------------------------------------------------------------------------------------

    /// The roster after fetching it, verified from this phone's anchor; `None` when this phone holds none (it never
    /// created or joined one, so the account is legacy for it).
    pub(crate) async fn refresh_roster(
        &self,
        session: &Session,
        account: &Account,
    ) -> Result<Option<LocalRoster>, CoreError> {
        let Some(local) = self.local_roster(&account.slot)? else {
            return Ok(None);
        };
        let fetched = api_call!(session, |api| api.roster()).map_err(ApiFailure::into_core)?;
        let updated = local.updated(&fetched.entries)?;
        if updated != local {
            self.save_roster(&account.slot, &updated)?;
        }
        self.reconcile_roles(account, &updated)?;
        Ok(Some(updated))
    }

    pub async fn phone_role(&self) -> Result<PhoneRole, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        match self.local_roster(&account.slot)? {
            Some(roster) => Ok(self.standing_in(&roster)?.into()),
            None => Ok(PhoneRole::Legacy),
        }
    }

    /// The roster this phone verified (`refresh`: fetched again first); `None` on a legacy account.
    pub async fn roster_summary(&self, refresh: bool) -> Result<Option<RosterSummary>, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let roster = if refresh {
            self.refresh_roster(&session, &account).await?
        } else {
            self.local_roster(&account.slot)?
        };
        let Some(roster) = roster else {
            return Ok(None);
        };
        let me = self.store.device_id()?;
        let current = roster.current();
        let members = current
            .members()
            .map(|m| RosterMemberView {
                device_id: m.device_id.clone(),
                name: m.name.clone(),
                primary: m.device_id == current.primary.device_id,
                this_phone: m.device_id == me,
            })
            .collect();
        Ok(Some(RosterSummary {
            epoch: roster.epoch(),
            role: self.standing_in(&roster)?.into(),
            members,
            owner_fingerprint: roster.owner_fingerprint(),
        }))
    }

    // ---- changing the roster ------------------------------------------------------------------------------------

    /// The roster and owner seed, when this phone is the primary.
    async fn as_primary(&self, session: &Session, account: &Account) -> Result<(LocalRoster, OwnerSeed), CoreError> {
        let roster = self
            .refresh_roster(session, account)
            .await?
            .ok_or_else(|| CoreError::invalid("This account has no phone roles yet."))?;
        if self.standing_in(&roster)? != Standing::Primary {
            return Err(CoreError::invalid("Only the primary phone changes the account's phones."));
        }
        let seed = self
            .owner_seed(&account.slot)?
            .ok_or_else(|| CoreError::invalid("This phone lost the account's owner key. Use the recovery code."))?;
        if seed.owner_key()?.public_b64() != roster.current().owner_pub {
            return Err(CoreError::invalid("This phone's owner key is not the account's. Use the recovery code."));
        }
        Ok((roster, seed))
    }

    /// Posts `append`'s entry, which follows `base` (the roster this phone verified), and returns the roster with it
    /// once the server has it: the server said so or, after an answer that could hide either outcome (a timeout, a
    /// 5xx), a fetch shows it. Only a definite refusal (4xx) is [`Commit::Refused`], after which the caller may undo
    /// what it kept for the entry. Anything else is [`Commit::Unconfirmed`]: the caller keeps it, and the next fetch
    /// that shows the entry finishes the change on this phone ([`Engine::reconcile_roles`]).
    async fn commit_entry(
        &self,
        session: &Session,
        account: &Account,
        base: &LocalRoster,
        append: &RosterAppend,
    ) -> Result<LocalRoster, Commit> {
        let mut next = base.clone();
        next.push(append.entry.clone()).map_err(Commit::Refused)?;
        let hash = append.entry.hash_hex().map_err(|e| Commit::Refused(CoreError::invalid(e.to_string())))?;
        match api_call!(session, |api| api.append_roster(append)) {
            Ok(_) => {
                self.save_roster(&account.slot, &next).map_err(Commit::Unconfirmed)?;
                self.reconcile_roles(account, &next).map_err(Commit::Unconfirmed)?;
                Ok(next)
            }
            Err(e) if is_definite(&e) => Err(Commit::Refused(roster_failure(e))),
            Err(e) => {
                let why = roster_failure(e);
                match self.refresh_roster(session, account).await {
                    Ok(Some(seen)) if seen.at(append.entry.epoch).and_then(|e| e.hash_hex().ok()) == Some(hash) => {
                        Ok(seen)
                    }
                    _ => Err(Commit::Unconfirmed(CoreError::invalid(format!(
                        "The server did not confirm the change ({why}). It completes on this phone once the server \
                         shows it; check again in a moment."
                    )))),
                }
            }
        }
    }

    /// Brings this phone's keys in line with the roster it verified:
    ///
    /// - a phone that holds the owner seed in force but is no longer primary (a handover it made, even one whose
    ///   answer it never got), or an owner seed a recovery superseded, drops it, the recovery wrap's key and the
    ///   card provider's key (Q1 = B: it keeps the account secret and the vault);
    /// - the primary holding the owner seed in force has every view key that seed brought (each `create`, `remove`
    ///   and `recovery` entry of its tenure, including one whose answer it never got), and owes a grant of each new
    ///   one to the viewers listed with it.
    ///
    /// A seed no entry names yet (a recovery the server has not confirmed) is kept. Never called because the server
    /// answered an empty roster: that is no reason to drop anything.
    pub(crate) fn reconcile_roles(&self, account: &Account, roster: &LocalRoster) -> Result<(), CoreError> {
        let Some(seed) = self.owner_seed(&account.slot)? else {
            return Ok(());
        };
        let seed_pub = seed.owner_key()?.public_b64();
        let current = roster.current();
        let standing = self.standing_in(roster)?;
        let superseded = seed_pub != current.owner_pub && roster.entries.iter().any(|e| e.owner_pub == seed_pub);
        if (seed_pub == current.owner_pub && standing != Standing::Primary) || superseded {
            self.store.device_secret_delete(OWNER_SEED, &account.slot)?;
            self.store.device_secret_delete(WRAP_KEY, &account.slot)?;
            for service in MOVING_SECRETS {
                for (acct, _) in self.store.secrets_of(service)? {
                    self.store.secret_delete(service, &acct)?;
                }
            }
            return Ok(());
        }
        if seed_pub != current.owner_pub || standing != Standing::Primary {
            return Ok(());
        }
        let mut ring = self.keyring(&account.slot)?;
        let mut made: BTreeSet<u64> = self.load(GRANTS_MADE, &account.slot)?.unwrap_or_default();
        let signer = self.sign_key()?;
        let mut owed = Vec::new();
        let brought = roster.entries.iter().filter(|e| {
            e.owner_pub == seed_pub
                && matches!(e.reason, RosterReason::Create | RosterReason::Remove | RosterReason::Recovery)
        });
        for e in brought {
            let key = seed.view_key(&account.id, e.epoch);
            ring.insert(key.clone());
            if e.reason != RosterReason::Create && made.insert(e.epoch) {
                for m in &e.viewers {
                    owed.push(OutgoingGrant {
                        device_id: m.device_id.clone(),
                        kid: key.kid,
                        sealed: sealed::seal_view_grant(&signer, &account.id, m, &key)?,
                    });
                }
            }
        }
        self.save_keyring(&account.slot, &ring)?;
        self.save(GRANTS_MADE, &account.slot, &made)?;
        self.owe_grants(&account.slot, owed)
    }

    /// Like [`Engine::require_decider`], then, on an account with phone roles, verifies with the server that this
    /// phone is still the primary: before anything that cannot be undone (sending an email, a purchase), so that
    /// nothing runs whose answer the server would refuse. A server that no longer shows the roster this phone
    /// verified is refused (a rollback), never taken as the end of phone roles.
    pub(crate) async fn require_decider_fresh(&self, session: &Session) -> Result<(), CoreError> {
        self.require_decider()?;
        let account = self.roles_account(session).await?;
        match self.refresh_roster(session, &account).await? {
            Some(roster) if self.standing_in(&roster)? != Standing::Primary => Err(CoreError::ViewerOnly),
            _ => Ok(()),
        }
    }

    /// The master key and proof of `typed` (a recovery code or the master password), or of the account secret this
    /// phone keeps when `typed` is `None`.
    async fn master_for(
        &self,
        session: &Session,
        account: &Account,
        typed: Option<Zeroizing<String>>,
    ) -> Result<Master, CoreError> {
        let from_secret = async move |secret: AccountSecret| {
            tokio::task::spawn_blocking(move || -> Result<Master, CoreError> {
                let key = secret.master_key()?;
                let hash = secret.master_password_hash(&key);
                Ok(Master {
                    key,
                    hash,
                    secret: Some(secret),
                })
            })
            .await
            .map_err(interrupted)?
        };
        let Some(typed) = typed else {
            let secret = self.secret_of(&account.id)?.ok_or_else(|| {
                CoreError::invalid("Type the account's master password: this phone keeps no recovery code for it.")
            })?;
            return from_secret(secret).await;
        };
        if let Some(secret) = AccountSecret::from_recovery_code(&typed) {
            return from_secret(secret).await;
        }
        // A recovery code mistyped is refused here: sent as a password it would only spend the server's proof budget.
        if looks_like_recovery_code(&typed) {
            return Err(CoreError::invalid(
                "That recovery code is not complete or has a typo. Check it and try again.",
            ));
        }
        if typed.trim().is_empty() {
            return Err(CoreError::invalid("Type your recovery code or master password."));
        }
        let email = session.email();
        let kdf = crate::vault::VaultClient::new(&session.http, &session.server).prelogin(&email).await?;
        tokio::task::spawn_blocking(move || -> Result<Master, CoreError> {
            let key = crate::crypto::master_key(&typed, &email, kdf)?;
            let hash = crate::crypto::master_password_hash(&key, &typed);
            Ok(Master {
                key,
                hash,
                secret: None,
            })
        })
        .await
        .map_err(interrupted)?
    }

    /// The key this primary seals recovery wraps with (derived from the master key; kept since `create`, a recovery,
    /// a rewrap or a handover).
    fn wrap_key(&self, slot: &str) -> Result<Zeroizing<[u8; 32]>, CoreError> {
        let raw = self.store.device_secret_get(WRAP_KEY, slot)?.ok_or_else(|| {
            CoreError::invalid("This phone lost the recovery wrap's key. Seal it again with the master password.")
        })?;
        <[u8; 32]>::try_from(raw.as_slice()).map(Zeroizing::new).map_err(|_| CoreError::storage("corrupt wrap key"))
    }

    fn keep_wrap_key(&self, slot: &str, key: &[u8; 32]) -> Result<(), CoreError> {
        self.store.device_secret_put(WRAP_KEY, slot, key)
    }

    /// Seals a new recovery wrap for `entry` (unsigned yet), at its place in the chain, and names it in the entry.
    fn rewrap_entry(
        account: &Account,
        entry: &mut RosterEntry,
        key: &[u8; 32],
        seed: &OwnerSeed,
        carried: &ViewKeyring,
    ) -> Result<String, CoreError> {
        let wrap = sealed::seal_recovery_wrap(key, &account.id, WrapPlace::of(entry)?, seed, carried)?;
        entry.recovery_wrap_hash =
            recovery_wrap_hash(&wrap).ok_or_else(|| CoreError::storage("invalid recovery wrap"))?;
        Ok(wrap)
    }

    /// Makes this phone (the approval device) the primary of a new roster: a new owner seed, roster epoch 1 and the
    /// recovery wrap. `code_or_password`: needed for an account with a master password (this phone keeps no secret
    /// of it); otherwise the account secret this phone keeps seals the wrap. Either way the server checks it (the
    /// master password hash goes with the entry), so a mistyped password never seals a wrap nobody can open.
    pub async fn roster_create(
        &self,
        device_name: &str,
        code_or_password: Option<Zeroizing<String>>,
    ) -> Result<RosterSummary, CoreError> {
        let session = self.session()?;
        let one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let existing = api_call!(&session, |api| api.roster()).map_err(|e| match e {
            ApiFailure::Status {
                status: 404,
                ..
            } => CoreError::invalid("This server does not offer phone roles yet."),
            other => other.into_core(),
        })?;
        if self.local_roster(&account.slot)?.is_some() {
            return Err(CoreError::invalid("This account has phone roles already."));
        }
        // A seed kept from an earlier attempt whose answer never came: its entry may be on the server after all.
        let kept = self.owner_seed(&account.slot)?;
        if let Some(first) = existing.entries.first() {
            let ours = match &kept {
                Some(seed) => seed.owner_key()?.public_b64() == first.owner_pub,
                None => false,
            };
            if !ours || first.primary.device_id != self.store.device_id()? {
                return Err(CoreError::invalid("This account has phone roles already."));
            }
            let roster = LocalRoster::created(first.clone())?.updated(&existing.entries)?;
            self.save_roster(&account.slot, &roster)?;
            self.reconcile_roles(&account, &roster)?;
            drop(one);
            return self.roster_summary(false).await?.ok_or(CoreError::NotFound);
        }
        let master = self.master_for(&session, &account, code_or_password).await?;
        let wrap_key = sealed::recovery_wrap_key(&master.key, &account.id);
        let seed = match kept {
            Some(seed) => seed,
            None => OwnerSeed::generate()?,
        };
        let owner = seed.owner_key()?;
        let mut entry = RosterEntry::genesis(
            &account.id,
            self.me(device_name)?,
            owner.public_b64(),
            seed.credential_key().public_b64(),
            String::new(),
            unix_now(),
        );
        let wrap = Self::rewrap_entry(&account, &mut entry, &wrap_key, &seed, &ViewKeyring::default())?;
        entry.sig = owner.sign_b64(&entry.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
        let roster = LocalRoster::created(entry.clone())?;
        // Kept before the server has the entry: a seed for an entry that never landed is harmless (the next attempt
        // uses it again), an entry whose seed was lost is not.
        self.keep_owner_seed(&account.slot, &seed)?;
        self.keep_wrap_key(&account.slot, &wrap_key)?;
        let mut ring = ViewKeyring::default();
        ring.insert(seed.view_key(&account.id, 1));
        self.save_keyring(&account.slot, &ring)?;
        let append = RosterAppend {
            entry,
            recovery_wrap: Some(wrap),
            bundle: None,
            master_password_hash: Some(master.hash.to_string()),
        };
        match api_call!(&session, |api| api.append_roster(&append)) {
            Ok(_) => {}
            Err(e) if is_definite(&e) => {
                for service in [OWNER_SEED, VIEW_KEYS, WRAP_KEY] {
                    self.store.device_secret_delete(service, &account.slot)?;
                }
                return Err(roster_failure(e));
            }
            Err(e) => {
                let why = roster_failure(e);
                let first = api_call!(&session, |api| api.roster()).ok().and_then(|r| r.entries.first().cloned());
                if first.as_ref() != Some(roster.current()) {
                    return Err(CoreError::invalid(format!(
                        "The server did not confirm the new roster ({why}). Try again: this phone keeps its keys."
                    )));
                }
            }
        }
        self.save_roster(&account.slot, &roster)?;
        self.reconcile_roles(&account, &roster)?;
        drop(one);
        self.roster_summary(false).await?.ok_or(CoreError::NotFound)
    }

    /// Takes `device_id` off the roster (the primary only) and gives the other viewers a new view key, so that the
    /// removed phone reads nothing written from now on.
    pub async fn roster_remove_member(&self, device_id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let (roster, seed) = self.as_primary(&session, &account).await?;
        self.remove_viewer(&session, &account, roster, &seed, device_id).await.map(drop)
    }

    async fn remove_viewer(
        &self,
        session: &Session,
        account: &Account,
        roster: LocalRoster,
        seed: &OwnerSeed,
        device_id: &str,
    ) -> Result<LocalRoster, CoreError> {
        let current = roster.current().clone();
        if !current.viewers.iter().any(|v| v.device_id == device_id) {
            return Err(CoreError::NotFound);
        }
        let viewers: Vec<Member> = current.viewers.iter().filter(|v| v.device_id != device_id).cloned().collect();
        let mut entry = current
            .successor(RosterReason::Remove, current.primary.clone(), viewers, unix_now())
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        // A new wrap, so the one from before the removal opens for no roster after it.
        let ring = self.keyring(&account.slot)?;
        let wrap = Self::rewrap_entry(account, &mut entry, &*self.wrap_key(&account.slot)?, seed, &ring)?;
        entry.sig = seed.owner_key()?.sign_b64(&entry.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
        let append = RosterAppend {
            entry,
            recovery_wrap: Some(wrap),
            bundle: None,
            master_password_hash: None,
        };
        // Committed, the new view key and its grants come with the roster (`reconcile_roles`).
        self.commit_entry(session, account, &roster, &append).await.map_err(Commit::into_error)
    }

    /// Seals the recovery wrap again for the account's new master password (`new_password`, as it is now on the
    /// server). The primary only. The server checks the password first (it counts like any proof).
    pub async fn roster_rewrap(&self, new_password: Zeroizing<String>) -> Result<(), CoreError> {
        let session = self.session()?;
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let (roster, seed) = self.as_primary(&session, &account).await?;
        let master = self.master_for(&session, &account, Some(new_password)).await?;
        // The server tells whether this is the account's password now (a wrap sealed under a mistyped one would
        // never open).
        let request = RecoveryWrapRequest {
            master_password_hash: master.hash.to_string(),
        };
        api_call!(&session, |api| api.recovery_wrap(&request)).map_err(|e| match e {
            ApiFailure::Status {
                status: 403,
                ..
            } => CoreError::invalid("That is not the account's master password now."),
            other => other.into_core(),
        })?;
        let wrap_key = sealed::recovery_wrap_key(&master.key, &account.id);
        let current = roster.current().clone();
        let mut entry = current
            .successor(RosterReason::Rewrap, current.primary.clone(), current.viewers.clone(), unix_now())
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        let wrap = Self::rewrap_entry(&account, &mut entry, &wrap_key, &seed, &self.keyring(&account.slot)?)?;
        entry.sig = seed.owner_key()?.sign_b64(&entry.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
        // The password changed on the server already: its key is the one from now on, whatever this append's fate.
        self.keep_wrap_key(&account.slot, &wrap_key)?;
        let append = RosterAppend {
            entry,
            recovery_wrap: Some(wrap),
            bundle: None,
            master_password_hash: None,
        };
        self.commit_entry(&session, &account, &roster, &append).await.map(drop).map_err(Commit::into_error)
    }

    /// Everything a recovery starts from: the master key, the verified roster, and the opened wrap in force.
    async fn prepare_recovery(
        &self,
        session: &Session,
        account: &Account,
        typed: Zeroizing<String>,
    ) -> Result<Recovering, CoreError> {
        let master = self.master_for(session, account, Some(typed)).await?;
        let fetched = api_call!(session, |api| api.roster()).map_err(ApiFailure::into_core)?.entries;
        // Every entry verifies from the first; a phone that verified the roster before also refuses anything older
        // than (or different from) what it saw.
        verify_chain(&fetched, None, ed25519_verify)
            .map_err(|e| CoreError::invalid(format!("The account's roster does not verify: {e}")))?;
        let base = match self.local_roster(&account.slot)? {
            Some(local) => local.updated(&fetched)?,
            None => LocalRoster::anchored(
                fetched.last().cloned().ok_or_else(|| CoreError::invalid("This account has no phone roles."))?,
            )?,
        };
        let latest = base.current().clone();
        let request = RecoveryWrapRequest {
            master_password_hash: master.hash.to_string(),
        };
        let wrap = api_call!(session, |api| api.recovery_wrap(&request)).map_err(|e| match e {
            ApiFailure::Status {
                status: 403,
                ..
            } => CoreError::invalid("That is neither the recovery code nor the master password."),
            other => other.into_core(),
        })?;
        if wrap.epoch != latest.epoch
            || recovery_wrap_hash(&wrap.recovery_wrap).as_deref() != Some(&latest.recovery_wrap_hash)
        {
            return Err(CoreError::invalid("The server's recovery wrap is not the one the roster names."));
        }
        // The wrap opens only for the place in the chain it was sealed for: the entry that brought it.
        let wrap_key = sealed::recovery_wrap_key(&master.key, &account.id);
        let place = WrapPlace::in_force(&fetched)?;
        let (old_seed, carried) = sealed::open_recovery_wrap(&wrap_key, &account.id, place, &wrap.recovery_wrap)?;
        if old_seed.owner_key()?.public_b64() != latest.owner_pub {
            return Err(CoreError::invalid("The recovery wrap holds another owner key than the roster's."));
        }
        Ok(Recovering {
            master,
            fetched,
            base,
            wrap_key,
            old_seed,
            carried,
        })
    }

    /// What a recovery with `code_or_password` would make of the account's phones, for the user to check before
    /// [`Engine::roster_recover`]: this phone primary, the viewers kept, the primary removed, and the roster's epoch and
    /// owner key fingerprint to compare with another phone's. `tip` names the roster entry it starts from; the
    /// recovery refuses any other.
    pub async fn roster_recover_preview(
        &self,
        code_or_password: Zeroizing<String>,
    ) -> Result<RecoveryPreview, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let recovering = self.prepare_recovery(&session, &account, code_or_password).await?;
        let latest = recovering.base.current();
        let me = self.store.device_id()?;
        let viewers = latest
            .viewers
            .iter()
            .filter(|v| v.device_id != me)
            .map(|v| RosterMemberView {
                device_id: v.device_id.clone(),
                name: v.name.clone(),
                primary: false,
                this_phone: false,
            })
            .collect();
        Ok(RecoveryPreview {
            tip: latest.hash_hex().map_err(|e| CoreError::invalid(e.to_string()))?,
            roster_epoch: latest.epoch,
            owner_fingerprint: recovering.base.owner_fingerprint(),
            epoch: latest.epoch.saturating_add(1),
            removed_primary: latest.primary.name.clone(),
            viewers,
        })
    }

    /// The primary is lost: with the recovery code (or the master password) this phone opens the recovery wrap, makes
    /// a new owner seed that the old owner key signs in, and becomes primary (epoch + 1, the old primary removed, a
    /// new view key for the viewers). `tip`: the [`RecoveryPreview::tip`] the user confirmed; the server's roster must
    /// still end there.
    pub async fn roster_recover(
        &self,
        code_or_password: Zeroizing<String>,
        device_name: &str,
        tip: &str,
    ) -> Result<RosterSummary, CoreError> {
        let session = self.session()?;
        let one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let Recovering {
            master,
            fetched,
            base,
            wrap_key,
            old_seed,
            carried,
        } = self.prepare_recovery(&session, &account, code_or_password).await?;
        let latest = base.current().clone();
        if latest.hash_hex().map_err(|e| CoreError::invalid(e.to_string()))? != tip {
            return Err(CoreError::invalid("The account's phones changed since you checked them. Check again."));
        }
        // What the old owner's tenure was sealed with: the view keys it introduced (and what its wrap carried).
        let mut ring = self.keyring(&account.slot)?;
        ring.extend(&carried.entries())?;
        for e in fetched.iter().filter(|e| e.owner_pub == latest.owner_pub) {
            if matches!(e.reason, RosterReason::Create | RosterReason::Remove | RosterReason::Recovery) {
                ring.insert(old_seed.view_key(&account.id, e.epoch));
            }
        }
        let seed = OwnerSeed::generate()?;
        let me = self.me(device_name)?;
        let viewers: Vec<Member> = latest.viewers.iter().filter(|v| v.device_id != me.device_id).cloned().collect();
        let mut entry = latest
            .successor(RosterReason::Recovery, me, viewers, unix_now())
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        entry.owner_pub = seed.owner_key()?.public_b64();
        entry.credential_pub = seed.credential_key().public_b64();
        let new_wrap = Self::rewrap_entry(&account, &mut entry, &wrap_key, &seed, &ring)?;
        entry.sig =
            old_seed.owner_key()?.sign_b64(&entry.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
        // As at `create`, everything is kept before the server has the entry: committed, or seen later on a fetch,
        // the recovery completes on this phone (the new view key and its grants come with `reconcile_roles`).
        let previous_seed = self.owner_seed(&account.slot)?;
        let previous_roster = self.local_roster(&account.slot)?;
        // The keyring (now with the old owner's keys) and the wrap key too: a refused recovery leaves none of it.
        let previous_raw = [VIEW_KEYS, WRAP_KEY]
            .map(|service| self.store.device_secret_get(service, &account.slot).map(|raw| (service, raw)));
        let previous_raw = previous_raw.into_iter().collect::<Result<Vec<_>, _>>()?;
        self.keep_owner_seed(&account.slot, &seed)?;
        self.keep_wrap_key(&account.slot, &wrap_key)?;
        self.save_keyring(&account.slot, &ring)?;
        self.save_roster(&account.slot, &base)?;
        let append = RosterAppend {
            entry,
            recovery_wrap: Some(new_wrap),
            bundle: None,
            master_password_hash: Some(master.hash.to_string()),
        };
        match self.commit_entry(&session, &account, &base, &append).await {
            Ok(_) => {}
            Err(Commit::Refused(e)) => {
                match previous_seed {
                    Some(previous) => self.keep_owner_seed(&account.slot, &previous)?,
                    None => self.store.device_secret_delete(OWNER_SEED, &account.slot)?,
                }
                match previous_roster {
                    Some(previous) => self.save_roster(&account.slot, &previous)?,
                    None => self.store.device_secret_delete(ROSTER, &account.slot)?,
                }
                for (service, raw) in previous_raw {
                    match raw {
                        Some(raw) => self.store.device_secret_put(service, &account.slot, &raw)?,
                        None => self.store.device_secret_delete(service, &account.slot)?,
                    }
                }
                return Err(e);
            }
            Err(Commit::Unconfirmed(e)) => return Err(e),
        }
        if let Some(secret) = master.secret
            && self.secret_of(&account.id)?.is_none()
        {
            self.adopt_secret(secret).await?;
        }
        drop(one);
        self.roster_summary(false).await?.ok_or(CoreError::NotFound)
    }

    // ---- handover -----------------------------------------------------------------------------------------------

    /// On a viewer: "Decide on this phone". The primary approves or declines; follow with [`Engine::handover_poll`].
    pub async fn handover_request(&self) -> Result<String, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let roster = self
            .refresh_roster(&session, &account)
            .await?
            .ok_or_else(|| CoreError::invalid("This account has no phone roles yet."))?;
        if self.standing_in(&roster)? != Standing::Viewer {
            return Err(CoreError::invalid("Only a viewer asks to decide."));
        }
        // Signed with this phone's roster key, so the primary knows who asks whatever the server relays.
        let nonce = reins_proto::enroll::encode_nonce(&crate::crypto::random_bytes::<32>()?);
        let at = unix_now();
        let message = handover_request_message(&account.id, roster.epoch(), &self.store.device_id()?, &nonce, at)
            .ok_or_else(|| CoreError::storage("invalid nonce"))?;
        let signed = HandoverSignature {
            epoch: roster.epoch(),
            nonce,
            at,
            sig: self.sign_key()?.sign_b64(&message),
        };
        let created = api_call!(&session, |api| api.create_handover(signed.clone())).map_err(ApiFailure::into_core)?;
        self.save(
            HANDOVER,
            &account.slot,
            &OpenHandover {
                id: created.id.clone(),
            },
        )?;
        Ok(created.id)
    }

    /// Where this phone's handover request stands. `Done`: the primary handed over, and this phone verified and kept
    /// the bundle (it is the primary now).
    pub async fn handover_poll(&self) -> Result<HandoverProgress, CoreError> {
        let session = self.session()?;
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let open: OpenHandover = self.load(HANDOVER, &account.slot)?.ok_or(CoreError::NotFound)?;
        let state = match api_call!(&session, |api| api.handover_state(&open.id)) {
            Ok(state) => state,
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => reins_proto::roster::HandoverState {
                status: HandoverStatus::Expired,
                epoch: None,
            },
            Err(e) => return Err(e.into_core()),
        };
        let progress = match state.status {
            HandoverStatus::Waiting => return Ok(HandoverProgress::Waiting),
            HandoverStatus::Denied => HandoverProgress::Denied,
            HandoverStatus::Expired => HandoverProgress::Expired,
            HandoverStatus::Done => {
                let epoch = state.epoch.ok_or_else(|| CoreError::invalid("The handover names no epoch."))?;
                self.take_handover(&session, &account, epoch).await?;
                HandoverProgress::Done
            }
        };
        self.store.device_secret_delete(HANDOVER, &account.slot)?;
        Ok(progress)
    }

    /// Verifies the `handover` entry at `epoch` that made this phone primary, and keeps what its bundle carries.
    async fn take_handover(&self, session: &Session, account: &Account, epoch: u64) -> Result<(), CoreError> {
        let roster = self
            .refresh_roster(session, account)
            .await?
            .ok_or_else(|| CoreError::invalid("This account has no phone roles yet."))?;
        let entry = roster.at(epoch).ok_or_else(|| CoreError::invalid("The roster lacks the handover's epoch."))?;
        let me = self.store.device_id()?;
        if entry.reason != RosterReason::Handover
            || entry.primary.device_id != me
            || self.standing_in(&roster)? != Standing::Primary
        {
            return Err(CoreError::invalid("The roster does not make this phone primary."));
        }
        let old = roster
            .primary_key_at(epoch - 1)
            .ok_or_else(|| CoreError::invalid("The roster lacks the epoch before the handover."))?;
        let bundle = api_call!(session, |api| api.handover_bundle()).map_err(ApiFailure::into_core)?;
        if bundle.epoch != epoch {
            return Err(CoreError::invalid("The handover bundle is for another epoch."));
        }
        let (payload, seed) = sealed::open_bundle(&self.box_key()?, &bundle.sealed, &account.id, entry, &old)?;
        if seed.owner_key()?.public_b64() != roster.current().owner_pub {
            return Err(CoreError::invalid("The handover bundle carries another owner key."));
        }
        let wrap_key = payload
            .wrap_key
            .as_deref()
            .and_then(|k| BASE64URL_NOPAD.decode(k.as_bytes()).ok())
            .and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
            .map(Zeroizing::new)
            .ok_or_else(|| CoreError::invalid("The handover bundle lacks the recovery wrap's key."))?;
        let mut ring = self.keyring(&account.slot)?;
        ring.extend(&payload.view_keys)?;
        self.keep_owner_seed(&account.slot, &seed)?;
        self.keep_wrap_key(&account.slot, &wrap_key)?;
        self.save_keyring(&account.slot, &ring)?;
        // The view keys the old primary brought were granted by it: this phone owes grants only for its own.
        let made: BTreeSet<u64> = roster
            .entries
            .iter()
            .filter(|e| matches!(e.reason, RosterReason::Create | RosterReason::Remove | RosterReason::Recovery))
            .map(|e| e.epoch)
            .collect();
        self.save(GRANTS_MADE, &account.slot, &made)?;
        for carried in &payload.device_secrets {
            if MOVING_SECRETS.contains(&carried.service.as_str()) {
                let value = Zeroizing::new(
                    BASE64URL_NOPAD
                        .decode(carried.value.as_bytes())
                        .map_err(|_| CoreError::invalid("The handover bundle is malformed."))?,
                );
                self.store.secret_put(&carried.service, &carried.account, &value)?;
            }
        }
        if let Some(secret) = &payload.account_secret
            && self.secret_of(&account.id)?.is_none()
        {
            let raw = Zeroizing::new(
                BASE64URL_NOPAD
                    .decode(secret.as_bytes())
                    .map_err(|_| CoreError::invalid("The handover bundle is malformed."))?,
            );
            self.adopt_secret(AccountSecret::from_bytes(&raw)?).await?;
        }
        match api_call!(session, |api| api.drop_handover_bundle()) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into_core()),
        }
    }

    /// Nonces of handover requests this primary answered (approved or declined), with their signed time, while that
    /// time is within the skew window (a request older than that is refused anyway).
    fn answered_nonces(&self, slot: &str, now: i64) -> Result<BTreeMap<String, i64>, CoreError> {
        let mut nonces: BTreeMap<String, i64> = self.load(HANDOVER_NONCES, slot)?.unwrap_or_default();
        nonces.retain(|_, at| (now - *at).abs() <= HANDOVER_REQUEST_SKEW_SECS);
        Ok(nonces)
    }

    fn answer_nonce(&self, slot: &str, (nonce, at): (String, i64), now: i64) -> Result<(), CoreError> {
        if nonce.is_empty() {
            return Ok(());
        }
        let mut nonces = self.answered_nonces(slot, now)?;
        nonces.insert(nonce, at);
        self.save(HANDOVER_NONCES, slot, &nonces)
    }

    /// On the primary: the viewers asking to decide (as the last poll of the waiting list carried them), only those
    /// whose request the viewer signed with its key in this phone's verified roster, at its current epoch, and named
    /// as that roster names them (never as the server does).
    pub async fn handover_requests(&self) -> Result<Vec<HandoverView>, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let Some(roster) = self.local_roster(&account.slot)? else {
            return Ok(Vec::new());
        };
        let seen = self.roles.handovers.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let now = unix_now();
        let answered = self.answered_nonces(&account.slot, now)?;
        let mut shown_nonces = BTreeSet::new();
        Ok(seen
            .iter()
            .filter_map(|h| match verified_handover(&roster, &account.id, h, now) {
                Ok(view) => {
                    // A request answered before, or the same one twice (a replay), is shown once at most.
                    let nonce = h.signed.as_ref().map(|s| s.nonce.clone()).unwrap_or_default();
                    (!answered.contains_key(&nonce) && shown_nonces.insert(nonce)).then_some(view)
                }
                Err(e) => {
                    log::warn!("a handover request was dropped: {e}");
                    None
                }
            })
            .collect())
    }

    /// On the primary, after biometrics: hands the primary role to the viewer that asked (`id`); `device_id` is the
    /// phone the user saw in [`Engine::handover_requests`] and approved, which the request must name and sign.
    /// This phone becomes a viewer and drops the owner seed and the keys that moved.
    pub async fn handover_approve(&self, id: &str, device_id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let _one = self.roles.changing.lock().await;
        let account = self.roles_account(&session).await?;
        let request = api_call!(&session, |api| api.handover_request(id)).map_err(ApiFailure::into_core)?;
        let (roster, seed) = self.as_primary(&session, &account).await?;
        let now = unix_now();
        let shown = verified_handover(&roster, &account.id, &request, now)?;
        if shown.device_id != device_id {
            return Err(CoreError::invalid("That request is from another phone than the one shown."));
        }
        let nonce = request.signed.as_ref().map(|s| (s.nonce.clone(), s.at)).unwrap_or_default();
        if self.answered_nonces(&account.slot, now)?.contains_key(&nonce.0) {
            return Err(CoreError::invalid("That request was answered already."));
        }
        let current = roster.current().clone();
        let new_primary =
            current.viewers.iter().find(|v| v.device_id == shown.device_id).cloned().ok_or(CoreError::NotFound)?;
        let mut viewers: Vec<Member> =
            current.viewers.iter().filter(|v| v.device_id != new_primary.device_id).cloned().collect();
        viewers.push(current.primary.clone());
        let mut entry = current
            .successor(RosterReason::Handover, new_primary, viewers, unix_now())
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        // A new wrap, so the one from before the handover opens for no roster after it.
        let wrap_key = self.wrap_key(&account.slot)?;
        let ring = self.keyring(&account.slot)?;
        let wrap = Self::rewrap_entry(&account, &mut entry, &wrap_key, &seed, &ring)?;
        entry.sig = seed.owner_key()?.sign_b64(&entry.signing_bytes().map_err(|e| CoreError::invalid(e.to_string()))?);
        let mut device_secrets = Vec::new();
        for service in MOVING_SECRETS {
            for (acct, value) in self.store.secrets_of(service)? {
                device_secrets.push(CarriedSecret {
                    service: (*service).to_owned(),
                    account: acct,
                    value: BASE64URL_NOPAD.encode(&value),
                });
            }
        }
        let payload = BundlePayload {
            v: PROTOCOL_VERSION,
            owner_seed: BASE64URL_NOPAD.encode(seed.as_bytes()),
            view_keys: ring.entries(),
            account_secret: self.secret_of(&account.id)?.map(|s| BASE64URL_NOPAD.encode(s.as_bytes())),
            device_secrets,
            wrap_key: Some(BASE64URL_NOPAD.encode(&wrap_key[..])),
        };
        let bundle = sealed::seal_bundle(&self.sign_key()?, &account.id, &entry, &payload)?;
        drop(payload);
        let append = RosterAppend {
            entry,
            recovery_wrap: Some(wrap),
            bundle: Some(bundle),
            master_password_hash: None,
        };
        // Committed (or seen later on a fetch), this phone is a viewer: what moved with the role goes then
        // (`reconcile_roles`; Q1 = B: it keeps the account secret and vault). Unconfirmed, it keeps everything.
        let committed = self.commit_entry(&session, &account, &roster, &append).await;
        if !matches!(committed, Err(Commit::Refused(_))) {
            self.answer_nonce(&account.slot, nonce, now)?;
        }
        committed.map_err(Commit::into_error)?;
        self.roles.handovers.lock().unwrap_or_else(PoisonError::into_inner).retain(|h| h.id != id);
        Ok(())
    }

    /// On the primary: declines a viewer's request to decide.
    pub async fn handover_deny(&self, id: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        // The primary answers handover requests; on a roster account no other phone does.
        let roster =
            self.local_roster(&account.slot)?.ok_or_else(|| CoreError::invalid("This account has no phone roles."))?;
        if self.standing_in(&roster)? != Standing::Primary {
            return Err(CoreError::ViewerOnly);
        }
        let kept =
            self.roles.handovers.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|h| h.id == id).cloned();
        let request = match kept {
            Some(request) => Some(request),
            None => api_call!(&session, |api| api.handover_request(id)).ok(),
        };
        // Declined once, the same signed request is never shown again (a replay within its time).
        if let Some(signed) = request.and_then(|r| r.signed) {
            self.answer_nonce(&account.slot, (signed.nonce, signed.at), unix_now())?;
        }
        match api_call!(&session, |api| api.deny_handover(id)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404 | 409,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        self.roles.handovers.lock().unwrap_or_else(PoisonError::into_inner).retain(|h| h.id != id);
        Ok(())
    }

    /// Whether this phone keeps the signed-in account's owner seed (tests: the old primary drops it).
    #[doc(hidden)]
    pub async fn holds_owner_seed(&self) -> Result<bool, CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        Ok(self.owner_seed(&account.slot)?.is_some())
    }

    /// The view key ids and a digest of the recovery wrap's key this phone keeps for the signed-in account (tests: a
    /// refused change leaves them as they were).
    #[doc(hidden)]
    pub async fn held_roles_keys(&self) -> Result<(Vec<u64>, Option<String>), CoreError> {
        let session = self.session()?;
        let account = self.roles_account(&session).await?;
        let wrap = self.store.device_secret_get(WRAP_KEY, &account.slot)?;
        let digest = wrap.map(|k| reins_proto::canon::hex32(&reins_proto::canon::sha256(&k)));
        Ok((self.keyring(&account.slot)?.kids(), digest))
    }

    // ---- signed answers -----------------------------------------------------------------------------------------

    /// `answer` (the exact body an account without a roster sends) signed for item `id` of `kind`, when this phone
    /// is the primary of a roster; `None` on a legacy account, where the body goes as it is. A phone that holds a
    /// roster but is not its primary sends nothing signed (the server refuses the answer).
    pub(crate) async fn signed_answer(
        &self,
        session: &Session,
        kind: AnswerKind,
        id: &str,
        answer: &[u8],
    ) -> Result<Option<SignedAnswer>, CoreError> {
        let account = self.roles_account(session).await?;
        let Some(roster) = self.local_roster(&account.slot)? else {
            return Ok(None);
        };
        if self.standing_in(&roster)? != Standing::Primary {
            return Ok(None);
        }
        Ok(Some(sealed::sign_answer(&self.sign_key()?, &account.id, kind, id, roster.epoch(), answer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_codes_are_told_from_passwords() {
        let code = AccountSecret::generate().unwrap().recovery_code();
        assert!(looks_like_recovery_code(&code));
        assert!(looks_like_recovery_code(&code.replace('-', " ")));
        // A typo still looks like a code: refused before it is sent, never spent as a password.
        assert!(looks_like_recovery_code(&code[..code.len() - 2]));
        assert!(!looks_like_recovery_code("correct horse battery staple"));
        assert!(!looks_like_recovery_code("hunter2-hunter2"));
    }

    #[test]
    fn member_names_are_one_clean_line() {
        assert_eq!(member_name("  Pixel 8  "), "Pixel 8");
        assert_eq!(member_name("Pixel\n8"), "Pixel 8");
        assert_eq!(member_name("Pixel   8"), "Pixel 8");
        assert_eq!(member_name("Pixel\u{202E}9"), "Pixel9");
        assert_eq!(member_name("\u{200B}"), "Phone");
        assert_eq!(member_name(&"x".repeat(100)).chars().count(), 64);
        let m = Member {
            device_id: "d".to_owned(),
            name: member_name("Pixel\u{202E}\t9 "),
            sign_pub: SigningKey::from_seed(&[1; 32]).unwrap().public_b64(),
            box_pub: BoxKey::from_bytes([2; 32]).public_b64(),
        };
        m.validate().unwrap();
    }
}
