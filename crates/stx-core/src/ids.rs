//! Lightweight identifier newtypes.
//!
//! Signatures, bundle-ids, blockhashes and pubkeys are kept as base58 strings
//! here: `stx-core` is an observability/state model and never signs or sends.
//! The real `solana_sdk` types live in `stx-jito`, which converts at the edge.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

string_newtype!(
    /// A base58 transaction signature.
    Signature
);
string_newtype!(
    /// A Jito bundle id (SHA-256 of the bundle's transaction signatures).
    BundleId
);
string_newtype!(
    /// A base58 recent blockhash.
    Blockhash
);
string_newtype!(
    /// A base58 account public key.
    Pubkey
);
string_newtype!(
    /// Trace id for one submission attempt (OpenTelemetry-style).
    TraceId
);
string_newtype!(
    /// Logical id grouping all retries of the same submission intent.
    LogicalTxId
);

impl TraceId {
    /// Generate a fresh random trace id.
    pub fn generate() -> Self {
        Self(format!("trc_{}", uuid::Uuid::new_v4().simple()))
    }
}

impl LogicalTxId {
    /// Generate a fresh random logical transaction id.
    pub fn generate() -> Self {
        Self(format!("ltx_{}", uuid::Uuid::new_v4().simple()))
    }
}

/// A Solana slot number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Slot(pub u64);

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An amount denominated in lamports (1 SOL = 1_000_000_000 lamports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Lamports(pub u64);

impl Lamports {
    pub const ZERO: Lamports = Lamports(0);
    pub const PER_SOL: u64 = 1_000_000_000;

    /// Convert a SOL amount (as returned by the Jito tip-floor API) to lamports,
    /// rounding up so we never under-tip below the observed percentile.
    pub fn from_sol(sol: f64) -> Self {
        if sol <= 0.0 {
            return Lamports::ZERO;
        }
        Lamports((sol * Lamports::PER_SOL as f64).ceil() as u64)
    }

    pub fn to_sol(self) -> f64 {
        self.0 as f64 / Lamports::PER_SOL as f64
    }

    /// Clamp into an inclusive range. Used by the guardrail validator.
    pub fn clamp_to(self, min: Lamports, max: Lamports) -> Lamports {
        Lamports(self.0.clamp(min.0, max.0))
    }
}

impl fmt::Display for Lamports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} lamports", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sol_lamports_roundtrip() {
        // 0.00003 SOL is the 50th-percentile tip observed during research.
        assert_eq!(Lamports::from_sol(0.00003), Lamports(30_000));
        assert_eq!(Lamports(30_000).to_sol(), 0.00003);
        // rounds up, never under-tips
        assert_eq!(Lamports::from_sol(0.0000000001), Lamports(1));
        assert_eq!(Lamports::from_sol(0.0), Lamports::ZERO);
    }

    #[test]
    fn clamp_bounds_tip() {
        assert_eq!(
            Lamports(5).clamp_to(Lamports(1000), Lamports(1_000_000)),
            Lamports(1000)
        );
        assert_eq!(
            Lamports(9_999_999).clamp_to(Lamports(1000), Lamports(1_000_000)),
            Lamports(1_000_000)
        );
    }

    #[test]
    fn ids_display_and_convert() {
        let s: Signature = "abc".into();
        assert_eq!(s.as_str(), "abc");
        assert_eq!(s.to_string(), "abc");
        assert!(TraceId::generate().as_str().starts_with("trc_"));
        assert!(LogicalTxId::generate().as_str().starts_with("ltx_"));
    }
}
