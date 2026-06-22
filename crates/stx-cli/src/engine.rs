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
use stx_ingestor::{run as ingestor_run, account_tx_request, IngestorConfig, Observation};
use stx_jito::{
    build_bundle, classify, detect_congestion, recommend_tip, submit_to_regions, BundleParams,
    FailureSignals, JitoClient, SolanaRpc, TipFloorClient,
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
    /// Validator identity that produced the landing slot (leader-window context).
    pub leader: Option<String>,
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

/// Short region name from a Jito block-engine URL
/// (`https://frankfurt.mainnet.block-engine.jito.wtf` -> `frankfurt`).
fn region_label(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('.')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Spawn a Yellowstone subscription on the fee `payer`'s non-vote transactions
/// at `processed` (BEFORE the bundle is submitted, so a sub-second landing is
/// never missed) and forward the first one whose signature matches as
/// `(slot, exec_failed)`. We filter by account (not by a bare `signature`
/// filter, which some geyser providers silently ignore) and match the exact
/// signature in code. With no endpoint configured the receiver is simply never
/// fed (RPC then confirms).
fn spawn_signature_stream(
    endpoint: Option<&str>,
    x_token: Option<String>,
    payer: &str,
    signature: &str,
) -> (mpsc::Receiver<(u64, bool)>, JoinHandle<()>) {
    let (hit_tx, hit_rx) = mpsc::channel::<(u64, bool)>(4);
    let Some(endpoint) = endpoint else {
        return (hit_rx, tokio::spawn(async {}));
    };
    let icfg = IngestorConfig::new(endpoint.to_string(), x_token);
    let req = account_tx_request(payer, Commitment::Processed);
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

/// Track a submitted signature through the commitment ladder, recording the
/// processed (landed), confirmed, and finalized milestones into `store` with
/// their real timestamps. Landing is detected from the pre-opened signature
/// stream (validator ground truth) with an RPC `getSignatureStatuses` fallback;
/// the confirmed -> finalized progression is then measured by fast RPC polling
/// (authoritative for commitment), so each delta reflects real wall-clock time.
/// Returns `(slot, exec_failed)` once the transaction is on-chain, or `None` if
/// it never lands within `land_timeout`. Once on-chain it is never resubmitted.
#[allow(clippy::too_many_arguments)]
async fn track_commitments(
    hit_rx: &mut mpsc::Receiver<(u64, bool)>,
    rpc: &SolanaRpc,
    signature: &str,
    store: &mut EventStore,
    trace_id: &TraceId,
    ltx: &LogicalTxId,
    land_timeout: Duration,
    finalize_timeout: Duration,
) -> Option<(u64, bool)> {
    let sigs = [signature.to_string()];
    let record = |store: &mut EventStore, ev: LifecycleEvent, slot: u64| {
        store.append(trace_id.clone(), ltx.clone(), Some(Slot(slot)), ev);
    };

    // Phase 1: await landing (processed). The signature stream (validator ground
    // truth, subscribed before submit) is the race-free safety net; RPC
    // `getSignatureStatuses` is polled every 500ms so the `processed` stage is
    // caught before the ~0.6s jump to confirmed (which keeps the first ladder
    // delta accurate). `biased` checks the stream first on every wake.
    let start = tokio::time::Instant::now();
    let mut rpc_tick = tokio::time::interval(Duration::from_millis(500));
    rpc_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut landed: Option<(u64, bool)> = None;
    let mut stream_open = true;
    while landed.is_none() {
        tokio::select! {
            biased;
            hit = hit_rx.recv(), if stream_open => match hit {
                Some((slot, failed)) => {
                    landed = Some((slot, failed));
                    eprintln!("  processed: slot {slot} (stream)");
                }
                None => stream_open = false,
            },
            _ = rpc_tick.tick() => {
                if let Ok(statuses) = rpc.get_signature_statuses(&sigs).await {
                    if let Some(Some(st)) = statuses.first() {
                        landed = Some((st.slot, !st.succeeded()));
                        eprintln!("  processed: slot {} (rpc)", st.slot);
                    }
                }
            }
        }
        if landed.is_none() && start.elapsed() >= land_timeout {
            return None;
        }
    }
    let (slot, failed) = landed.unwrap();
    record(store, LifecycleEvent::Landed { slot: Slot(slot) }, slot);

    // Phase 2: measure the confirmed -> finalized progression by fast polling.
    let mut got_confirmed = false;
    let phase2 = tokio::time::Instant::now();
    while phase2.elapsed() < finalize_timeout {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let Ok(statuses) = rpc.get_signature_statuses(&sigs).await else {
            continue;
        };
        let Some(Some(st)) = statuses.first() else {
            continue;
        };
        let c = st.commitment();
        if !got_confirmed && matches!(c, Some(Commitment::Confirmed) | Some(Commitment::Finalized)) {
            got_confirmed = true;
            record(
                store,
                LifecycleEvent::CommitmentReached {
                    commitment: Commitment::Confirmed,
                    slot: Slot(slot),
                },
                slot,
            );
            eprintln!("  confirmed: slot {slot}");
        }
        if matches!(c, Some(Commitment::Finalized)) {
            record(
                store,
                LifecycleEvent::CommitmentReached {
                    commitment: Commitment::Finalized,
                    slot: Slot(slot),
                },
                slot,
            );
            eprintln!("  finalized: slot {slot}");
            break;
        }
    }

    Some((slot, failed))
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
            max_tip: Lamports(cfg.max_tip_lamports),
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
    let mut tip = recommend_tip(&floor, detect_congestion(&floor));
    // Never start above the spend ceiling (escalations past it abort below).
    if tip.0 > cfg.max_tip_lamports {
        tip = Lamports(cfg.max_tip_lamports);
    }
    let mut cu_limit = cfg.cu_limit;

    // Headroom over the tip for the base fee + priority fee.
    const FEE_BUFFER_LAMPORTS: u64 = 10_000;
    // Pre-flight balance. A tip is only charged on landing, but submitting a
    // bundle we cannot fund just burns attempts. Fetch once (the balance only
    // moves when a tx lands, at which point we stop). A failed fetch never
    // blocks submission.
    let balance = rpc
        .get_balance(&cfg.payer_pubkey())
        .await
        .unwrap_or(u64::MAX);

    let mut landed = false;
    let mut signature: Option<String> = None;
    let mut slot_out: Option<u64> = None;
    let mut leader_out: Option<String> = None;
    let mut attempt = 0u32;
    let mut decision_records: Vec<DecisionRecord> = Vec::new();
    let mut history: Vec<serde_json::Value> = Vec::new();

    while attempt < cfg.max_attempts {
        attempt += 1;

        // Affordability guard: never submit a bundle we cannot fund.
        if tip.0.saturating_add(FEE_BUFFER_LAMPORTS) > balance {
            store.append(
                trace_id.clone(),
                ltx.clone(),
                None,
                LifecycleEvent::Aborted {
                    reason: format!(
                        "insufficient balance: {balance} lamports cannot cover tip {} + fees",
                        tip.0
                    ),
                },
            );
            eprintln!(
                "aborting: insufficient balance ({balance} lamports) for tip {} + fees",
                tip.0
            );
            break;
        }

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

        // Open the signature subscription BEFORE submitting (so a sub-second
        // landing is never missed; streams only go forward).
        let (mut hit_rx, stream_handle) = spawn_signature_stream(
            cfg.yellowstone_endpoint.as_deref(),
            cfg.yellowstone_x_token.clone(),
            &cfg.payer_pubkey().to_string(),
            &sig,
        );

        // Fan the bundle out to every configured region concurrently. Each region
        // forwards to its own connected leaders, so this raises landing odds; the
        // same signature can only land once, so there is no double-submit risk.
        let region_refs: Vec<&str> = cfg.jito_regions.iter().map(|s| s.as_str()).collect();
        let results = submit_to_regions(http, &region_refs, &built.transactions).await;
        let accepted: Vec<String> = results
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(url, _)| region_label(url))
            .collect();
        let bundle_id = results
            .into_iter()
            .find_map(|(_, r)| r.ok())
            .ok_or_else(|| anyhow!("every region rejected the bundle"))?;
        eprintln!(
            "  dispatched to {}/{} regions: {}",
            accepted.len(),
            region_refs.len(),
            accepted.join(", ")
        );
        store.append(
            trace_id.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::Dispatched {
                bundle_id: bundle_id.clone(),
                regions: accepted.clone(),
            },
        );
        store.append(trace_id.clone(), ltx.clone(), None, LifecycleEvent::MarkedInflight);
        eprintln!(
            "attempt {attempt}: bundle={} sig={} tip={} lamports",
            bundle_id.as_str(),
            sig,
            tip.0
        );

        // Track the commitment ladder (processed -> confirmed -> finalized),
        // recording each stage with its real timestamp.
        let land_timeout = Duration::from_secs(opts.confirm_timeout_secs);
        let finalize_timeout = Duration::from_secs(25);
        let landing = track_commitments(
            &mut hit_rx,
            &rpc,
            &sig,
            &mut store,
            &trace_id,
            &ltx,
            land_timeout,
            finalize_timeout,
        )
        .await;
        stream_handle.abort();

        // Success: landed and executed cleanly (the ladder is already recorded).
        if let Some((slot, false)) = landing {
            // Annotate with the leader that produced the landing slot.
            let leader = rpc
                .get_slot_leaders(slot, 1)
                .await
                .ok()
                .and_then(|v| v.into_iter().next());
            match &leader {
                Some(l) => eprintln!("LANDED slot={slot} leader={l}  https://solscan.io/tx/{sig}"),
                None => eprintln!("LANDED slot={slot}  https://solscan.io/tx/{sig}"),
            }
            leader_out = leader;
            landed = true;
            slot_out = Some(slot);
            break;
        }

        // Not landed (or landed but execution failed): classify and retry.
        let landed_slot: Option<u64> = landing.map(|(slot, _)| slot);
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
        // Spend-safety ceiling: refuse to overpay. Tips are only charged on a
        // landing, so capping the tip we will submit hard-bounds the cost of the
        // run. If even the escalated tip exceeds the ceiling, abort rather than
        // land at any price (or pointlessly retry at a clamped tip).
        if remedy.params.tip_lamports.0 > cfg.max_tip_lamports {
            store.append(
                trace_id.clone(),
                ltx.clone(),
                None,
                LifecycleEvent::Aborted {
                    reason: format!(
                        "tip ceiling reached: next tip {} exceeds max {} lamports",
                        remedy.params.tip_lamports.0, cfg.max_tip_lamports
                    ),
                },
            );
            eprintln!(
                "aborting: required tip {} exceeds ceiling {} lamports (refusing to overpay)",
                remedy.params.tip_lamports.0, cfg.max_tip_lamports
            );
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
        leader: leader_out,
        attempts: attempt,
        trace_id,
        store,
        decision_records,
    })
}
