//! Engine configuration, loaded from `.env.local` + a keypair file.

use anyhow::{anyhow, Result};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::keypair::read_keypair_file;
use solana_sdk::signer::Signer;

pub struct EngineConfig {
    pub rpc_url: String,
    pub jito_base_url: String,
    /// Regional Jito endpoints the bundle is fanned out to on submit. Each region
    /// has an independent per-IP rate budget and forwards to its own connected
    /// leaders, so fanning out raises landing probability. The same signature can
    /// only land once, so there is no double-submit risk.
    pub jito_regions: Vec<String>,
    pub yellowstone_endpoint: Option<String>,
    pub yellowstone_x_token: Option<String>,
    pub keypair: Keypair,
    pub cu_limit: u32,
    pub cu_price_micro: u64,
    pub max_attempts: u32,
    pub anthropic_key: Option<String>,
}

impl EngineConfig {
    /// Load from `.env.local` (RPC + optional gRPC + Anthropic key) and a keypair.
    pub fn load(keypair_path: &str) -> Result<Self> {
        let _ = dotenvy::from_filename(".env.local");
        let rpc_url = std::env::var("SOLINFRA_RPC_URL")
            .or_else(|_| std::env::var("HELIUS_RPC_ENDPOINT"))
            .or_else(|_| std::env::var("RPC_URL"))
            .map_err(|_| anyhow!("set SOLINFRA_RPC_URL or HELIUS_RPC_ENDPOINT in .env.local"))?;
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow!("failed to read keypair {keypair_path}: {e}"))?;
        Ok(Self {
            rpc_url,
            // A regional endpoint keeps submit + status queries on the same
            // backend (the global LB can answer status "Invalid" for a bundle a
            // different backend accepted). Frankfurt matches our infra region.
            jito_base_url: std::env::var("JITO_BLOCK_ENGINE_URL")
                .unwrap_or_else(|_| "https://frankfurt.mainnet.block-engine.jito.wtf".to_string()),
            // Default to all mainnet regions; override with a comma-separated
            // JITO_REGIONS to narrow the fan-out.
            jito_regions: std::env::var("JITO_REGIONS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|r| r.trim().to_string())
                        .filter(|r| !r.is_empty())
                        .collect()
                })
                .unwrap_or_else(|| {
                    stx_jito::MAINNET_REGIONS
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
            yellowstone_endpoint: std::env::var("SOLINFRA_GRPC_ENDPOINT").ok(),
            yellowstone_x_token: std::env::var("SOLINFRA_GRPC_X_TOKEN")
                .or_else(|_| std::env::var("SOLINFRA_GRPC_SECRET"))
                .ok(),
            keypair,
            // A bundle of compute-budget ixs + a SOL transfer needs very little
            // compute; a tight limit keeps the priority fee tiny.
            cu_limit: 10_000,
            cu_price_micro: 1_000,
            max_attempts: std::env::var("STX_MAX_ATTEMPTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|&n| n >= 1)
                .unwrap_or(4),
            anthropic_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        })
    }

    pub fn payer_pubkey(&self) -> String {
        self.keypair.pubkey().to_string()
    }
}
