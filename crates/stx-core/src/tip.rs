//! Jito tip-floor percentiles.
//!
//! The Jito tip-floor API returns landed-tip percentiles in **SOL**; we store
//! them as [`Lamports`] (rounded up) because `sendBundle` takes lamports. Tips
//! are never hardcoded - this struct is refreshed from live data.

use crate::ids::Lamports;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TipPercentile {
    P25,
    P50,
    P75,
    P95,
    P99,
}

/// A snapshot of the Jito landed-tip distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipFloor {
    pub at: DateTime<Utc>,
    pub p25: Lamports,
    pub p50: Lamports,
    pub p75: Lamports,
    pub p95: Lamports,
    pub p99: Lamports,
    /// Smoothed 50th-percentile baseline.
    pub ema_p50: Lamports,
}

impl TipFloor {
    pub fn percentile(&self, p: TipPercentile) -> Lamports {
        match p {
            TipPercentile::P25 => self.p25,
            TipPercentile::P50 => self.p50,
            TipPercentile::P75 => self.p75,
            TipPercentile::P95 => self.p95,
            TipPercentile::P99 => self.p99,
        }
    }
}

/// Where a chosen tip came from - for the lifecycle log and the AI-vs-fallback
/// comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipSource {
    /// The deterministic fallback policy.
    StaticPolicy,
    /// The AI agent's bounded decision.
    Agent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_lookup() {
        let now = Utc::now();
        let floor = TipFloor {
            at: now,
            p25: Lamports(12_300),
            p50: Lamports(30_000),
            p75: Lamports(91_862),
            p95: Lamports(549_094),
            p99: Lamports(4_069_628),
            ema_p50: Lamports(22_696),
        };
        assert_eq!(floor.percentile(TipPercentile::P50), Lamports(30_000));
        assert_eq!(floor.percentile(TipPercentile::P99), Lamports(4_069_628));
    }
}
