//! Batches (`reins_batch`): several calls an AI makes at once. Each is relayed, decided, audited and answered on its
//! own, exactly as if it had been called alone; the batch only lets the phone show them together, as one queue the
//! user can go through item by item or approve at once.

use serde::{Deserialize, Serialize};

/// The most calls one batch may hold.
pub const MAX_BATCH_CALLS: usize = 10;
const MAX_BATCH_ID_LEN: usize = 64;

/// The place of one relayed call in its batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchRef {
    /// Shared by every call of the batch (the server's random id).
    pub id: String,
    /// 0-based.
    pub index: u32,
    pub size: u32,
}

impl BatchRef {
    /// Whether it is well formed; the phone ignores a batch reference that is not (the call is shown on its own).
    #[must_use]
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= MAX_BATCH_ID_LEN
            && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && self.size >= 1
            && usize::try_from(self.size).is_ok_and(|n| n <= MAX_BATCH_CALLS)
            && self.index < self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, index: u32, size: u32) -> BatchRef {
        BatchRef {
            id: id.to_owned(),
            index,
            size,
        }
    }

    #[test]
    fn only_well_formed_references_are_valid() {
        assert!(r("0b9c-11", 0, 2).valid());
        assert!(r("a", 9, 10).valid());
        assert!(!r("", 0, 1).valid());
        assert!(!r("a b", 0, 1).valid());
        assert!(!r(&"a".repeat(65), 0, 1).valid());
        assert!(!r("a", 1, 1).valid(), "index within size");
        assert!(!r("a", 0, 0).valid());
        assert!(!r("a", 0, 11).valid(), "at most MAX_BATCH_CALLS");
    }
}
