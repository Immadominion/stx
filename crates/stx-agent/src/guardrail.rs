//! The deterministic guardrail validator.
//!
//! The LLM *proposes* a [`Decision`]; this layer *disposes*. It never trusts the
//! model: it clamps the tip into a policy envelope, caps retries, refuses to
//! refresh a blockhash that hasn't actually expired (the double-submit hazard),
//! and routes low-confidence decisions to a conservative fallback. Every
//! override is recorded in the [`GuardrailReport`]. Bounding the decision does
//! not make it not-a-decision: the agent still chooses *which* action and *what*
//! tip within a meaningful envelope.

use stx_core::{AgentAction, Decision, GuardrailReport, Lamports};

/// The policy envelope the agent must stay within.
#[derive(Debug, Clone)]
pub struct GuardrailPolicy {
    pub min_tip: Lamports,
    /// Ceiling on the tip (a function of value-at-risk; the caller sets it).
    pub max_tip: Lamports,
    pub max_attempts: u32,
    pub min_confidence: f32,
}

impl Default for GuardrailPolicy {
    fn default() -> Self {
        Self {
            min_tip: Lamports(1000),
            max_tip: Lamports(10_000_000),
            max_attempts: 3,
            min_confidence: 0.4,
        }
    }
}

/// Ground-truth context the validator checks the decision against.
#[derive(Debug, Clone, Copy)]
pub struct ValidationContext {
    /// 1-based attempt number this decision is for.
    pub attempt: u32,
    /// Whether the blockhash has *actually* expired (from the deterministic
    /// layer, not the LLM's claim).
    pub blockhash_expired: bool,
}

/// Validate and bound the agent's decision. Returns the bounded decision and a
/// report of what was overridden.
pub fn validate(
    mut decision: Decision,
    policy: &GuardrailPolicy,
    ctx: &ValidationContext,
    fallback: &Decision,
) -> (Decision, GuardrailReport) {
    let mut report = GuardrailReport::clean();

    // 1. Low confidence -> conservative fallback policy.
    if decision.confidence < policy.min_confidence {
        report.fell_back_to_default = true;
        report.overrides.push(format!(
            "confidence {:.2} < min {:.2}; used fallback policy",
            decision.confidence, policy.min_confidence
        ));
        decision = fallback.clone();
    }

    // 2. Out of retries -> abort rather than resubmit again.
    if ctx.attempt >= policy.max_attempts && decision.action.resubmits() {
        report.overrides.push(format!(
            "attempt {} >= max {}; {:?} -> Abort",
            ctx.attempt, policy.max_attempts, decision.action
        ));
        decision.action = AgentAction::Abort;
        report.action_allowed = false;
    }

    // 3. Only refresh the blockhash if it actually expired (double-submit guard).
    if decision.action == AgentAction::ResubmitNewBlockhash && !ctx.blockhash_expired {
        report.overrides.push(
            "requested blockhash refresh but blockhash is not expired; downgraded to Resubmit"
                .to_string(),
        );
        decision.action = AgentAction::Resubmit;
    }
    // Keep the refresh flag consistent with the (possibly overridden) action.
    decision.params.refresh_blockhash = decision.action == AgentAction::ResubmitNewBlockhash;

    // 4. Clamp the tip into the policy envelope.
    let clamped = decision
        .params
        .tip_lamports
        .clamp_to(policy.min_tip, policy.max_tip);
    if clamped != decision.params.tip_lamports {
        report.tip_clamped = true;
        report.overrides.push(format!(
            "tip {} clamped into [{}, {}]",
            decision.params.tip_lamports.0, policy.min_tip.0, policy.max_tip.0
        ));
        decision.params.tip_lamports = clamped;
    }

    // 5. Clamp confidence to [0, 1] (strict schemas don't enforce numeric ranges).
    decision.confidence = decision.confidence.clamp(0.0, 1.0);

    (decision, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stx_core::{AgentAction, DecisionParams};

    fn decision(action: AgentAction, tip: u64, confidence: f32) -> Decision {
        Decision {
            action,
            params: DecisionParams {
                tip_lamports: Lamports(tip),
                cu_limit: None,
                refresh_blockhash: matches!(action, AgentAction::ResubmitNewBlockhash),
            },
            hypotheses: vec!["h".into()],
            chosen_cause: "c".into(),
            justification: "j".into(),
            confidence,
            expected_effect: "e".into(),
        }
    }

    fn fallback() -> Decision {
        decision(AgentAction::Resubmit, 30_000, 1.0)
    }

    fn ctx(attempt: u32, expired: bool) -> ValidationContext {
        ValidationContext {
            attempt,
            blockhash_expired: expired,
        }
    }

    #[test]
    fn clamps_tip_up_and_down() {
        let policy = GuardrailPolicy::default();
        let (d, r) = validate(
            decision(AgentAction::RaiseTip, 5, 0.9),
            &policy,
            &ctx(1, false),
            &fallback(),
        );
        assert_eq!(d.params.tip_lamports, policy.min_tip);
        assert!(r.tip_clamped);

        let (d, r) = validate(
            decision(AgentAction::RaiseTip, 99_999_999, 0.9),
            &policy,
            &ctx(1, false),
            &fallback(),
        );
        assert_eq!(d.params.tip_lamports, policy.max_tip);
        assert!(r.tip_clamped);
    }

    #[test]
    fn low_confidence_falls_back() {
        let (d, r) = validate(
            decision(AgentAction::Abort, 1_000_000, 0.1),
            &GuardrailPolicy::default(),
            &ctx(1, false),
            &fallback(),
        );
        assert!(r.fell_back_to_default);
        assert_eq!(d.action, AgentAction::Resubmit); // = fallback
    }

    #[test]
    fn out_of_retries_aborts() {
        let (d, r) = validate(
            decision(AgentAction::RaiseTip, 30_000, 0.9),
            &GuardrailPolicy::default(),
            &ctx(3, true),
            &fallback(),
        );
        assert_eq!(d.action, AgentAction::Abort);
        assert!(!r.action_allowed);
    }

    #[test]
    fn refresh_only_when_actually_expired() {
        // Requested refresh but NOT expired -> downgraded, no refresh.
        let (d, r) = validate(
            decision(AgentAction::ResubmitNewBlockhash, 30_000, 0.9),
            &GuardrailPolicy::default(),
            &ctx(1, false),
            &fallback(),
        );
        assert_eq!(d.action, AgentAction::Resubmit);
        assert!(!d.params.refresh_blockhash);
        assert!(!r.overrides.is_empty());

        // Requested refresh AND expired -> honored.
        let (d, _r) = validate(
            decision(AgentAction::ResubmitNewBlockhash, 30_000, 0.9),
            &GuardrailPolicy::default(),
            &ctx(1, true),
            &fallback(),
        );
        assert_eq!(d.action, AgentAction::ResubmitNewBlockhash);
        assert!(d.params.refresh_blockhash);
    }
}
