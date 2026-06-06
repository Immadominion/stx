//! The submit-and-track orchestrator.
//!
//! Wires the deterministic core end to end: fetch a `confirmed` blockhash, pick
//! a tip from live floor data, build and submit a Jito bundle, confirm landing
//! from Jito's authoritative bundle status, and on failure classify the cause
//! and apply the deterministic fallback remedy (refresh blockhash / raise tip /
//! raise CU / abort) before a bounded retry. Every step appends a lifecycle
//! event, so the trace, its span durations, and the exportable log all derive
//! from one append-only source.
//!
//! Stream-based confirmation (Yellowstone) is added on top of this once the
//! gRPC endpoint is configured; the structure here already records the
//! commitment that confirmation reports.

use crate::config::EngineConfig;
use anyhow::{anyhow, Result};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use std::str::FromStr;
use std::time::Duration;
use stx_core::{
    fallback_remedy, AgentAction, Commitment, EventStore, Lamports, LifecycleEvent, LogicalTxId,
    Slot, TipFloor, TipSource, TraceId,
};
use stx_jito::{
    build_bundle, classify, recommend_tip, BundleParams, Congestion, FailureSignals, InflightStatus,
    JitoClient, SolanaRpc, TipFloorClient,
};

pub struct SubmitOptions {
    pub dry_run: bool,
    pub tip_account: Option<String>,
    pub confirm_timeout_secs: u64,
}

pub struct SubmitOutcome {
    pub landed: bool,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub attempts: u32,
    pub trace_id: TraceId,
    pub store: EventStore,
}

enum Confirm {
    Landed { slot: u64, commitment: Commitment },
    FailedOrTimeout { reason: String },
}

/// Conservative static floor if the live tip-floor fetch fails.
fn default_floor() -> TipFloor {
    TipFloor {
        at: chrono::Utc::now(),
        p25: Lamports(1_000),
        p50: Lamports(10_000),
        p75: Lamports(100_000),
        p95: Lamports(500_000),
        p99: Lamports(1_000_000),
        ema_p50: Lamports(10_000),
    }
}

async fn confirm_via_jito(
    jito: &JitoClient,
    bundle_id: &str,
    timeout: Duration,
) -> Result<Confirm> {
    let start = tokio::time::Instant::now();
    let ids = [bundle_id.to_string()];
    loop {
        let inflight = jito.get_inflight_bundle_statuses(&ids).await?;
        if let Some(s) = inflight.first() {
            match s.status {
                InflightStatus::Landed => {
                    // Pull the authoritative slot + commitment.
                    if let Ok(statuses) = jito.get_bundle_statuses(&ids).await {
                        if let Some(Some(bs)) = statuses.first() {
                            let commitment = bs.commitment().unwrap_or(Commitment::Processed);
                            return Ok(Confirm::Landed {
                                slot: bs.slot,
                                commitment,
                            });
                        }
                    }
                    return Ok(Confirm::Landed {
                        slot: s.landed_slot.unwrap_or(0),
                        commitment: Commitment::Processed,
                    });
                }
                InflightStatus::Failed | InflightStatus::Invalid => {
                    return Ok(Confirm::FailedOrTimeout {
                        reason: format!("bundle status {:?}", s.status),
                    });
                }
                InflightStatus::Pending => {}
            }
        }
        if start.elapsed() >= timeout {
            return Ok(Confirm::FailedOrTimeout {
                reason: "confirmation timeout".to_string(),
            });
        }
        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}

/// Submit a bundle and track it through its lifecycle, retrying on failure with
/// a cause-appropriate remedy.
pub async fn submit_and_track(
    cfg: &EngineConfig,
    http: &reqwest::Client,
    opts: &SubmitOptions,
) -> Result<SubmitOutcome> {
    let rpc = SolanaRpc::new(http.clone(), cfg.rpc_url.clone());
    let jito = JitoClient::new(http.clone(), cfg.jito_base_url.clone());
    let tip_client = TipFloorClient::new(http.clone());

    let tip_account = match &opts.tip_account {
        Some(s) => Pubkey::from_str(s).map_err(|e| anyhow!("invalid tip account: {e}"))?,
        None => {
            let accounts = jito.get_tip_accounts().await?;
            let first = accounts
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("no tip accounts returned"))?;
            Pubkey::from_str(&first).map_err(|e| anyhow!("invalid tip account: {e}"))?
        }
    };

    let mut store = EventStore::new();
    let trace_id = TraceId::generate();
    let ltx = LogicalTxId::generate();
    store.append(
        trace_id.clone(),
        ltx.clone(),
        None,
        LifecycleEvent::Drafted {
            logical_tx_id: ltx.clone(),
        },
    );

    let floor = tip_client.fetch().await.ok().unwrap_or_else(default_floor);
    let mut tip = recommend_tip(&floor, Congestion::Normal);
    let mut cu_limit = cfg.cu_limit;

    let mut landed = false;
    let mut signature: Option<String> = None;
    let mut slot_out: Option<u64> = None;
    let mut attempt = 0u32;

    while attempt < cfg.max_attempts {
        attempt += 1;

        let bh = rpc.get_latest_blockhash(Commitment::Confirmed).await?;
        let blockhash = Hash::from_str(&bh.blockhash).map_err(|e| anyhow!("invalid blockhash: {e}"))?;

        store.append(
            trace_id.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::TipDecided {
                tip_lamports: tip,
                source: TipSource::StaticPolicy,
            },
        );

        let built = build_bundle(BundleParams {
            payer: &cfg.keypair,
            recent_blockhash: blockhash,
            tip_account,
            tip_lamports: tip.0,
            cu_limit,
            cu_price_micro_lamports: cfg.cu_price_micro,
            extra_instructions: vec![],
        })?;
        let sig = built.signatures[0].as_str().to_string();
        signature = Some(sig.clone());
        store.append(
            trace_id.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::Built {
                signatures: built.signatures.clone(),
            },
        );

        if opts.dry_run {
            let sim = rpc
                .simulate_transaction(&built.transactions[0], Commitment::Confirmed)
                .await?;
            eprintln!(
                "[dry-run] attempt {attempt}: payer={} sig={} tip={} sim_ok={} cu_consumed={:?} err={}",
                cfg.keypair.pubkey(),
                sig,
                tip.0,
                sim.succeeded(),
                sim.units_consumed,
                sim.err
            );
            break;
        }

        let bundle_id = jito.send_bundle(&built.transactions).await?;
        store.append(
            trace_id.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::Dispatched {
                bundle_id: bundle_id.clone(),
                regions: vec![cfg.jito_base_url.clone()],
            },
        );
        store.append(trace_id.clone(), ltx.clone(), None, LifecycleEvent::MarkedInflight);
        eprintln!(
            "attempt {attempt}: bundle={} sig={} tip={} lamports",
            bundle_id.as_str(),
            sig,
            tip.0
        );

        match confirm_via_jito(&jito, bundle_id.as_str(), Duration::from_secs(opts.confirm_timeout_secs)).await? {
            Confirm::Landed { slot, commitment } => {
                store.append(
                    trace_id.clone(),
                    ltx.clone(),
                    Some(Slot(slot)),
                    LifecycleEvent::Landed { slot: Slot(slot) },
                );
                store.append(
                    trace_id.clone(),
                    ltx.clone(),
                    Some(Slot(slot)),
                    LifecycleEvent::CommitmentReached {
                        commitment,
                        slot: Slot(slot),
                    },
                );
                eprintln!("LANDED slot={slot} commitment={commitment:?}  https://solscan.io/tx/{sig}");
                landed = true;
                slot_out = Some(slot);
                break;
            }
            Confirm::FailedOrTimeout { reason } => {
                // Compute blockhash age for classification.
                let cur_height = rpc.get_block_height(Commitment::Confirmed).await.unwrap_or(0);
                let fetch_height = bh.last_valid_block_height.saturating_sub(150);
                let age = cur_height.saturating_sub(fetch_height);
                let signals = FailureSignals {
                    landed: false,
                    blockhash_age_blocks: Some(age),
                    tip_lamports: Some(tip.0),
                    tip_floor_p50: Some(floor.p50.0),
                    ..Default::default()
                };
                let class = classify(&signals);
                store.append(
                    trace_id.clone(),
                    ltx.clone(),
                    None,
                    LifecycleEvent::Failed {
                        class: class.clone(),
                    },
                );
                eprintln!(
                    "attempt {attempt} not landed: {reason}; classified={:?} ({})",
                    class.kind, class.evidence
                );

                if attempt >= cfg.max_attempts {
                    store.append(
                        trace_id.clone(),
                        ltx.clone(),
                        None,
                        LifecycleEvent::Aborted {
                            reason: "max attempts reached".to_string(),
                        },
                    );
                    break;
                }

                let remedy = fallback_remedy(&class, &floor, tip, cu_limit);
                if remedy.action == AgentAction::Abort {
                    store.append(
                        trace_id.clone(),
                        ltx.clone(),
                        None,
                        LifecycleEvent::Aborted {
                            reason: remedy.chosen_cause.clone(),
                        },
                    );
                    eprintln!("aborting: {}", remedy.justification);
                    break;
                }
                tip = remedy.params.tip_lamports;
                if let Some(cu) = remedy.params.cu_limit {
                    cu_limit = cu;
                }
                store.append(
                    trace_id.clone(),
                    ltx.clone(),
                    None,
                    LifecycleEvent::RetryScheduled {
                        child_trace: TraceId::generate(),
                        attempt: attempt + 1,
                    },
                );
                eprintln!(
                    "retry {} with tip={} cu={} ({})",
                    attempt + 1,
                    tip.0,
                    cu_limit,
                    remedy.chosen_cause
                );
            }
        }
    }

    Ok(SubmitOutcome {
        landed,
        signature,
        slot: slot_out,
        attempts: attempt,
        trace_id,
        store,
    })
}
