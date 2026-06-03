//! The observe -> reason -> decide loop.
//!
//! The response-handling logic ([`interpret`], [`reasoning_summary`]) is pure
//! and unit-tested; [`ReasoningAgent::decide`] drives the HTTP loop around it.

use crate::anthropic::{AnthropicClient, ContentBlock, Message, MessagesRequest, MessagesResponse, Usage, DEFAULT_MODEL};
use crate::error::AgentError;
use crate::tools::{all_tools, parse_decision, AgentTools, COMMIT_DECISION};
use serde_json::{json, Value};
use stx_core::Decision;

/// A tool call requested by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// What to do after interpreting one model response.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopAction {
    Commit(Decision),
    CallTools(Vec<ToolCall>),
    Stop,
}

/// Interpret a model response into the next loop action. Pure - no I/O.
pub fn interpret(response: &MessagesResponse) -> Result<LoopAction, AgentError> {
    let mut calls = Vec::new();
    for block in &response.content {
        if let ContentBlock::ToolUse { id, name, input } = block {
            if name == COMMIT_DECISION {
                let decision = parse_decision(input)?;
                return Ok(LoopAction::Commit(decision));
            }
            calls.push(ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            });
        }
    }
    if calls.is_empty() {
        Ok(LoopAction::Stop)
    } else {
        Ok(LoopAction::CallTools(calls))
    }
}

/// Collect a human-readable reasoning summary from text/thinking blocks.
pub fn reasoning_summary(response: &MessagesResponse) -> Option<String> {
    let mut parts = Vec::new();
    for block in &response.content {
        match block {
            ContentBlock::Thinking { thinking, .. } if !thinking.is_empty() => {
                parts.push(thinking.clone())
            }
            ContentBlock::Text { text } if !text.is_empty() => parts.push(text.clone()),
            _ => {}
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// The default system prompt: role, the policy envelope contract, and the
/// diagnose-before-acting instruction.
pub fn default_system_prompt() -> String {
    "You own the retry/tip policy for a Solana transaction submitter (Jito bundles).\n\
     A submission has failed (or a fault was injected). Diagnose the ROOT CAUSE before acting:\n\
     the same surface error can have different causes needing different, sometimes counterintuitive, remedies.\n\
     - A blockhash expiry needs a refresh and NO tip change.\n\
     - Compute exhaustion (CU consumed ~ CU requested) needs a higher CU limit, NOT a higher tip.\n\
     - Fee starvation (never landed, tip below the floor) needs a higher tip toward a higher percentile.\n\
     - An adverse-market/program error means ABORT; retrying only burns fees.\n\
     Gather evidence with the read-only tools (you choose which and in what order). \
     Use simulate_with_params to TEST a hypothesis before committing. \
     Then call commit_decision exactly once. In `justification`, cite the specific observed values you reasoned from. \
     Your decision is bounded by a deterministic guardrail (tip clamp, max retries, blockhash-expiry check), so propose your best judgment within reason."
        .to_string()
}

/// Tunables for the agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_tokens: u32,
    pub system_prompt: String,
    pub max_turns: u32,
    /// Optional `thinking` request value (caller supplies the exact current
    /// shape); omitted by default so the request is always valid.
    pub thinking: Option<Value>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            max_tokens: 8000,
            system_prompt: default_system_prompt(),
            max_turns: 6,
            thinking: None,
        }
    }
}

/// The raw output of one agent run, before the guardrail bounds it.
#[derive(Debug, Clone)]
pub struct AgentRun {
    pub decision: Decision,
    pub thinking_summary: Option<String>,
    /// Ordered array of `{tool, input, result}` for the audit log.
    pub tool_calls: Value,
    pub model: String,
    pub request_id: Option<String>,
    pub usage: Usage,
}

pub struct ReasoningAgent {
    client: AnthropicClient,
    config: AgentConfig,
}

impl ReasoningAgent {
    pub fn new(client: AnthropicClient, config: AgentConfig) -> Self {
        Self { client, config }
    }

    /// Run the loop until the agent commits a decision (or the turn budget is
    /// exhausted). Returns the un-guardrailed [`AgentRun`].
    pub async fn decide(
        &self,
        trigger: &str,
        observations: Value,
        tools: &dyn AgentTools,
    ) -> Result<AgentRun, AgentError> {
        let prompt = format!(
            "A submission needs a decision. Trigger: {trigger}.\nInitial observations:\n{}",
            serde_json::to_string_pretty(&observations).unwrap_or_default()
        );
        let mut messages = vec![Message::user(json!([{ "type": "text", "text": prompt }]))];
        let tool_defs = all_tools();

        let mut thinking_summary: Option<String> = None;
        let mut tool_trace: Vec<Value> = Vec::new();
        let mut usage = Usage::default();
        let mut request_id: Option<String> = None;

        for turn in 0..self.config.max_turns {
            // Force the commit tool on the final allowed turn.
            let force_commit = turn + 1 == self.config.max_turns;
            let tool_choice = if force_commit {
                Some(json!({ "type": "tool", "name": COMMIT_DECISION }))
            } else {
                Some(json!({ "type": "auto" }))
            };

            let req = MessagesRequest {
                model: self.config.model.clone(),
                max_tokens: self.config.max_tokens,
                system: Some(self.config.system_prompt.clone()),
                messages: messages.clone(),
                tools: tool_defs.clone(),
                tool_choice,
                thinking: self.config.thinking.clone(),
            };

            let response = self.client.create(&req).await?;
            if let Some(rid) = &response.request_id {
                request_id = Some(rid.clone());
            }
            if let Some(u) = &response.usage {
                usage.input_tokens += u.input_tokens;
                usage.output_tokens += u.output_tokens;
                usage.cache_read_input_tokens += u.cache_read_input_tokens;
                usage.cache_creation_input_tokens += u.cache_creation_input_tokens;
            }
            if let Some(s) = reasoning_summary(&response) {
                thinking_summary = Some(match thinking_summary {
                    Some(prev) => format!("{prev}\n{s}"),
                    None => s,
                });
            }

            match interpret(&response)? {
                LoopAction::Commit(decision) => {
                    return Ok(AgentRun {
                        decision,
                        thinking_summary,
                        tool_calls: Value::Array(tool_trace),
                        model: self.config.model.clone(),
                        request_id,
                        usage,
                    });
                }
                LoopAction::CallTools(calls) => {
                    messages.push(Message::assistant(response.raw_content()));
                    let mut results = Vec::new();
                    for call in calls {
                        let result = tools.dispatch(&call.name, &call.input).await;
                        tool_trace.push(json!({
                            "tool": call.name, "input": call.input, "result": result
                        }));
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": call.id,
                            "content": result.to_string()
                        }));
                    }
                    messages.push(Message::user(Value::Array(results)));
                }
                LoopAction::Stop => {
                    messages.push(Message::assistant(response.raw_content()));
                    messages.push(Message::user(json!([{
                        "type": "text",
                        "text": "Call commit_decision now with your final decision."
                    }])));
                }
            }
        }

        Err(AgentError::NoDecision(self.config.max_turns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::Usage;

    fn response(content: Vec<ContentBlock>) -> MessagesResponse {
        MessagesResponse {
            id: "msg_1".into(),
            role: "assistant".into(),
            content,
            stop_reason: Some("tool_use".into()),
            usage: Some(Usage::default()),
            request_id: None,
            raw: Value::Null,
        }
    }

    #[test]
    fn interpret_commit() {
        let input = json!({
            "action": "raise_cu",
            "tip_lamports": 50000,
            "cu_limit": 300000,
            "hypotheses": ["compute exhaustion"],
            "chosen_cause": "compute exhaustion",
            "justification": "cu_consumed 199950 of 200000",
            "confidence": 0.8,
            "expected_effect": "lands with more CU"
        });
        let r = response(vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: COMMIT_DECISION.into(),
            input,
        }]);
        match interpret(&r).unwrap() {
            LoopAction::Commit(d) => assert_eq!(d.params.cu_limit, Some(300000)),
            other => panic!("expected commit, got {other:?}"),
        }
    }

    #[test]
    fn interpret_tool_call() {
        let r = response(vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "get_tip_floor".into(),
            input: json!({}),
        }]);
        match interpret(&r).unwrap() {
            LoopAction::CallTools(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "get_tip_floor");
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }

    #[test]
    fn interpret_stop_on_text_only() {
        let r = response(vec![ContentBlock::Text {
            text: "thinking out loud".into(),
        }]);
        assert_eq!(interpret(&r).unwrap(), LoopAction::Stop);
    }

    #[test]
    fn reasoning_summary_collects_text_and_thinking() {
        let r = response(vec![
            ContentBlock::Thinking {
                thinking: "the CU ran to the limit".into(),
                signature: None,
            },
            ContentBlock::Text {
                text: "I'll raise the CU limit".into(),
            },
        ]);
        let s = reasoning_summary(&r).unwrap();
        assert!(s.contains("CU ran to the limit"));
        assert!(s.contains("raise the CU limit"));
    }
}
