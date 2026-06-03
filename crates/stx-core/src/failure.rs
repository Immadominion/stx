//! Failure taxonomy.
//!
//! These classes are *derived from real signals* (chain `err` fields, blockhash
//! height math, Jito bundle-result reasons) by `stx-jito`'s classifier - never
//! a hardcoded `match` on a string. A [`FailureClass`] carries the evidence that
//! produced it so the AI agent (and a human) can audit the classification.

use crate::ids::Slot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// `recentBlockhash` aged past the ~150-block window before inclusion.
    /// Surfaces as `BlockhashNotFound`, blockheight-exceeded, or bundle
    /// `Dropped{BlockhashExpired}`.
    ExpiredBlockhash,
    /// Lost the priority/tip auction; the transaction was never included and no
    /// explicit error is returned.
    FeeTooLow,
    /// `InstructionError::ComputationalBudgetExceeded` - ran out of compute units.
    ComputeExceeded,
    /// Bundle marked `Failed`/`Invalid` by the Block Engine.
    BundleFailed,
    /// Bundle `Dropped` for a non-blockhash reason, or transaction dropped in flight.
    Dropped,
    /// Simulation failed (`Rejected{SimulationFailure}` or preflight error).
    SimulationFailure,
    /// Program-level failure that resubmitting won't fix (e.g. slippage in an
    /// adverse market). The agent may choose to abort rather than burn fees.
    AdverseMarket,
    /// Same signature already landed; a duplicate resubmit.
    AlreadyProcessed,
    /// Could not be classified from available signals.
    Unknown,
}

impl FailureKind {
    /// Whether resubmitting *could* succeed. `AlreadyProcessed`/`AdverseMarket`
    /// are not worth a blind retry; the agent decides the actual remedy.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            FailureKind::ExpiredBlockhash
                | FailureKind::FeeTooLow
                | FailureKind::ComputeExceeded
                | FailureKind::BundleFailed
                | FailureKind::Dropped
                | FailureKind::SimulationFailure
        )
    }
}

/// A classified failure plus the evidence that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureClass {
    pub kind: FailureKind,
    /// Human-readable justification for this classification (cites the signal).
    pub evidence: String,
    /// 0.0-1.0 confidence in the classification.
    pub confidence: f32,
    /// The raw error string / bundle-result reason, if any.
    pub raw_error: Option<String>,
    pub at_slot: Option<Slot>,
}

impl FailureClass {
    pub fn new(kind: FailureKind, evidence: impl Into<String>, confidence: f32) -> Self {
        Self {
            kind,
            evidence: evidence.into(),
            confidence: confidence.clamp(0.0, 1.0),
            raw_error: None,
            at_slot: None,
        }
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw_error = Some(raw.into());
        self
    }

    pub fn at(mut self, slot: Slot) -> Self {
        self.at_slot = Some(slot);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryability() {
        assert!(FailureKind::ExpiredBlockhash.is_retryable());
        assert!(!FailureKind::AlreadyProcessed.is_retryable());
        assert!(!FailureKind::AdverseMarket.is_retryable());
    }

    #[test]
    fn confidence_is_clamped() {
        let c = FailureClass::new(FailureKind::FeeTooLow, "never landed; lost auction", 1.5);
        assert_eq!(c.confidence, 1.0);
    }
}
