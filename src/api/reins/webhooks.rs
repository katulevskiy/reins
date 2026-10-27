//! Ticket webhooks (Decisions v2, 03 §4.4, 08 R9): a connection registers an https URL that proves it is listening
//! by echoing a challenge, and Reins POSTs it the ticket events the **approval device signed**. The server never signs
//! one: an agent verifies the signature against the device's key (`Reins-Key`, also shown by `GET
//! /reins/agent/callback`), so a compromised server can delay or drop events but not fake an approval.

use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use reins_proto::ticket::SignedTicketEvent;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::outbound::{self, OutboundError, allow_loopback, check_url};
use crate::db::{DbConn, models::ReinsCallback};

/// How long the URL has to answer a challenge or an event.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Delays before each delivery attempt.
const ATTEMPTS: [Duration; 3] = [Duration::ZERO, Duration::from_secs(10), Duration::from_secs(60)];
/// Longest answer read from a challenge.
const MAX_CHALLENGE_ANSWER: usize = 1024;

/// Whether `answer` (the body a callback URL returned) echoes `challenge`: the challenge itself, or JSON
/// `{"challenge": "<challenge>"}`.
pub fn echoes(answer: &[u8], challenge: &str) -> bool {
    let text = String::from_utf8_lossy(answer);
    let text = text.trim();
    if text == challenge {
        return true;
    }
    serde_json::from_str::<Value>(text).is_ok_and(|v| v.get("challenge").and_then(Value::as_str) == Some(challenge))
}

/// Checks that `url` may be a callback (https, a public address) and that it echoes a fresh challenge.
pub async fn verify_url(url: &str) -> Result<(), String> {
    let checked = check_url(url, allow_loopback()).map_err(|e| e.to_string())?;
    if checked.scheme() != "https" && !allow_loopback() {
        return Err("The callback must be an https URL.".to_owned());
    }
    let challenge = BASE64URL_NOPAD.encode(&rand::random::<[u8; 32]>());
    let body = json!({"type": "reins.challenge", "challenge": challenge}).to_string().into_bytes();
    let response = outbound::post_once(checked.as_str(), HeaderMap::new(), body, Duration::from_secs(5))
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("The callback answered the challenge with HTTP {}.", response.status().as_u16()));
    }
    let (answer, _) = outbound::read_limited(response, MAX_CHALLENGE_ANSWER).await.map_err(|e| e.to_string())?;
    if echoes(&answer, &challenge) {
        Ok(())
    } else {
        Err("The callback did not echo the challenge.".to_owned())
    }
}

/// The headers of one delivery: the event type, the device's signature and key.
pub fn headers(event: &SignedTicketEvent) -> Option<HeaderMap> {
    let kind = event.event.state.webhook_type()?;
    let mut map = HeaderMap::new();
    for (name, value) in [
        ("reins-event", kind.to_owned()),
        ("reins-signature", format!("ed25519={}", event.sig)),
        ("reins-key", event.key.clone()),
    ] {
        map.insert(HeaderName::from_static(name), HeaderValue::from_str(&value).ok()?);
    }
    Some(map)
}

/// Delivers `event` (signed by the approval device) to `connection`'s callback, if it has one: a few attempts in the
/// background, never logged beyond the failure.
pub async fn fire(connection: &str, event: &SignedTicketEvent, conn: &DbConn) {
    let Some(callback) = ReinsCallback::find(connection, conn).await else {
        return;
    };
    let (Some(headers), Ok(body)) = (headers(event), serde_json::to_vec(event)) else {
        return;
    };
    tokio::spawn(async move {
        for delay in ATTEMPTS {
            tokio::time::sleep(delay).await;
            match outbound::post_once(&callback.url, headers.clone(), body.clone(), TIMEOUT).await {
                Ok(r) if r.status().is_success() => return,
                Ok(_) | Err(OutboundError::Failed(_)) => {}
                Err(OutboundError::Refused(_)) => return,
            }
        }
        debug!("A ticket webhook could not be delivered");
    });
}

#[cfg(test)]
mod tests {
    use reins_proto::{
        ids::ConnectionId,
        ticket::{TicketEvent, TicketState},
    };

    use super::*;

    #[test]
    fn challenge_must_be_echoed() {
        assert!(echoes(b"abc", "abc"));
        assert!(echoes(b" abc\n", "abc"));
        assert!(echoes(br#"{"challenge":"abc"}"#, "abc"));
        assert!(!echoes(b"ok", "abc"));
        assert!(!echoes(br#"{"challenge":"abd"}"#, "abc"));
        assert!(!echoes(b"", "abc"));
    }

    #[test]
    fn only_https_public_urls_are_accepted() {
        for bad in ["http://example.com/hook", "https://127.0.0.1/hook", "https://10.0.0.1/x", "ftp://example.com"] {
            assert!(check_url(bad, false).is_err(), "{bad}");
        }
        assert!(check_url("https://example.com/hook", false).is_ok());
    }

    fn signed(state: TicketState) -> SignedTicketEvent {
        SignedTicketEvent {
            event: TicketEvent {
                v: 1,
                ticket: "rt_1".to_owned(),
                from: TicketState::Deferred,
                state,
                at: 1,
                digest: "d".to_owned(),
                connection_id: ConnectionId("c".to_owned()),
                deadline: None,
                note: None,
                result: None,
                stale: None,
            },
            key: "KEY".to_owned(),
            sig: "SIG".to_owned(),
        }
    }

    #[test]
    fn deliveries_carry_the_device_signature_and_only_for_outcomes() {
        let h = headers(&signed(TicketState::Approved)).unwrap();
        assert_eq!(h["reins-event"], "ticket.approved");
        assert_eq!(h["reins-signature"], "ed25519=SIG");
        assert_eq!(h["reins-key"], "KEY");
        assert!(headers(&signed(TicketState::Deferred)).is_none(), "not an outcome an agent resumes on");
    }
}
