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

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use stx_agent::{
    build_record, validate, AgentConfig, AnthropicClient, FaultScenario, GuardrailPolicy, MockTools,
    ReasoningAgent, ValidationContext,
};
use stx_core::{AgentAction, Decision, DecisionParams, Lamports, LogicalTxId, TraceId};
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
    }

    Ok(())
}
