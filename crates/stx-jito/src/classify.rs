//! Failure classifier.
//!
//! Maps a bundle of *observed signals* to a [`FailureClass`]. This is
//! deliberately not a single `match` on an error string: the same surface error
//! can have different root causes, so the classifier combines the error text,
//! the Jito bundle-drop reason, the inflight status, the blockhash age, the
//! compute-units-consumed-vs-requested ratio, and the tip-vs-floor gap. Each
//! verdict carries the evidence that produced it and a confidence, so the AI
//! agent (and a human) can audit and, if needed, override it.

use crate::rpc::InflightStatus;
use stx_core::{FailureClass, FailureKind};

/// A `recentBlockhash` is valid for ~150 blocks.
pub const BLOCKHASH_VALIDITY_BLOCKS: u64 = 150;

/// Everything observed about a (possibly) failed submission.
#[derive(Debug, Clone, Default)]
pub struct FailureSignals {
    /// Raw `meta.err` / simulation error string, if any.
    pub tx_error: Option<String>,
    /// Jito gRPC `BundleResult::Dropped` reason, if any.
    pub bundle_dropped_reason: Option<String>,
    /// `getInflightBundleStatuses` status, if polled.
    pub inflight_status: Option<InflightStatus>,
    /// Whether the transaction landed on-chain at any commitment.
    pub landed: bool,
    /// How many blocks old the blockhash was at the time of the failure.
    pub blockhash_age_blocks: Option<u64>,
    pub cu_requested: Option<u32>,
    pub cu_consumed: Option<u64>,
    pub tip_lamports: Option<u64>,
    pub tip_floor_p50: Option<u64>,
}

/// Classify a failure from its signals.
pub fn classify(s: &FailureSignals) -> FailureClass {
    // 1. Explicit, high-confidence signals first.
    if let Some(reason) = &s.bundle_dropped_reason {
        let r = reason.to_lowercase();
        if r.contains("blockhashexpired") || r.contains("blockhash expired") {
            return FailureClass::new(
                FailureKind::ExpiredBlockhash,
                "bundle dropped with reason BlockhashExpired",
                0.98,
            )
            .with_raw(reason.clone());
        }
        if r.contains("simulationfailure") || r.contains("simulation") {
            return FailureClass::new(
                FailureKind::SimulationFailure,
                "bundle dropped due to simulation failure",
                0.9,
            )
            .with_raw(reason.clone());
        }
    }

    if let Some(err) = &s.tx_error {
        let e = err.to_lowercase();
        if e.contains("blockhashnotfound")
            || e.contains("blockheightexceeded")
            || e.contains("transactionexpired")
        {
            return FailureClass::new(
                FailureKind::ExpiredBlockhash,
                "error indicates blockhash not found / blockheight exceeded",
                0.95,
            )
            .with_raw(err.clone());
        }
        if e.contains("computationalbudgetexceeded") || e.contains("exceeded cus") {
            return FailureClass::new(
                FailureKind::ComputeExceeded,
                "error indicates compute budget exceeded",
                0.95,
            )
            .with_raw(err.clone());
        }
        if e.contains("alreadyprocessed") {
            return FailureClass::new(
                FailureKind::AlreadyProcessed,
                "transaction signature was already processed (duplicate resubmit)",
                0.99,
            )
            .with_raw(err.clone());
        }
        if e.contains("simulation") {
            return FailureClass::new(
                FailureKind::SimulationFailure,
                "simulation failed before submission",
                0.85,
            )
            .with_raw(err.clone());
        }
        // Program-defined errors (e.g. slippage 0x1771) point at the trade, not
        // the infra - retrying blindly burns fees. Lower confidence; the agent
        // weighs market context.
        if e.contains("custom") || e.contains("0x1771") || e.contains("slippage") {
            return FailureClass::new(
                FailureKind::AdverseMarket,
                "program returned a custom error (e.g. slippage); likely adverse market",
                0.6,
            )
            .with_raw(err.clone());
        }
    }

    // 2. Inferred compute exhaustion: ran to (>=99% of) the requested CU limit.
    if let (Some(req), Some(used)) = (s.cu_requested, s.cu_consumed) {
        if req > 0 && used >= (req as u64).saturating_sub((req as u64) / 100) {
            return FailureClass::new(
                FailureKind::ComputeExceeded,
                format!("consumed {used} of {req} CU (>=99%); ran to the compute limit"),
                0.7,
            );
        }
    }

    // 3. Blockhash aged past the window, even without an explicit error.
    if let Some(age) = s.blockhash_age_blocks {
        if age > BLOCKHASH_VALIDITY_BLOCKS {
            return FailureClass::new(
                FailureKind::ExpiredBlockhash,
                format!(
                    "blockhash age {age} blocks exceeds the {BLOCKHASH_VALIDITY_BLOCKS}-block window"
                ),
                0.85,
            );
        }
    }

    // 4. Never landed: distinguish under-tip (fee too low) from a marked bundle
    // failure from an unexplained drop.
    if !s.landed {
        if let (Some(tip), Some(p50)) = (s.tip_lamports, s.tip_floor_p50) {
            if tip < p50 {
                return FailureClass::new(
                    FailureKind::FeeTooLow,
                    format!("never landed; tip {tip} lamports is below floor p50 {p50}"),
                    0.7,
                );
            }
        }
        if matches!(
            s.inflight_status,
            Some(InflightStatus::Failed) | Some(InflightStatus::Invalid)
        ) {
            return FailureClass::new(
                FailureKind::BundleFailed,
                "bundle marked Failed/Invalid by the Block Engine and never landed",
                0.65,
            );
        }
        return FailureClass::new(
            FailureKind::Dropped,
            "never landed and no specific signal; treated as dropped",
            0.5,
        );
    }

    FailureClass::new(FailureKind::Unknown, "no decisive failure signal", 0.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_reason_blockhash_expiry() {
        let s = FailureSignals {
            bundle_dropped_reason: Some("Dropped(BlockhashExpired)".into()),
            ..Default::default()
        };
        let c = classify(&s);
        assert_eq!(c.kind, FailureKind::ExpiredBlockhash);
        assert!(c.confidence > 0.9);
    }

    #[test]
    fn compute_exceeded_from_error() {
        let s = FailureSignals {
            tx_error: Some("InstructionError(2, ComputationalBudgetExceeded)".into()),
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::ComputeExceeded);
    }

    #[test]
    fn compute_exceeded_inferred_from_cu() {
        let s = FailureSignals {
            cu_requested: Some(200_000),
            cu_consumed: Some(199_500),
            landed: true,
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::ComputeExceeded);
    }

    #[test]
    fn already_processed() {
        let s = FailureSignals {
            tx_error: Some("AlreadyProcessed".into()),
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::AlreadyProcessed);
    }

    #[test]
    fn slippage_is_adverse_market() {
        let s = FailureSignals {
            tx_error: Some("Custom(6001) slippage exceeded".into()),
            ..Default::default()
        };
        let c = classify(&s);
        assert_eq!(c.kind, FailureKind::AdverseMarket);
        assert!(!c.kind.is_retryable());
    }

    #[test]
    fn blockhash_age_expiry_without_error() {
        let s = FailureSignals {
            blockhash_age_blocks: Some(162),
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::ExpiredBlockhash);
    }

    #[test]
    fn under_tip_is_fee_too_low() {
        let s = FailureSignals {
            landed: false,
            inflight_status: Some(InflightStatus::Pending),
            tip_lamports: Some(10_000),
            tip_floor_p50: Some(30_000),
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::FeeTooLow);
    }

    #[test]
    fn marked_failed_is_bundle_failed() {
        let s = FailureSignals {
            landed: false,
            inflight_status: Some(InflightStatus::Failed),
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::BundleFailed);
    }

    #[test]
    fn no_signal_is_unknown() {
        let s = FailureSignals {
            landed: true,
            ..Default::default()
        };
        assert_eq!(classify(&s).kind, FailureKind::Unknown);
    }
}
