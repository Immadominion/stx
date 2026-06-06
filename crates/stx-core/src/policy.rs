//! The deterministic fallback policy.
//!
//! This is the explicit, documented baseline that runs when the AI brain is
//! disabled, slow, or low-confidence - it keeps the stack landing transactions
//! (the liveness property), and it is what the agent's decisions are *measured
//! against*. It is never presented as "intelligence": it is a small rule set,
//! one cause-appropriate remedy per failure class.

use crate::decision::{AgentAction, Decision, DecisionParams};
use crate::failure::{FailureClass, FailureKind};
use crate::ids::Lamports;
use crate::tip::{TipFloor, TipPercentile};

/// The next tip-floor percentile strictly above `tip` (a one-bracket bump).
/// Falls back to p99 when already at/above the top.
pub fn next_percentile_above(floor: &TipFloor, tip: Lamports) -> Lamports {
    for p in [
        TipPercentile::P25,
        TipPercentile::P50,
        TipPercentile::P75,
        TipPercentile::P95,
        TipPercentile::P99,
    ] {
        let v = floor.percentile(p);
        if v > tip {
            return v;
        }
    }
    floor.percentile(TipPercentile::P99)
}

/// Max compute-unit limit the runtime accepts.
pub const MAX_CU_LIMIT: u32 = 1_400_000;

fn remedy(
    action: AgentAction,
    tip: Lamports,
    cu_limit: Option<u32>,
    refresh_blockhash: bool,
    chosen_cause: &str,
    justification: String,
) -> Decision {
    Decision {
        action,
        params: DecisionParams {
            tip_lamports: tip,
            cu_limit,
            refresh_blockhash,
        },
        hypotheses: vec![],
        chosen_cause: chosen_cause.to_string(),
        justification,
        confidence: 1.0, // deterministic
        expected_effect: "deterministic fallback remedy".to_string(),
    }
}

/// The cause-appropriate fallback remedy for a classified failure.
pub fn fallback_remedy(
    class: &FailureClass,
    floor: &TipFloor,
    current_tip: Lamports,
    current_cu_limit: u32,
) -> Decision {
    match class.kind {
        FailureKind::ExpiredBlockhash => remedy(
            AgentAction::ResubmitNewBlockhash,
            current_tip,
            Some(current_cu_limit),
            true,
            "expired blockhash",
            "refresh the blockhash and resubmit; the tip was not the problem".into(),
        ),
        FailureKind::FeeTooLow => {
            let tip = next_percentile_above(floor, current_tip);
            remedy(
                AgentAction::RaiseTip,
                tip,
                Some(current_cu_limit),
                false,
                "fee too low",
                format!("raise tip to the next percentile bracket ({} lamports)", tip.0),
            )
        }
        FailureKind::ComputeExceeded => {
            let cu = ((current_cu_limit as u64) * 13 / 10).min(MAX_CU_LIMIT as u64) as u32;
            remedy(
                AgentAction::RaiseCu,
                current_tip,
                Some(cu),
                false,
                "compute exceeded",
                format!("raise CU limit to {cu}; the tip was not the problem"),
            )
        }
        // A bundle that never landed with no explicit error almost always lost
        // the tip auction; escalate the tip to the next percentile bracket.
        FailureKind::BundleFailed | FailureKind::Dropped => {
            let tip = next_percentile_above(floor, current_tip);
            remedy(
                AgentAction::RaiseTip,
                tip,
                Some(current_cu_limit),
                false,
                "lost the bundle auction",
                format!(
                    "bundle did not land; escalate tip to the next percentile ({} lamports)",
                    tip.0
                ),
            )
        }
        FailureKind::SimulationFailure => remedy(
            AgentAction::Resubmit,
            current_tip,
            Some(current_cu_limit),
            false,
            "transient simulation failure",
            "resubmit unchanged".into(),
        ),
        FailureKind::AdverseMarket | FailureKind::AlreadyProcessed | FailureKind::Unknown => remedy(
            AgentAction::Abort,
            current_tip,
            Some(current_cu_limit),
            false,
            "non-retryable",
            "retrying would not help (or would double-submit); abort".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::FailureClass;
    use chrono::Utc;

    fn floor() -> TipFloor {
        TipFloor {
            at: Utc::now(),
            p25: Lamports(12_300),
            p50: Lamports(30_000),
            p75: Lamports(91_863),
            p95: Lamports(549_095),
            p99: Lamports(4_069_629),
            ema_p50: Lamports(22_696),
        }
    }

    #[test]
    fn next_percentile_bumps_one_bracket() {
        let f = floor();
        assert_eq!(next_percentile_above(&f, Lamports(0)), f.p25);
        assert_eq!(next_percentile_above(&f, Lamports(30_000)), f.p75); // above p50 -> p75
        assert_eq!(next_percentile_above(&f, Lamports(10_000_000)), f.p99);
    }

    #[test]
    fn expiry_refreshes_keeps_tip() {
        let class = FailureClass::new(FailureKind::ExpiredBlockhash, "expired", 0.95);
        let d = fallback_remedy(&class, &floor(), Lamports(30_000), 200_000);
        assert_eq!(d.action, AgentAction::ResubmitNewBlockhash);
        assert!(d.params.refresh_blockhash);
        assert_eq!(d.params.tip_lamports, Lamports(30_000)); // unchanged
    }

    #[test]
    fn fee_too_low_raises_tip_not_cu() {
        let class = FailureClass::new(FailureKind::FeeTooLow, "under tip", 0.7);
        let d = fallback_remedy(&class, &floor(), Lamports(30_000), 200_000);
        assert_eq!(d.action, AgentAction::RaiseTip);
        assert_eq!(d.params.tip_lamports, Lamports(91_863)); // next bracket
        assert!(!d.params.refresh_blockhash);
    }

    #[test]
    fn compute_exceeded_raises_cu_not_tip() {
        let class = FailureClass::new(FailureKind::ComputeExceeded, "ran to limit", 0.9);
        let d = fallback_remedy(&class, &floor(), Lamports(30_000), 200_000);
        assert_eq!(d.action, AgentAction::RaiseCu);
        assert_eq!(d.params.cu_limit, Some(260_000)); // 1.3x
        assert_eq!(d.params.tip_lamports, Lamports(30_000)); // unchanged
    }

    #[test]
    fn adverse_market_aborts() {
        let class = FailureClass::new(FailureKind::AdverseMarket, "slippage", 0.6);
        let d = fallback_remedy(&class, &floor(), Lamports(30_000), 200_000);
        assert_eq!(d.action, AgentAction::Abort);
    }

    #[test]
    fn dropped_escalates_tip() {
        let class = FailureClass::new(FailureKind::Dropped, "never landed", 0.5);
        let d = fallback_remedy(&class, &floor(), Lamports(30_000), 200_000);
        assert_eq!(d.action, AgentAction::RaiseTip);
        assert_eq!(d.params.tip_lamports, Lamports(91_863)); // next bracket above p50
    }
}
