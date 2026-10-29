//! Batches (`reins_batch`): calls an AI made together, shown as one queue. Every call stays its own request: its own
//! approval, its own audit entry, its own answer. The batch only adds "Approve all" and "Deny all" over what is left
//! of the queue.

use reins_proto::gmail::ToolCall;

use crate::CoreError;
use crate::engine::Engine;
use crate::store::unix_now;
use crate::types::{PendingBatch, PendingKind};
use crate::views::ParkedRequest;

/// The batch a parked request belongs to, as the phone shows it; `None` for a lone call or a malformed reference.
pub fn pending_batch(parked: &ParkedRequest) -> Option<PendingBatch> {
    parked.request.batch.as_ref().filter(|b| b.valid()).map(|b| PendingBatch {
        id: b.id.clone(),
        index: b.index,
        size: b.size,
    })
}

/// Whether "Approve all" may approve it untouched: never a standing permission or a purchase (their own screens).
fn approvable_in_bulk(parked: &ParkedRequest) -> bool {
    !matches!(parked.request.call, ToolCall::RequestGrant { .. })
        && !parked.connector.as_ref().is_some_and(|h| h.purchase.is_some())
}

impl Engine {
    /// The waiting requests of one batch, in batch order.
    fn batch_members(&self, batch_id: &str) -> Result<Vec<(u32, String, ParkedRequest)>, CoreError> {
        let mut members = Vec::new();
        for row in self.store.pending_rows(unix_now())? {
            if row.kind != PendingKind::Request {
                continue;
            }
            let Ok((_, parked)) = self.parked(&row.id) else {
                continue;
            };
            if let Some(batch) = pending_batch(&parked).filter(|b| b.id == batch_id) {
                members.push((batch.index, row.id, parked));
            }
        }
        members.sort_by_key(|(index, _, _)| *index);
        Ok(members)
    }

    /// "Approve all" of a batch: approves, one by one and each as its sheet would untouched (`approve_quick`), every
    /// waiting call that may be approved without opening it; returns how many were. What must be opened (asked every
    /// time, a standing permission, a purchase) stays in the queue for the owner.
    pub async fn approve_batch(&self, batch_id: &str) -> Result<u32, CoreError> {
        let mut approved = 0;
        for (_, id, parked) in self.batch_members(batch_id)? {
            if !approvable_in_bulk(&parked) {
                continue;
            }
            match self.approve_quick(&id).await {
                Ok(()) => approved += 1,
                Err(e) => log::info!("Approve all left one of a batch for later: {e}"),
            }
        }
        Ok(approved)
    }

    /// "Deny all" of a batch: denies every call of it still waiting; returns how many were.
    pub async fn deny_batch(&self, batch_id: &str) -> Result<u32, CoreError> {
        let mut denied = 0;
        for (_, id, _) in self.batch_members(batch_id)? {
            match self.deny(&id).await {
                Ok(()) => denied += 1,
                Err(e) => log::info!("Deny all skipped one of a batch: {e}"),
            }
        }
        Ok(denied)
    }
}
