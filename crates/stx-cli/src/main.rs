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

mod agent_tools;
mod config;
mod engine;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use stx_agent::{
    build_record, validate, AgentConfig, AnthropicClient, FaultScenario, GuardrailPolicy, MockTools,
    ReasoningAgent, ValidationContext,
};
use stx_core::{
    spans_for_trace, AgentAction, Decision, DecisionParams, Lamports, LogicalTxId, TraceId,
};
use stx_jito::{JitoClient, TipFloorClient};

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
                "result: landed={} attempts={} sig={:?} slot={:?}",
                outcome.landed, outcome.attempts, outcome.signature, outcome.slot
            );

            // The lifecycle log (a bounty deliverable) to stdout as JSON.
            println!("{}", serde_json::to_string_pretty(outcome.store.events())?);
        }
    }

    Ok(())
}
