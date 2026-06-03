//! Jito Block Engine HTTP JSON-RPC client.
//!
//! Talks to `{base}/api/v1/bundles`: `sendBundle`, `getTipAccounts`,
//! `getInflightBundleStatuses`, `getBundleStatuses`. Field parsing tolerates
//! both snake_case and camelCase (the API mixes Jito-native `bundle_id` with
//! Solana-style `confirmationStatus`) via serde aliases.
//!
//! The rate limit is 1 req/s per IP **per region**, and each region's budget is
//! independent - so the reliability pattern is to fan the same bundle out to
//! several regions concurrently ([`submit_to_regions`]).

use crate::error::JitoError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stx_core::{BundleId, Commitment};

const JSONRPC: &str = "2.0";

/// Global mainnet endpoint (auto-routes).
pub const MAINNET_GLOBAL: &str = "https://mainnet.block-engine.jito.wtf";

/// Regional mainnet endpoints (independent per-IP rate budgets).
pub const MAINNET_REGIONS: &[&str] = &[
    "https://amsterdam.mainnet.block-engine.jito.wtf",
    "https://frankfurt.mainnet.block-engine.jito.wtf",
    "https://london.mainnet.block-engine.jito.wtf",
    "https://ny.mainnet.block-engine.jito.wtf",
    "https://slc.mainnet.block-engine.jito.wtf",
    "https://singapore.mainnet.block-engine.jito.wtf",
    "https://tokyo.mainnet.block-engine.jito.wtf",
];

/// Testnet endpoint (used for development / fault testing).
pub const TESTNET_GLOBAL: &str = "https://testnet.block-engine.jito.wtf";

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Deserialize)]
struct RpcResponse<T> {
    // `Option<_>` fields are already optional in serde (absent -> None); no
    // `#[serde(default)]` needed, which would wrongly impose `T: Default`.
    result: Option<T>,
    error: Option<RpcErrorBody>,
}

#[derive(Deserialize)]
struct RpcErrorBody {
    code: i64,
    message: String,
}

/// The `{ context, value }` envelope used by the status methods.
#[derive(Deserialize)]
struct ContextValue<T> {
    value: T,
}

/// Real-time inflight status (5-minute look-back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum InflightStatus {
    /// Bundle id not in Jito's system (5-min look-back) - expired or never seen.
    Invalid,
    /// Not failed, not landed, not invalid.
    Pending,
    /// All regions marked it failed; not forwarded.
    Failed,
    /// Landed on-chain.
    Landed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InflightBundleStatus {
    pub bundle_id: String,
    pub status: InflightStatus,
    pub landed_slot: Option<u64>,
}

/// On-chain bundle status (analogous to `getSignatureStatuses`).
#[derive(Debug, Clone, Deserialize)]
pub struct BundleStatusValue {
    #[serde(alias = "bundleId")]
    pub bundle_id: String,
    #[serde(default)]
    pub transactions: Vec<String>,
    pub slot: u64,
    #[serde(alias = "confirmationStatus")]
    pub confirmation_status: String,
    #[serde(default)]
    pub err: Value,
}

impl BundleStatusValue {
    /// The bundle's commitment, parsed from `confirmation_status`.
    pub fn commitment(&self) -> Option<Commitment> {
        parse_commitment(&self.confirmation_status)
    }
}

/// Map a Solana confirmation-status string to a [`Commitment`].
pub fn parse_commitment(s: &str) -> Option<Commitment> {
    match s {
        "processed" => Some(Commitment::Processed),
        "confirmed" => Some(Commitment::Confirmed),
        "finalized" => Some(Commitment::Finalized),
        _ => None,
    }
}

/// A client bound to a single Block Engine endpoint.
#[derive(Clone)]
pub struct JitoClient {
    http: reqwest::Client,
    base_url: String,
}

impl JitoClient {
    pub fn new(http: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
        }
    }

    pub fn mainnet(http: reqwest::Client) -> Self {
        Self::new(http, MAINNET_GLOBAL)
    }

    pub fn testnet(http: reqwest::Client) -> Self {
        Self::new(http, TESTNET_GLOBAL)
    }

    fn bundles_endpoint(&self) -> String {
        format!("{}/api/v1/bundles", self.base_url.trim_end_matches('/'))
    }

    async fn call<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        method: &str,
        params: Value,
    ) -> Result<T, JitoError> {
        let req = RpcRequest {
            jsonrpc: JSONRPC,
            id: 1,
            method,
            params,
        };
        let resp = self.http.post(endpoint).json(&req).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(JitoError::Invalid(format!("HTTP {status}: {text}")));
        }
        let parsed: RpcResponse<T> = serde_json::from_str(&text)?;
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

    /// `sendBundle` - returns the bundle id (SHA-256 of the tx signatures).
    pub async fn send_bundle(&self, txs_base64: &[String]) -> Result<BundleId, JitoError> {
        let params = json!([txs_base64, { "encoding": "base64" }]);
        let id: String = self
            .call(&self.bundles_endpoint(), "sendBundle", params)
            .await?;
        Ok(BundleId::new(id))
    }

    /// `getTipAccounts` - the 8 tip accounts (fetch rather than hardcode).
    pub async fn get_tip_accounts(&self) -> Result<Vec<String>, JitoError> {
        self.call(&self.bundles_endpoint(), "getTipAccounts", json!([]))
            .await
    }

    /// `getInflightBundleStatuses` - fast triage (max 5 ids).
    pub async fn get_inflight_bundle_statuses(
        &self,
        ids: &[String],
    ) -> Result<Vec<InflightBundleStatus>, JitoError> {
        let cv: ContextValue<Vec<InflightBundleStatus>> = self
            .call(
                &self.bundles_endpoint(),
                "getInflightBundleStatuses",
                json!([ids]),
            )
            .await?;
        Ok(cv.value)
    }

    /// `getBundleStatuses` - on-chain confirmation (max 5 ids). Entries are
    /// `null` for bundles not found / not landed.
    pub async fn get_bundle_statuses(
        &self,
        ids: &[String],
    ) -> Result<Vec<Option<BundleStatusValue>>, JitoError> {
        let cv: ContextValue<Vec<Option<BundleStatusValue>>> = self
            .call(&self.bundles_endpoint(), "getBundleStatuses", json!([ids]))
            .await?;
        Ok(cv.value)
    }
}

/// Submit the same bundle to several regions concurrently. Each region has an
/// independent rate budget; the returned `bundle_id`s are identical across
/// regions. Returns one `(endpoint, result)` per region.
pub async fn submit_to_regions(
    http: &reqwest::Client,
    base_urls: &[&str],
    txs_base64: &[String],
) -> Vec<(String, Result<BundleId, JitoError>)> {
    let futs = base_urls.iter().map(|url| {
        let client = JitoClient::new(http.clone(), *url);
        let txs = txs_base64;
        async move {
            let res = client.send_bundle(txs).await;
            ((*url).to_string(), res)
        }
    });
    futures::future::join_all(futs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inflight_response() {
        let body = r#"{"jsonrpc":"2.0","result":{"context":{"slot":280000000},"value":[
            {"bundle_id":"abc","status":"Landed","landed_slot":280000001},
            {"bundle_id":"def","status":"Pending","landed_slot":null}
        ]},"id":1}"#;
        let parsed: RpcResponse<ContextValue<Vec<InflightBundleStatus>>> =
            serde_json::from_str(body).unwrap();
        let v = parsed.result.unwrap().value;
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].status, InflightStatus::Landed);
        assert_eq!(v[0].landed_slot, Some(280000001));
        assert_eq!(v[1].status, InflightStatus::Pending);
        assert_eq!(v[1].landed_slot, None);
    }

    #[test]
    fn parses_bundle_statuses_with_nulls_and_camelcase() {
        let body = r#"{"jsonrpc":"2.0","result":{"context":{"slot":1},"value":[
            {"bundle_id":"abc","transactions":["sig1"],"slot":280000001,"confirmationStatus":"confirmed","err":null},
            null
        ]},"id":1}"#;
        let parsed: RpcResponse<ContextValue<Vec<Option<BundleStatusValue>>>> =
            serde_json::from_str(body).unwrap();
        let v = parsed.result.unwrap().value;
        assert_eq!(v.len(), 2);
        let first = v[0].as_ref().unwrap();
        assert_eq!(first.slot, 280000001);
        assert_eq!(first.commitment(), Some(Commitment::Confirmed));
        assert!(v[1].is_none());
    }

    #[test]
    fn parses_rpc_error() {
        let body = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"bad params"},"id":1}"#;
        let parsed: RpcResponse<String> = serde_json::from_str(body).unwrap();
        assert!(parsed.result.is_none());
        assert_eq!(parsed.error.unwrap().code, -32602);
    }

    #[test]
    fn commitment_mapping() {
        assert_eq!(parse_commitment("finalized"), Some(Commitment::Finalized));
        assert_eq!(parse_commitment("nonsense"), None);
    }
}
