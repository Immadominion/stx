// Projections over the raw lifecycle event log: the same derivations the Rust
// core computes (spans, milestones, funnel), in TypeScript for the UI.

import type { DecisionRecord, Event, FailureKind } from "./types";

export type TraceStatus =
  | "confirmed"
  | "finalized"
  | "failed"
  | "aborted"
  | "pending";

export interface Span {
  name: string;
  label: string;
  offsetMs: number;
  durationMs: number;
}

export interface AttemptInfo {
  attempt: number;
  tip: number;
  source: string;
  signature?: string;
  outcome: "landed" | "failed" | "aborted" | "pending";
  slot?: number;
  failKind?: FailureKind;
}

export interface Milestone {
  key: string;
  label: string;
  at: number;
  slot?: number;
  deltaMs?: number;
}

export interface Trace {
  traceId: string;
  logicalTxId: string;
  status: TraceStatus;
  attempts: AttemptInfo[];
  attemptCount: number;
  landedSlot?: number;
  finalSignature?: string;
  headlineTip?: number;
  /// Jito regions the landing attempt was fanned out to.
  regions?: string[];
  startedAt: number;
  endedAt: number;
  totalMs: number;
  spans: Span[];
  milestones: Milestone[];
  decisions: DecisionRecord[];
  usedAgent: boolean;
}

const ms = (iso: string) => Date.parse(iso);

export function buildTrace(events: Event[], decisions: DecisionRecord[]): Trace {
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  const start = ms(first.at);

  const firstAt: Record<string, number> = {};
  const slotAt: Record<string, number> = {};
  for (const e of sorted) {
    const t = e.event.type;
    if (!(t in firstAt)) firstAt[t] = ms(e.at);
    if (e.event.type === "landed" && slotAt.landed === undefined) {
      slotAt.landed = e.event.slot;
    }
    if (e.event.type === "commitment_reached") {
      const key = `commitment_${e.event.commitment}`;
      if (!(key in firstAt)) firstAt[key] = ms(e.at);
      slotAt[key] = e.event.slot;
    }
  }

  const attempts: AttemptInfo[] = [];
  let cur: AttemptInfo | null = null;
  let regions: string[] | undefined;
  for (const e of sorted) {
    switch (e.event.type) {
      case "tip_decided":
        cur = {
          attempt: attempts.length + 1,
          tip: e.event.tip_lamports,
          source: e.event.source,
          outcome: "pending",
        };
        attempts.push(cur);
        break;
      case "built":
        if (cur) cur.signature = e.event.signatures[0];
        break;
      case "dispatched":
        regions = e.event.regions;
        break;
      case "landed":
        if (cur) {
          cur.outcome = "landed";
          cur.slot = e.event.slot;
        }
        break;
      case "failed":
        if (cur) {
          cur.outcome = "failed";
          cur.failKind = e.event.class.kind;
        }
        break;
      case "aborted":
        if (cur) cur.outcome = "aborted";
        break;
    }
  }

  const landedAttempt = attempts.find((a) => a.outcome === "landed");
  let status: TraceStatus = "pending";
  if (landedAttempt) status = "confirmed";
  else if (sorted.some((e) => e.event.type === "aborted")) status = "aborted";
  else if (sorted.some((e) => e.event.type === "failed")) status = "failed";
  if (firstAt.commitment_finalized) status = "finalized";

  const spanPlan: [string, string, string, string][] = [
    ["tip.decide", "tip decision", "drafted", "tip_decided"],
    ["bundle.build", "bundle build", "tip_decided", "built"],
    ["dispatch", "dispatch", "built", "dispatched"],
    ["auction.wait", "auction wait", "dispatched", "landed"],
    ["confirm", "to confirmed", "landed", "commitment_confirmed"],
    ["finalize", "to finalized", "commitment_confirmed", "commitment_finalized"],
  ];
  const spans: Span[] = [];
  for (const [name, label, from, to] of spanPlan) {
    const a = firstAt[from];
    const b = firstAt[to];
    if (a !== undefined && b !== undefined && b >= a) {
      spans.push({ name, label, offsetMs: a - start, durationMs: b - a });
    }
  }

  const mDefs: [string, string][] = [
    ["drafted", "Submitted"],
    ["dispatched", "Dispatched"],
    ["landed", "Landed"],
    ["commitment_confirmed", "Confirmed"],
    ["commitment_finalized", "Finalized"],
  ];
  const milestones: Milestone[] = [];
  let prev: number | undefined;
  for (const [key, label] of mDefs) {
    const at = firstAt[key];
    if (at === undefined) continue;
    milestones.push({
      key,
      label,
      at,
      slot: slotAt[key],
      deltaMs: prev === undefined ? undefined : at - prev,
    });
    prev = at;
  }

  return {
    traceId: first.trace_id,
    logicalTxId: first.logical_tx_id,
    status,
    attempts,
    attemptCount: attempts.length,
    landedSlot: landedAttempt?.slot,
    finalSignature:
      landedAttempt?.signature ?? attempts[attempts.length - 1]?.signature,
    headlineTip: landedAttempt?.tip ?? attempts[attempts.length - 1]?.tip,
    regions,
    startedAt: start,
    endedAt: ms(last.at),
    totalMs: ms(last.at) - start,
    spans,
    milestones,
    decisions: [...decisions].sort((a, b) => a.attempt - b.attempt),
    usedAgent:
      decisions.length > 0 || attempts.some((a) => a.source === "agent"),
  };
}

export interface Funnel {
  submitted: number;
  landed: number;
  confirmed: number;
  finalized: number;
  failedAttempts: number;
}

export function funnel(traces: Trace[]): Funnel {
  const f: Funnel = {
    submitted: 0,
    landed: 0,
    confirmed: 0,
    finalized: 0,
    failedAttempts: 0,
  };
  for (const t of traces) {
    f.submitted++;
    if (t.landedSlot !== undefined) f.landed++;
    if (t.status === "confirmed" || t.status === "finalized") f.confirmed++;
    if (t.status === "finalized") f.finalized++;
    f.failedAttempts += t.attempts.filter(
      (a) => a.outcome === "failed" || a.outcome === "aborted",
    ).length;
  }
  return f;
}

// ---- formatting ----

export function fmtLamports(n: number): string {
  return n.toLocaleString("en-US");
}

export function fmtSol(lamports: number): string {
  const sol = lamports / 1e9;
  return sol >= 0.001 ? `${sol.toFixed(4)} SOL` : `${sol.toFixed(6)} SOL`;
}

export function fmtDuration(msv: number): string {
  if (msv < 1000) return `${Math.round(msv)} ms`;
  if (msv < 60000) return `${(msv / 1000).toFixed(1)} s`;
  const m = Math.floor(msv / 60000);
  const s = Math.round((msv % 60000) / 1000);
  return `${m}m ${s}s`;
}

export function shortId(s: string, n = 4): string {
  return s.length <= n * 2 + 1 ? s : `${s.slice(0, n)}…${s.slice(-n)}`;
}

export function fmtSlot(slot: number): string {
  return slot.toLocaleString("en-US");
}

export const solscanTx = (sig: string) => `https://solscan.io/tx/${sig}`;
export const solscanSlot = (slot: number) => `https://solscan.io/block/${slot}`;
