// Real artifacts from mainnet runs, bundled so the dashboard renders genuine
// on-chain data with no backend. Regenerate by copying fresh files from
// ../../docs/evidence after a run.

import agentEvents from "./lifecycle-agent-run.json";
import fallbackEvents from "./lifecycle-fallback-run.json";
import agentDecisions from "./agent-live-decisions.json";
import faultBlockhash from "./agent-blockhash-expiry.json";
import faultFee from "./agent-fee-starvation.json";
import faultCompute from "./agent-compute-exhaustion.json";

import { buildTrace, funnel, type Trace } from "../lib/trace";
import type { DecisionRecord, Event } from "../lib/types";

export const traces: Trace[] = [
  buildTrace(
    agentEvents as unknown as Event[],
    agentDecisions as unknown as DecisionRecord[],
  ),
  buildTrace(fallbackEvents as unknown as Event[], []),
];

export const overallFunnel = funnel(traces);

// Controlled fault-injection demos (the agent against three different failures).
export const faultInjections: DecisionRecord[] = [
  faultBlockhash as unknown as DecisionRecord,
  faultFee as unknown as DecisionRecord,
  faultCompute as unknown as DecisionRecord,
];
