//! `stx-agent` - the AI reasoning layer (the "policy" brain).
//!
//! The agent owns one real operational decision: *given a failed (or
//! fault-injected) submission, diagnose the root cause and choose a
//! cause-appropriate remedy*. It runs an observe -> reason -> decide tool-use
//! loop against the Anthropic Messages API, and its proposal is bounded by a
//! deterministic [`guardrail`] validator before the core executes it.
//!
//! - [`guardrail`] - the safety boundary on the LLM (pure, fully tested).
//! - [`tools`] - observation tools, the strict `commit_decision` tool, the
//!   [`tools::AgentTools`] interface, and fault-injection scenarios.
//! - [`anthropic`] - a minimal, version-robust Messages API client.
//! - [`agent`] - the loop ([`agent::interpret`] is pure and tested).
//! - [`record`] - assembling the auditable [`stx_core::DecisionRecord`].
//!
//! The deterministic core stays fully functional with this layer disabled; the
//! agent only improves decision *quality* at named decision points.

pub mod agent;
pub mod anthropic;
pub mod error;
pub mod guardrail;
pub mod record;
pub mod tools;

pub use agent::{
    default_system_prompt, interpret, reasoning_summary, AgentConfig, AgentRun, LoopAction,
    ReasoningAgent, ToolCall,
};
pub use anthropic::{AnthropicClient, MessagesRequest, MessagesResponse, DEFAULT_MODEL};
pub use error::AgentError;
pub use guardrail::{validate, GuardrailPolicy, ValidationContext};
pub use record::build_record;
pub use tools::{
    all_tools, commit_decision_tool, observation_tools, parse_decision, AgentTools, FaultScenario,
    MockTools, ToolDef, COMMIT_DECISION,
};
