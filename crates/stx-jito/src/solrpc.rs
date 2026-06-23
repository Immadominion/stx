//! Solana JSON-RPC client (the subset the stack needs).
//!
//! We talk to RPC over plain JSON-RPC via `reqwest` rather than pulling the
//! heavy `solana-client`, keeping the dependency surface small. Crucially,
//! [`SolanaRpc::get_latest_blockhash`] is called at **`confirmed`** (never
//! `finalized`) for time-sensitive sends - a finalized blockhash is already
//! ~13s old and burns the validity window.

use crate::error::JitoError;
use crate::rpc::parse_commitment;
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

/// On-chain signature status (authoritative landing check, never a false
/// negative for a landed tx, unlike Jito's `getInflightBundleStatuses`).
#[derive(Debug, Clone, Deserialize)]
pub struct SignatureStatusValue {
    pub slot: u64,
    /// `null` on success.
    #[serde(default)]
    pub err: Value,
    #[serde(rename = "confirmationStatus", default)]
    pub confirmation_status: Option<String>,
}

impl SignatureStatusValue {
    pub fn succeeded(&self) -> bool {
        self.err.is_null()
    }
    pub fn commitment(&self) -> Option<Commitment> {
        self.confirmation_status.as_deref().and_then(parse_commitment)
    }
}

/// Execution metadata from a landed transaction (`getTransaction` -> `meta`).
#[derive(Debug, Clone, Deserialize)]
pub struct TxMeta {
    /// The execution error, if the transaction failed on-chain (`null` if it
    /// succeeded). Kept as raw JSON so the classifier sees the exact shape.
    pub err: Option<Value>,
    #[serde(default)]
    pub fee: u64,
    #[serde(default, rename = "computeUnitsConsumed")]
    pub compute_units_consumed: Option<u64>,
    #[serde(default, rename = "logMessages")]
    pub log_messages: Option<Vec<String>>,
}

/// A landed transaction as returned by `getTransaction`.
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionInfo {
    pub slot: u64,
    #[serde(rename = "blockTime")]
    pub block_time: Option<i64>,
    pub meta: Option<TxMeta>,
}

impl TransactionInfo {
    /// Whether the transaction executed without error.
    pub fn succeeded(&self) -> bool {
        self.meta.as_ref().map(|m| m.err.is_none()).unwrap_or(true)
    }
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
        let text = crate::net::post_json_with_retry(&self.http, &self.url, &req).await?;
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

    /// Like [`call`], but a `null` result is a valid `None` (not an error) -
    /// for methods like `getTransaction` where "not found" is expressed as null.
    async fn call_opt<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Option<T>, JitoError> {
        let req = Req {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        };
        let text = crate::net::post_json_with_retry(&self.http, &self.url, &req).await?;
        let parsed: Resp<T> = serde_json::from_str(&text)?;
        if let Some(e) = parsed.error {
            return Err(JitoError::Rpc {
                code: e.code,
                message: e.message,
            });
        }
        Ok(parsed.result)
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

    /// `getSlot` - the cluster's current slot at `commitment`.
    pub async fn get_slot(&self, commitment: Commitment) -> Result<u64, JitoError> {
        self.call(
            "getSlot",
            json!([{ "commitment": commitment_str(commitment) }]),
        )
        .await
    }

    /// `getBalance` - the lamport balance of `pubkey` at `confirmed`.
    pub async fn get_balance(&self, pubkey: &str) -> Result<u64, JitoError> {
        let cv: Ctx<u64> = self
            .call("getBalance", json!([pubkey, { "commitment": "confirmed" }]))
            .await?;
        Ok(cv.value)
    }

    /// `getTransaction` - the landed transaction's slot, fee, compute, error, and
    /// logs. Returns `None` if the signature is not found on-chain (dropped, or
    /// never landed). Used by the transaction autopsy.
    pub async fn get_transaction(
        &self,
        signature: &str,
    ) -> Result<Option<TransactionInfo>, JitoError> {
        self.call_opt(
            "getTransaction",
            json!([
                signature,
                {
                    "encoding": "json",
                    "maxSupportedTransactionVersion": 0,
                    "commitment": "confirmed"
                }
            ]),
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

    /// `getSignatureStatuses` - authoritative on-chain landing check. Entries are
    /// `null` for signatures not found. `search_history` searches the longer
    /// transaction history (needed for older signatures, e.g. the autopsy); the
    /// submit hot path leaves it off, since it only checks fresh signatures.
    pub async fn get_signature_statuses(
        &self,
        signatures: &[String],
        search_history: bool,
    ) -> Result<Vec<Option<SignatureStatusValue>>, JitoError> {
        let cv: Ctx<Vec<Option<SignatureStatusValue>>> = self
            .call(
                "getSignatureStatuses",
                json!([signatures, { "searchTransactionHistory": search_history }]),
            )
            .await?;
        Ok(cv.value)
    }

    /// `getSlotLeaders` - the validator identities scheduled to produce
    /// `[start_slot, start_slot + limit)`. Used to annotate which leader
    /// produced a landed slot and to show the upcoming leader window. Solana
    /// rotates leaders every 4 consecutive slots.
    pub async fn get_slot_leaders(
        &self,
        start_slot: u64,
        limit: u64,
    ) -> Result<Vec<String>, JitoError> {
        self.call("getSlotLeaders", json!([start_slot, limit]))
            .await
    }
}

/// A run of consecutive slots produced by the same leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderWindow {
    pub leader: String,
    pub first_slot: u64,
    pub last_slot: u64,
}

impl LeaderWindow {
    pub fn slot_count(&self) -> u64 {
        self.last_slot - self.first_slot + 1
    }
}

/// Group a leader list (as returned by `getSlotLeaders` starting at
/// `start_slot`) into consecutive same-leader windows.
pub fn leader_windows(start_slot: u64, leaders: &[String]) -> Vec<LeaderWindow> {
    let mut out: Vec<LeaderWindow> = Vec::new();
    for (i, leader) in leaders.iter().enumerate() {
        let slot = start_slot + i as u64;
        match out.last_mut() {
            Some(w) if &w.leader == leader && w.last_slot + 1 == slot => w.last_slot = slot,
            _ => out.push(LeaderWindow {
                leader: leader.clone(),
                first_slot: slot,
                last_slot: slot,
            }),
        }
    }
    out
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
    fn groups_leader_windows() {
        let leaders: Vec<String> = ["A", "A", "A", "A", "B", "B", "C"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let w = leader_windows(100, &leaders);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].leader, "A");
        assert_eq!((w[0].first_slot, w[0].last_slot), (100, 103));
        assert_eq!(w[0].slot_count(), 4);
        assert_eq!((w[1].leader.as_str(), w[1].first_slot, w[1].last_slot), ("B", 104, 105));
        assert_eq!((w[2].leader.as_str(), w[2].first_slot), ("C", 106));
    }

    #[test]
    fn parses_signature_statuses() {
        let body = r#"{"jsonrpc":"2.0","result":{"context":{"slot":2},"value":[{"slot":424775191,"confirmations":null,"err":null,"confirmationStatus":"confirmed"},null]},"id":1}"#;
        let parsed: Resp<Ctx<Vec<Option<SignatureStatusValue>>>> =
            serde_json::from_str(body).unwrap();
        let v = parsed.result.unwrap().value;
        assert_eq!(v.len(), 2);
        let first = v[0].as_ref().unwrap();
        assert_eq!(first.slot, 424775191);
        assert!(first.succeeded());
        assert_eq!(first.commitment(), Some(Commitment::Confirmed));
        assert!(v[1].is_none());
    }

    #[test]
    fn expiry_math() {
        assert!(blockhash_expired(151, 150));
        assert!(!blockhash_expired(150, 150));
    }
}
