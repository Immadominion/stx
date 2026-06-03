//! The AI agent's bounded decision and its auditable record.
//!
//! The agent proposes a [`Decision`]; the deterministic guardrail validator
//! bounds it and records what it did in a [`GuardrailReport`]; the whole thing,
//! plus the model's reasoning and the eventual [`DecisionOutcome`], is persisted
//! as a [`DecisionRecord`] - the artifact that makes the agent's reasoning
//! visible and lets us compute agent-vs-fallback landing rate.

use crate::ids::{Lamports, LogicalTxId, Signature, Slot, TraceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The cause-appropriate remedy the agent commits to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAction {
    /// Resubmit unchanged (same blockhash still valid).
    Resubmit,
    /// Refresh the blockhash, then resubmit (only honored if actually expired).
    ResubmitNewBlockhash,
    /// Raise the tip toward a higher percentile and resubmit.
    RaiseTip,
    /// Raise the compute-unit limit and resubmit.
    RaiseCu,
    /// Widen slippage tolerance and resubmit.
    WidenSlippage,
    /// Stop - retrying would only burn fees.
    Abort,
    /// Hold and re-evaluate later (e.g. wait for a Jito leader window).
    Hold,
}

impl AgentAction {
    pub fn resubmits(self) -> bool {
        matches!(
            self,
            AgentAction::Resubmit
                | AgentAction::ResubmitNewBlockhash
                | AgentAction::RaiseTip
                | AgentAction::RaiseCu
                | AgentAction::WidenSlippage
        )
    }
}

/// Concrete parameters for the chosen action, after guardrail bounding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionParams {
    pub tip_lamports: Lamports,
    pub cu_limit: Option<u32>,
    pub refresh_blockhash: bool,
}

/// The agent's structured decision (the strict `commit_decision` tool payload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub action: AgentAction,
    pub params: DecisionParams,
    /// Ranked candidate root causes the agent considered.
    pub hypotheses: Vec<String>,
    /// The single root cause it is acting on.
    pub chosen_cause: String,
    /// Why this action follows from the observations (must cite observed values).
    pub justification: String,
    /// 0.0-1.0.
    pub confidence: f32,
    pub expected_effect: String,
}

/// What the deterministic guardrail layer did with the agent's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailReport {
    pub tip_clamped: bool,
    pub action_allowed: bool,
    pub overrides: Vec<String>,
    pub fell_back_to_default: bool,
}

impl GuardrailReport {
    pub fn clean() -> Self {
        Self {
            tip_clamped: false,
            action_allowed: true,
            overrides: Vec::new(),
            fell_back_to_default: false,
        }
    }
}

/// The realized result of acting on a decision (backfilled after execution).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOutcome {
    pub landed: bool,
    pub slot: Option<Slot>,
    pub signature: Option<Signature>,
}

/// One fully-auditable decision: observed -> reasoned -> decided -> governed ->
/// outcome. Rendered as the dashboard's reasoning feed and exported with the
/// lifecycle log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub trace_id: TraceId,
    pub logical_tx_id: LogicalTxId,
    pub attempt: u32,
    pub at: DateTime<Utc>,
    /// e.g. "tx_failed", "fault_injected:blockhash_expiry".
    pub trigger: String,
    /// Exactly what the agent observed (the inputs).
    pub observations: serde_json::Value,
    /// Summarized chain-of-thought from the model's thinking blocks.
    pub thinking_summary: Option<String>,
    /// The observe->act tool-call sequence, in order.
    pub tool_calls: serde_json::Value,
    pub decision: Decision,
    pub guardrail: GuardrailReport,
    /// Backfilled once the action's result is known.
    pub outcome: Option<DecisionOutcome>,
    pub model: String,
    pub request_id: Option<String>,
}

impl DecisionRecord {
    pub fn generate_id() -> String {
        format!("dec_{}", uuid::Uuid::new_v4().simple())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_classification() {
        assert!(AgentAction::ResubmitNewBlockhash.resubmits());
        assert!(!AgentAction::Abort.resubmits());
        assert!(!AgentAction::Hold.resubmits());
    }

    #[test]
    fn decision_record_roundtrips_json() {
        let rec = DecisionRecord {
            decision_id: DecisionRecord::generate_id(),
            trace_id: TraceId::from("trc_x"),
            logical_tx_id: LogicalTxId::from("ltx_x"),
            attempt: 2,
            at: Utc::now(),
            trigger: "fault_injected:blockhash_expiry".into(),
            observations: serde_json::json!({ "blockhash_age_slots": 162 }),
            thinking_summary: Some("blockhash aged past 150; refresh, keep tip".into()),
            tool_calls: serde_json::json!([{ "tool": "simulate_with_params", "result": "ok" }]),
            decision: Decision {
                action: AgentAction::ResubmitNewBlockhash,
                params: DecisionParams {
                    tip_lamports: Lamports(50_000),
                    cu_limit: None,
                    refresh_blockhash: true,
                },
                hypotheses: vec!["blockhash expiry".into(), "fee too low".into()],
                chosen_cause: "blockhash expiry".into(),
                justification: "blockhash_age_slots=162 > 150 window".into(),
                confidence: 0.92,
                expected_effect: "lands on resubmit with fresh blockhash".into(),
            },
            guardrail: GuardrailReport::clean(),
            outcome: None,
            model: "claude-opus-4-8".into(),
            request_id: Some("req_018Ee".into()),
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: DecisionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
    }
}
