//! stx CLI.
//!
//! Runnable demonstrations of the stack:
//! - `tip-floor` / `tip-accounts` - live Jito data (no credentials needed).
//! - `fault-inject <scenario>` - run the AI agent against an injected failure
//!   and print the bounded, auditable decision record (needs `ANTHROPIC_API_KEY`).
//!
//! Live bundle submission (`submit`) lands once the Yellowstone/RPC/keypair
//! credentials are wired; the deterministic pieces it composes are already built
//! and tested in the library crates.

use stx_cli::{config, diagnose, engine};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::time::Duration;
use stx_agent::{
    build_record, validate, AgentConfig, AnthropicClient, FaultScenario, GuardrailPolicy, MockTools,
    ReasoningAgent, ValidationContext,
};
use stx_core::{
    spans_for_trace, AgentAction, Commitment, Decision, DecisionParams, Lamports, LogicalTxId,
    TraceId,
};
use stx_jito::{leader_windows, JitoClient, SolanaRpc, TipFloorClient};

#[derive(Parser)]
#[command(name = "stx", version, about = "Smart Solana transaction control tower")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch and print the live Jito tip floor (percentiles, lamports).
    TipFloor,
    /// Fetch the Jito tip accounts.
    TipAccounts {
        #[arg(long, default_value = "mainnet")]
        network: String,
    },
    /// Run the AI agent against an injected failure and print its decision.
    FaultInject {
        #[arg(value_enum)]
        scenario: ScenarioArg,
    },
    /// Connect to the Yellowstone gRPC stream and print N slot updates. Verifies
    /// the gRPC endpoint + x-token (reads SOLINFRA_GRPC_* from .env.local).
    WatchSlots {
        #[arg(long, default_value_t = 10)]
        count: u32,
    },
    /// Show the upcoming leader schedule (validator per slot window) from the
    /// RPC, grouped into 4-slot leader windows. Read-only.
    Leaders {
        #[arg(long, default_value_t = 16)]
        count: u64,
    },
    /// Build, submit and track a real Jito bundle. Use --dry-run to build and
    /// simulate only. Reads RPC from .env.local and the keypair from wallet.json.
    Submit {
        /// Build and simulate only; do not submit on-chain.
        #[arg(long)]
        dry_run: bool,
        /// Path to the keypair file.
        #[arg(long, default_value = "wallet.json")]
        keypair: String,
        /// Override the Jito tip account (default: fetched, first of 8).
        #[arg(long)]
        tip_account: Option<String>,
        /// Confirmation timeout in seconds.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        /// Let the AI agent own the retry decision (needs ANTHROPIC_API_KEY).
        #[arg(long)]
        use_agent: bool,
    },
    /// Race the naive baseline against the full stx engine on the same
    /// transaction, same instant, same floor snapshot. Prints the comparison and
    /// writes both traces (for the dashboard side-by-side).
    Race {
        /// Confirmation timeout in seconds, per attempt, applied to both lanes.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        /// Let the AI agent drive the stx lane's retries.
        #[arg(long)]
        use_agent: bool,
        /// Path to the keypair file.
        #[arg(long, default_value = "wallet.json")]
        keypair: String,
        /// Where to write the naive lane's trace (JSON).
        #[arg(long, default_value = "runs/race-naive.json")]
        out_naive: String,
        /// Where to write the stx lane's trace (JSON).
        #[arg(long, default_value = "runs/race-stx.json")]
        out_stx: String,
    },
    /// Autopsy any transaction: fetch what happened on-chain, classify it, and
    /// have the AI explain why it landed or died and what to change. Read-only,
    /// works on any signature (not just stx's own).
    Diagnose {
        /// The transaction signature to diagnose.
        signature: String,
    },
}

#[derive(ValueEnum, Clone, Copy)]
enum ScenarioArg {
    BlockhashExpiry,
    FeeStarvation,
    ComputeExhaustion,
}

impl From<ScenarioArg> for FaultScenario {
    fn from(s: ScenarioArg) -> Self {
        match s {
            ScenarioArg::BlockhashExpiry => FaultScenario::BlockhashExpiry,
            ScenarioArg::FeeStarvation => FaultScenario::FeeStarvation,
            ScenarioArg::ComputeExhaustion => FaultScenario::ComputeExhaustion,
        }
    }
}

/// The deterministic fallback decision the guardrail uses for low-confidence
/// agent proposals.
fn fallback_decision() -> Decision {
    Decision {
        action: AgentAction::Resubmit,
        params: DecisionParams {
            tip_lamports: Lamports(30_000),
            cu_limit: None,
            refresh_blockhash: false,
        },
        hypotheses: vec![],
        chosen_cause: "fallback".into(),
        justification: "static fallback policy".into(),
        confidence: 1.0,
        expected_effect: "resubmit with the default tip".into(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(120))
        .build()
        .context("building http client")?;

    match cli.command {
        Command::TipFloor => {
            let floor = TipFloorClient::new(http).fetch().await?;
            println!("Jito tip floor @ {}", floor.at);
            println!(
                "  p25 {:>10} | p50 {:>10} | p75 {:>10} | p95 {:>10} | p99 {:>10} | ema {}",
                floor.p25.0, floor.p50.0, floor.p75.0, floor.p95.0, floor.p99.0, floor.ema_p50.0
            );
        }
        Command::TipAccounts { network } => {
            let client = if network == "testnet" {
                JitoClient::testnet(http)
            } else {
                JitoClient::mainnet(http)
            };
            for account in client.get_tip_accounts().await? {
                println!("{account}");
            }
        }
        Command::FaultInject { scenario } => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow!("set ANTHROPIC_API_KEY to run the AI agent"))?;
            let scenario: FaultScenario = scenario.into();
            let observations = scenario.observations();

            let agent = ReasoningAgent::new(
                AnthropicClient::new(http, key),
                AgentConfig::default(),
            );
            let tools = MockTools { scenario };

            eprintln!("running agent on {} ...", scenario.label());
            let run = agent
                .decide(scenario.label(), observations.clone(), &tools)
                .await?;

            let (bounded, report) = validate(
                run.decision.clone(),
                &GuardrailPolicy::default(),
                &ValidationContext {
                    attempt: 1,
                    blockhash_expired: matches!(scenario, FaultScenario::BlockhashExpiry),
                },
                &fallback_decision(),
            );

            let record = build_record(
                TraceId::generate(),
                LogicalTxId::generate(),
                1,
                scenario.label(),
                observations,
                &run,
                bounded,
                report,
            );
            println!("{}", serde_json::to_string_pretty(&record)?);
        }
        Command::WatchSlots { count } => {
            let _ = dotenvy::from_filename(".env.local");
            let endpoint = std::env::var("SOLINFRA_GRPC_ENDPOINT")
                .map_err(|_| anyhow!("set SOLINFRA_GRPC_ENDPOINT in .env.local"))?;
            let token = std::env::var("SOLINFRA_GRPC_X_TOKEN")
                .or_else(|_| std::env::var("SOLINFRA_GRPC_SECRET"))
                .ok();
            let cfg = stx_ingestor::IngestorConfig::new(endpoint, token);
            let req = stx_ingestor::slots_request(stx_core::Commitment::Confirmed);
            let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
            let handle = tokio::spawn(async move {
                if let Err(e) = stx_ingestor::run(cfg, req, tx).await {
                    eprintln!("stream error: {e}");
                }
            });
            let mut n = 0u32;
            while let Some(obs) = rx.recv().await {
                match obs {
                    stx_ingestor::Observation::Slot { slot, status, .. } => {
                        println!("slot {slot} {status:?}");
                        n += 1;
                    }
                    stx_ingestor::Observation::Ping => {}
                    other => println!("{other:?}"),
                }
                if n >= count {
                    break;
                }
            }
            handle.abort();
        }
        Command::Leaders { count } => {
            let _ = dotenvy::from_filename(".env.local");
            let rpc_url = std::env::var("SOLINFRA_RPC_URL")
                .or_else(|_| std::env::var("HELIUS_RPC_ENDPOINT"))
                .or_else(|_| std::env::var("RPC_URL"))
                .map_err(|_| anyhow!("set SOLINFRA_RPC_URL or HELIUS_RPC_ENDPOINT in .env.local"))?;
            let rpc = SolanaRpc::new(http.clone(), rpc_url);
            let slot = rpc.get_slot(Commitment::Confirmed).await?;
            let leaders = rpc.get_slot_leaders(slot, count).await?;
            let windows = leader_windows(slot, &leaders);
            println!(
                "current slot {slot} - next {count} slots, {} leader windows:",
                windows.len()
            );
            for w in &windows {
                let here = if w.first_slot <= slot && slot <= w.last_slot {
                    "  <- current"
                } else {
                    ""
                };
                println!(
                    "  slots {}-{} ({} slots)  {}{}",
                    w.first_slot,
                    w.last_slot,
                    w.slot_count(),
                    w.leader,
                    here
                );
            }
        }
        Command::Submit {
            dry_run,
            keypair,
            tip_account,
            timeout,
            use_agent,
        } => {
            let cfg = config::EngineConfig::load(&keypair)?;
            eprintln!("payer: {}", cfg.payer_pubkey());
            let outcome = engine::submit_and_track(
                &cfg,
                &http,
                &engine::SubmitOptions {
                    dry_run,
                    tip_account,
                    confirm_timeout_secs: timeout,
                    use_agent,
                    floor_override: None,
                },
            )
            .await?;

            if !outcome.decision_records.is_empty() {
                eprintln!("AI decision records:");
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&outcome.decision_records)?
                );
            }

            // Span waterfall (the latency deltas) from the real run.
            let events = outcome.store.events_for_trace(&outcome.trace_id);
            let spans = spans_for_trace(&events);
            if !spans.is_empty() {
                eprintln!("span waterfall:");
                for s in &spans {
                    eprintln!("  {:?}: {} ms", s.name, s.duration_ms().unwrap_or(0));
                }
            }
            eprintln!(
                "result: landed={} attempts={} sig={:?} slot={:?} leader={:?}",
                outcome.landed, outcome.attempts, outcome.signature, outcome.slot, outcome.leader
            );

            // The lifecycle log (a bounty deliverable) to stdout as JSON.
            println!("{}", serde_json::to_string_pretty(outcome.store.events())?);
        }
        Command::Race {
            timeout,
            use_agent,
            keypair,
            out_naive,
            out_stx,
        } => {
            let cfg = config::EngineConfig::load(&keypair)?;
            eprintln!("payer: {}", cfg.payer_pubkey());
            eprintln!("racing naive baseline vs stx (same floor, same instant)...\n");
            let (naive, smart) = engine::run_race(&cfg, &http, timeout, use_agent).await?;
            let (n_tip, n_ttl) = engine::race_metrics(&naive);
            let (s_tip, s_ttl) = engine::race_metrics(&smart);
            let fmt_ttl = |t: Option<u64>| {
                t.map(|ms| format!("{:.1}s", ms as f64 / 1000.0))
                    .unwrap_or_else(|| "-".to_string())
            };
            let opt = |o: Option<u64>| o.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string());
            let row = |label: &str, a: String, b: String| eprintln!("{label:<22}{a:<18}{b}");
            eprintln!("================= RACE RESULT =================");
            row("", "naive".to_string(), "stx".to_string());
            row("landed", naive.landed.to_string(), smart.landed.to_string());
            row("attempts", naive.attempts.to_string(), smart.attempts.to_string());
            row("final tip (lamports)", n_tip.to_string(), s_tip.to_string());
            row("time to land", fmt_ttl(n_ttl), fmt_ttl(s_ttl));
            row("landed slot", opt(naive.slot), opt(smart.slot));
            eprintln!("==============================================");

            std::fs::create_dir_all("runs").ok();
            std::fs::write(&out_naive, serde_json::to_string_pretty(naive.store.events())?)?;
            std::fs::write(&out_stx, serde_json::to_string_pretty(smart.store.events())?)?;
            if !smart.decision_records.is_empty() {
                std::fs::write(
                    "runs/race-stx-decisions.json",
                    serde_json::to_string_pretty(&smart.decision_records)?,
                )?;
            }
            eprintln!("traces written: {out_naive}  {out_stx}");
        }
        Command::Diagnose { signature } => {
            let _ = dotenvy::from_filename(".env.local");
            // The autopsy needs full transaction history, so prefer a
            // history-capable RPC (Helius); some providers (e.g. Solinfra) return
            // null for getTransaction on older signatures.
            let rpc_url = std::env::var("HELIUS_RPC_ENDPOINT")
                .or_else(|_| std::env::var("SOLINFRA_RPC_URL"))
                .or_else(|_| std::env::var("RPC_URL"))
                .map_err(|_| anyhow!("set HELIUS_RPC_ENDPOINT or SOLINFRA_RPC_URL in .env.local"))?;
            let rpc = SolanaRpc::new(http.clone(), rpc_url);
            let anthropic = std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .map(|k| AnthropicClient::new(http.clone(), k));
            let d = diagnose::diagnose(&rpc, anthropic.as_ref(), &signature).await?;
            eprintln!("=== TRANSACTION AUTOPSY ===");
            eprintln!("{}", d.headline);
            eprintln!();
            for f in &d.facts {
                eprintln!("  - {f}");
            }
            if let Some(e) = &d.explanation {
                eprintln!("\ndiagnosis:\n  {e}");
            }
            // Structured result to stdout.
            println!("{}", serde_json::to_string_pretty(&d)?);
        }
    }

    Ok(())
}
