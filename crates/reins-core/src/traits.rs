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
    /// `service` ("gmail": gmail.readonly + gmail.send; "gcalendar": calendar; "gcontacts": contacts.readonly), or
    /// NeedsUserInteraction.
    async fn access_token(&self, account: String, service: String) -> Result<String, ForeignError>;
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
}
