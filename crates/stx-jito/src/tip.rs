//! Dynamic tip engine.
//!
//! Tips are **never hardcoded**. We fetch the live Jito tip-floor distribution
//! (`bundles.jito.wtf/api/v1/bundles/tip_floor`), which returns landed-tip
//! percentiles in **SOL**, and convert to lamports (rounding up). The static
//! fallback policy here is what runs when the AI brain is disabled; the agent
//! consumes the same [`TipFloor`] as one input to its bounded decision.

use crate::error::JitoError;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use stx_core::{Lamports, TipFloor, TipPercentile};

/// Jito tip-floor REST endpoint (returns a single-element JSON array).
pub const DEFAULT_TIP_FLOOR_URL: &str = "https://bundles.jito.wtf/api/v1/bundles/tip_floor";

/// Jito's hard minimum tip.
pub const MIN_TIP_LAMPORTS: Lamports = Lamports(1000);

/// The raw tip-floor item exactly as Jito returns it (values in SOL).
#[derive(Debug, Clone, Deserialize)]
struct TipFloorRaw {
    time: DateTime<Utc>,
    landed_tips_25th_percentile: f64,
    landed_tips_50th_percentile: f64,
    landed_tips_75th_percentile: f64,
    landed_tips_95th_percentile: f64,
    landed_tips_99th_percentile: f64,
    ema_landed_tips_50th_percentile: f64,
}

impl From<TipFloorRaw> for TipFloor {
    fn from(r: TipFloorRaw) -> Self {
        TipFloor {
            at: r.time,
            p25: Lamports::from_sol(r.landed_tips_25th_percentile),
            p50: Lamports::from_sol(r.landed_tips_50th_percentile),
            p75: Lamports::from_sol(r.landed_tips_75th_percentile),
            p95: Lamports::from_sol(r.landed_tips_95th_percentile),
            p99: Lamports::from_sol(r.landed_tips_99th_percentile),
            ema_p50: Lamports::from_sol(r.ema_landed_tips_50th_percentile),
        }
    }
}

/// Parse the tip-floor REST body (a one-element array) into a [`TipFloor`].
pub fn parse_tip_floor(body: &str) -> Result<TipFloor, JitoError> {
    let arr: Vec<TipFloorRaw> = serde_json::from_str(body)?;
    let first = arr.into_iter().next().ok_or(JitoError::EmptyTipFloor)?;
    Ok(first.into())
}

/// Fetches the live Jito tip floor over HTTP.
#[derive(Clone)]
pub struct TipFloorClient {
    http: reqwest::Client,
    url: String,
}

impl TipFloorClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            url: DEFAULT_TIP_FLOOR_URL.to_string(),
        }
    }

    pub fn with_url(http: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            http,
            url: url.into(),
        }
    }

    pub async fn fetch(&self) -> Result<TipFloor, JitoError> {
        let body = self
            .http
            .get(&self.url)
            .timeout(crate::net::REQUEST_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_tip_floor(&body)
    }
}

/// A coarse read of current network pressure, used by the static fallback to
/// pick a percentile. The AI brain reasons over richer inputs than this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Congestion {
    Low,
    Normal,
    High,
}

/// Infer network pressure from the tip-floor shape alone: a current p50 that has
/// spiked to twice its EMA means the floor is climbing fast (contention) and
/// warrants p95. Otherwise it is `Normal`, where the EMA-smoothed baseline (the
/// max of p50 and its EMA) sizes the tip - that already protects against a
/// momentary p50 dip, so there is deliberately no auto-`Low` branch that would
/// under-tip when the EMA sits above a dipped p50. Conservative and needs no RPC.
pub fn detect_congestion(floor: &TipFloor) -> Congestion {
    let p50 = floor.p50.0;
    let ema = floor.ema_p50.0.max(1);
    if p50 >= ema.saturating_mul(2) {
        Congestion::High
    } else {
        Congestion::Normal
    }
}

/// The deterministic fallback tip policy (runs when the AI is disabled).
/// Picks a percentile by congestion and enforces the 1000-lamport floor.
/// This is intentionally simple and *documented as the baseline* - it is never
/// presented as "intelligence"; the agent is measured against it.
pub fn recommend_tip(floor: &TipFloor, congestion: Congestion) -> Lamports {
    let tip = match congestion {
        Congestion::Low => floor.percentile(TipPercentile::P25),
        // Smoothed baseline: the higher of the current p50 and its EMA, so a
        // momentary dip in the noisy raw p50 does not cause an under-tip.
        Congestion::Normal => Lamports(floor.p50.0.max(floor.ema_p50.0)),
        Congestion::High => floor.percentile(TipPercentile::P95),
    };
    tip.clamp_to(MIN_TIP_LAMPORTS, Lamports(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact shape observed live from the Jito API during research.
    const LIVE_BODY: &str = r#"[{"time":"2026-06-02T17:08:35+00:00","landed_tips_25th_percentile":0.0000123,"landed_tips_50th_percentile":0.00003,"landed_tips_75th_percentile":0.0000918625,"landed_tips_95th_percentile":0.0005490940000000001,"landed_tips_99th_percentile":0.0040696285,"ema_landed_tips_50th_percentile":0.000022696327264285852}]"#;

    #[test]
    fn parses_live_tip_floor_shape() {
        let floor = parse_tip_floor(LIVE_BODY).unwrap();
        // SOL -> lamports, exact for these values.
        assert_eq!(floor.p25, Lamports(12_300));
        assert_eq!(floor.p50, Lamports(30_000));
        // p95/p99 round up; just assert ordering holds.
        assert!(floor.p75 > floor.p50);
        assert!(floor.p95 > floor.p75);
        assert!(floor.p99 > floor.p95);
    }

    #[test]
    fn empty_array_is_error() {
        assert!(matches!(parse_tip_floor("[]"), Err(JitoError::EmptyTipFloor)));
    }

    #[test]
    fn fallback_policy_picks_percentile() {
        let floor = parse_tip_floor(LIVE_BODY).unwrap();
        // p50 (30000) >= ema_p50 (22697), so the Normal baseline is p50.
        assert_eq!(recommend_tip(&floor, Congestion::Normal), floor.p50);
        assert_eq!(recommend_tip(&floor, Congestion::High), floor.p95);
        assert_eq!(recommend_tip(&floor, Congestion::Low), floor.p25);
    }

    fn floor_with(p50: u64, ema: u64) -> TipFloor {
        TipFloor {
            at: Utc::now(),
            p25: Lamports(1_000),
            p50: Lamports(p50),
            p75: Lamports(20_000),
            p95: Lamports(80_000),
            p99: Lamports(200_000),
            ema_p50: Lamports(ema),
        }
    }

    #[test]
    fn normal_baseline_holds_at_ema_when_p50_dips() {
        // A momentary p50 dip below the EMA: the baseline holds at the EMA
        // rather than chasing the noisy low value down into an under-tip.
        let floor = floor_with(5_000, 12_000);
        assert_eq!(recommend_tip(&floor, Congestion::Normal), Lamports(12_000));
    }

    #[test]
    fn congestion_detection_from_floor_shape() {
        // A p50 at 2x the EMA is a clear spike -> High.
        assert_eq!(detect_congestion(&floor_with(40_000, 20_000)), Congestion::High);
        // A p50 below its EMA is NOT treated as Low (that would under-tip); the
        // EMA-smoothed Normal baseline handles it.
        assert_eq!(detect_congestion(&floor_with(10_000, 20_000)), Congestion::Normal);
        assert_eq!(detect_congestion(&floor_with(22_000, 20_000)), Congestion::Normal);
    }

    #[test]
    fn fallback_enforces_min_tip() {
        let floor = TipFloor {
            at: Utc::now(),
            p25: Lamports(100),
            p50: Lamports(200),
            p75: Lamports(300),
            p95: Lamports(400),
            p99: Lamports(500),
            ema_p50: Lamports(150),
        };
        assert_eq!(recommend_tip(&floor, Congestion::Low), MIN_TIP_LAMPORTS);
    }
}
