//! Engine configuration, loaded from `.env.local` + a keypair file.

use anyhow::{anyhow, Result};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::keypair::read_keypair_file;
use solana_sdk::signer::Signer;
use stx_jito::MAINNET_GLOBAL;

pub struct EngineConfig {
    pub rpc_url: String,
    pub jito_base_url: String,
    pub keypair: Keypair,
    pub cu_limit: u32,
    pub cu_price_micro: u64,
    pub max_attempts: u32,
    pub anthropic_key: Option<String>,
}

impl EngineConfig {
    /// Load from `.env.local` (RPC + optional Anthropic key) and a keypair file.
    pub fn load(keypair_path: &str) -> Result<Self> {
        let _ = dotenvy::from_filename(".env.local");
        let rpc_url = std::env::var("HELIUS_RPC_ENDPOINT")
            .or_else(|_| std::env::var("RPC_URL"))
            .map_err(|_| anyhow!("set HELIUS_RPC_ENDPOINT (or RPC_URL) in .env.local"))?;
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow!("failed to read keypair {keypair_path}: {e}"))?;
        Ok(Self {
            rpc_url,
            jito_base_url: MAINNET_GLOBAL.to_string(),
            keypair,
            // A bundle of compute-budget ixs + a SOL transfer needs very little
            // compute; a tight limit keeps the priority fee tiny.
            cu_limit: 10_000,
            cu_price_micro: 1_000,
            max_attempts: 3,
            anthropic_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        })
    }

    pub fn payer_pubkey(&self) -> String {
        self.keypair.pubkey().to_string()
    }
}
