//! In-memory event store - the append-only source of truth.
//!
//! Assigns monotonic sequence numbers, indexes by trace, and derives the
//! current [`LifecycleState`] of any trace by folding its events. A real
//! deployment would back this with SQLite/Postgres; the interface is the same
//! (append + replay), so projections never change.

use crate::event::Event;
use crate::ids::{LogicalTxId, Slot, TraceId};
use crate::lifecycle::{LifecycleEvent, LifecycleState};

#[derive(Debug, Default)]
pub struct EventStore {
    events: Vec<Event>,
    next_seq: u64,
}

impl EventStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new event, assigning the next sequence number and a timestamp.
    /// Returns the assigned sequence number.
    pub fn append(
        &mut self,
        trace_id: TraceId,
        logical_tx_id: LogicalTxId,
        slot: Option<Slot>,
        event: LifecycleEvent,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let mut e = Event::new(seq, trace_id, logical_tx_id, event);
        if let Some(s) = slot {
            e = e.at_slot(s);
        }
        self.events.push(e);
        seq
    }

    /// Push a pre-built event (for replay from a durable log). Keeps `next_seq`
    /// ahead of any replayed sequence number.
    pub fn push(&mut self, event: Event) {
        self.next_seq = self.next_seq.max(event.seq + 1);
        self.events.push(event);
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn events_for_trace(&self, trace_id: &TraceId) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|e| &e.trace_id == trace_id)
            .collect()
    }

    /// The current state of a trace, folded from its events.
    pub fn state_of(&self, trace_id: &TraceId) -> LifecycleState {
        let mut state = LifecycleState::default();
        for e in self.events.iter().filter(|e| &e.trace_id == trace_id) {
            state.apply(&e.event);
        }
        state
    }

    /// All trace ids, in first-seen order.
    pub fn traces(&self) -> Vec<TraceId> {
        let mut seen: Vec<TraceId> = Vec::new();
        for e in &self.events {
            if !seen.contains(&e.trace_id) {
                seen.push(e.trace_id.clone());
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment::Commitment;
    use crate::ids::Slot;

    #[test]
    fn append_assigns_seq_and_folds_state() {
        let mut store = EventStore::new();
        let trace = TraceId::from("trc_1");
        let ltx = LogicalTxId::from("ltx_1");
        let s0 = store.append(
            trace.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::Drafted {
                logical_tx_id: ltx.clone(),
            },
        );
        let s1 = store.append(
            trace.clone(),
            ltx.clone(),
            Some(Slot(10)),
            LifecycleEvent::Landed { slot: Slot(10) },
        );
        store.append(
            trace.clone(),
            ltx.clone(),
            Some(Slot(10)),
            LifecycleEvent::CommitmentReached {
                commitment: Commitment::Confirmed,
                slot: Slot(10),
            },
        );
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(store.state_of(&trace), LifecycleState::Confirmed { slot: Slot(10) });
        assert_eq!(store.events_for_trace(&trace).len(), 3);
        assert_eq!(store.traces(), vec![trace]);
    }
}
