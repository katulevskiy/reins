//! Batches (`reins_batch`): calls an AI made together, shown as one queue. Every call stays its own request: its own
//! approval, its own audit entry, its own answer. The apps' "Approve all" over a batch approves each of its calls one
//! by one, exactly as for any burst of requests (`approve_quick`, which refuses what must be opened).

use crate::types::PendingBatch;
use crate::views::ParkedRequest;

/// The batch a parked request belongs to, as the phone shows it; `None` for a lone call or a malformed reference.
pub fn pending_batch(parked: &ParkedRequest) -> Option<PendingBatch> {
    parked.request.batch.as_ref().filter(|b| b.valid()).map(|b| PendingBatch {
        id: b.id.clone(),
        index: b.index,
        size: b.size,
    })
}
