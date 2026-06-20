//! `SubscribeRequest` builders.
//!
//! Two subscriptions matter for the lifecycle:
//! - **slots** (with interslot updates) - drives commitment progression and
//!   leader-window timing.
//! - **a specific transaction signature** - confirms landing from validator
//!   ground truth instead of RPC polling.

use std::collections::HashMap;
use stx_core::Commitment;
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions,
};

/// Map a domain [`Commitment`] to the proto enum's i32 discriminant.
pub fn commitment_to_proto(c: Commitment) -> i32 {
    match c {
        Commitment::Processed => CommitmentLevel::Processed as i32,
        Commitment::Confirmed => CommitmentLevel::Confirmed as i32,
        Commitment::Finalized => CommitmentLevel::Finalized as i32,
    }
}

/// Subscribe to slot updates. `interslot_updates` adds the fine-grained
/// pre-confirmation statuses (first-shred / completed / created-bank) used for
/// leader-window timing; we deliberately do not `filter_by_commitment` so we
/// observe the full processed -> confirmed -> finalized progression.
pub fn slots_request(commitment: Commitment) -> SubscribeRequest {
    let mut slots = HashMap::new();
    slots.insert(
        "slots".to_string(),
        SubscribeRequestFilterSlots {
            filter_by_commitment: Some(false),
            interslot_updates: Some(true),
        },
    );
    SubscribeRequest {
        slots,
        commitment: Some(commitment_to_proto(commitment)),
        ..Default::default()
    }
}

/// Subscribe to a single transaction signature to confirm landing via the
/// stream. Vote transactions are excluded.
pub fn signature_request(signature: &str, commitment: Commitment) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    transactions.insert(
        "tx".to_string(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: None,
            signature: Some(signature.to_string()),
            account_include: vec![],
            account_exclude: vec![],
            account_required: vec![],
        },
    );
    SubscribeRequest {
        transactions,
        commitment: Some(commitment_to_proto(commitment)),
        ..Default::default()
    }
}

/// Subscribe to non-vote transactions that touch `account` (e.g. the fee
/// payer). A bare `signature` filter is silently ignored by some geyser
/// providers, but an `account_include` filter is well supported; the caller
/// matches the exact signature in code. Emits at `commitment`.
pub fn account_tx_request(account: &str, commitment: Commitment) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    transactions.insert(
        "tx".to_string(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: None,
            signature: None,
            account_include: vec![account.to_string()],
            account_exclude: vec![],
            account_required: vec![],
        },
    );
    SubscribeRequest {
        transactions,
        commitment: Some(commitment_to_proto(commitment)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_request_shape() {
        let req = slots_request(Commitment::Confirmed);
        assert_eq!(req.commitment, Some(1));
        let f = req.slots.get("slots").unwrap();
        assert_eq!(f.interslot_updates, Some(true));
        assert_eq!(f.filter_by_commitment, Some(false));
        assert!(req.transactions.is_empty());
    }

    #[test]
    fn signature_request_shape() {
        let req = signature_request("SIGabc", Commitment::Processed);
        assert_eq!(req.commitment, Some(0));
        let f = req.transactions.get("tx").unwrap();
        assert_eq!(f.signature.as_deref(), Some("SIGabc"));
        assert_eq!(f.vote, Some(false));
    }

    #[test]
    fn account_tx_request_shape() {
        let req = account_tx_request("PAYERpubkey", Commitment::Processed);
        assert_eq!(req.commitment, Some(0));
        let f = req.transactions.get("tx").unwrap();
        assert_eq!(f.account_include, vec!["PAYERpubkey".to_string()]);
        assert_eq!(f.signature, None);
        assert_eq!(f.vote, Some(false));
    }
}
