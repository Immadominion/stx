//! Transaction autopsy. Given any signature, fetch what actually happened
//! on-chain, classify it with the same failure taxonomy the submit loop uses,
//! and have the AI explain it in plain language. Read-only: it holds no keys and
//! submits nothing, so it is safe to run against any arbitrary signature.

use anyhow::Result;
use serde::Serialize;
use stx_agent::AnthropicClient;
use stx_core::FailureClass;
use stx_jito::{classify, FailureSignals, SolanaRpc};

/// The result of a transaction autopsy.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub signature: String,
    /// Found on-chain at all (vs dropped / never landed).
    pub found: bool,
    /// Landed and executed without error.
    pub succeeded: bool,
    pub slot: Option<u64>,
    pub commitment: Option<String>,
    pub leader: Option<String>,
    pub fee_lamports: Option<u64>,
    pub compute_units: Option<u64>,
    pub error: Option<String>,
    /// Failure classification (the same taxonomy the submit loop uses).
    pub classification: Option<FailureClass>,
    /// The last few program log lines, when present.
    pub log_tail: Vec<String>,
    /// Human-readable facts the verdict and explanation are built from.
    pub facts: Vec<String>,
    /// One-line verdict.
    pub headline: String,
    /// The AI's plain-language explanation (`None` if no API key configured).
    pub explanation: Option<String>,
}

const AUTOPSY_SYSTEM: &str = "You are a senior Solana infrastructure engineer acting as a transaction \
diagnostician. Given the facts about one transaction, explain in 2 to 4 plain sentences what happened \
and, if it failed or never landed, exactly what the developer should change to make it land next time \
(tip size, blockhash freshness, compute limit, targeting a Jito leader). Be concrete. Interpret the \
facts, do not just restate them. No preamble and no markdown headers.";

/// Run the autopsy. `anthropic` is optional: without it, the structured facts and
/// classification are still produced, just no plain-language explanation.
pub async fn diagnose(
    rpc: &SolanaRpc,
    anthropic: Option<&AnthropicClient>,
    signature: &str,
) -> Result<Diagnosis> {
    let sigs = [signature.to_string()];

    // 1. Authoritative status (searches transaction history).
    let status = rpc
        .get_signature_statuses(&sigs, true)
        .await
        .ok()
        .and_then(|v| v.into_iter().next().flatten());

    let mut d = Diagnosis {
        signature: signature.to_string(),
        found: false,
        succeeded: false,
        slot: None,
        commitment: None,
        leader: None,
        fee_lamports: None,
        compute_units: None,
        error: None,
        classification: None,
        log_tail: vec![],
        facts: vec![],
        headline: String::new(),
        explanation: None,
    };

    if let Some(st) = &status {
        d.found = true;
        d.slot = Some(st.slot);
        d.commitment = st.commitment().map(|c| format!("{c:?}").to_lowercase());
    }

    // 2. If on-chain, fetch the full transaction for fee / compute / error / logs.
    if let Some(tx) = rpc.get_transaction(signature).await.ok().flatten() {
        d.found = true;
        d.slot = Some(tx.slot);
        if let Some(meta) = &tx.meta {
            d.fee_lamports = Some(meta.fee);
            d.compute_units = meta.compute_units_consumed;
            if let Some(logs) = &meta.log_messages {
                d.log_tail = logs.iter().rev().take(4).rev().cloned().collect();
            }
            if let Some(err) = &meta.err {
                d.error = Some(serde_json::to_string(err).unwrap_or_default());
            }
        }
        d.succeeded = tx.succeeded();
        d.leader = rpc
            .get_slot_leaders(tx.slot, 1)
            .await
            .ok()
            .and_then(|v| v.into_iter().next());
    }

    // 3. Classify, build the facts, set the headline.
    if !d.found {
        d.headline = "Never landed: dropped or expired before inclusion".to_string();
        d.facts.push(
            "Not found on-chain (getSignatureStatuses with history search and getTransaction both empty)."
                .into(),
        );
        d.facts.push(
            "A signature that never appears on-chain was dropped before inclusion: the blockhash expired, the tip lost the auction, or the targeted leader was not running Jito."
                .into(),
        );
    } else if d.succeeded {
        d.headline = format!(
            "Landed and succeeded at slot {}{}",
            d.slot.unwrap_or(0),
            d.commitment
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default(),
        );
        d.facts.push(format!("Landed at slot {}.", d.slot.unwrap_or(0)));
        if let Some(l) = &d.leader {
            d.facts.push(format!("Block leader: {l}."));
        }
        if let Some(f) = d.fee_lamports {
            d.facts.push(format!("Fee paid: {f} lamports."));
        }
        if let Some(cu) = d.compute_units {
            d.facts.push(format!("Compute units consumed: {cu}."));
        }
    } else {
        let signals = FailureSignals {
            tx_error: d.error.clone(),
            cu_consumed: d.compute_units,
            landed: true,
            ..Default::default()
        };
        let class = classify(&signals);
        d.headline = format!("Landed but FAILED: {:?}", class.kind);
        d.facts
            .push(format!("Landed at slot {} but execution failed.", d.slot.unwrap_or(0)));
        if let Some(e) = &d.error {
            d.facts.push(format!("On-chain error: {e}."));
        }
        if let Some(cu) = d.compute_units {
            d.facts.push(format!("Compute units consumed: {cu}."));
        }
        d.facts.push(format!(
            "Classified as {:?} ({}), confidence {:.0}%.",
            class.kind,
            class.evidence,
            class.confidence * 100.0
        ));
        d.classification = Some(class);
    }

    // 4. The AI autopsy.
    if let Some(client) = anthropic {
        let prompt = format!("Signature: {}\nFacts:\n- {}", signature, d.facts.join("\n- "));
        if let Ok(text) = client.complete(AUTOPSY_SYSTEM, &prompt, 400).await {
            let text = text.trim();
            if !text.is_empty() {
                d.explanation = Some(text.to_string());
            }
        }
    }

    Ok(d)
}
