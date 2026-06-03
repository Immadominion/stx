//! The agent's tools: read-only observation tools, the terminal
//! `commit_decision` tool, the [`AgentTools`] observation interface, and the
//! fault-injection scenarios used to demonstrate real diagnosis.

use crate::error::AgentError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stx_core::{AgentAction, Decision, DecisionParams, Lamports};

/// The name of the terminal decision tool.
pub const COMMIT_DECISION: &str = "commit_decision";

/// A tool definition for the Anthropic request.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

fn no_args_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// The read-only observation tools the agent may call to gather evidence. The
/// model chooses *which* of these to call and in what order - that variability
/// is the proof of reasoning.
pub fn observation_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "get_failure_context".into(),
            description: "Get the failed transaction's structured error, submitted params (CU requested vs consumed, tip paid, blockhash age in blocks) and landing status.".into(),
            input_schema: no_args_schema(),
        },
        ToolDef {
            name: "get_network_conditions".into(),
            description: "Get current network conditions: recent prioritization-fee percentiles, current slot, and a congestion estimate.".into(),
            input_schema: no_args_schema(),
        },
        ToolDef {
            name: "get_tip_floor".into(),
            description: "Get the live Jito tip-floor percentiles in lamports: p25/p50/p75/p95/p99 and the EMA.".into(),
            input_schema: no_args_schema(),
        },
        ToolDef {
            name: "get_retry_history".into(),
            description: "Get prior attempts for this logical transaction: what parameters were tried and what happened.".into(),
            input_schema: no_args_schema(),
        },
        ToolDef {
            name: "simulate_with_params".into(),
            description: "Simulate a retry with adjusted parameters to test a hypothesis before committing. Returns whether it succeeds and the compute units consumed.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cu_limit": {"type":"integer","description":"compute unit limit to test"},
                    "tip_lamports": {"type":"integer"},
                    "refresh_blockhash": {"type":"boolean"}
                },
                "additionalProperties": false
            }),
        },
    ]
}

/// The terminal `commit_decision` tool. A flat schema the agent fills once; we
/// map it onto [`Decision`].
pub fn commit_decision_tool() -> ToolDef {
    ToolDef {
        name: COMMIT_DECISION.into(),
        description: "Commit the final operational decision after diagnosing the failure. Call exactly once. Cite the specific observed values in `justification`.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {"type":"string","enum":["resubmit","resubmit_new_blockhash","raise_tip","raise_cu","widen_slippage","abort","hold"]},
                "tip_lamports": {"type":"integer","description":"the tip to use, in lamports"},
                "cu_limit": {"type":["integer","null"],"description":"compute unit limit, if raising it"},
                "refresh_blockhash": {"type":"boolean"},
                "hypotheses": {"type":"array","items":{"type":"string"},"description":"ranked candidate root causes considered"},
                "chosen_cause": {"type":"string"},
                "justification": {"type":"string","description":"why this action follows from the observations; cite observed values"},
                "confidence": {"type":"number"},
                "expected_effect": {"type":"string"}
            },
            "required": ["action","tip_lamports","hypotheses","chosen_cause","justification","confidence","expected_effect"],
            "additionalProperties": false
        }),
    }
}

/// All tools for the request: observation tools plus the commit tool.
pub fn all_tools() -> Vec<ToolDef> {
    let mut tools = observation_tools();
    tools.push(commit_decision_tool());
    tools
}

#[derive(Debug, Deserialize)]
struct RawDecision {
    action: AgentAction,
    tip_lamports: u64,
    #[serde(default)]
    cu_limit: Option<u32>,
    #[serde(default)]
    refresh_blockhash: bool,
    #[serde(default)]
    hypotheses: Vec<String>,
    chosen_cause: String,
    justification: String,
    confidence: f32,
    expected_effect: String,
}

/// Map a `commit_decision` tool input onto a [`Decision`].
pub fn parse_decision(input: &Value) -> Result<Decision, AgentError> {
    let raw: RawDecision = serde_json::from_value(input.clone())
        .map_err(|e| AgentError::InvalidDecision(e.to_string()))?;
    Ok(Decision {
        action: raw.action,
        params: DecisionParams {
            tip_lamports: Lamports(raw.tip_lamports),
            cu_limit: raw.cu_limit,
            refresh_blockhash: raw.refresh_blockhash,
        },
        hypotheses: raw.hypotheses,
        chosen_cause: raw.chosen_cause,
        justification: raw.justification,
        confidence: raw.confidence,
        expected_effect: raw.expected_effect,
    })
}

/// The "world" the agent observes. The deterministic core implements this to
/// wire real data; tests use [`MockTools`]. `dispatch` returns the tool_result
/// content as JSON.
#[async_trait]
pub trait AgentTools: Send + Sync {
    async fn dispatch(&self, name: &str, input: &Value) -> Value;
}

/// A deliberately injected failure, used to demonstrate that the agent
/// diagnoses *different* causes rather than scripting a fixed retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultScenario {
    BlockhashExpiry,
    FeeStarvation,
    ComputeExhaustion,
}

impl FaultScenario {
    pub fn label(self) -> &'static str {
        match self {
            Self::BlockhashExpiry => "fault_injected:blockhash_expiry",
            Self::FeeStarvation => "fault_injected:fee_starvation",
            Self::ComputeExhaustion => "fault_injected:compute_exhaustion",
        }
    }

    /// The `get_failure_context` payload for this scenario.
    pub fn observations(self) -> Value {
        match self {
            Self::BlockhashExpiry => json!({
                "error": "TransactionExpiredBlockheightExceeded",
                "blockhash_age_blocks": 162,
                "cu_requested": 200000, "cu_consumed": 0,
                "tip_paid_lamports": 50000, "landed": false
            }),
            Self::FeeStarvation => json!({
                "error": null,
                "blockhash_age_blocks": 40,
                "cu_requested": 200000, "cu_consumed": 0,
                "tip_paid_lamports": 8000, "tip_floor_p50": 30000,
                "inflight_status": "Pending", "landed": false
            }),
            Self::ComputeExhaustion => json!({
                "error": "InstructionError(2, ComputationalBudgetExceeded)",
                "blockhash_age_blocks": 20,
                "cu_requested": 200000, "cu_consumed": 199950,
                "tip_paid_lamports": 50000, "landed": false
            }),
        }
    }
}

/// A canned [`AgentTools`] implementation for a fault scenario (tests + offline
/// demos). The `simulate_with_params` response makes a *raised CU limit* the fix
/// for compute exhaustion, so the agent can validate its hypothesis.
pub struct MockTools {
    pub scenario: FaultScenario,
}

#[async_trait]
impl AgentTools for MockTools {
    async fn dispatch(&self, name: &str, input: &Value) -> Value {
        match name {
            "get_failure_context" => self.scenario.observations(),
            "get_tip_floor" => json!({
                "p25":12300,"p50":30000,"p75":91863,"p95":549095,"p99":4069629,"ema_p50":22696
            }),
            "get_network_conditions" => json!({
                "current_slot":312000901,"congestion":"normal","recent_priority_fee_p50":12000
            }),
            "get_retry_history" => json!({ "attempts": [] }),
            "simulate_with_params" => {
                let cu = input.get("cu_limit").and_then(|v| v.as_u64()).unwrap_or(200_000);
                json!({ "ok": cu >= 250_000 || self.scenario != FaultScenario::ComputeExhaustion,
                        "cu_consumed": 199_950u64 })
            }
            _ => json!({ "error": "unknown tool" }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_present_and_well_formed() {
        assert_eq!(observation_tools().len(), 5);
        let commit = commit_decision_tool();
        assert_eq!(commit.name, COMMIT_DECISION);
        assert!(all_tools().iter().any(|t| t.name == COMMIT_DECISION));
    }

    #[test]
    fn parse_decision_roundtrips() {
        let input = json!({
            "action": "resubmit_new_blockhash",
            "tip_lamports": 50000,
            "cu_limit": null,
            "refresh_blockhash": true,
            "hypotheses": ["blockhash expiry", "fee too low"],
            "chosen_cause": "blockhash expiry",
            "justification": "blockhash_age_blocks=162 > 150",
            "confidence": 0.92,
            "expected_effect": "lands on resubmit"
        });
        let d = parse_decision(&input).unwrap();
        assert_eq!(d.action, AgentAction::ResubmitNewBlockhash);
        assert_eq!(d.params.tip_lamports, Lamports(50000));
        assert!(d.params.refresh_blockhash);
        assert_eq!(d.hypotheses.len(), 2);
    }

    #[test]
    fn parse_decision_rejects_garbage() {
        assert!(parse_decision(&json!({ "action": "nope" })).is_err());
    }

    #[test]
    fn fault_scenarios_have_distinct_signals() {
        assert_eq!(
            FaultScenario::BlockhashExpiry.observations()["blockhash_age_blocks"],
            json!(162)
        );
        assert_eq!(
            FaultScenario::FeeStarvation.observations()["tip_paid_lamports"],
            json!(8000)
        );
        assert_eq!(
            FaultScenario::ComputeExhaustion.observations()["cu_consumed"],
            json!(199950)
        );
    }

    #[tokio::test]
    async fn mock_tools_dispatch() {
        let tools = MockTools {
            scenario: FaultScenario::ComputeExhaustion,
        };
        let ctx = tools.dispatch("get_failure_context", &json!({})).await;
        assert_eq!(ctx["cu_consumed"], json!(199950));
        // a raised CU limit makes the simulation pass.
        let sim = tools
            .dispatch("simulate_with_params", &json!({ "cu_limit": 300000 }))
            .await;
        assert_eq!(sim["ok"], json!(true));
    }
}
