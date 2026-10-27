//! Traits implemented in Kotlin (contracts §D).

use crate::autopilot::{AutoDecisionView, AutopilotEvent};
use crate::error::ForeignError;
use crate::types::PendingItem;

#[uniffi::export(with_foreign)]
pub trait KeyWrapper: Send + Sync {
    /// Android Keystore AES-256-GCM (non-exportable key). Output includes the IV.
    fn wrap(&self, plaintext: Vec<u8>) -> Result<Vec<u8>, ForeignError>;
    fn unwrap(&self, wrapped: Vec<u8>) -> Result<Vec<u8>, ForeignError>;
}

#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait GoogleTokenProvider: Send + Sync {
    /// Fresh access token for `account` (an address; empty = the phone's default Google account) with the scopes of
    /// `service` ("gmail": gmail.modify + gmail.settings.basic; "gcalendar": calendar; "gcontacts":
    /// contacts.readonly), or NeedsUserInteraction.
    async fn access_token(&self, account: String, service: String) -> Result<String, ForeignError>;
}

/// The phone's owner check (biometrics, or the device's passcode), which the core asks for itself before what only
/// the owner may do: starting Overnight or changing its length, and changing what it does with what is too risky.
/// `reason` is the core's words, for the prompt. `Ok(true)` only when the owner passed it now; anything else (a
/// cancel, no biometrics and no passcode, an error) refuses.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait OwnerCheck: Send + Sync {
    async fn confirm(&self, reason: String) -> Result<bool, ForeignError>;
}

/// Keeps the process from being suspended while it holds the store's cross-process file lock (iOS: a file in the App
/// Group container, shared with the notification extension; iOS ends a process suspended with such a lock held,
/// `0xdead10cc`). Installed once per process with `install_store_lock_guard`, before the first `ReinsCore`.
#[uniffi::export(with_foreign)]
pub trait StoreLockGuard: Send + Sync {
    /// The process is about to take the lock: hold off suspension (a background task, an expiring activity) until
    /// `unlocked`. False when that cannot be promised (the process is being suspended): the lock is not taken and the
    /// call that needed it fails with a storage error.
    fn locking(&self) -> bool;
    /// The lock was given back (each `locking` that returned true is followed by exactly one of these).
    fn unlocked(&self);
}

#[uniffi::export(with_foreign)]
pub trait Notifier: Send + Sync {
    /// An item waits for the user (after Autopilot looked at it: `item.suggestion` says what it would do).
    fn item_pending(&self, item: PendingItem);
    fn item_resolved(&self, id: String);
    /// Autopilot (or a bypass, or Lockdown) decided a request on its own: the quiet "Autopilot" notification.
    fn auto_decided(&self, decision: AutoDecisionView);
    /// A mode changed, a bypass ended, or auto-approvals paused for a connection.
    fn autopilot_changed(&self, event: AutopilotEvent);
    /// Circles: a review of another account waits for this phone (the one Circle notification), or something else
    /// changed that the screens refresh quietly.
    fn circle_changed(&self, event: crate::circles::CircleEvent);
}
