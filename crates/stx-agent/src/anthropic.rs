//! A minimal Anthropic Messages API client.
//!
//! Deliberately built against only the **stable** surface of the API (model,
//! max_tokens, system, messages, tools, tool_choice and the tool-use loop).
//! Version-sensitive extras (extended/adaptive thinking) are an optional,
//! caller-supplied `thinking` value, so the request is always valid; we never
//! send `temperature` (rejected by Opus 4.8). Response parsing tolerates
//! unknown content-block types for forward compatibility.

use crate::error::AgentError;
use crate::tools::ToolDef;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// Per-request timeout for the Messages API (Opus reasoning turns can be slow).
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const MAX_TRIES: u32 = 3;

/// 1s, 2s, 4s, ...
fn backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(500u64 << attempt.min(4))
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    /// An array of content blocks.
    pub content: Value,
}

impl Message {
    pub fn user(content: Value) -> Self {
        Self {
            role: "user".into(),
            content,
        }
    }
    pub fn assistant(content: Value) -> Self {
        Self {
            role: "assistant".into(),
            content,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub role: String,
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Set from the `request-id` response header (not part of the JSON body).
    #[serde(skip)]
    pub request_id: Option<String>,
    /// The full raw response, used to echo the assistant turn back verbatim
    /// (preserving tool_use ids and any thinking signatures).
    #[serde(skip)]
    pub raw: Value,
}

impl MessagesResponse {
    /// The raw `content` array, for echoing the assistant turn back.
    pub fn raw_content(&self) -> Value {
        self.raw
            .get("content")
            .cloned()
            .unwrap_or_else(|| json!([]))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Any other block type (e.g. redacted_thinking, server_tool_use).
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

#[derive(Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicClient {
    pub fn new(http: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
            base_url: DEFAULT_API_BASE.into(),
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    pub async fn create(&self, req: &MessagesRequest) -> Result<MessagesResponse, AgentError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let send = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .timeout(REQUEST_TIMEOUT)
                .json(req)
                .send()
                .await;
            let resp = match send {
                Ok(r) => r,
                Err(e) => {
                    if attempt < MAX_TRIES && (e.is_timeout() || e.is_connect()) {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(AgentError::Http(e));
                }
            };
            let status = resp.status();
            // 429 (rate limit), 529 (overloaded), and 5xx are transient.
            if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504 | 529)
                && attempt < MAX_TRIES
            {
                tokio::time::sleep(backoff(attempt)).await;
                continue;
            }
            let request_id = resp
                .headers()
                .get("request-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let text = resp.text().await?;
            if !status.is_success() {
                return Err(AgentError::Api {
                    status: status.as_u16(),
                    body: text,
                });
            }
            let value: Value = serde_json::from_str(&text)?;
            let mut parsed: MessagesResponse = serde_json::from_value(value.clone())?;
            parsed.raw = value;
            parsed.request_id = request_id;
            return Ok(parsed);
        }
    }

    /// A single-turn, tool-free text completion. Used for plain-language
    /// explanations (the transaction autopsy) where no tool loop is needed.
    pub async fn complete(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, AgentError> {
        let req = MessagesRequest {
            model: DEFAULT_MODEL.to_string(),
            max_tokens,
            system: Some(system.to_string()),
            messages: vec![Message::user(json!([{ "type": "text", "text": user }]))],
            tools: vec![],
            tool_choice: None,
            thinking: None,
        };
        let resp = self.create(&req).await?;
        let text = resp
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Ok(text)
    }
}
