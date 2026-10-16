//! This phone's view of its account's roster: the entries it verified, from a trust anchor it pinned.
//!
//! The anchor is the first entry this phone trusts without the server: the genesis it signed itself (`create`), the
//! entry whose owner key and primary signing key the enrollment code covered (a viewer), or the entry its own
//! recovery signed. Every later entry must chain to it ([`reins_proto::roster::verify_step`]: the epoch is the next,
//! `prev_hash` names the entry before, the owner key in force signed it, and the reason's rules hold). A fetched roster
//! that lacks the anchor, ends before the newest entry this phone verified, or differs from it at any epoch this phone
//! has seen is refused: a rollback or a fork.

use reins_proto::canon::hex32;
use reins_proto::roster::{Member, RosterEntry, RosterError, RosterReason, verify_step};
use serde::{Deserialize, Serialize};

use super::keys::ed25519_verify;
use crate::CoreError;

/// The roster as this phone verified it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRoster {
    /// The server's id of the account.
    pub account: String,
    /// The pinned anchor's hash (hex); its epoch is `entries[0].epoch`.
    pub anchor_hash: String,
    /// Verified entries, from the anchor to the newest, ascending.
    pub entries: Vec<RosterEntry>,
}

/// What this phone is in its account's roster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Primary,
    Viewer,
    /// Listed before, not now (removed, or replaced by a recovery).
    Removed,
}

fn roster_error(e: &RosterError) -> CoreError {
    match e {
        RosterError::BadSignature => CoreError::invalid("The account's roster is not signed by its owner key"),
        other => CoreError::invalid(format!("The account's roster does not verify: {other}")),
    }
}

impl LocalRoster {
    /// A roster anchored at `anchor`, which this phone trusts already (it signed it, or a code covered its keys).
    pub fn anchored(anchor: RosterEntry) -> Result<Self, CoreError> {
        anchor.validate().map_err(|e| CoreError::invalid(e.to_string()))?;
        Ok(Self {
            account: anchor.account.clone(),
            anchor_hash: anchor.hash_hex().map_err(|e| CoreError::invalid(e.to_string()))?,
            entries: vec![anchor],
        })
    }

    /// A genesis this phone signed: verified like any first entry.
    pub fn created(genesis: RosterEntry) -> Result<Self, CoreError> {
        verify_step(None, &genesis, ed25519_verify).map_err(|e| roster_error(&e))?;
        Self::anchored(genesis)
    }

    pub fn current(&self) -> &RosterEntry {
        self.entries.last().expect("a local roster has its anchor")
    }

    pub fn epoch(&self) -> u64 {
        self.current().epoch
    }

    pub fn at(&self, epoch: u64) -> Option<&RosterEntry> {
        self.entries.iter().find(|e| e.epoch == epoch)
    }

    /// What `device_id`, with signing key `sign_pub`, is now.
    pub fn standing(&self, device_id: &str, sign_pub: &str) -> Standing {
        let current = self.current();
        if current.primary.device_id == device_id && current.primary.sign_pub == sign_pub {
            Standing::Primary
        } else if current.viewers.iter().any(|v| v.device_id == device_id && v.sign_pub == sign_pub) {
            Standing::Viewer
        } else {
            Standing::Removed
        }
    }

    /// The signing key of the primary at `epoch`, if this phone verified that epoch.
    pub fn primary_key_at(&self, epoch: u64) -> Option<[u8; 32]> {
        self.at(epoch).and_then(|e| e.primary.sign_key().ok())
    }

    /// The key that may sign a view-key grant of key id `kid`: the current primary's (of the newest entry this phone
    /// verified), and only for a key id an entry that brought a view key names (`create`, `remove`, `recovery`). A
    /// replaced primary grants nothing, and no one grants a key id no such entry names
    /// ([`super::sealed::open_view_grant`]).
    pub fn view_key_signer(&self, kid: u64) -> Option<[u8; 32]> {
        let entry = self.at(kid)?;
        matches!(entry.reason, RosterReason::Create | RosterReason::Remove | RosterReason::Recovery)
            .then(|| self.current().primary.sign_key().ok())
            .flatten()
    }

    /// Adds `next` (an entry this phone just made, or one fetched): it must follow the newest entry.
    pub fn push(&mut self, next: RosterEntry) -> Result<(), CoreError> {
        verify_step(Some(self.current()), &next, ed25519_verify).map_err(|e| roster_error(&e))?;
        self.entries.push(next);
        Ok(())
    }

    /// The roster after a fetch of the whole chain (`fetched`, oldest first): refused unless it holds the anchor and
    /// every entry this phone verified, unchanged, and each newer entry follows from them.
    pub fn updated(&self, fetched: &[RosterEntry]) -> Result<Self, CoreError> {
        let anchor_epoch = self.entries[0].epoch;
        let start = fetched
            .iter()
            .position(|e| e.epoch == anchor_epoch)
            .ok_or_else(|| CoreError::invalid("The server's roster lacks the entry this phone trusts (a rollback)"))?;
        let tail = &fetched[start..];
        if tail.len() < self.entries.len() {
            return Err(CoreError::invalid(format!(
                "The server's roster ends at epoch {}, before epoch {} this phone saw (a rollback)",
                tail.last().map_or(0, |e| e.epoch),
                self.epoch()
            )));
        }
        for (seen, got) in self.entries.iter().zip(tail) {
            if seen != got {
                return Err(CoreError::invalid(format!(
                    "The server's roster differs at epoch {} from the one this phone verified (a fork)",
                    seen.epoch
                )));
            }
        }
        let mut next = self.clone();
        for entry in &tail[self.entries.len()..] {
            next.push(entry.clone())?;
        }
        Ok(next)
    }

    /// A short fingerprint of the anchor's owner key, for people to compare ("a1b2 c3d4").
    pub fn owner_fingerprint(&self) -> String {
        let key = reins_proto::canon::sha256(self.current().owner_pub.as_bytes());
        let hex = hex32(&key);
        format!("{} {}", &hex[..4], &hex[4..8])
    }

    /// The member `device_id` of the current entry.
    pub fn member(&self, device_id: &str) -> Option<&Member> {
        self.current().member(device_id)
    }
}

/// A chain fetched by a phone that pins `owner_pub` and `primary_sign_pub` at `epoch` (the enrollment code covered
/// them): the roster anchored there, once a later entry signed by that owner key confirms the anchor. Earlier entries
/// are checked too, but trusted for nothing.
pub fn anchored_by_code(
    fetched: &[RosterEntry],
    epoch: u64,
    owner_pub: &str,
    primary_sign_pub: &str,
) -> Result<LocalRoster, CoreError> {
    let start = fetched
        .iter()
        .position(|e| e.epoch == epoch)
        .ok_or_else(|| CoreError::invalid("The server's roster lacks the epoch the other phone named"))?;
    let anchor = &fetched[start];
    if anchor.owner_pub != owner_pub || anchor.primary.sign_pub != primary_sign_pub {
        return Err(CoreError::invalid("The server's roster names other keys than the ones the code covered"));
    }
    if fetched.len() <= start + 1 {
        return Err(CoreError::invalid("The roster does not list this phone yet"));
    }
    let mut local = LocalRoster::anchored(anchor.clone())?;
    // The next entry is signed by the owner key the code covered, over the anchor's hash: that is what makes the
    // anchor (which the server served) trustworthy.
    for entry in &fetched[start + 1..] {
        local.push(entry.clone())?;
    }
    Ok(local)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::roles::keys::{OwnerSeed, SigningKey};

    pub(crate) fn member(id: &str, n: u8) -> Member {
        Member {
            device_id: id.to_owned(),
            name: format!("Phone {id}"),
            sign_pub: SigningKey::from_seed(&[n; 32]).unwrap().public_b64(),
            box_pub: crate::roles::keys::BoxKey::from_bytes([n.wrapping_add(100); 32]).public_b64(),
        }
    }

    pub(crate) fn sign(mut entry: RosterEntry, owner: &OwnerSeed) -> RosterEntry {
        entry.sig = owner.owner_key().unwrap().sign_b64(&entry.signing_bytes().unwrap());
        entry
    }

    pub(crate) fn genesis(owner: &OwnerSeed) -> RosterEntry {
        sign(
            RosterEntry::genesis(
                "acct",
                member("p", 1),
                owner.owner_key().unwrap().public_b64(),
                owner.credential_key().public_b64(),
                hex32(&[1; 32]),
                10,
            ),
            owner,
        )
    }

    pub(crate) fn next(
        prev: &RosterEntry,
        reason: RosterReason,
        primary: Member,
        viewers: Vec<Member>,
        owner: &OwnerSeed,
    ) -> RosterEntry {
        let mut entry = prev.successor(reason, primary, viewers, 20).unwrap();
        if reason.brings_wrap() {
            entry.recovery_wrap_hash = hex32(&[u8::try_from(entry.epoch).unwrap(); 32]);
        }
        sign(entry, owner)
    }

    #[test]
    fn a_chain_verifies_from_the_anchor_and_names_each_phones_standing() {
        let owner = OwnerSeed::generate().unwrap();
        let g = genesis(&owner);
        let mut local = LocalRoster::created(g.clone()).unwrap();
        assert_eq!(local.standing("p", &member("p", 1).sign_pub), Standing::Primary);
        let e2 = next(&g, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], &owner);
        let e3 = next(&e2, RosterReason::Handover, member("v", 2), vec![member("p", 1)], &owner);
        local = local.updated(&[g.clone(), e2.clone(), e3.clone()]).unwrap();
        assert_eq!(local.epoch(), 3);
        assert_eq!(local.standing("v", &member("v", 2).sign_pub), Standing::Primary);
        assert_eq!(local.standing("p", &member("p", 1).sign_pub), Standing::Viewer);
        assert_eq!(local.standing("p", &member("x", 9).sign_pub), Standing::Removed, "another key");
        assert_eq!(local.primary_key_at(2), Some(member("p", 1).sign_key().unwrap()));
        assert_eq!(local.primary_key_at(9), None);
        // Only the current primary grants, and only key ids an entry that brought a view key names: genesis yes, an
        // enrollment or handover no. The replaced primary grants nothing, not even the key its own entry brought.
        assert_eq!(local.view_key_signer(1), Some(member("v", 2).sign_key().unwrap()));
        assert_ne!(local.view_key_signer(1), Some(member("p", 1).sign_key().unwrap()));
        assert_eq!(local.view_key_signer(2), None);
        assert_eq!(local.view_key_signer(3), None);
        assert_eq!(local.view_key_signer(9), None);
        // The same chain again changes nothing.
        assert_eq!(local.updated(&[g, e2, e3]).unwrap(), local);
    }

    #[test]
    fn a_rollback_a_fork_a_forgery_or_a_missing_anchor_is_refused() {
        let owner = OwnerSeed::generate().unwrap();
        let g = genesis(&owner);
        let e2 = next(&g, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], &owner);
        let local = LocalRoster::created(g.clone()).unwrap().updated(&[g.clone(), e2.clone()]).unwrap();
        // Rollback: the chain ends before what this phone saw.
        assert!(local.updated(std::slice::from_ref(&g)).is_err());
        // Fork: another entry at epoch 2 (signed by the real owner key, even).
        let fork = next(&g, RosterReason::Enroll, member("p", 1), vec![member("w", 3)], &owner);
        assert!(local.updated(&[g.clone(), fork]).is_err());
        // Forgery: an entry the owner key did not sign.
        let stranger = OwnerSeed::generate().unwrap();
        let forged = next(&e2, RosterReason::Enroll, member("p", 1), vec![member("v", 2), member("w", 3)], &stranger);
        assert!(local.updated(&[g.clone(), e2.clone(), forged]).is_err());
        // A whole other chain (another owner's genesis): no anchor.
        let other = genesis(&stranger);
        assert!(local.updated(&[other]).is_err());
        // A tampered entry this phone saw.
        let mut renamed = e2.clone();
        renamed.viewers[0].name = "Evil".to_owned();
        assert!(local.updated(&[g.clone(), renamed]).is_err());
        // An entry that breaks its reason's rules, even signed by the owner.
        let mut sneaky =
            e2.successor(RosterReason::Enroll, member("q", 4), vec![member("v", 2), member("w", 3)], 30).unwrap();
        sneaky = sign(sneaky, &owner);
        assert!(local.updated(&[g, e2, sneaky]).is_err());
        // A genesis not signed by its own owner key.
        let mut bad = genesis(&owner);
        bad.sig = stranger.owner_key().unwrap().sign_b64(&bad.signing_bytes().unwrap());
        assert!(LocalRoster::created(bad).is_err());
    }

    #[test]
    fn a_code_anchors_the_chain_at_the_keys_it_covered() {
        let owner = OwnerSeed::generate().unwrap();
        let g = genesis(&owner);
        let e2 = next(&g, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], &owner);
        let owner_pub = owner.owner_key().unwrap().public_b64();
        let primary = member("p", 1).sign_pub;
        let local = anchored_by_code(&[g.clone(), e2.clone()], 1, &owner_pub, &primary).unwrap();
        assert_eq!((local.entries[0].epoch, local.epoch()), (1, 2));
        // Other keys than the code covered, no entry after the anchor, or no such epoch: refused.
        let other = OwnerSeed::generate().unwrap().owner_key().unwrap().public_b64();
        assert!(anchored_by_code(&[g.clone(), e2.clone()], 1, &other, &primary).is_err());
        assert!(anchored_by_code(&[g.clone(), e2.clone()], 1, &owner_pub, &member("x", 9).sign_pub).is_err());
        assert!(anchored_by_code(std::slice::from_ref(&g), 1, &owner_pub, &primary).is_err());
        assert!(anchored_by_code(&[g, e2], 5, &owner_pub, &primary).is_err());
        // A server that made its own chain naming the covered owner key at the anchor cannot sign what follows.
        let fake_owner = OwnerSeed::generate().unwrap();
        let mut fake = RosterEntry::genesis(
            "acct",
            member("p", 1),
            owner_pub.clone(),
            owner.credential_key().public_b64(),
            hex32(&[1; 32]),
            10,
        );
        fake.sig = fake_owner.owner_key().unwrap().sign_b64(&fake.signing_bytes().unwrap());
        let fake_next = next(&fake, RosterReason::Enroll, member("p", 1), vec![member("v", 2)], &fake_owner);
        assert!(anchored_by_code(&[fake, fake_next], 1, &owner_pub, &primary).is_err());
    }
}
