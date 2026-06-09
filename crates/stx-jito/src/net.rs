//! Shared HTTP resilience: a per-request timeout and bounded retry with
//! exponential backoff on transient failures, so a slow or flaky endpoint can
//! never stall the caller indefinitely.

use crate::error::JitoError;
use std::time::Duration;

/// Per-request timeout for RPC / Jito calls. Long enough for a slow node, short
/// enough that a hung endpoint cannot stall the submit loop.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

const MAX_TRIES: u32 = 3;

/// 400ms, 800ms, 1600ms, ...
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(200u64 << attempt.min(4))
}

/// POST a JSON body with a per-request timeout, retrying on transient failures
/// (connect/timeout errors, HTTP 429, and 5xx) with exponential backoff. Returns
/// the response body once a non-retryable outcome is reached.
pub(crate) async fn post_json_with_retry<B: serde::Serialize>(
    http: &reqwest::Client,
    url: &str,
    body: &B,
) -> Result<String, JitoError> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match http
            .post(url)
            .json(body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if (status.as_u16() == 429 || status.is_server_error()) && attempt < MAX_TRIES {
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                let text = resp.text().await?;
                if !status.is_success() {
                    return Err(JitoError::Invalid(format!("HTTP {status}: {text}")));
                }
                return Ok(text);
            }
            Err(e) => {
                if attempt < MAX_TRIES && (e.is_timeout() || e.is_connect()) {
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                return Err(JitoError::Http(e));
            }
        }
    }
}
