//! Opaque, end-to-end encrypted phone state. Account ownership comes only from the authenticated session.
use serde::{Deserialize, Serialize};
pub const MAX_CIPHERTEXT_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountState {
    pub revision: u64,
    pub ciphertext: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct AccountStateUpdate {
    pub revision: u64,
    pub ciphertext: String,
}
