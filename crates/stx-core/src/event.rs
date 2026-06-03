//! The event envelope - the append-only source of truth.
//!
//! Every [`LifecycleEvent`] is wrapped with a monotonic sequence number, a wall
//! clock timestamp, the slot it was observed at (when known), and the trace it
//! belongs to. The store is an ordered log of these; all read models are folds.

use crate::ids::{LogicalTxId, Slot, TraceId};
use crate::lifecycle::LifecycleEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic, store-assigned sequence number.
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub trace_id: TraceId,
    pub logical_tx_id: LogicalTxId,
    /// The slot this event was observed at, when applicable.
    pub slot: Option<Slot>,
    pub event: LifecycleEvent,
}

impl Event {
    pub fn new(
        seq: u64,
        trace_id: TraceId,
        logical_tx_id: LogicalTxId,
        event: LifecycleEvent,
    ) -> Self {
        Self {
            seq,
            at: Utc::now(),
            trace_id,
            logical_tx_id,
            slot: None,
            event,
        }
    }

    pub fn at_slot(mut self, slot: Slot) -> Self {
        self.slot = Some(slot);
        self
    }
}
