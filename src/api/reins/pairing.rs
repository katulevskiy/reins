//! AI-connection pairing (spec §4.5 step 2-3, contracts A5/A6): the browser shows a
//! two-digit code, the phone picks it among three choices.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use rand::RngExt;
use reins_proto::{
    PROTOCOL_VERSION,
    ids::{ConnectionId, PairingId},
    pairing::{PairingRequest, PairingResponse},
};

use super::{ITEM_TTL, relay::ItemSignal, ttl::TtlMap};
use crate::util::get_uuid;

pub const CODE_MIN: u8 = 10;
pub const CODE_MAX: u8 = 99;
/// Longest client name or connection label kept (characters).
pub const MAX_NAME_CHARS: usize = 64;
/// Upper bound on pairings (including decoys) held in memory. It never refuses a party that is under
/// [`MAX_PER_PARTY`] while someone else holds more, and a decoy is dropped before any real pairing.
pub const MAX_PAIRINGS: usize = 1_000;
/// Open pairings one account may have at once. Decoys for an unknown email share this number, per client address
/// ([`super::limits::client_network`]). Above the default pairing burst of 5, so one burst still fits, and far below
/// [`MAX_PAIRINGS`], so one party cannot fill the map. Evicting a pending approval would drop it silently, so a party
/// at this cap is refused alone instead.
pub const MAX_PER_PARTY: usize = 8;
pub const UNKNOWN_CLIENT: &str = "Unknown client";

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
    /// `None` for decoys (unknown email): no phone ever sees them.
    user: Option<String>,
    /// Set for a decoy: the requester's address, so one address cannot fill the map with unknown emails.
    decoy_party: Option<String>,
    client: PairingClient,
    code: u8,
    request: PairingRequest,
    delivered: bool,
    stage: Stage,
}

/// Why a pairing was not started.
#[derive(Debug, PartialEq, Eq)]
pub enum StartError {
    /// This account, or this address's decoys, already has [`MAX_PER_PARTY`] open. Nobody else is affected.
    Party,
    /// The map is full of this party's own entries, or of approvals already in progress. Another party is not
    /// refused for that: its start drops a decoy, or the oldest open pairing of whoever holds the most.
    Busy,
}

/// Who a new pairing is limited as.
enum Slot<'a> {
    User(&'a str),
    /// Unknown email. `party` is the requester's network, not the email (which is not stored).
    Decoy(&'a str),
}

fn party_limited(map: &TtlMap<PairingId, PairingEntry>, slot: &Slot<'_>) -> bool {
    let held = match slot {
        Slot::User(user) => map.values().filter(|e| e.user.as_deref() == Some(*user)).count(),
        Slot::Decoy(party) => map.values().filter(|e| e.decoy_party.as_deref() == Some(*party)).count(),
    };
    held >= MAX_PER_PARTY
}

/// The party in `counts` with strictly more than `mine` entries, the fullest such. Ties are arbitrary.
fn fullest_over(counts: HashMap<String, usize>, mine: usize) -> Option<String> {
    counts.into_iter().filter(|(_, n)| *n > mine).max_by_key(|(_, n)| *n).map(|(party, _)| party)
}

/// Makes one free slot for `slot`. A real pairing drops a decoy first (a decoy can never be approved), then the
/// oldest still-open pairing of another account that holds more open pairings than this one. An approval already
/// being stored, or one the browser has not collected yet, is kept. A decoy drops only another address's decoy, and
/// only when that address holds more of them.
fn make_room(map: &mut TtlMap<PairingId, PairingEntry>, slot: &Slot<'_>) -> bool {
    match slot {
        Slot::User(user) => {
            if map.evict_where(|e| e.user.is_none()) {
                return true;
            }
            let mine =
                map.values().filter(|e| e.user.as_deref() == Some(*user) && matches!(e.stage, Stage::Open)).count();
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
            map.evict_where(|e| e.user.as_deref() == Some(hog.as_str()) && matches!(e.stage, Stage::Open))
        }
        Slot::Decoy(party) => {
            let mine = map.values().filter(|e| e.decoy_party.as_deref() == Some(*party)).count();
            let mut counts: HashMap<String, usize> = HashMap::new();
            for entry in map.values() {
                if let Some(owner) = entry.decoy_party.as_deref()
                    && owner != *party
                {
                    *counts.entry(owner.to_owned()).or_default() += 1;
                }
            }
            let Some(hog) = fullest_over(counts, mine) else {
                return false;
            };
            map.evict_where(|e| e.decoy_party.as_deref() == Some(hog.as_str()))
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
        let (user, decoy_party) = match slot {
            Slot::User(user) => (Some((*user).to_owned()), None),
            Slot::Decoy(party) => (None, Some((*party).to_owned())),
        };
        let entry = PairingEntry {
            user,
            decoy_party,
            client,
            code,
            request: request.clone(),
            delivered,
            stage: Stage::Open,
        };
        let mut map = self.lock();
        map.purge();
        if party_limited(&map, slot) {
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

    /// A pairing `user`'s phone asked for itself (it scanned a computer's code, see `device_flow`): `code` is the
    /// number the computer shows, among `choices`. It is delivered already, so it is neither pushed nor listed again.
    pub fn start_claimed(
        &self,
        user: &str,
        client: PairingClient,
        code: u8,
        choices: [u8; 3],
        now_unix: i64,
    ) -> Result<PairingRequest, StartError> {
        self.insert_with(&Slot::User(user), client, code, choices, true, now_unix)
    }

    /// Starts a pairing for `user`'s approval device; returns it and the browser code.
    pub fn start(&self, user: &str, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), StartError> {
        let started = self.insert(&Slot::User(user), client, now_unix)?;
        self.signal.notify();
        Ok(started)
    }

    /// Indistinguishable from [`PairingHub::start`] for the browser, but bound to no user (Decision 19). `party` is
    /// the requester's address ([`super::limits::client_network`]), not the email, which is not stored.
    pub fn start_decoy(
        &self,
        client: PairingClient,
        party: &str,
        now_unix: i64,
    ) -> Result<(PairingRequest, u8), StartError> {
        self.insert(&Slot::Decoy(party), client, now_unix)
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
}
