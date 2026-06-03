//! Core error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid lifecycle transition: {0}")]
    InvalidTransition(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
