//! Endpoint probe: confirm a Yellowstone endpoint actually streams transactions
//! for a given `account_include` filter, and measure their arrival. Defaults to
//! the SPL Token program (very high traffic); set `WATCH_ACCOUNT` to a specific
//! account (e.g. a fee payer) to verify that account's transactions are
//! delivered. Used to prove that a bare `signature` filter is silently ignored
//! by some providers while an `account_include` filter works.
//!
//! Run with the gRPC env sourced:
//!   set -a; . ./.env.local; set +a
//!   WATCH_ACCOUNT=<pubkey> WATCH_SECS=30 cargo run -p stx-ingestor --example watch_tx

use std::collections::HashMap;
use stx_ingestor::{run, IngestorConfig, Observation};
use yellowstone_grpc_proto::geyser::{SubscribeRequest, SubscribeRequestFilterTransactions};

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let endpoint = std::env::var("SOLINFRA_GRPC_ENDPOINT").expect("SOLINFRA_GRPC_ENDPOINT");
    let token = std::env::var("SOLINFRA_GRPC_X_TOKEN")
        .or_else(|_| std::env::var("SOLINFRA_GRPC_SECRET"))
        .ok();
    let cfg = IngestorConfig::new(endpoint, token);

    let account = std::env::var("WATCH_ACCOUNT")
        .unwrap_or_else(|_| "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string());
    let secs: u64 = std::env::var("WATCH_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
    eprintln!("watching account {account} for {secs}s");
    let mut transactions = HashMap::new();
    transactions.insert(
        "tx".to_string(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: None,
            signature: None,
            account_include: vec![account],
            account_exclude: vec![],
            account_required: vec![],
        },
    );
    let req = SubscribeRequest {
        transactions,
        commitment: Some(0), // processed
        ..Default::default()
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    tokio::spawn(async move {
        if let Err(e) = run(cfg, req, tx).await {
            eprintln!("stream error: {e}");
        }
    });

    let mut n = 0u32;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    while let Ok(Some(obs)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if let Observation::Transaction { slot, signature, .. } = obs {
            n += 1;
            eprintln!("tx #{n} at slot {slot}: {}...", &signature[..16.min(signature.len())]);
        }
    }
    eprintln!("total transaction observations in {secs}s: {n}");
}
