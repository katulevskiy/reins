//! AI-connection pairing (spec §4.5 step 2-3, contracts A5/A6): the browser shows a
//! two-digit code, the phone picks it among three choices.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError},
};

use data_encoding::HEXLOWER;
use rand::RngExt;
use reins_proto::{
    PROTOCOL_VERSION,
    ids::{ConnectionId, PairingId},
    pairing::{PairingRequest, PairingResponse},
};
use ring::hmac;

use super::{ITEM_TTL, relay::ItemSignal, ttl::TtlMap};
use crate::util::get_uuid;

pub const CODE_MIN: u8 = 10;
pub const CODE_MAX: u8 = 99;
/// Longest client name or connection label kept (characters).
pub const MAX_NAME_CHARS: usize = 64;
/// Upper bound on pairings (including decoys) held in memory. It never refuses a party that is under
/// [`MAX_PER_PARTY`] while someone else holds more, and a decoy is dropped before any real pairing.
pub const MAX_PAIRINGS: usize = 1_000;
/// Open pairings one email may have at once, real and decoy together, and the most one client network may hold for
/// every email together. Above the default pairing burst of 5, so one burst still fits, and far below
/// [`MAX_PAIRINGS`]. A network that already holds this many is refused before the account is looked up. A network
/// that holds none of an email's slots can still take the oldest open, undelivered one from another network: evicting
/// an approval already in progress would drop it silently, so those stay.
pub const MAX_PER_PARTY: usize = 8;
pub const UNKNOWN_CLIENT: &str = "Unknown client";
/// Network stored on a pairing the phone claimed itself (device flow). It is not a client address.
const CLAIMED_NETWORK: &str = "device";

/// Key a normalised email shares between a real pairing and a decoy, so the two cannot be told apart by which cap
/// fills up. The email is not recoverable from it. Stable for this process; pairings do not outlive it.
pub fn email_cap_key(email: &str) -> String {
    static SECRET: LazyLock<[u8; 32]> = LazyLock::new(crate::crypto::get_random_bytes);
    let key = hmac::Key::new(hmac::HMAC_SHA256, SECRET.as_slice());
    HEXLOWER.encode(hmac::sign(&key, email.trim().to_lowercase().as_bytes()).as_ref())
}

/// Returns the browser code and three distinct codes in `CODE_MIN..=CODE_MAX`, in random
/// order, exactly one of which is the browser code.
pub fn generate_choices() -> (u8, [u8; 3]) {
    let mut rng = rand::rng();
    let mut picked: Vec<u8> = Vec::with_capacity(3);
    while picked.len() < 3 {
        let candidate = rng.random_range(CODE_MIN..=CODE_MAX);
        if !picked.contains(&candidate) {
            picked.push(candidate);
        }
    }
    let code = picked[rng.random_range(0..3)];
    (code, [picked[0], picked[1], picked[2]])
}

/// Control, bidi-override, isolate and zero-width characters: never shown to the user.
fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Makes untrusted display text safe to show: whitespace collapsed to single spaces,
/// invisible characters removed, at most `max_chars` characters. HTML is escaped by the renderer.
pub fn sanitize_display(raw: &str, max_chars: usize) -> String {
    let visible: String = raw
        .chars()
        .map(|c| {
            if c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !is_invisible(*c))
        .collect();
    visible.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max_chars).collect()
}

/// A client-declared name as shown on the phone and stored with the connection.
pub fn sanitize_client_name(raw: &str) -> String {
    let name = sanitize_display(raw, MAX_NAME_CHARS);
    if name.is_empty() {
        UNKNOWN_CLIENT.to_owned()
    } else {
        name
    }
}

/// Server-verified facts about the AI client, fixed when the pairing starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingClient {
    pub client_id: String,
    pub client_name: String,
    /// Host of the validated redirect URI.
    pub client_host: String,
    /// The desktop app's public key (`reins_client_key`, checked with `desktop::decode_key`), relayed so the phone
    /// can show its fingerprint and pin it to the new connection.
    pub client_key: Option<String>,
}

/// What the browser's wait page sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingStatus {
    Waiting,
    Approved {
        connection_id: ConnectionId,
    },
    /// Denied, wrong code, or the connection could not be stored.
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingAnswer {
    /// Create the connection, then call [`PairingHub::complete`] (or [`PairingHub::fail`]).
    Approved {
        client: PairingClient,
        label: String,
    },
    Denied,
    WrongCode,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingAnswerError {
    /// Unknown, expired, a decoy, or another user's pairing.
    NotFound,
    AlreadyAnswered,
    /// Malformed answer (400); the pairing stays open.
    Invalid(String),
}

enum Stage {
    Open,
    /// Approved with the right code; the connection is being stored.
    Answering,
    Done(PairingStatus),
}

struct PairingEntry {
    /// `None` for a decoy (unknown email): no phone ever sees it.
    user: Option<String>,
    /// [`email_cap_key`] of the submitted email, or the account id when the phone claimed the pairing itself.
    cap_key: String,
    /// Client network the browser came from, or [`CLAIMED_NETWORK`] for a pairing the phone claimed itself.
    network: String,
    client: PairingClient,
    code: u8,
    request: PairingRequest,
    delivered: bool,
    stage: Stage,
}

/// Why a pairing was not started.
#[derive(Debug, PartialEq, Eq)]
pub enum StartError {
    /// This network, or this email on this network, already holds [`MAX_PER_PARTY`], or the email is full and no
    /// open, undelivered pairing of another network can be dropped. The browser still sees the ordinary wait page.
    Party,
    /// The map is full of this party's own entries, or of approvals already in progress. Another party is not
    /// refused for that: its start drops a decoy, or the oldest open pairing of whoever holds the most.
    Busy,
}

/// Who a new pairing is limited as.
#[derive(Clone, Copy)]
enum Slot<'a> {
    /// A browser pairing. `user` is `None` for a decoy. `cap_key` is shared by the real pairing and the decoy.
    Email {
        user: Option<&'a str>,
        cap_key: &'a str,
        network: &'a str,
    },
    /// The phone scanned a computer's code. Already delivered, so another network cannot evict it.
    Claimed {
        user: &'a str,
    },
}

fn is_open_undelivered(entry: &PairingEntry) -> bool {
    !entry.delivered && matches!(entry.stage, Stage::Open)
}

/// Drops the oldest open, undelivered entry for which `same` holds, from a network other than `network` that holds
/// strictly more of them than `network` does. Answering and done pairings, and ones already handed to the phone, stay.
fn evict_other_network(
    map: &mut TtlMap<PairingId, PairingEntry>,
    network: &str,
    same: impl Fn(&PairingEntry) -> bool,
) -> bool {
    let mine = map.values().filter(|entry| entry.network == network && same(entry)).count();
    let mut held: HashMap<String, usize> = HashMap::new();
    for entry in map.values() {
        if entry.network != network && same(entry) {
            *held.entry(entry.network.clone()).or_default() += 1;
        }
    }
    map.evict_where(|entry| {
        same(entry)
            && entry.network != network
            && is_open_undelivered(entry)
            && held.get(&entry.network).is_some_and(|count| *count > mine)
    })
}

/// Brings this email, and the account when there is one, back under [`MAX_PER_PARTY`] by dropping another network's
/// open, undelivered pairing. `false` when the only way under the cap would drop an approval already in progress,
/// a pairing the phone already has, or a pairing of a network that does not hold more than this one.
fn release_email_cap(
    map: &mut TtlMap<PairingId, PairingEntry>,
    cap_key: &str,
    network: &str,
    user: Option<&str>,
) -> bool {
    loop {
        let key_over = map.values().filter(|entry| entry.cap_key == cap_key).count() >= MAX_PER_PARTY;
        let user_over = user
            .is_some_and(|id| map.values().filter(|entry| entry.user.as_deref() == Some(id)).count() >= MAX_PER_PARTY);
        if !key_over && !user_over {
            return true;
        }
        if key_over && evict_other_network(map, network, |entry| entry.cap_key == cap_key) {
            continue;
        }
        if user_over && evict_other_network(map, network, |entry| entry.user.as_deref() == user) {
            continue;
        }
        return false;
    }
}

/// The party in `counts` with strictly more than `mine` entries, the fullest such. Ties are arbitrary.
fn fullest_over(counts: HashMap<String, usize>, mine: usize) -> Option<String> {
    counts.into_iter().filter(|(_, n)| *n > mine).max_by_key(|(_, n)| *n).map(|(party, _)| party)
}

/// Makes one free slot for `slot` when the whole map is full. A real pairing drops a decoy first (a decoy can never
/// be approved), then the oldest still-open pairing of another account that holds more open pairings than this one.
/// An approval already being stored, or one the browser has not collected yet, is kept. A decoy drops only another
/// network's decoy, and only when that network holds more of them.
fn make_room(map: &mut TtlMap<PairingId, PairingEntry>, slot: &Slot<'_>) -> bool {
    match slot {
        Slot::Email {
            user: None,
            network,
            ..
        } => {
            let mine = map.values().filter(|entry| entry.user.is_none() && entry.network == *network).count();
            let mut counts: HashMap<String, usize> = HashMap::new();
            for entry in map.values() {
                if entry.user.is_none() && entry.network != *network {
                    *counts.entry(entry.network.clone()).or_default() += 1;
                }
            }
            let Some(hog) = fullest_over(counts, mine) else {
                return false;
            };
            map.evict_where(|entry| entry.user.is_none() && entry.network == hog)
        }
        Slot::Email {
            user: Some(user),
            ..
        }
        | Slot::Claimed {
            user,
        } => {
            if map.evict_where(|entry| entry.user.is_none()) {
                return true;
            }
            let mine = map
                .values()
                .filter(|entry| entry.user.as_deref() == Some(*user) && matches!(entry.stage, Stage::Open))
                .count();
            let mut counts: HashMap<String, usize> = HashMap::new();
            for entry in map.values() {
                if let Some(owner) = entry.user.as_deref()
                    && owner != *user
                    && matches!(entry.stage, Stage::Open)
                {
                    *counts.entry(owner.to_owned()).or_default() += 1;
                }
            }
            let Some(hog) = fullest_over(counts, mine) else {
                return false;
            };
            map.evict_where(|entry| entry.user.as_deref() == Some(hog.as_str()) && matches!(entry.stage, Stage::Open))
        }
    }
}

enum Verdict {
    Approve {
        label: Option<String>,
    },
    Deny,
    WrongCode,
}

/// Decision 22: approval needs a chosen code among the choices; a valid but wrong choice cancels.
fn judge_response(response: &PairingResponse, code: u8, choices: [u8; 3]) -> Result<Verdict, String> {
    if !response.approved {
        return Ok(Verdict::Deny);
    }
    let Some(chosen) = response.chosen_code else {
        return Err("chosen_code is required when approved".to_owned());
    };
    if !choices.contains(&chosen) {
        return Err(format!("chosen_code {chosen} is not one of the offered choices"));
    }
    if chosen != code {
        return Ok(Verdict::WrongCode);
    }
    let label = response.label.as_deref().map(|l| sanitize_display(l, MAX_NAME_CHARS)).filter(|l| !l.is_empty());
    Ok(Verdict::Approve {
        label,
    })
}

pub struct PairingHub {
    signal: Arc<ItemSignal>,
    pairings: Mutex<TtlMap<PairingId, PairingEntry>>,
}

impl PairingHub {
    pub fn new(signal: Arc<ItemSignal>) -> Self {
        Self::with_capacity(signal, MAX_PAIRINGS)
    }

    pub fn with_capacity(signal: Arc<ItemSignal>, capacity: usize) -> Self {
        Self {
            signal,
            pairings: Mutex::new(TtlMap::new(ITEM_TTL, capacity)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TtlMap<PairingId, PairingEntry>> {
        self.pairings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn insert(
        &self,
        slot: &Slot<'_>,
        client: PairingClient,
        now_unix: i64,
    ) -> Result<(PairingRequest, u8), StartError> {
        let (code, choices) = generate_choices();
        self.insert_with(slot, client, code, choices, false, now_unix).map(|request| (request, code))
    }

    fn insert_with(
        &self,
        slot: &Slot<'_>,
        client: PairingClient,
        code: u8,
        choices: [u8; 3],
        delivered: bool,
        now_unix: i64,
    ) -> Result<PairingRequest, StartError> {
        let client = PairingClient {
            client_name: sanitize_client_name(&client.client_name),
            ..client
        };
        let request = PairingRequest {
            v: PROTOCOL_VERSION,
            id: PairingId(get_uuid()),
            client_name: client.client_name.clone(),
            client_host: client.client_host.clone(),
            choices,
            created_at: now_unix,
            client_key: client.client_key.clone(),
        };
        let (user, cap_key, network) = match *slot {
            Slot::Email {
                user,
                cap_key,
                network,
            } => (user.map(str::to_owned), cap_key.to_owned(), network.to_owned()),
            Slot::Claimed {
                user,
            } => (Some(user.to_owned()), user.to_owned(), CLAIMED_NETWORK.to_owned()),
        };
        let entry = PairingEntry {
            user,
            cap_key,
            network,
            client,
            code,
            request: request.clone(),
            delivered,
            stage: Stage::Open,
        };
        let mut map = self.lock();
        map.purge();
        if !Self::admit(&mut map, slot) {
            return Err(StartError::Party);
        }
        // A party under its cap is never refused because the map is full of someone else. A decoy never takes the
        // place of a real pairing; a real pairing drops a decoy before it drops anyone's open request.
        if !map.has_room() && !make_room(&mut map, slot) {
            return Err(StartError::Busy);
        }
        map.insert(request.id.clone(), entry).map_err(|_| StartError::Busy)?;
        Ok(request)
    }

    /// `false` when this party is at its cap and nothing evictable on another network can make room. The caller
    /// reports that as [`StartError::Party`].
    fn admit(map: &mut TtlMap<PairingId, PairingEntry>, slot: &Slot<'_>) -> bool {
        match *slot {
            Slot::Email {
                user,
                cap_key,
                network,
            } => {
                let network_full = map.values().filter(|entry| entry.network == network).count() >= MAX_PER_PARTY;
                let key_here =
                    map.values().filter(|entry| entry.cap_key == cap_key && entry.network == network).count()
                        >= MAX_PER_PARTY;
                if network_full || key_here {
                    return false;
                }
                release_email_cap(map, cap_key, network, user)
            }
            Slot::Claimed {
                user,
            } => map.values().filter(|entry| entry.user.as_deref() == Some(user)).count() < MAX_PER_PARTY,
        }
    }

    /// Whether another pairing for `cap_key` from `network` would be refused before an account lookup. A full email
    /// whose slots all sit on other networks is not capped: that request may still evict one of them.
    pub fn capped_for(&self, cap_key: &str, network: &str) -> bool {
        let mut map = self.lock();
        map.purge();
        let network_full = map.values().filter(|entry| entry.network == network).count() >= MAX_PER_PARTY;
        let key_here =
            map.values().filter(|entry| entry.cap_key == cap_key && entry.network == network).count() >= MAX_PER_PARTY;
        network_full || key_here
    }

    /// A pairing `user`'s phone asked for itself (it scanned a computer's code, see `device_flow`): `code` is the
    /// number the computer shows, among `choices`. It is delivered already, so it is neither pushed nor listed again,
    /// and another network cannot evict it.
    pub fn start_claimed(
        &self,
        user: &str,
        client: PairingClient,
        code: u8,
        choices: [u8; 3],
        now_unix: i64,
    ) -> Result<PairingRequest, StartError> {
        self.insert_with(
            &Slot::Claimed {
                user,
            },
            client,
            code,
            choices,
            true,
            now_unix,
        )
    }

    /// Starts a pairing limited as `cap_key` (an [`email_cap_key`], or an account id) from `network`. `user` is the
    /// account whose phone may approve it, or `None` for a decoy. Notifies the phone only for a real pairing.
    pub fn start_email(
        &self,
        cap_key: &str,
        network: &str,
        user: Option<&str>,
        client: PairingClient,
        now_unix: i64,
    ) -> Result<(PairingRequest, u8), StartError> {
        let started = self.insert(
            &Slot::Email {
                user,
                cap_key,
                network,
            },
            client,
            now_unix,
        )?;
        if user.is_some() {
            self.signal.notify();
        }
        Ok(started)
    }

    /// Starts a pairing for `user`'s approval device; returns it and the browser code. The account id is the cap
    /// key and the network, which is how callers that do not know a client address stay per account. The HTTP flow
    /// uses [`PairingHub::start_email`].
    #[cfg(test)]
    pub fn start(&self, user: &str, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), StartError> {
        self.start_email(user, user, Some(user), client, now_unix)
    }

    /// Indistinguishable from [`PairingHub::start`] for the browser, but bound to no user (Decision 19). `party` is
    /// both the cap key and the requester's address ([`super::limits::client_network`]), not the email, which is
    /// not stored. The HTTP flow uses [`PairingHub::start_email`] with [`email_cap_key`] instead, so a decoy shares
    /// the email's cap.
    #[cfg(test)]
    pub fn start_decoy(
        &self,
        client: PairingClient,
        party: &str,
        now_unix: i64,
    ) -> Result<(PairingRequest, u8), StartError> {
        self.start_email(party, party, None, client, now_unix)
    }

    /// Open pairings of `user` never handed out before, oldest first; marks them delivered.
    pub fn take_undelivered(&self, user: &str) -> Vec<PairingRequest> {
        let mut map = self.lock();
        let ids: Vec<PairingId> = map
            .values()
            .filter(|e| e.user.as_deref() == Some(user) && !e.delivered && matches!(e.stage, Stage::Open))
            .map(|e| e.request.id.clone())
            .collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(entry) = map.get_mut(&id) {
                entry.delivered = true;
                out.push(entry.request.clone());
            }
        }
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// One open pairing of `user`; marks it delivered.
    pub fn fetch(&self, user: &str, id: &PairingId) -> Option<PairingRequest> {
        let mut map = self.lock();
        let entry = map.get_mut(id).filter(|e| e.user.as_deref() == Some(user) && matches!(e.stage, Stage::Open))?;
        entry.delivered = true;
        Some(entry.request.clone())
    }

    /// Applies the phone's answer (Decision 22). Only the first valid answer counts.
    pub fn answer(
        &self,
        user: &str,
        id: &PairingId,
        response: &PairingResponse,
    ) -> Result<PairingAnswer, PairingAnswerError> {
        let mut map = self.lock();
        let entry = map.get_mut(id).filter(|e| e.user.as_deref() == Some(user)).ok_or(PairingAnswerError::NotFound)?;
        if !matches!(entry.stage, Stage::Open) {
            return Err(PairingAnswerError::AlreadyAnswered);
        }
        let verdict =
            judge_response(response, entry.code, entry.request.choices).map_err(PairingAnswerError::Invalid)?;
        entry.delivered = true;
        Ok(match verdict {
            Verdict::Deny => {
                entry.stage = Stage::Done(PairingStatus::Rejected);
                PairingAnswer::Denied
            }
            Verdict::WrongCode => {
                entry.stage = Stage::Done(PairingStatus::Rejected);
                PairingAnswer::WrongCode
            }
            Verdict::Approve {
                label,
            } => {
                entry.stage = Stage::Answering;
                PairingAnswer::Approved {
                    label: label.unwrap_or_else(|| entry.client.client_name.clone()),
                    client: entry.client.clone(),
                }
            }
        })
    }

    /// Records the connection created for an approved pairing.
    pub fn complete(&self, id: &PairingId, connection_id: ConnectionId) {
        if let Some(entry) = self.lock().get_mut(id)
            && matches!(entry.stage, Stage::Answering)
        {
            entry.stage = Stage::Done(PairingStatus::Approved {
                connection_id,
            });
        }
    }

    /// The approved pairing could not be completed (e.g. database error).
    pub fn fail(&self, id: &PairingId) {
        if let Some(entry) = self.lock().get_mut(id) {
            entry.stage = Stage::Done(PairingStatus::Rejected);
        }
    }

    /// Status for the browser; `None` once expired or unknown.
    pub fn status(&self, id: &PairingId) -> Option<PairingStatus> {
        self.lock().get(id).map(|e| match &e.stage {
            Stage::Open | Stage::Answering => PairingStatus::Waiting,
            Stage::Done(status) => status.clone(),
        })
    }

    pub fn purge(&self) {
        self.lock().purge();
    }

    /// Drops the pairings waiting for `user`'s phone (a deleted account).
    pub fn forget_user(&self, user: &str) {
        self.lock().retain(|e| e.user.as_deref() != Some(user));
    }

    /// Open pairings of `user`, oldest first, without marking them delivered (what a viewer shows).
    pub fn peek_waiting(&self, user: &str) -> Vec<PairingRequest> {
        let mut out: Vec<PairingRequest> = self
            .lock()
            .values()
            .filter(|e| e.user.as_deref() == Some(user) && matches!(e.stage, Stage::Open))
            .map(|e| e.request.clone())
            .collect();
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// Ids of `user`'s answered pairings still held.
    pub fn answered_ids(&self, user: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .values()
            .filter(|e| e.user.as_deref() == Some(user) && !matches!(e.stage, Stage::Open))
            .map(|e| e.request.id.0.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Hands `user`'s open pairings to the next `GET /pending` again (a new primary took the role).
    pub fn requeue(&self, user: &str) {
        for e in self.lock().values_mut().filter(|e| e.user.as_deref() == Some(user) && matches!(e.stage, Stage::Open))
        {
            e.delivered = false;
        }
        self.signal.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use super::*;

    fn hub() -> PairingHub {
        PairingHub::new(Arc::new(ItemSignal::new()))
    }

    fn client() -> PairingClient {
        PairingClient {
            client_id: "https://chatgpt.com/oauth/client.json".to_owned(),
            client_name: "Chat\u{202E}GPT".to_owned(),
            client_host: "chatgpt.com".to_owned(),
            client_key: None,
        }
    }

    fn approve(code: u8, label: Option<&str>) -> PairingResponse {
        PairingResponse {
            v: 1,
            approved: true,
            chosen_code: Some(code),
            label: label.map(str::to_owned),
        }
    }

    fn other_choice(req: &PairingRequest, code: u8) -> u8 {
        *req.choices.iter().find(|c| **c != code).unwrap()
    }

    #[test]
    fn choices_are_three_distinct_two_digit_codes_containing_the_code() {
        let mut positions = HashSet::new();
        let mut codes = HashSet::new();
        for _ in 0..2000 {
            let (code, choices) = generate_choices();
            assert!(choices.iter().all(|c| (CODE_MIN..=CODE_MAX).contains(c)), "{choices:?}");
            assert!(choices[0] != choices[1] && choices[1] != choices[2] && choices[0] != choices[2], "{choices:?}");
            positions.insert(choices.iter().position(|c| *c == code).expect("code offered"));
            codes.insert(code);
        }
        assert_eq!(positions.len(), 3, "the code must not always sit in the same slot");
        assert!(codes.len() > 50, "codes must vary");
    }

    #[test]
    fn sanitize_strips_invisible_and_bidi_characters() {
        assert_eq!(sanitize_client_name("Chat\u{202E}GPT"), "ChatGPT");
        assert_eq!(sanitize_client_name("Cl\u{200B}au\u{FEFF}de\u{2066}"), "Claude");
        assert_eq!(sanitize_client_name("  Evil \n\t  Bot\u{7}  "), "Evil Bot");
        assert_eq!(sanitize_client_name("\u{061C}\u{200E}\u{200F}"), UNKNOWN_CLIENT);
        assert_eq!(sanitize_client_name(""), UNKNOWN_CLIENT);
        assert_eq!(sanitize_client_name(&"x".repeat(10_000)).chars().count(), MAX_NAME_CHARS);
        // HTML is kept here and escaped where it is rendered (pages.rs).
        assert_eq!(sanitize_client_name("<b>Bot</b>"), "<b>Bot</b>");
    }

    #[tokio::test(start_paused = true)]
    async fn start_sanitizes_and_offers_the_code() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 7).unwrap();
        assert_eq!(req.client_name, "ChatGPT");
        assert_eq!(req.client_host, "chatgpt.com");
        assert!(req.choices.contains(&code));
        assert_eq!((req.v, req.created_at), (1, 7));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn the_desktop_key_reaches_real_and_decoy_pairings() {
        let h = hub();
        let key = reins_proto::desktop::encode_key(&[7u8; 32]);
        let desktop = PairingClient {
            client_key: Some(key.clone()),
            ..client()
        };
        let (req, _) = h.start("u1", desktop.clone(), 0).unwrap();
        assert_eq!(req.client_key.as_deref(), Some(key.as_str()));
        assert_eq!(h.take_undelivered("u1")[0].client_key.as_deref(), Some(key.as_str()));
        let (decoy, _) = h.start_decoy(desktop, "203.0.113.9", 0).unwrap();
        assert_eq!(decoy.client_key, Some(key), "a decoy must look exactly like a real pairing");
        assert_eq!(h.start("u1", client(), 0).unwrap().0.client_key, None);
    }

    #[tokio::test(start_paused = true)]
    async fn delivery_is_per_user_and_one_shot() {
        let h = hub();
        let (req, _) = h.start("u1", client(), 0).unwrap();
        assert_eq!(h.take_undelivered("u2").len(), 0);
        assert_eq!(h.take_undelivered("u1").len(), 1);
        assert_eq!(h.take_undelivered("u1").len(), 0);
        assert!(h.fetch("u2", &req.id).is_none());
        assert_eq!(h.fetch("u1", &req.id).unwrap().id, req.id);
    }

    #[tokio::test(start_paused = true)]
    async fn correct_code_approves_exactly_once() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let answer = h.answer("u1", &req.id, &approve(code, Some(" Work\u{200B} GPT "))).unwrap();
        let PairingAnswer::Approved {
            client,
            label,
        } = answer
        else {
            panic!("expected approval, got {answer:?}")
        };
        assert_eq!(label, "Work GPT");
        assert_eq!(client.client_name, "ChatGPT");
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting), "still storing the connection");
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::AlreadyAnswered));
        h.complete(&req.id, "conn-1".into());
        assert_eq!(
            h.status(&req.id),
            Some(PairingStatus::Approved {
                connection_id: "conn-1".into()
            })
        );
        assert!(h.fetch("u1", &req.id).is_none(), "answered pairings are no longer offered");
    }

    #[tokio::test(start_paused = true)]
    async fn label_defaults_to_the_client_name() {
        let h = hub();
        for label in [None, Some("   \u{200B} ")] {
            let (req, code) = h.start("u1", client(), 0).unwrap();
            match h.answer("u1", &req.id, &approve(code, label)).unwrap() {
                PairingAnswer::Approved {
                    label,
                    ..
                } => assert_eq!(label, "ChatGPT"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wrong_code_cancels_the_pairing() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let wrong = other_choice(&req, code);
        assert_eq!(h.answer("u1", &req.id, &approve(wrong, None)), Ok(PairingAnswer::WrongCode));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::AlreadyAnswered));
    }

    #[tokio::test(start_paused = true)]
    async fn malformed_answers_leave_the_pairing_open() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        let mut no_code = approve(code, None);
        no_code.chosen_code = None;
        assert!(matches!(h.answer("u1", &req.id, &no_code), Err(PairingAnswerError::Invalid(_))));
        let not_offered = (CODE_MIN..=CODE_MAX).find(|c| !req.choices.contains(c)).unwrap();
        assert!(matches!(h.answer("u1", &req.id, &approve(not_offered, None)), Err(PairingAnswerError::Invalid(_))));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
        assert!(matches!(h.answer("u1", &req.id, &approve(code, None)), Ok(PairingAnswer::Approved { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn deny_rejects_and_other_users_cannot_answer() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        assert_eq!(h.answer("u2", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
        let deny = PairingResponse {
            v: 1,
            approved: false,
            chosen_code: None,
            label: None,
        };
        assert_eq!(h.answer("u1", &req.id, &deny), Ok(PairingAnswer::Denied));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
    }

    #[tokio::test(start_paused = true)]
    async fn fail_rejects_an_approved_pairing() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        h.answer("u1", &req.id, &approve(code, None)).unwrap();
        h.fail(&req.id);
        assert_eq!(h.status(&req.id), Some(PairingStatus::Rejected));
    }

    #[tokio::test(start_paused = true)]
    async fn decoys_look_real_but_nobody_can_see_or_answer_them() {
        let h = hub();
        let (req, code) = h.start_decoy(client(), "203.0.113.9", 0).unwrap();
        assert!(req.choices.contains(&code));
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting));
        assert!(h.take_undelivered("").is_empty());
        assert!(h.take_undelivered("u1").is_empty());
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn pairings_expire_after_ten_minutes() {
        let h = hub();
        let (req, code) = h.start("u1", client(), 0).unwrap();
        tokio::time::advance(Duration::from_secs(600)).await;
        assert_eq!(h.status(&req.id), None);
        assert_eq!(h.answer("u1", &req.id, &approve(code, None)), Err(PairingAnswerError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn one_account_cannot_fill_pairings_for_everyone() {
        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), MAX_PER_PARTY + 4);
        let (first, _) = h.start("attacker", client(), 1).unwrap();
        for n in 1..MAX_PER_PARTY {
            h.start("attacker", client(), i64::try_from(n).unwrap_or(0)).unwrap();
        }
        assert_eq!(h.start("attacker", client(), 99).unwrap_err(), StartError::Party);
        let (victim, _) = h.start("victim", client(), 100).unwrap();
        assert_eq!(h.status(&victim.id), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&first.id), Some(PairingStatus::Waiting), "refusing the attacker drops nothing of theirs");
    }

    #[tokio::test(start_paused = true)]
    async fn one_address_of_unknown_emails_cannot_block_another_or_a_real_pairing() {
        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), MAX_PER_PARTY + 4);
        for _ in 0..MAX_PER_PARTY {
            h.start_decoy(client(), "203.0.113.1", 0).unwrap();
        }
        assert_eq!(h.start_decoy(client(), "203.0.113.1", 0).unwrap_err(), StartError::Party);
        let (other, _) = h.start_decoy(client(), "203.0.113.2", 0).unwrap();
        let (real, _) = h.start("victim", client(), 0).unwrap();
        assert_eq!(h.status(&other.id), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&real.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_map_gives_a_new_account_the_fullest_accounts_oldest_open_pairing() {
        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), 2);
        let (oldest, _) = h.start("attacker", client(), 1).unwrap();
        let (newer, _) = h.start("attacker", client(), 2).unwrap();
        assert_eq!(h.start("attacker", client(), 3).unwrap_err(), StartError::Busy, "it does not drop its own");
        assert_eq!(h.status(&oldest.id), Some(PairingStatus::Waiting));
        let (victim, _) = h.start("victim", client(), 4).unwrap();
        assert_eq!(h.status(&oldest.id), None, "the account that held the most loses its oldest");
        assert_eq!(h.status(&newer.id), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&victim.id), Some(PairingStatus::Waiting));
        // Tied with the victim: the attacker cannot take the slot back.
        assert_eq!(h.start("attacker", client(), 5).unwrap_err(), StartError::Busy);
        assert_eq!(h.status(&victim.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn a_decoy_cannot_push_out_a_real_pairing_and_a_real_pairing_can_push_out_a_decoy() {
        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), 1);
        let (real, _) = h.start("victim", client(), 0).unwrap();
        assert_eq!(h.start_decoy(client(), "203.0.113.1", 1).unwrap_err(), StartError::Busy);
        assert_eq!(h.status(&real.id), Some(PairingStatus::Waiting));

        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), 1);
        let (decoy, _) = h.start_decoy(client(), "203.0.113.1", 0).unwrap();
        let (real, _) = h.start("victim", client(), 1).unwrap();
        assert_eq!(h.status(&decoy.id), None);
        assert_eq!(h.status(&real.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn an_approval_in_progress_is_not_dropped_to_make_room() {
        let h = PairingHub::with_capacity(Arc::new(ItemSignal::new()), 1);
        let (req, code) = h.start("attacker", client(), 0).unwrap();
        h.answer("attacker", &req.id, &approve(code, None)).unwrap();
        assert_eq!(h.start("victim", client(), 1).unwrap_err(), StartError::Busy);
        assert_eq!(h.status(&req.id), Some(PairingStatus::Waiting), "still storing the connection");
    }

    #[test]
    fn the_cap_key_follows_the_email_and_does_not_contain_it() {
        assert_eq!(email_cap_key("Ada@Example.com"), email_cap_key("  ada@example.com "));
        assert_ne!(email_cap_key("ada@example.com"), email_cap_key("bob@example.com"));
        let key = email_cap_key("ada@example.com");
        assert_eq!(key.len(), 64, "{key}");
        // A hex digest, so the address cannot appear in it (a check for "ada" would match random hex now and then).
        assert!(key.bytes().all(|b| b.is_ascii_hexdigit()), "{key}");
        assert!(!key.contains('@') && !key.contains("example"), "{key}");
    }

    fn fill_email(h: &PairingHub, key: &str, network: &str, user: Option<&str>) -> Vec<PairingId> {
        let mut ids = Vec::new();
        for n in 0..MAX_PER_PARTY {
            let (req, _) = h.start_email(key, network, user, client(), i64::try_from(n).unwrap_or(0)).unwrap();
            ids.push(req.id);
        }
        ids
    }

    #[tokio::test(start_paused = true)]
    async fn a_ninth_real_pairing_and_a_ninth_decoy_for_one_email_are_refused_alike() {
        let h = hub();
        let key = email_cap_key("ada@example.com");
        fill_email(&h, &key, "203.0.113.1", Some("user"));
        assert_eq!(h.start_email(&key, "203.0.113.1", Some("user"), client(), 9).unwrap_err(), StartError::Party);
        let h = hub();
        fill_email(&h, &key, "203.0.113.1", None);
        assert_eq!(h.start_email(&key, "203.0.113.1", None, client(), 9).unwrap_err(), StartError::Party);
        // Four of each share the one cap.
        let h = hub();
        for n in 0..4 {
            h.start_email(&key, "203.0.113.1", Some("user"), client(), i64::try_from(n).unwrap_or(0)).unwrap();
            h.start_email(&key, "203.0.113.1", None, client(), i64::try_from(n).unwrap_or(0)).unwrap();
        }
        assert_eq!(h.start_email(&key, "203.0.113.1", Some("user"), client(), 9).unwrap_err(), StartError::Party);
        assert_eq!(h.start_email(&key, "203.0.113.1", None, client(), 9).unwrap_err(), StartError::Party);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_network_is_capped_for_every_email_before_another_network_is() {
        let h = hub();
        let key = email_cap_key("ada@example.com");
        let decoys = fill_email(&h, &key, "203.0.113.1", None);
        assert!(h.capped_for(&key, "203.0.113.1"));
        assert!(h.capped_for("any-other-email", "203.0.113.1"), "the network cap does not depend on the email");
        assert!(!h.capped_for(&key, "198.51.100.2"), "another network can still evict");
        assert_eq!(h.start_email(&key, "203.0.113.1", Some("user"), client(), 1).unwrap_err(), StartError::Party);
        let (real, _) = h.start_email(&key, "198.51.100.2", Some("user"), client(), 2).unwrap();
        assert_eq!(h.status(&decoys[0]), None, "a decoy for this email counts toward the same cap");
        assert_eq!(h.status(&decoys[1]), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&real.id), Some(PairingStatus::Waiting));

        let h = hub();
        let decoys = fill_email(&h, &key, "203.0.113.1", None);
        let (other, _) = h.start_email("other-email", "198.51.100.2", Some("user"), client(), 3).unwrap();
        assert_eq!(h.status(&decoys[0]), Some(PairingStatus::Waiting), "another email does not spend this cap");
        assert_eq!(h.status(&other.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn a_flood_from_one_network_does_not_block_the_owner_on_another() {
        let h = hub();
        let key = email_cap_key("ada@example.com");
        let from_a = fill_email(&h, &key, "203.0.113.1", Some("owner"));
        assert_eq!(h.start_email(&key, "203.0.113.1", Some("owner"), client(), 20).unwrap_err(), StartError::Party);
        let (owner, _) = h.start_email(&key, "198.51.100.2", Some("owner"), client(), 21).unwrap();
        assert_eq!(h.status(&from_a[0]), None, "the oldest open pairing from the flooding network is dropped");
        assert_eq!(h.status(&from_a[1]), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&owner.id), Some(PairingStatus::Waiting));
        // The flood still holds more slots than the owner, so it cannot take the owner's slot back.
        assert_eq!(h.start_email(&key, "203.0.113.1", Some("owner"), client(), 22).unwrap_err(), StartError::Party);
        assert_eq!(h.status(&owner.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn an_in_progress_or_finished_pairing_is_not_evicted_for_another_network() {
        let h = hub();
        let key = email_cap_key("ada@example.com");
        let (answering, code) = h.start_email(&key, "203.0.113.1", Some("owner"), client(), 1).unwrap();
        h.answer("owner", &answering.id, &approve(code, None)).unwrap();
        let (done, done_code) = h.start_email(&key, "203.0.113.1", Some("owner"), client(), 2).unwrap();
        h.answer("owner", &done.id, &approve(done_code, None)).unwrap();
        h.complete(&done.id, "conn-1".into());
        let mut open = Vec::new();
        for n in 3..=MAX_PER_PARTY {
            let (req, _) =
                h.start_email(&key, "203.0.113.1", Some("owner"), client(), i64::try_from(n).unwrap_or(0)).unwrap();
            open.push(req.id);
        }
        let (owner, _) = h.start_email(&key, "198.51.100.2", Some("owner"), client(), 20).unwrap();
        assert_eq!(h.status(&answering.id), Some(PairingStatus::Waiting), "answering stays");
        assert_eq!(
            h.status(&done.id),
            Some(PairingStatus::Approved {
                connection_id: "conn-1".into()
            })
        );
        assert_eq!(h.status(&open[0]), None, "the oldest open, undelivered pairing is the one that goes");
        assert_eq!(h.status(&open[1]), Some(PairingStatus::Waiting));
        assert_eq!(h.status(&owner.id), Some(PairingStatus::Waiting));
    }

    #[tokio::test(start_paused = true)]
    async fn a_network_cannot_evict_pairings_the_phone_already_has() {
        let h = hub();
        let key = email_cap_key("ada@example.com");
        let ids = fill_email(&h, &key, "203.0.113.1", Some("owner"));
        assert_eq!(h.take_undelivered("owner").len(), MAX_PER_PARTY);
        assert_eq!(h.start_email(&key, "198.51.100.2", Some("owner"), client(), 20).unwrap_err(), StartError::Party);
        for id in &ids {
            assert_eq!(h.status(id), Some(PairingStatus::Waiting));
        }
    }
}
