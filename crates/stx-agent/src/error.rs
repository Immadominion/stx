//! Errors for the AI reasoning layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("anthropic api error {status}: {body}")]
    Api { status: u16, body: String },

    #[error("agent did not reach a decision within {0} turns")]
    NoDecision(u32),

    #[error("invalid decision payload: {0}")]
    InvalidDecision(String),
}
