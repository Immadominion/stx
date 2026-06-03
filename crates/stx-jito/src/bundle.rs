//! Bundle builder.
//!
//! Builds a signed, wire-ready Jito bundle. Per Jito guidance the tip transfer
//! sits in the **same transaction** as the main logic (here the compute-budget
//! setup + any payload instructions). Output transactions are base64-encoded,
//! which is the encoding `sendBundle` should be called with.
//!
//! ## A note on the compute-budget instructions
//!
//! `solana-compute-budget-interface` (latest 2.2.x) pins `solana-instruction` at
//! `^2`, which is incompatible with the `3.4.0` used by the rest of the
//! solana-sdk 4 stack - pulling it in would create two distinct `Instruction`
//! types. So we construct those two instructions by hand, using the **exact**
//! borsh discriminants and little-endian encoding copied from that crate's
//! source (`SetComputeUnitLimit` = 2, `SetComputeUnitPrice` = 3), against the
//! unified `Instruction` type. The system-program tip transfer, by contrast, is
//! built with `solana-system-interface` (which already uses `solana-instruction`
//! 3.4.0, so it unifies cleanly).

use crate::error::JitoError;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use std::str::FromStr;
use stx_core::Signature as CoreSignature;

/// The well-known Compute Budget program id.
pub const COMPUTE_BUDGET_PROGRAM_ID: &str = "ComputeBudget111111111111111111111111111111";

fn compute_budget_program() -> Pubkey {
    Pubkey::from_str(COMPUTE_BUDGET_PROGRAM_ID).expect("valid compute budget program id")
}

/// `ComputeBudgetInstruction::SetComputeUnitLimit(units)` - discriminant `2`,
/// followed by the `u32` value little-endian. Encoding matches
/// `solana-compute-budget-interface` verbatim.
pub fn set_compute_unit_limit_ix(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2u8);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction::new_with_bytes(compute_budget_program(), &data, vec![])
}

/// `ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports)` -
/// discriminant `3`, followed by the `u64` value little-endian.
pub fn set_compute_unit_price_ix(micro_lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3u8);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction::new_with_bytes(compute_budget_program(), &data, vec![])
}

/// A tip transfer to a Jito tip account (verified system-program transfer).
pub fn tip_ix(from: &Pubkey, tip_account: &Pubkey, lamports: u64) -> Instruction {
    system_instruction::transfer(from, tip_account, lamports)
}

/// Inputs to build one bundle transaction.
pub struct BundleParams<'a> {
    pub payer: &'a Keypair,
    pub recent_blockhash: Hash,
    pub tip_account: Pubkey,
    pub tip_lamports: u64,
    pub cu_limit: u32,
    pub cu_price_micro_lamports: u64,
    /// Payload instructions executed before the tip (the "main logic"). May be
    /// empty, in which case the tip transfer itself is the on-chain action.
    pub extra_instructions: Vec<Instruction>,
}

/// A built, signed, wire-ready bundle.
#[derive(Debug, Clone)]
pub struct BuiltBundle {
    /// base64-encoded transactions, in execution order (what `sendBundle` takes).
    pub transactions: Vec<String>,
    /// Transaction signatures (base58), for lifecycle tracking via the stream.
    pub signatures: Vec<CoreSignature>,
}

/// Build a single-transaction Jito bundle: `[setCuLimit, setCuPrice, ...payload,
/// tipTransfer]`, signed by `payer`.
pub fn build_bundle(params: BundleParams<'_>) -> Result<BuiltBundle, JitoError> {
    let payer_pubkey = params.payer.pubkey();

    let mut ixs = Vec::with_capacity(3 + params.extra_instructions.len());
    ixs.push(set_compute_unit_limit_ix(params.cu_limit));
    ixs.push(set_compute_unit_price_ix(params.cu_price_micro_lamports));
    ixs.extend(params.extra_instructions);
    ixs.push(tip_ix(&payer_pubkey, &params.tip_account, params.tip_lamports));

    let tx = Transaction::new_signed_with_payer(
        &ixs,
        Some(&payer_pubkey),
        &[params.payer],
        params.recent_blockhash,
    );

    let wire = bincode::serialize(&tx)
        .map_err(|e| JitoError::Invalid(format!("serialize transaction: {e}")))?;
    let b64 = STANDARD.encode(wire);

    let sig = tx
        .signatures
        .first()
        .map(|s| CoreSignature::new(s.to_string()))
        .ok_or_else(|| JitoError::Invalid("transaction was not signed".into()))?;

    Ok(BuiltBundle {
        transactions: vec![b64],
        signatures: vec![sig],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // First Jito tip account (from getTipAccounts; see docs/research/01).
    const TIP_ACCOUNT: &str = "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5";

    #[test]
    fn compute_budget_encoding_matches_spec() {
        let ix = set_compute_unit_limit_ix(300_000);
        assert_eq!(ix.program_id, compute_budget_program());
        assert!(ix.accounts.is_empty());
        assert_eq!(ix.data.len(), 5);
        assert_eq!(ix.data[0], 2);
        assert_eq!(&ix.data[1..], &300_000u32.to_le_bytes());

        let ix = set_compute_unit_price_ix(50_000);
        assert_eq!(ix.data.len(), 9);
        assert_eq!(ix.data[0], 3);
        assert_eq!(&ix.data[1..], &50_000u64.to_le_bytes());
    }

    #[test]
    fn builds_signed_base64_bundle() {
        let payer = Keypair::new();
        let tip_account = Pubkey::from_str(TIP_ACCOUNT).unwrap();
        let built = build_bundle(BundleParams {
            payer: &payer,
            recent_blockhash: Hash::default(),
            tip_account,
            tip_lamports: 50_000,
            cu_limit: 300_000,
            cu_price_micro_lamports: 1_000,
            extra_instructions: vec![],
        })
        .unwrap();

        assert_eq!(built.transactions.len(), 1);
        assert_eq!(built.signatures.len(), 1);
        // base64 round-trips to non-empty wire bytes.
        let raw = STANDARD.decode(&built.transactions[0]).unwrap();
        assert!(!raw.is_empty());
        // a base58 ed25519 signature is ~88 chars.
        assert!(built.signatures[0].as_str().len() >= 80);
    }
}
