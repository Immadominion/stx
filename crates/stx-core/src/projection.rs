//! Read-model projections over the event log.
//!
//! - [`spans_for_trace`] turns one trace's events into the span waterfall (each
//!   span's duration is the latency delta between two lifecycle milestones).
//! - [`funnel`] counts how far traces progressed (a cumulative stage funnel:
//!   reaching `finalized` implies reaching every earlier stage).

use crate::commitment::Commitment;
use crate::event::Event;
use crate::lifecycle::LifecycleEvent;
use crate::span::{Span, SpanName, SpanStatus};
use crate::store::EventStore;
use chrono::{DateTime, Utc};

/// Build the span waterfall for a single trace from its (seq-ordered) events.
pub fn spans_for_trace(events: &[&Event]) -> Vec<Span> {
    let mut drafted: Option<DateTime<Utc>> = None;
    let mut tip: Option<DateTime<Utc>> = None;
    let mut built: Option<DateTime<Utc>> = None;
    let mut dispatched: Option<DateTime<Utc>> = None;
    let mut landed: Option<DateTime<Utc>> = None;
    let mut processed: Option<DateTime<Utc>> = None;
    let mut confirmed: Option<DateTime<Utc>> = None;
    let mut finalized: Option<DateTime<Utc>> = None;

    for e in events {
        match &e.event {
            LifecycleEvent::Drafted { .. } => {
                drafted.get_or_insert(e.at);
            }
            LifecycleEvent::TipDecided { .. } => {
                tip.get_or_insert(e.at);
            }
            LifecycleEvent::Built { .. } => {
                built.get_or_insert(e.at);
            }
            LifecycleEvent::Dispatched { .. } => {
                dispatched.get_or_insert(e.at);
            }
            LifecycleEvent::Landed { .. } => {
                landed.get_or_insert(e.at);
            }
            LifecycleEvent::CommitmentReached { commitment, .. } => match commitment {
                Commitment::Processed => {
                    processed.get_or_insert(e.at);
                }
                Commitment::Confirmed => {
                    confirmed.get_or_insert(e.at);
                }
                Commitment::Finalized => {
                    finalized.get_or_insert(e.at);
                }
            },
            _ => {}
        }
    }

    let plan = [
        (SpanName::TipDecide, drafted, tip),
        (SpanName::BundleBuild, tip, built),
        (SpanName::Dispatch, built, dispatched),
        (SpanName::AuctionWait, dispatched, landed),
        (SpanName::Processed, landed, processed),
        (SpanName::Confirmed, processed, confirmed),
        (SpanName::Finalized, confirmed, finalized),
    ];

    let mut spans = Vec::new();
    for (name, start, end) in plan {
        if let (Some(s), Some(en)) = (start, end) {
            let mut span = Span::open(name, s);
            span.close(en, SpanStatus::Ok);
            spans.push(span);
        }
    }
    spans
}

/// A cumulative funnel of how far traces progressed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StageFunnel {
    pub submitted: u64,
    pub landed: u64,
    pub processed: u64,
    pub confirmed: u64,
    pub finalized: u64,
    pub failed: u64,
    pub aborted: u64,
}

/// Compute the stage funnel across all traces in the store.
pub fn funnel(store: &EventStore) -> StageFunnel {
    let mut f = StageFunnel::default();
    for trace in store.traces() {
        let events = store.events_for_trace(&trace);
        let (mut submitted, mut landed, mut processed, mut confirmed, mut finalized) =
            (false, false, false, false, false);
        let (mut failed, mut aborted) = (false, false);

        for e in &events {
            match &e.event {
                LifecycleEvent::Drafted { .. }
                | LifecycleEvent::TipDecided { .. }
                | LifecycleEvent::Built { .. }
                | LifecycleEvent::Dispatched { .. }
                | LifecycleEvent::MarkedInflight => submitted = true,
                LifecycleEvent::Landed { .. } => landed = true,
                LifecycleEvent::CommitmentReached { commitment, .. } => match commitment {
                    Commitment::Processed => processed = true,
                    Commitment::Confirmed => confirmed = true,
                    Commitment::Finalized => finalized = true,
                },
                LifecycleEvent::Failed { .. } => failed = true,
                LifecycleEvent::Aborted { .. } => aborted = true,
                LifecycleEvent::RetryScheduled { .. } => {}
            }
        }

        // Cumulative roll-down: a higher stage implies all lower ones.
        if finalized {
            confirmed = true;
        }
        if confirmed {
            processed = true;
        }
        if processed {
            landed = true;
        }
        if landed {
            submitted = true;
        }

        f.submitted += submitted as u64;
        f.landed += landed as u64;
        f.processed += processed as u64;
        f.confirmed += confirmed as u64;
        f.finalized += finalized as u64;
        f.failed += failed as u64;
        f.aborted += aborted as u64;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{LogicalTxId, Slot, TraceId};
    use chrono::Duration;

    fn ev(seq: u64, at: DateTime<Utc>, event: LifecycleEvent) -> Event {
        Event {
            seq,
            at,
            trace_id: TraceId::from("trc_1"),
            logical_tx_id: LogicalTxId::from("ltx_1"),
            slot: None,
            event,
        }
    }

    #[test]
    fn spans_have_milestone_durations() {
        let t0 = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let events = vec![
            ev(
                0,
                t0,
                LifecycleEvent::Drafted {
                    logical_tx_id: LogicalTxId::from("ltx_1"),
                },
            ),
            ev(
                1,
                t0 + Duration::milliseconds(5),
                LifecycleEvent::TipDecided {
                    tip_lamports: crate::ids::Lamports(50_000),
                    source: crate::tip::TipSource::Agent,
                },
            ),
            ev(
                2,
                t0 + Duration::milliseconds(8),
                LifecycleEvent::Built { signatures: vec![] },
            ),
            ev(
                3,
                t0 + Duration::milliseconds(10),
                LifecycleEvent::Dispatched {
                    bundle_id: crate::ids::BundleId::from("b1"),
                    regions: vec![],
                },
            ),
            ev(
                4,
                t0 + Duration::milliseconds(410),
                LifecycleEvent::Landed { slot: Slot(10) },
            ),
            ev(
                5,
                t0 + Duration::milliseconds(420),
                LifecycleEvent::CommitmentReached {
                    commitment: Commitment::Processed,
                    slot: Slot(10),
                },
            ),
            ev(
                6,
                t0 + Duration::milliseconds(1020),
                LifecycleEvent::CommitmentReached {
                    commitment: Commitment::Confirmed,
                    slot: Slot(10),
                },
            ),
            ev(
                7,
                t0 + Duration::milliseconds(13020),
                LifecycleEvent::CommitmentReached {
                    commitment: Commitment::Finalized,
                    slot: Slot(10),
                },
            ),
        ];
        let refs: Vec<&Event> = events.iter().collect();
        let spans = spans_for_trace(&refs);
        assert_eq!(spans.len(), 7);
        // processed -> confirmed delta (the network-health probe) is 600ms here.
        let confirmed_span = spans.iter().find(|s| s.name == SpanName::Confirmed).unwrap();
        assert_eq!(confirmed_span.duration_ms(), Some(600));
        // confirmed -> finalized ~12s (rooting).
        let finalized_span = spans.iter().find(|s| s.name == SpanName::Finalized).unwrap();
        assert_eq!(finalized_span.duration_ms(), Some(12000));
    }

    #[test]
    fn funnel_is_cumulative() {
        let mut store = EventStore::new();
        let trace = TraceId::from("trc_1");
        let ltx = LogicalTxId::from("ltx_1");
        store.append(
            trace.clone(),
            ltx.clone(),
            None,
            LifecycleEvent::Drafted {
                logical_tx_id: ltx.clone(),
            },
        );
        store.append(
            trace.clone(),
            ltx.clone(),
            Some(Slot(10)),
            LifecycleEvent::Landed { slot: Slot(10) },
        );
        store.append(
            trace.clone(),
            ltx.clone(),
            Some(Slot(10)),
            LifecycleEvent::CommitmentReached {
                commitment: Commitment::Finalized,
                slot: Slot(10),
            },
        );
        let f = funnel(&store);
        assert_eq!(f.finalized, 1);
        assert_eq!(f.confirmed, 1); // cumulative
        assert_eq!(f.processed, 1);
        assert_eq!(f.landed, 1);
        assert_eq!(f.submitted, 1);
        assert_eq!(f.failed, 0);
    }
}
