//! Wakes the approval device: APNs for the iOS app's tokens (`apns:` and `apns-sandbox:`), FCM for every other token.
//! Either sender may be unconfigured; the phone then has to poll.

use reins_proto::pairing::PushMessage;

use super::{
    apns,
    fcm::{self, SendOutcome},
};
use crate::db::{
    DbPool,
    models::{ReinsDevice, UserId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    Apns,
    Fcm,
}

enum Sender {
    Apns(&'static apns::ApnsSender),
    Fcm(&'static fcm::FcmSender),
}

fn service_for(token: &str) -> Service {
    if apns::split_token(token).is_some() {
        Service::Apns
    } else {
        Service::Fcm
    }
}

/// Test-only: pushes go to this loopback URL as a JSON `{t, id}` POST instead of FCM or APNs, so the end-to-end tests
/// can time the whole round trip with a phone that is woken like a real one. Read only together with
/// `REINS_TEST_ALLOW_LOOPBACK`; never set in production.
pub const TEST_PUSH_URL_ENV: &str = "REINS_TEST_PUSH_URL";

static TEST_PUSH_URL: std::sync::LazyLock<Option<String>> = std::sync::LazyLock::new(|| {
    super::outbound::allow_loopback()
        .then(|| std::env::var(TEST_PUSH_URL_ENV).ok())
        .flatten()
        .filter(|u| u.starts_with("http://127.0.0.1:"))
});

/// One client for the test pushes, as the real senders keep theirs (a new client per push costs milliseconds).
static TEST_PUSH_CLIENT: std::sync::LazyLock<Option<reqwest::Client>> =
    std::sync::LazyLock::new(|| crate::http_client::get_reqwest_client_builder(false).build().ok());

/// Wakes the approval device without blocking the caller (Decision 31).
pub fn spawn_push(pool: DbPool, user_uuid: UserId, token: Option<String>, push: PushMessage) {
    spawn_push_then(pool, user_uuid, token, push, || {});
}

/// Like [`spawn_push`], and says whether a push is under way: `false` when there is no token or no sender (the phone
/// sees the item only once its app polls). `failed` runs when the push service refused or could not be reached.
pub fn spawn_push_then(
    pool: DbPool,
    user_uuid: UserId,
    token: Option<String>,
    push: PushMessage,
    failed: impl FnOnce() + Send + 'static,
) -> bool {
    if let Some(url) = TEST_PUSH_URL.as_ref() {
        let url = url.clone();
        tokio::spawn(async move {
            let sent = match TEST_PUSH_CLIENT.as_ref() {
                Some(client) => client
                    .post(url)
                    .json(&push)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .map(drop)
                    .map_err(|e| e.to_string()),
                None => Err("no HTTP client".to_owned()),
            };
            if let Err(e) = sent {
                warn!("Reins test push failed: {e}");
                failed();
            }
        });
        return true;
    }
    let Some(token) = token else {
        debug!("Reins approval device of {user_uuid} has no push token; it must poll");
        return false;
    };
    let service = service_for(&token);
    let sender = match service {
        Service::Apns => apns::sender().map(Sender::Apns),
        Service::Fcm => fcm::sender().map(Sender::Fcm),
    };
    let Some(sender) = sender else {
        debug!("Reins {service:?} push not configured; the phone must poll for {:?} {}", push.t, push.id);
        return false;
    };
    tokio::spawn(async move {
        let result = match sender {
            Sender::Apns(sender) => sender.send(&token, &push).await,
            Sender::Fcm(sender) => sender.send(&token, &push).await,
        };
        match result {
            Ok(SendOutcome::Sent) => debug!("Reins {service:?} push {:?} {} sent", push.t, push.id),
            Ok(SendOutcome::Unregistered) => {
                warn!("{service:?} token of the Reins device of {user_uuid} is unregistered; clearing it");
                failed();
                if let Ok(conn) = pool.get().await
                    && let Err(e) = ReinsDevice::clear_fcm_token(&user_uuid, &token, &conn).await
                {
                    warn!("Could not clear the Reins push token: {e:?}");
                }
            }
            Err(e) => {
                warn!("Reins {service:?} push failed: {e}");
                failed();
            }
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apns_tokens_go_to_apple_and_everything_else_to_firebase() {
        let device = "ab".repeat(32);
        assert_eq!(service_for(&format!("apns:{device}")), Service::Apns);
        assert_eq!(service_for(&format!("apns-sandbox:{device}")), Service::Apns);
        assert_eq!(service_for("fMEP0vJqS0:APA91bHqX"), Service::Fcm);
        assert_eq!(service_for("apnsx:abc"), Service::Fcm);
    }
}
