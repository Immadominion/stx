//! The submit-and-track orchestrator.
//!
//! Wires the deterministic core end to end: fetch a `confirmed` blockhash, pick
//! a tip from live floor data, build and submit a Jito bundle, confirm landing
//! from the Yellowstone stream (subscribed BEFORE submit) cross-checked against
//! an authoritative RPC `getSignatureStatuses` poll, and on failure classify the
//! cause and apply a remedy (the AI agent, or the deterministic fallback) before
//! a bounded retry. Every step appends a lifecycle event, so the trace, its span
//! durations, and the exportable log all derive from one append-only source.
//!
//! Confirmation deliberately does NOT trust Jito's `getInflightBundleStatuses`
//! for the landing decision: it can report `Invalid` for a bundle that actually
//! landed, which would false-negative and trigger a dangerous double-submit.

use crate::config::EngineConfig;
use anyhow::{anyhow, Result};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use crate::agent_tools::LiveTools;
use stx_agent::{
    build_record, validate, AgentConfig, AnthropicClient, GuardrailPolicy, ReasoningAgent,
    ValidationContext,
};
use stx_core::{
    fallback_remedy, AgentAction, Commitment, Decision, DecisionRecord, EventStore, FailureClass,
    FailureKind, Lamports, LifecycleEvent, LogicalTxId, Slot, TipFloor, TipSource, TraceId,
};
use stx_ingestor::{run as ingestor_run, signature_request, IngestorConfig, Observation};
use stx_jito::{
    build_bundle, classify, recommend_tip, BundleParams, Congestion, FailureSignals, JitoClient,
    SolanaRpc, TipFloorClient,
};

pub struct SubmitOptions {
    pub dry_run: bool,
    pub tip_account: Option<String>,
    pub confirm_timeout_secs: u64,
    /// Let the AI agent own the retry decision (else the deterministic fallback).
    pub use_agent: bool,
}

pub struct SubmitOutcome {
    pub landed: bool,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub attempts: u32,
    pub trace_id: TraceId,
    pub store: EventStore,
    /// AI decision records, when `--use-agent` drove a retry.
    pub decision_records: Vec<DecisionRecord>,
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

/// Spawn a Yellowstone subscription on `signature` (at `confirmed`) and forward
/// the first matching landing as `(slot, exec_failed)`. Spawned BEFORE the
/// bundle is submitted so a sub-second landing is never missed. With no endpoint
/// configured the receiver is simply never fed (RPC then carries confirmation).
fn spawn_signature_stream(
    endpoint: Option<&str>,
    x_token: Option<String>,
    signature: &str,
) -> (mpsc::Receiver<(u64, bool)>, JoinHandle<()>) {
    let (hit_tx, hit_rx) = mpsc::channel::<(u64, bool)>(4);
    let Some(endpoint) = endpoint else {
        return (hit_rx, tokio::spawn(async {}));
    };
    let icfg = IngestorConfig::new(endpoint.to_string(), x_token);
    let req = signature_request(signature, Commitment::Confirmed);
    let target = signature.to_string();
    let handle = tokio::spawn(async move {
        let (obs_tx, mut obs_rx) = mpsc::channel(64);
        let run_task = tokio::spawn(async move {
            let _ = ingestor_run(icfg, req, obs_tx).await;
        });
        while let Some(obs) = obs_rx.recv().await {
            if let Observation::Transaction {
                signature: s,
                slot,
                failed,
            } = obs
            {
                if s == target {
                    let _ = hit_tx.send((slot, failed)).await;
                    break;
                }
            }
        }
        run_task.abort();
    });
    (hit_rx, handle)
}

/// Await landing via the pre-opened stream OR an authoritative RPC
/// `getSignatureStatuses` poll, whichever is first. Returns
/// `(slot, exec_failed, source)`, or `None` on timeout.
async fn await_landing(
    hit_rx: &mut mpsc::Receiver<(u64, bool)>,
    rpc: &SolanaRpc,
    signature: &str,
    timeout: Duration,
) -> Option<(u64, bool, &'static str)> {
    let start = tokio::time::Instant::now();
    let sigs = [signature.to_string()];
    loop {
        if let Ok((slot, failed)) = hit_rx.try_recv() {
            return Some((slot, failed, "stream"));
        }
        if let Ok(statuses) = rpc.get_signature_statuses(&sigs).await {
            if let Some(Some(st)) = statuses.first() {
                return Some((st.slot, !st.succeeded(), "rpc"));
            }
        }
        if start.elapsed() >= timeout {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

/// Ask the AI agent to choose the retry remedy, grounded in live tools and
/// bounded by the guardrail. Returns the bounded decision and its audit record,
/// or `None` if no API key is configured.
#[allow(clippy::too_many_arguments)]
async fn agent_decide(
    cfg: &EngineConfig,
    http: &reqwest::Client,
    tip_account: Pubkey,
    floor: &TipFloor,
    class: &FailureClass,
    tip: Lamports,
    cu_limit: u32,
    attempt: u32,
    history: Vec<serde_json::Value>,
    failure_ctx: serde_json::Value,
    trace_id: &TraceId,
    ltx: &LogicalTxId,
) -> Result<Option<(Decision, DecisionRecord)>> {
    let Some(key) = &cfg.anthropic_key else {
        return Ok(None);
    };
    let tools = LiveTools {
        rpc: SolanaRpc::new(http.clone(), cfg.rpc_url.clone()),
        keypair: cfg.keypair.insecure_clone(),
        tip_account,
        cu_price_micro: cfg.cu_price_micro,
        floor: floor.clone(),
        failure_ctx: failure_ctx.clone(),
        current_tip: tip.0,
        current_cu: cu_limit,
        attempt,
        history,
    };
    let agent = ReasoningAgent::new(
        AnthropicClient::new(http.clone(), key.clone()),
        AgentConfig::default(),
    );
    let run = agent.decide("tx_failed", failure_ctx.clone(), &tools).await?;
    let blockhash_expired = matches!(class.kind, FailureKind::ExpiredBlockhash);
    let fallback = fallback_remedy(class, floor, tip, cu_limit);
    let (bounded, report) = validate(
        run.decision.clone(),
        &GuardrailPolicy {
            max_attempts: cfg.max_attempts,
            ..GuardrailPolicy::default()
        },
        &ValidationContext {
            attempt,
            blockhash_expired,
        },
        &fallback,
    );
    let record = build_record(
        trace_id.clone(),
        ltx.clone(),
        attempt,
        "tx_failed",
        failure_ctx,
        &run,
        bounded.clone(),
        report,
    );
    Ok(Some((bounded, record)))
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
    let mut decision_records: Vec<DecisionRecord> = Vec::new();
    let mut history: Vec<serde_json::Value> = Vec::new();

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

        // Open the signature subscription BEFORE submitting (avoids the race
        // where a sub-second landing is missed; streams only go forward).
        let (mut hit_rx, stream_handle) = spawn_signature_stream(
            cfg.yellowstone_endpoint.as_deref(),
            cfg.yellowstone_x_token.clone(),
            &sig,
        );

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

        let timeout = Duration::from_secs(opts.confirm_timeout_secs);
        let landing = await_landing(&mut hit_rx, &rpc, &sig, timeout).await;
        stream_handle.abort();

        // Success: landed and executed cleanly.
        if let Some((slot, false, source)) = landing {
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
                    commitment: Commitment::Confirmed,
                    slot: Slot(slot),
                },
            );
            eprintln!("LANDED slot={slot} via {source}  https://solscan.io/tx/{sig}");
            landed = true;
            slot_out = Some(slot);
            break;
        }

        // Not landed (or landed but execution failed): record + classify.
        let landed_slot: Option<u64> = landing.map(|(slot, _, _)| slot);
        if let Some(slot) = landed_slot {
            store.append(
                trace_id.clone(),
                ltx.clone(),
                Some(Slot(slot)),
                LifecycleEvent::Landed { slot: Slot(slot) },
            );
        }
        let reason = if landed_slot.is_some() {
            "landed but execution failed".to_string()
        } else {
            "not confirmed within timeout".to_string()
        };
        let cur_height = rpc.get_block_height(Commitment::Confirmed).await.unwrap_or(0);
        let fetch_height = bh.last_valid_block_height.saturating_sub(150);
        let age = cur_height.saturating_sub(fetch_height);
        let signals = FailureSignals {
            landed: landed_slot.is_some(),
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
            "attempt {attempt} not confirmed: {reason}; classified={:?} ({})",
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

        history.push(serde_json::json!({
            "attempt": attempt,
            "tip_lamports": tip.0,
            "cu_limit": cu_limit,
            "outcome": "not_landed",
            "classified": format!("{:?}", class.kind),
        }));
        let failure_ctx = serde_json::json!({
            "error": reason,
            "landed": landed_slot.is_some(),
            "blockhash_age_blocks": age,
            "cu_requested": cu_limit,
            "tip_paid_lamports": tip.0,
            "tip_floor_p50": floor.p50.0,
            "classified_kind": format!("{:?}", class.kind),
        });
        let remedy = if opts.use_agent && cfg.anthropic_key.is_some() {
            match agent_decide(
                cfg, http, tip_account, &floor, &class, tip, cu_limit, attempt, history.clone(),
                failure_ctx, &trace_id, &ltx,
            )
            .await
            {
                Ok(Some((decision, record))) => {
                    eprintln!(
                        "agent: {:?} (confidence {:.2}) - {}",
                        decision.action, decision.confidence, decision.chosen_cause
                    );
                    decision_records.push(record);
                    decision
                }
                Ok(None) => fallback_remedy(&class, &floor, tip, cu_limit),
                Err(e) => {
                    eprintln!("agent error: {e}; using fallback policy");
                    fallback_remedy(&class, &floor, tip, cu_limit)
                }
            }
        } else {
            fallback_remedy(&class, &floor, tip, cu_limit)
        };
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

    Ok(SubmitOutcome {
        landed,
        signature,
        slot: slot_out,
        attempts: attempt,
        trace_id,
        store,
        decision_records,
    })
}
