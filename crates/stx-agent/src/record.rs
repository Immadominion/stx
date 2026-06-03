//! Assemble the auditable [`DecisionRecord`] from an agent run and the
//! guardrail's verdict: observed -> reasoned -> decided -> governed. The
//! `outcome` is left `None` and backfilled by the core after execution.

use crate::agent::AgentRun;
use chrono::Utc;
use serde_json::Value;
use stx_core::{Decision, DecisionRecord, GuardrailReport, LogicalTxId, TraceId};

#[allow(clippy::too_many_arguments)]
pub fn build_record(
    trace_id: TraceId,
    logical_tx_id: LogicalTxId,
    attempt: u32,
    trigger: &str,
    observations: Value,
    run: &AgentRun,
    bounded_decision: Decision,
    guardrail: GuardrailReport,
) -> DecisionRecord {
    DecisionRecord {
        decision_id: DecisionRecord::generate_id(),
        trace_id,
        logical_tx_id,
        attempt,
        at: Utc::now(),
        trigger: trigger.to_string(),
        observations,
        thinking_summary: run.thinking_summary.clone(),
        tool_calls: run.tool_calls.clone(),
        decision: bounded_decision,
        guardrail,
        outcome: None,
        model: run.model.clone(),
        request_id: run.request_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentRun;
    use crate::anthropic::Usage;
    use serde_json::json;
    use stx_core::{AgentAction, DecisionParams, Lamports};

    #[test]
    fn assembles_auditable_record() {
        let decision = Decision {
            action: AgentAction::RaiseCu,
            params: DecisionParams {
                tip_lamports: Lamports(50_000),
                cu_limit: Some(300_000),
                refresh_blockhash: false,
            },
            hypotheses: vec!["compute exhaustion".into()],
            chosen_cause: "compute exhaustion".into(),
            justification: "cu_consumed 199950 of 200000".into(),
            confidence: 0.8,
            expected_effect: "lands with more CU".into(),
        };
        let run = AgentRun {
            decision: decision.clone(),
            thinking_summary: Some("ran to the compute limit".into()),
            tool_calls: json!([{"tool":"simulate_with_params","result":{"ok":true}}]),
            model: "claude-opus-4-8".into(),
            request_id: Some("req_018Ee".into()),
            usage: Usage::default(),
        };
        let rec = build_record(
            TraceId::from("trc_1"),
            LogicalTxId::from("ltx_1"),
            1,
            "fault_injected:compute_exhaustion",
            json!({ "cu_consumed": 199950, "cu_requested": 200000 }),
            &run,
            decision,
            GuardrailReport::clean(),
        );
        assert_eq!(rec.trigger, "fault_injected:compute_exhaustion");
        assert_eq!(rec.model, "claude-opus-4-8");
        assert_eq!(rec.decision.action, AgentAction::RaiseCu);
        assert_eq!(rec.request_id.as_deref(), Some("req_018Ee"));
        assert!(rec.outcome.is_none());
        // The whole record serializes for the dashboard / lifecycle log.
        assert!(serde_json::to_string(&rec).is_ok());
    }
}
