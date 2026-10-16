//! The server's op-log endpoints (`/reins/api`, [`reins_proto::oplog`]).

use std::time::Duration;

use reins_proto::oplog::{Ack, Checkpoint, Heartbeat, Op, OpBatch, OpsAppended, SyncResponse};
use reqwest::Method;

use crate::phone_api::{ApiFailure, PhoneApi};

/// The longest a sync long-poll waits (the server's limit).
const MAX_WAIT_SECS: u32 = 25;

impl PhoneApi<'_> {
    /// `GET /sync?after=<seq>&seen=<cursor>&wait=<s>`: ops after `after`, the epoch, the heartbeat and what waits.
    pub(crate) async fn sync(
        &self,
        after: u64,
        seen: Option<&str>,
        wait_secs: u32,
    ) -> Result<SyncResponse, ApiFailure> {
        let wait = wait_secs.min(MAX_WAIT_SECS);
        let mut builder = self.request(Method::GET, "/sync").query(&[("after", after.to_string())]);
        builder = builder.query(&[("wait", wait.to_string())]);
        if let Some(seen) = seen {
            builder = builder.query(&[("seen", seen)]);
        }
        Self::json(builder.timeout(Duration::from_secs(u64::from(wait) + 15))).await
    }

    /// `POST /ops`: this phone's ops, in counter order.
    pub(crate) async fn post_ops(&self, ops: Vec<Op>) -> Result<OpsAppended, ApiFailure> {
        Self::json(self.request(Method::POST, "/ops").json(&OpBatch {
            ops,
        }))
        .await
    }

    /// `POST /ack`: this phone applied the log up to `seq`.
    pub(crate) async fn ack(&self, seq: u64) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/ack").json(&Ack {
            seq,
        }))
        .await
        .map(drop)
    }

    /// `GET /checkpoint`: the latest checkpoint.
    pub(crate) async fn checkpoint(&self) -> Result<Checkpoint, ApiFailure> {
        Self::json(self.request(Method::GET, "/checkpoint").timeout(Duration::from_secs(120))).await
    }

    /// `PUT /checkpoint` (the primary).
    pub(crate) async fn put_checkpoint(&self, checkpoint: &Checkpoint) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/checkpoint").json(checkpoint).timeout(Duration::from_secs(120)))
            .await
            .map(drop)
    }

    /// `POST /heartbeat` (the primary).
    pub(crate) async fn heartbeat(&self, heartbeat: &Heartbeat) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/heartbeat").json(heartbeat)).await.map(drop)
    }
}
