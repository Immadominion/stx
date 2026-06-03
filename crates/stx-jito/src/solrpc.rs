//! Solana JSON-RPC client (the subset the stack needs).
//!
//! We talk to RPC over plain JSON-RPC via `reqwest` rather than pulling the
//! heavy `solana-client`, keeping the dependency surface small. Crucially,
//! [`SolanaRpc::get_latest_blockhash`] is called at **`confirmed`** (never
//! `finalized`) for time-sensitive sends - a finalized blockhash is already
//! ~13s old and burns the validity window.

use crate::error::JitoError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stx_core::Commitment;

fn commitment_str(c: Commitment) -> &'static str {
    match c {
        Commitment::Processed => "processed",
        Commitment::Confirmed => "confirmed",
        Commitment::Finalized => "finalized",
    }
}

#[derive(Serialize)]
struct Req<'a> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Deserialize)]
struct Resp<T> {
    result: Option<T>,
    error: Option<RpcErr>,
}

#[derive(Deserialize)]
struct RpcErr {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct Ctx<T> {
    value: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestBlockhash {
    pub blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    pub last_valid_block_height: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SimulationValue {
    /// `null` on success; a `TransactionError` object on failure.
    #[serde(default)]
    pub err: Value,
    #[serde(default)]
    pub logs: Option<Vec<String>>,
    #[serde(rename = "unitsConsumed", default)]
    pub units_consumed: Option<u64>,
}

impl SimulationValue {
    pub fn succeeded(&self) -> bool {
        self.err.is_null()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrioritizationFee {
    pub slot: u64,
    #[serde(rename = "prioritizationFee")]
    pub prioritization_fee: u64,
}

/// A thin Solana JSON-RPC client.
#[derive(Clone)]
pub struct SolanaRpc {
    http: reqwest::Client,
    url: String,
}

impl SolanaRpc {
    pub fn new(http: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            http,
            url: url.into(),
        }
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, JitoError> {
        let req = Req {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        };
        let resp = self.http.post(&self.url).json(&req).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(JitoError::Invalid(format!("HTTP {status}: {text}")));
        }
        let parsed: Resp<T> = serde_json::from_str(&text)?;
        if let Some(e) = parsed.error {
            return Err(JitoError::Rpc {
                code: e.code,
                message: e.message,
            });
        }
        parsed
            .result
            .ok_or_else(|| JitoError::Invalid("jsonrpc response had no result".into()))
    }

    /// `getLatestBlockhash` - fetch at `confirmed` for time-sensitive sends.
    pub async fn get_latest_blockhash(
        &self,
        commitment: Commitment,
    ) -> Result<LatestBlockhash, JitoError> {
        let cv: Ctx<LatestBlockhash> = self
            .call(
                "getLatestBlockhash",
                json!([{ "commitment": commitment_str(commitment) }]),
            )
            .await?;
        Ok(cv.value)
    }

    /// `getBlockHeight` - used to detect blockhash expiry
    /// (`current_height > last_valid_block_height`).
    pub async fn get_block_height(&self, commitment: Commitment) -> Result<u64, JitoError> {
        self.call(
            "getBlockHeight",
            json!([{ "commitment": commitment_str(commitment) }]),
        )
        .await
    }

    /// `simulateTransaction` - replaces the blockhash so a stale one doesn't
    /// reject the sim; `sigVerify` must then be false.
    pub async fn simulate_transaction(
        &self,
        tx_base64: &str,
        commitment: Commitment,
    ) -> Result<SimulationValue, JitoError> {
        let params = json!([
            tx_base64,
            {
                "encoding": "base64",
                "commitment": commitment_str(commitment),
                "replaceRecentBlockhash": true,
                "sigVerify": false
            }
        ]);
        let cv: Ctx<SimulationValue> = self.call("simulateTransaction", params).await?;
        Ok(cv.value)
    }

    /// `getRecentPrioritizationFees` for the given writable accounts.
    pub async fn get_recent_prioritization_fees(
        &self,
        accounts: &[String],
    ) -> Result<Vec<PrioritizationFee>, JitoError> {
        self.call("getRecentPrioritizationFees", json!([accounts]))
            .await
    }
}

/// Whether a blockhash has expired: the cluster's block height has passed the
/// blockhash's `lastValidBlockHeight`.
pub fn blockhash_expired(current_block_height: u64, last_valid_block_height: u64) -> bool {
    current_block_height > last_valid_block_height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_blockhash() {
        let body = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":{"blockhash":"9abc","lastValidBlockHeight":280000150}},"id":1}"#;
        let parsed: Resp<Ctx<LatestBlockhash>> = serde_json::from_str(body).unwrap();
        let v = parsed.result.unwrap().value;
        assert_eq!(v.blockhash, "9abc");
        assert_eq!(v.last_valid_block_height, 280000150);
    }

    #[test]
    fn parses_simulation_success_and_failure() {
        let ok = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":{"err":null,"logs":["Program log: ok"],"unitsConsumed":4521}},"id":1}"#;
        let parsed: Resp<Ctx<SimulationValue>> = serde_json::from_str(ok).unwrap();
        let v = parsed.result.unwrap().value;
        assert!(v.succeeded());
        assert_eq!(v.units_consumed, Some(4521));

        let fail = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":{"err":{"InstructionError":[2,"ComputationalBudgetExceeded"]},"logs":[],"unitsConsumed":200000}},"id":1}"#;
        let parsed: Resp<Ctx<SimulationValue>> = serde_json::from_str(fail).unwrap();
        assert!(!parsed.result.unwrap().value.succeeded());
    }

    #[test]
    fn parses_prioritization_fees() {
        let body = r#"{"jsonrpc":"2.0","result":[{"slot":280000000,"prioritizationFee":0},{"slot":280000001,"prioritizationFee":12000}],"id":1}"#;
        let parsed: Resp<Vec<PrioritizationFee>> = serde_json::from_str(body).unwrap();
        let v = parsed.result.unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].prioritization_fee, 12000);
    }

    #[test]
    fn expiry_math() {
        assert!(blockhash_expired(151, 150));
        assert!(!blockhash_expired(150, 150));
    }
}
