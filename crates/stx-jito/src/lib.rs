//! `stx-jito` - Jito bundle construction, dynamic tips, and submission.
//!
//! - [`tip`] - the dynamic tip engine: live tip-floor fetch + the deterministic
//!   fallback policy.
//! - [`error`] - the integration-layer error type.
//!
//! Bundle building (signing, compute-budget + tip instructions) and the
//! multi-region submitter / bundle-result tracking are added in subsequent
//! modules; they use `solana-sdk` at the edge and convert to `stx-core`
//! string ids for the lifecycle model.

pub mod bundle;
pub mod classify;
pub mod error;
mod net;
pub mod rpc;
pub mod solrpc;
pub mod tip;

pub use classify::{classify, FailureSignals, BLOCKHASH_VALIDITY_BLOCKS};
pub use bundle::{
    build_bundle, set_compute_unit_limit_ix, set_compute_unit_price_ix, tip_ix, BuiltBundle,
    BundleParams, COMPUTE_BUDGET_PROGRAM_ID,
};
pub use error::JitoError;
pub use rpc::{
    parse_commitment, submit_to_regions, BundleStatusValue, InflightBundleStatus, InflightStatus,
    JitoClient, MAINNET_GLOBAL, MAINNET_REGIONS, TESTNET_GLOBAL,
};
pub use solrpc::{
    blockhash_expired, LatestBlockhash, PrioritizationFee, SignatureStatusValue, SimulationValue,
    SolanaRpc,
};
pub use tip::{
    parse_tip_floor, recommend_tip, Congestion, TipFloorClient, DEFAULT_TIP_FLOOR_URL,
    MIN_TIP_LAMPORTS,
};
