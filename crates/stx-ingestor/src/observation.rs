//! Normalized observations.
//!
//! Raw `SubscribeUpdate`s are reduced to a small [`Observation`] vocabulary that
//! the lifecycle engine understands, decoupling the rest of the system from the
//! proto types.

use stx_core::Commitment;
use yellowstone_grpc_proto::geyser::{subscribe_update::UpdateOneof, SlotStatus, SubscribeUpdate};

/// The slot-status values Yellowstone emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotStatusKind {
    Processed,
    Confirmed,
    Finalized,
    FirstShredReceived,
    Completed,
    CreatedBank,
    Dead,
    Unknown,
}

impl SlotStatusKind {
    pub fn from_i32(v: i32) -> Self {
        match SlotStatus::try_from(v) {
            Ok(SlotStatus::SlotProcessed) => Self::Processed,
            Ok(SlotStatus::SlotConfirmed) => Self::Confirmed,
            Ok(SlotStatus::SlotFinalized) => Self::Finalized,
            Ok(SlotStatus::SlotFirstShredReceived) => Self::FirstShredReceived,
            Ok(SlotStatus::SlotCompleted) => Self::Completed,
            Ok(SlotStatus::SlotCreatedBank) => Self::CreatedBank,
            Ok(SlotStatus::SlotDead) => Self::Dead,
            Err(_) => Self::Unknown,
        }
    }

    /// The commitment level this slot status corresponds to, if any.
    pub fn to_commitment(self) -> Option<Commitment> {
        match self {
            Self::Processed => Some(Commitment::Processed),
            Self::Confirmed => Some(Commitment::Confirmed),
            Self::Finalized => Some(Commitment::Finalized),
            _ => None,
        }
    }
}

/// A normalized event from the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    /// A slot reached a status.
    Slot {
        slot: u64,
        parent: Option<u64>,
        status: SlotStatusKind,
    },
    /// A transaction we're tracking was seen on-chain at this slot.
    Transaction {
        signature: String,
        slot: u64,
        failed: bool,
    },
    /// Server keep-alive ping (we reply with a pong).
    Ping,
    /// Server pong reply.
    Pong { id: i32 },
}

/// Reduce a raw `SubscribeUpdate` to an [`Observation`], or `None` for update
/// kinds we don't track.
pub fn normalize(update: SubscribeUpdate) -> Option<Observation> {
    match update.update_oneof? {
        UpdateOneof::Slot(s) => Some(Observation::Slot {
            slot: s.slot,
            parent: s.parent,
            status: SlotStatusKind::from_i32(s.status),
        }),
        UpdateOneof::Transaction(t) => {
            let info = t.transaction?;
            let signature = bs58::encode(info.signature).into_string();
            let failed = info.meta.map(|m| m.err.is_some()).unwrap_or(false);
            Some(Observation::Transaction {
                signature,
                slot: t.slot,
                failed,
            })
        }
        UpdateOneof::TransactionStatus(ts) => {
            let signature = bs58::encode(ts.signature).into_string();
            Some(Observation::Transaction {
                signature,
                slot: ts.slot,
                failed: ts.err.is_some(),
            })
        }
        UpdateOneof::Ping(_) => Some(Observation::Ping),
        UpdateOneof::Pong(p) => Some(Observation::Pong { id: p.id }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yellowstone_grpc_proto::geyser::{SubscribeUpdatePing, SubscribeUpdateSlot};

    #[test]
    fn normalizes_slot_update() {
        let update = SubscribeUpdate {
            filters: vec![],
            created_at: None,
            update_oneof: Some(UpdateOneof::Slot(SubscribeUpdateSlot {
                slot: 100,
                parent: Some(99),
                status: SlotStatus::SlotConfirmed as i32,
                dead_error: None,
            })),
        };
        assert_eq!(
            normalize(update),
            Some(Observation::Slot {
                slot: 100,
                parent: Some(99),
                status: SlotStatusKind::Confirmed,
            })
        );
    }

    #[test]
    fn slot_status_maps_to_commitment() {
        assert_eq!(
            SlotStatusKind::from_i32(SlotStatus::SlotFinalized as i32),
            SlotStatusKind::Finalized
        );
        assert_eq!(
            SlotStatusKind::Finalized.to_commitment(),
            Some(Commitment::Finalized)
        );
        assert_eq!(SlotStatusKind::Dead.to_commitment(), None);
    }

    #[test]
    fn normalizes_ping() {
        let update = SubscribeUpdate {
            filters: vec![],
            created_at: None,
            update_oneof: Some(UpdateOneof::Ping(SubscribeUpdatePing {})),
        };
        assert_eq!(normalize(update), Some(Observation::Ping));
    }
}
