//! AI-connection pairing (spec §4.5 step 2-3, contracts A5/A6): the browser shows a
//! two-digit code, the phone picks it among three choices.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rand::RngExt;
use reins_proto::{
    PROTOCOL_VERSION,
    ids::{ConnectionId, PairingId},
    pairing::{PairingRequest, PairingResponse},
};

use super::{
    ITEM_TTL,
    relay::ItemSignal,
    ttl::{Full, TtlMap},
};
use crate::util::get_uuid;

pub const CODE_MIN: u8 = 10;
pub const CODE_MAX: u8 = 99;
/// Longest client name or connection label kept (characters).
pub const MAX_NAME_CHARS: usize = 64;
/// Upper bound on pairings (including decoys) held in memory.
pub const MAX_PAIRINGS: usize = 1_000;
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
    client: PairingClient,
    code: u8,
    request: PairingRequest,
    delivered: bool,
    stage: Stage,
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

    fn insert(&self, user: Option<String>, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full> {
        let (code, choices) = generate_choices();
        self.insert_with(user, client, code, choices, false, now_unix).map(|request| (request, code))
    }

    fn insert_with(
        &self,
        user: Option<String>,
        client: PairingClient,
        code: u8,
        choices: [u8; 3],
        delivered: bool,
        now_unix: i64,
    ) -> Result<PairingRequest, Full> {
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
        let entry = PairingEntry {
            user,
            client,
            code,
            request: request.clone(),
            delivered,
            stage: Stage::Open,
        };
        self.lock().insert(request.id.clone(), entry)?;
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
    ) -> Result<PairingRequest, Full> {
        self.insert_with(Some(user.to_owned()), client, code, choices, true, now_unix)
    }

    /// Starts a pairing for `user`'s approval device; returns it and the browser code.
    pub fn start(&self, user: &str, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full> {
        let started = self.insert(Some(user.to_owned()), client, now_unix)?;
        self.signal.notify();
        Ok(started)
    }

    /// Indistinguishable from `start` for the browser, but bound to no user (Decision 19).
    pub fn start_decoy(&self, client: PairingClient, now_unix: i64) -> Result<(PairingRequest, u8), Full> {
        self.insert(None, client, now_unix)
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
        let (decoy, _) = h.start_decoy(desktop, 0).unwrap();
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
        let (req, code) = h.start_decoy(client(), 0).unwrap();
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
}
