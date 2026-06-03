//! Errors for the Jito integration layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum JitoError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("tip floor response was empty")]
    EmptyTipFloor,

    #[error("jsonrpc error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("invalid response: {0}")]
    Invalid(String),
}
