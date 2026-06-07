//! Live [`AgentTools`]: backs the agent's read-only observation tools with real
//! data, so a `--use-agent` retry decision is grounded in the actual failure
//! context, live tip floor, live network conditions, and a real simulation.
//!
//! `simulate_with_params` is the load-bearing one: it rebuilds the bundle with
//! the agent's proposed parameters and simulates it against live RPC, letting
//! the agent test a hypothesis before committing.

use async_trait::async_trait;
use serde_json::{json, Value};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use std::str::FromStr;
use stx_agent::AgentTools;
use stx_core::{Commitment, TipFloor};
use stx_jito::{build_bundle, BundleParams, SolanaRpc};

pub struct LiveTools {
    pub rpc: SolanaRpc,
    pub keypair: Keypair,
    pub tip_account: Pubkey,
    pub cu_price_micro: u64,
    pub floor: TipFloor,
    pub failure_ctx: Value,
    pub current_tip: u64,
    pub current_cu: u32,
    pub attempt: u32,
    /// Prior attempts (tip/cu/outcome) so the agent can see what already failed.
    pub history: Vec<Value>,
}

#[async_trait]
impl AgentTools for LiveTools {
    async fn dispatch(&self, name: &str, input: &Value) -> Value {
        match name {
            "get_failure_context" => self.failure_ctx.clone(),
            "get_tip_floor" => json!({
                "p25": self.floor.p25.0,
                "p50": self.floor.p50.0,
                "p75": self.floor.p75.0,
                "p95": self.floor.p95.0,
                "p99": self.floor.p99.0,
                "ema_p50": self.floor.ema_p50.0
            }),
            "get_network_conditions" => {
                let fees = self
                    .rpc
                    .get_recent_prioritization_fees(&[])
                    .await
                    .unwrap_or_default();
                let mut vals: Vec<u64> = fees.iter().map(|f| f.prioritization_fee).collect();
                vals.sort_unstable();
                let p50 = vals.get(vals.len() / 2).copied().unwrap_or(0);
                json!({
                    "recent_priority_fee_p50_microlamports": p50,
                    "samples": vals.len()
                })
            }
            "get_retry_history" => json!({
                "current_attempt": self.attempt,
                "current_tip_lamports": self.current_tip,
                "current_cu_limit": self.current_cu,
                "prior_attempts": self.history,
            }),
            "simulate_with_params" => self.simulate(input).await,
            _ => json!({ "error": format!("unknown tool {name}") }),
        }
    }
}

impl LiveTools {
    async fn simulate(&self, input: &Value) -> Value {
        let cu_limit = input
            .get("cu_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.current_cu as u64) as u32;
        let tip = input
            .get("tip_lamports")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.current_tip);

        let bh = match self.rpc.get_latest_blockhash(Commitment::Confirmed).await {
            Ok(bh) => bh,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        let blockhash = match Hash::from_str(&bh.blockhash) {
            Ok(h) => h,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        let built = match build_bundle(BundleParams {
            payer: &self.keypair,
            recent_blockhash: blockhash,
            tip_account: self.tip_account,
            tip_lamports: tip,
            cu_limit,
            cu_price_micro_lamports: self.cu_price_micro,
            extra_instructions: vec![],
        }) {
            Ok(b) => b,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        match self
            .rpc
            .simulate_transaction(&built.transactions[0], Commitment::Confirmed)
            .await
        {
            Ok(sim) => json!({
                "ok": sim.succeeded(),
                "cu_consumed": sim.units_consumed,
                "err": sim.err
            }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }
}
