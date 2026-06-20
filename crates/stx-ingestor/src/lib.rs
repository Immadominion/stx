//! `stx-ingestor` - Yellowstone gRPC (Dragon's Mouth) streaming.
//!
//! - [`request`] - `SubscribeRequest` builders for slot and signature streams.
//! - [`observation`] - the normalized [`observation::Observation`] vocabulary and
//!   the reduction from raw proto updates.
//! - [`client`] - connect + subscribe + forward observations, with native-roots
//!   TLS, a raised decode limit, auto-reconnect (Yellowstone 13.1) and ping
//!   keep-alive.

pub mod client;
pub mod error;
pub mod observation;
pub mod request;

pub use client::{connect, run, IngestorConfig, DEFAULT_MAX_DECODING_SIZE};
pub use error::IngestError;
pub use observation::{normalize, Observation, SlotStatusKind};
pub use request::{
    account_tx_request, commitment_to_proto, signature_request, slots_request,
};
