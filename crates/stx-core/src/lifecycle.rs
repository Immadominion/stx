//! The event-sourced lifecycle finite state machine.
//!
//! A transaction's [`LifecycleState`] is a deterministic fold over its
//! append-only [`LifecycleEvent`] log - the events are facts, the state is
//! derived. This is the spine of the system: the dashboard, the lifecycle log,
//! and the span waterfall are all projections of the same event stream.

use crate::commitment::Commitment;
use crate::failure::FailureClass;
use crate::ids::{BundleId, Lamports, LogicalTxId, Signature, Slot, TraceId};
use crate::tip::TipSource;
use serde::{Deserialize, Serialize};

/// A fact about a transaction's journey. Appended, never mutated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleEvent {
    /// A new submission intent was created.
    Drafted { logical_tx_id: LogicalTxId },
    /// A tip was chosen (by the agent or the fallback policy).
    TipDecided {
        tip_lamports: Lamports,
        source: TipSource,
    },
    /// The bundle was assembled and signed.
    Built { signatures: Vec<Signature> },
    /// The bundle was dispatched to one or more Block Engine regions.
    Dispatched {
        bundle_id: BundleId,
        regions: Vec<String>,
    },
    /// The bundle is in flight (accepted by at least one region).
    MarkedInflight,
    /// The bundle landed in a block at `slot`.
    Landed { slot: Slot },
    /// The landing block reached a commitment level.
    CommitmentReached { commitment: Commitment, slot: Slot },
    /// The attempt failed, with a classified cause.
    Failed { class: FailureClass },
    /// A retry was scheduled as a new, linked child trace.
    RetryScheduled { child_trace: TraceId, attempt: u32 },
    /// The submission was abandoned (terminal).
    Aborted { reason: String },
}

/// The state of one submission attempt, derived by folding its events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LifecycleState {
    Drafted,
    TipDecided,
    Built,
    Dispatched,
    Inflight,
    Landed { slot: Slot },
    Processed { slot: Slot },
    Confirmed { slot: Slot },
    Finalized { slot: Slot },
    /// Failed, but a retry may still follow (so not terminal on its own).
    Failed,
    /// Abandoned - terminal.
    Aborted,
}

impl Default for LifecycleState {
    fn default() -> Self {
        LifecycleState::Drafted
    }
}

impl LifecycleState {
    /// A state from which this trace will never advance further.
    /// `Failed` is *not* terminal: the retry orchestrator may schedule a retry
    /// (which is a separate child trace).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LifecycleState::Finalized { .. } | LifecycleState::Aborted
        )
    }

    /// Whether the transaction has landed on-chain at any commitment.
    pub fn has_landed(&self) -> bool {
        matches!(
            self,
            LifecycleState::Landed { .. }
                | LifecycleState::Processed { .. }
                | LifecycleState::Confirmed { .. }
                | LifecycleState::Finalized { .. }
        )
    }

    /// Apply one event, advancing the state. Total function: events are facts.
    pub fn apply(&mut self, ev: &LifecycleEvent) {
        let next = match ev {
            LifecycleEvent::Drafted { .. } => LifecycleState::Drafted,
            LifecycleEvent::TipDecided { .. } => LifecycleState::TipDecided,
            LifecycleEvent::Built { .. } => LifecycleState::Built,
            LifecycleEvent::Dispatched { .. } => LifecycleState::Dispatched,
            LifecycleEvent::MarkedInflight => LifecycleState::Inflight,
            LifecycleEvent::Landed { slot } => LifecycleState::Landed { slot: *slot },
            LifecycleEvent::CommitmentReached { commitment, slot } => match commitment {
                Commitment::Processed => LifecycleState::Processed { slot: *slot },
                Commitment::Confirmed => LifecycleState::Confirmed { slot: *slot },
                Commitment::Finalized => LifecycleState::Finalized { slot: *slot },
            },
            LifecycleEvent::Failed { .. } => LifecycleState::Failed,
            LifecycleEvent::Aborted { .. } => LifecycleState::Aborted,
            // A scheduled retry is tracked on the child trace; this trace's
            // state is unchanged (it stays Failed).
            LifecycleEvent::RetryScheduled { .. } => self.clone(),
        };
        *self = next;
    }

    /// Fold an event log into the current state.
    pub fn fold(events: &[LifecycleEvent]) -> LifecycleState {
        let mut state = LifecycleState::default();
        for ev in events {
            state.apply(ev);
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::FailureKind;

    fn happy_path() -> Vec<LifecycleEvent> {
        vec![
            LifecycleEvent::Drafted {
                logical_tx_id: LogicalTxId::from("ltx_1"),
            },
            LifecycleEvent::TipDecided {
                tip_lamports: Lamports(50_000),
                source: TipSource::Agent,
            },
            LifecycleEvent::Built {
                signatures: vec![Signature::from("sig1")],
            },
            LifecycleEvent::Dispatched {
                bundle_id: BundleId::from("b1"),
                regions: vec!["ny".into(), "frankfurt".into()],
            },
            LifecycleEvent::MarkedInflight,
            LifecycleEvent::Landed { slot: Slot(312_000_901) },
            LifecycleEvent::CommitmentReached {
                commitment: Commitment::Processed,
                slot: Slot(312_000_901),
            },
            LifecycleEvent::CommitmentReached {
                commitment: Commitment::Confirmed,
                slot: Slot(312_000_901),
            },
            LifecycleEvent::CommitmentReached {
                commitment: Commitment::Finalized,
                slot: Slot(312_000_901),
            },
        ]
    }

    #[test]
    fn happy_path_folds_to_finalized() {
        let state = LifecycleState::fold(&happy_path());
        assert_eq!(state, LifecycleState::Finalized { slot: Slot(312_000_901) });
        assert!(state.is_terminal());
        assert!(state.has_landed());
    }

    #[test]
    fn partial_path_tracks_progress() {
        let evs = &happy_path()[..6]; // up to Landed
        let state = LifecycleState::fold(evs);
        assert_eq!(state, LifecycleState::Landed { slot: Slot(312_000_901) });
        assert!(state.has_landed());
        assert!(!state.is_terminal());
    }

    #[test]
    fn failure_is_not_terminal_then_retry_links_out() {
        let evs = vec![
            LifecycleEvent::Drafted {
                logical_tx_id: LogicalTxId::from("ltx_2"),
            },
            LifecycleEvent::Failed {
                class: FailureClass::new(
                    FailureKind::ExpiredBlockhash,
                    "blockheight exceeded lastValidBlockHeight",
                    0.95,
                ),
            },
            LifecycleEvent::RetryScheduled {
                child_trace: TraceId::from("trc_child"),
                attempt: 2,
            },
        ];
        let state = LifecycleState::fold(&evs);
        assert_eq!(state, LifecycleState::Failed);
        assert!(!state.is_terminal());
        assert!(!state.has_landed());
    }

    #[test]
    fn aborted_is_terminal() {
        let evs = vec![
            LifecycleEvent::Drafted {
                logical_tx_id: LogicalTxId::from("ltx_3"),
            },
            LifecycleEvent::Aborted {
                reason: "adverse market; retry would burn fees".into(),
            },
        ];
        let state = LifecycleState::fold(&evs);
        assert_eq!(state, LifecycleState::Aborted);
        assert!(state.is_terminal());
    }
}
