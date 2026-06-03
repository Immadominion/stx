//! `stx-core` - the pure domain model for the stx transaction control tower.
//!
//! This crate has no I/O. It encodes the architecture in the type system so that
//! illegal states are hard to represent:
//!
//! - [`commitment`] - the processed/confirmed/finalized ladder, ordered.
//! - [`failure`] - the failure taxonomy, derived from real chain/bundle signals.
//! - [`tip`] - Jito tip-floor percentiles (lamports), and where a tip came from.
//! - [`lifecycle`] - the event-sourced finite state machine: a transaction's
//!   state is a fold over an append-only [`lifecycle::LifecycleEvent`] log.
//! - [`event`] - the timestamped, slot-stamped event envelope (the source of truth).
//! - [`span`] - the trace/span projection (each lifecycle stage is a span).
//! - [`decision`] - the AI agent's bounded [`decision::Decision`] and the
//!   auditable [`decision::DecisionRecord`].
//!
//! Everything the dashboard renders and the gateway serves is a projection of
//! the event log; nothing here reaches the network.

pub mod commitment;
pub mod decision;
pub mod error;
pub mod event;
pub mod failure;
pub mod ids;
pub mod lifecycle;
pub mod policy;
pub mod projection;
pub mod span;
pub mod store;
pub mod tip;

pub use commitment::Commitment;
pub use decision::{
    AgentAction, Decision, DecisionOutcome, DecisionParams, DecisionRecord, GuardrailReport,
};
pub use error::CoreError;
pub use event::Event;
pub use failure::{FailureClass, FailureKind};
pub use ids::{Blockhash, BundleId, Lamports, LogicalTxId, Pubkey, Signature, Slot, TraceId};
pub use lifecycle::{LifecycleEvent, LifecycleState};
pub use policy::{fallback_remedy, next_percentile_above};
pub use projection::{funnel, spans_for_trace, StageFunnel};
pub use span::{Span, SpanName, SpanStatus};
pub use store::EventStore;
pub use tip::{TipFloor, TipPercentile, TipSource};
