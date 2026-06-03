//! The trace/span projection.
//!
//! Each lifecycle stage becomes a span with a real start/end; the duration of a
//! span *is* the latency delta the bounty asks us to capture. The actual
//! projection from an event log lives in `stx-gateway`; this module defines
//! the span vocabulary and the duration helper.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanName {
    /// AI/policy tip decision.
    TipDecide,
    /// Local build + sign.
    BundleBuild,
    /// Network dispatch to a Block Engine region (region in `detail`).
    Dispatch,
    /// Dispatch → inclusion (auction + leader window).
    AuctionWait,
    /// Inclusion → first `processed` observation.
    LeaderInclusion,
    /// `processed` boundary.
    Processed,
    /// `processed` → `confirmed` (the consensus-latency / network-health probe).
    Confirmed,
    /// `confirmed` → `finalized` (rooting latency).
    Finalized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Ok,
    Error,
    Pending,
}

/// One span in a transaction's trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub name: SpanName,
    /// Free-form qualifier, e.g. the Block Engine region for a `Dispatch` span.
    pub detail: Option<String>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub status: SpanStatus,
}

impl Span {
    pub fn open(name: SpanName, start: DateTime<Utc>) -> Self {
        Self {
            name,
            detail: None,
            start,
            end: None,
            status: SpanStatus::Pending,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn close(&mut self, end: DateTime<Utc>, status: SpanStatus) {
        self.end = Some(end);
        self.status = status;
    }

    /// Span duration in milliseconds, once closed.
    pub fn duration_ms(&self) -> Option<i64> {
        self.end.map(|end| (end - self.start).num_milliseconds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn duration_is_delta() {
        let start = Utc::now();
        let mut span = Span::open(SpanName::Confirmed, start);
        assert_eq!(span.duration_ms(), None);
        span.close(start + Duration::milliseconds(640), SpanStatus::Ok);
        assert_eq!(span.duration_ms(), Some(640));
    }
}
