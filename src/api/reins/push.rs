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

/// Wakes the approval device without blocking the caller (Decision 31).
pub fn spawn_push(pool: DbPool, user_uuid: UserId, token: Option<String>, push: PushMessage) {
    let Some(token) = token else {
        debug!("Reins approval device of {user_uuid} has no push token; it must poll");
        return;
    };
    let service = service_for(&token);
    let sender = match service {
        Service::Apns => apns::sender().map(Sender::Apns),
        Service::Fcm => fcm::sender().map(Sender::Fcm),
    };
    let Some(sender) = sender else {
        debug!("Reins {service:?} push not configured; the phone must poll for {:?} {}", push.t, push.id);
        return;
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
                if let Ok(conn) = pool.get().await
                    && let Err(e) = ReinsDevice::clear_fcm_token(&user_uuid, &token, &conn).await
                {
                    warn!("Could not clear the Reins push token: {e:?}");
                }
            }
            Err(e) => warn!("Reins {service:?} push failed: {e}"),
        }
    });
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
