//! OAuth 2.0 Device Authorization Grant (RFC 8628) for the desktop app: the computer shows a QR code, the phone scans
//! it, and the phone's answer logs the computer in. No browser and nothing to type.
//!
//! 1. The desktop app asks for a grant (`POST /reins/oauth/device_authorization`) and gets a secret device code, a
//!    short user code (`BCDF-GHJK`, also in the QR code's link) and a two-digit number to show.
//! 2. The phone of the account (its approval device) claims the user code (`POST /reins/api/pairings/claim`). That
//!    starts an ordinary pairing ([`PairingHub`]) for the phone's account, with the number the computer shows among
//!    the three choices, and the desktop app's key to compare and pin.
//! 3. The phone answers the pairing as always (A6): the right number, a name, biometrics. The connection is created.
//! 4. The desktop app polls the token endpoint with the device code; once the pairing is approved, the single poll that
//!    takes the grant gets the tokens.
//!
//! Everything is in memory for at most [`DEVICE_GRANT_TTL`], like the rest of the relay. Device codes are stored only
//! as SHA-256 hashes.

use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rand::RngExt;
use reins_proto::{
    ids::{ConnectionId, PairingId},
    pairing::{PairingRequest, USER_CODE_ALPHABET, USER_CODE_LEN, normalize_user_code},
};
use tokio::time::Instant;

use super::{
    pairing::{PairingClient, PairingHub, PairingStatus, generate_choices},
    ttl::{Full, TtlMap},
};
use crate::auth::reins::{hash_token, random_token};

/// `grant_type` of the token request that redeems a device code.
pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// How long a device code and its user code are valid (the pairing it starts lives as long again at most).
pub const DEVICE_GRANT_TTL: Duration = Duration::from_secs(600);
/// Seconds the desktop app waits between polls; each `slow_down` adds [`SLOW_DOWN_STEP`] (RFC 8628 §3.5).
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);
pub const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);
/// Upper bound on grants held in memory.
pub const MAX_DEVICE_GRANTS: usize = 5_000;

/// A fresh user code, `BCDF-GHJK`: eight letters of [`USER_CODE_ALPHABET`].
pub fn generate_user_code() -> String {
    let alphabet: Vec<char> = USER_CODE_ALPHABET.chars().collect();
    let mut rng = rand::rng();
    let letters: String = (0..USER_CODE_LEN).map(|_| alphabet[rng.random_range(0..alphabet.len())]).collect();
    normalize_user_code(&letters).unwrap_or(letters)
}

/// What the desktop app is told when its grant starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedGrant {
    /// Secret: only the app that asked knows it.
    pub device_code: String,
    pub user_code: String,
    /// The number the app shows, which the user taps on the phone among three.
    pub confirm_code: u8,
}

/// The phone that claimed a user code, and the pairing that started for it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Claim {
    user: String,
    pairing: PairingId,
}

struct Grant {
    client: PairingClient,
    user_code: String,
    code: u8,
    choices: [u8; 3],
    interval: Duration,
    last_poll: Option<Instant>,
    claim: Option<Claim>,
}

/// Why a claim failed.
#[derive(Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// Unknown, expired, or no longer waiting (answered or used).
    NotFound,
    /// Another account's phone claimed it first.
    Taken,
    /// The pairing could not be started (the hub is full).
    Busy,
}

/// The answer to one poll of the token endpoint (RFC 8628 §3.5).
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    /// `authorization_pending`: nobody has answered yet.
    Pending,
    /// `slow_down`: polled sooner than the interval; the interval grew to this.
    SlowDown {
        interval: Duration,
    },
    /// The pairing was approved: issue tokens for this connection. The grant is gone; the next poll is `Expired`.
    Approved {
        user: String,
        connection: ConnectionId,
    },
    /// `access_denied`: denied on the phone (or the wrong number was tapped).
    Denied,
    /// `expired_token`: unknown, expired or already used.
    Expired,
    /// `invalid_grant`: the device code belongs to another client.
    WrongClient,
}

pub struct DeviceGrants {
    grants: Mutex<TtlMap<String, Grant>>,
}

impl DeviceGrants {
    pub fn new() -> Self {
        Self::with_capacity(MAX_DEVICE_GRANTS)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            grants: Mutex::new(TtlMap::new(DEVICE_GRANT_TTL, capacity)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TtlMap<String, Grant>> {
        self.grants.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a grant for `client` (its name and key are what the phone will show).
    pub fn start(&self, client: PairingClient) -> Result<StartedGrant, Full> {
        let mut grants = self.lock();
        grants.purge();
        // 20^8 codes: a collision with a live one is all but impossible, but cheap to rule out.
        let user_code = loop {
            let candidate = generate_user_code();
            if !grants.values().any(|g| g.user_code == candidate) {
                break candidate;
            }
        };
        let (code, choices) = generate_choices();
        let device_code = random_token();
        grants.insert(
            hash_token(&device_code),
            Grant {
                client,
                user_code: user_code.clone(),
                code,
                choices,
                interval: POLL_INTERVAL,
                last_poll: None,
                claim: None,
            },
        )?;
        Ok(StartedGrant {
            device_code,
            user_code,
            confirm_code: code,
        })
    }

    /// The phone of `user` scanned `user_code`: starts the pairing for it (marked delivered, so it is not pushed or
    /// listed again) and returns it. Claiming again from the same phone returns the same pairing while it is open.
    pub fn claim(
        &self,
        user: &str,
        user_code: &str,
        pairings: &PairingHub,
        now_unix: i64,
    ) -> Result<PairingRequest, ClaimError> {
        let code = normalize_user_code(user_code).ok_or(ClaimError::NotFound)?;
        let mut grants = self.lock();
        let grant = grants.values_mut().find(|g| g.user_code == code).ok_or(ClaimError::NotFound)?;
        if let Some(claim) = &grant.claim {
            if claim.user != user {
                return Err(ClaimError::Taken);
            }
            return pairings.fetch(user, &claim.pairing).ok_or(ClaimError::NotFound);
        }
        let request = pairings
            .start_claimed(user, grant.client.clone(), grant.code, grant.choices, now_unix)
            .map_err(|Full| ClaimError::Busy)?;
        grant.claim = Some(Claim {
            user: user.to_owned(),
            pairing: request.id.clone(),
        });
        Ok(request)
    }

    /// One poll of the token endpoint with `device_code` by `client_id`.
    pub fn poll(&self, device_code: &str, client_id: &str, pairings: &PairingHub) -> Poll {
        let key = hash_token(device_code);
        let mut grants = self.lock();
        let Some(grant) = grants.get_mut(&key) else {
            return Poll::Expired;
        };
        if grant.client.client_id != client_id {
            return Poll::WrongClient;
        }
        let now = Instant::now();
        let too_soon = grant.last_poll.is_some_and(|last| now.duration_since(last) < grant.interval);
        grant.last_poll = Some(now);
        if too_soon {
            grant.interval += SLOW_DOWN_STEP;
            return Poll::SlowDown {
                interval: grant.interval,
            };
        }
        let Some(claim) = grant.claim.clone() else {
            return Poll::Pending;
        };
        let outcome = match pairings.status(&claim.pairing) {
            Some(PairingStatus::Waiting) => return Poll::Pending,
            Some(PairingStatus::Approved {
                connection_id,
            }) => Poll::Approved {
                user: claim.user,
                connection: connection_id,
            },
            Some(PairingStatus::Rejected) => Poll::Denied,
            None => Poll::Expired,
        };
        // Only the poll that removes the grant gets the outcome: approved tokens are issued once.
        grants.remove(&key);
        outcome
    }

    pub fn purge(&self) {
        self.lock().purge();
    }
}

impl Default for DeviceGrants {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide device grants.
pub static DEVICE_GRANTS: LazyLock<DeviceGrants> = LazyLock::new(DeviceGrants::new);

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use reins_proto::pairing::PairingResponse;

    use super::*;
    use crate::api::reins::{
        pairing::{PairingAnswer, PairingAnswerError},
        relay::ItemSignal,
    };

    fn hub() -> PairingHub {
        PairingHub::new(Arc::new(ItemSignal::new()))
    }

    fn desktop() -> PairingClient {
        PairingClient {
            client_id: "desktop-client".to_owned(),
            client_name: "Reins desktop app on mac".to_owned(),
            client_host: "127.0.0.1".to_owned(),
            client_key: Some(reins_proto::desktop::encode_key(&[7u8; 32])),
        }
    }

    fn answer(code: u8) -> PairingResponse {
        PairingResponse {
            v: 1,
            approved: true,
            chosen_code: Some(code),
            label: None,
        }
    }

    /// Polls after the interval has passed.
    async fn poll(grants: &DeviceGrants, device_code: &str, pairings: &PairingHub) -> Poll {
        tokio::time::advance(POLL_INTERVAL).await;
        grants.poll(device_code, "desktop-client", pairings)
    }

    #[test]
    fn user_codes_are_eight_consonants_in_two_groups() {
        let mut seen = HashSet::new();
        for _ in 0..500 {
            let code = generate_user_code();
            assert_eq!(code.len(), 9, "{code}");
            assert_eq!(&code[4..5], "-");
            assert!(code.chars().filter(|c| *c != '-').all(|c| USER_CODE_ALPHABET.contains(c)), "{code}");
            seen.insert(code);
        }
        assert!(seen.len() > 490, "codes must vary");
    }

    #[tokio::test(start_paused = true)]
    async fn the_phone_claims_the_code_and_its_approval_logs_the_desktop_in_once() {
        let (grants, pairings) = (DeviceGrants::new(), hub());
        let started = grants.start(desktop()).unwrap();
        assert_ne!(started.device_code, started.user_code);
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), Poll::Pending);

        // Scanned: an ordinary pairing for the phone's account, with the computer's number among the choices.
        let scanned = started.user_code.to_lowercase().replace('-', " ");
        let request = grants.claim("u1", &scanned, &pairings, 5).unwrap();
        assert!(request.choices.contains(&started.confirm_code));
        assert_eq!(request.client_key, desktop().client_key);
        assert_eq!((request.client_name.as_str(), request.created_at), ("Reins desktop app on mac", 5));
        assert!(pairings.take_undelivered("u1").is_empty(), "the phone has it already: no second delivery");
        assert_eq!(grants.claim("u1", &started.user_code, &pairings, 6).unwrap().id, request.id, "scanned twice");
        assert_eq!(poll(&grants, &started.device_code, &pairings).await, Poll::Pending);

        let PairingAnswer::Approved {
            client,
            ..
        } = pairings.answer("u1", &request.id, &answer(started.confirm_code)).unwrap()
        else {
            panic!("not approved")
        };
        assert_eq!(client.client_id, "desktop-client");
        assert_eq!(poll(&grants, &started.device_code, &pairings).await, Poll::Pending, "storing the connection");
        pairings.complete(&request.id, "conn-1".into());
        assert_eq!(
            poll(&grants, &started.device_code, &pairings).await,
            Poll::Approved {
                user: "u1".to_owned(),
                connection: "conn-1".into()
            }
        );
        assert_eq!(poll(&grants, &started.device_code, &pairings).await, Poll::Expired, "tokens are issued once");
        assert_eq!(grants.claim("u1", &started.user_code, &pairings, 7), Err(ClaimError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn a_denial_or_the_wrong_number_is_access_denied() {
        let (grants, pairings) = (DeviceGrants::new(), hub());
        let started = grants.start(desktop()).unwrap();
        let request = grants.claim("u1", &started.user_code, &pairings, 0).unwrap();
        let wrong = *request.choices.iter().find(|c| **c != started.confirm_code).unwrap();
        assert_eq!(pairings.answer("u1", &request.id, &answer(wrong)), Ok(PairingAnswer::WrongCode));
        assert_eq!(poll(&grants, &started.device_code, &pairings).await, Poll::Denied);
        assert_eq!(poll(&grants, &started.device_code, &pairings).await, Poll::Expired);
    }

    #[tokio::test(start_paused = true)]
    async fn only_one_account_can_claim_a_code() {
        let (grants, pairings) = (DeviceGrants::new(), hub());
        let started = grants.start(desktop()).unwrap();
        let request = grants.claim("u1", &started.user_code, &pairings, 0).unwrap();
        assert_eq!(grants.claim("u2", &started.user_code, &pairings, 0), Err(ClaimError::Taken));
        assert_eq!(
            pairings.answer("u2", &request.id, &answer(started.confirm_code)),
            Err(PairingAnswerError::NotFound)
        );
        for bad in ["", "BCDF-GHJ", "not a code", "ABCD-EFGH"] {
            assert_eq!(grants.claim("u1", bad, &pairings, 0), Err(ClaimError::NotFound), "{bad}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn polling_too_fast_slows_the_desktop_down() {
        let (grants, pairings) = (DeviceGrants::new(), hub());
        let started = grants.start(desktop()).unwrap();
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), Poll::Pending);
        let slower = |secs| Poll::SlowDown {
            interval: Duration::from_secs(secs),
        };
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), slower(10));
        tokio::time::advance(POLL_INTERVAL).await;
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), slower(15), "10 s now");
        tokio::time::advance(Duration::from_secs(15)).await;
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), Poll::Pending);
    }

    #[tokio::test(start_paused = true)]
    async fn the_device_code_works_only_for_its_client_and_for_ten_minutes() {
        let (grants, pairings) = (DeviceGrants::new(), hub());
        let started = grants.start(desktop()).unwrap();
        assert_eq!(grants.poll(&started.device_code, "someone-else", &pairings), Poll::WrongClient);
        assert_eq!(grants.poll("a-guess", "desktop-client", &pairings), Poll::Expired);
        tokio::time::advance(DEVICE_GRANT_TTL).await;
        assert_eq!(grants.poll(&started.device_code, "desktop-client", &pairings), Poll::Expired);
        assert_eq!(grants.claim("u1", &started.user_code, &pairings, 0), Err(ClaimError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_server_says_so() {
        let (grants, pairings) = (DeviceGrants::new(), PairingHub::with_capacity(Arc::new(ItemSignal::new()), 1));
        let first = grants.start(desktop()).unwrap();
        let second = grants.start(desktop()).unwrap();
        grants.claim("u1", &first.user_code, &pairings, 0).unwrap();
        assert_eq!(grants.claim("u1", &second.user_code, &pairings, 0), Err(ClaimError::Busy), "the hub is full");
        let capped = DeviceGrants::with_capacity(1);
        capped.start(desktop()).unwrap();
        assert!(capped.start(desktop()).is_err());
    }
}
