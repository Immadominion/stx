// Mirrors the stx-core domain model as serialized to JSON.

export type Commitment = "processed" | "confirmed" | "finalized";

export type FailureKind =
  | "ExpiredBlockhash"
  | "FeeTooLow"
  | "ComputeExceeded"
  | "BundleFailed"
  | "Dropped"
  | "SimulationFailure"
  | "AdverseMarket"
  | "AlreadyProcessed"
  | "Unknown";

export interface FailureClass {
  kind: FailureKind;
  evidence: string;
  confidence: number;
  raw_error?: string | null;
  at_slot?: number | null;
}

// LifecycleEvent is serde-tagged: { type, ...fields }.
export type LifecycleEvent =
  | { type: "drafted"; logical_tx_id: string }
  | { type: "tip_decided"; tip_lamports: number; source: "static_policy" | "agent" }
  | { type: "built"; signatures: string[] }
  | { type: "dispatched"; bundle_id: string; regions: string[] }
  | { type: "marked_inflight" }
  | { type: "landed"; slot: number }
  | { type: "commitment_reached"; commitment: Commitment; slot: number }
  | { type: "failed"; class: FailureClass }
  | { type: "retry_scheduled"; child_trace: string; attempt: number }
  | { type: "aborted"; reason: string };

export interface Event {
  seq: number;
  at: string;
  trace_id: string;
  logical_tx_id: string;
  slot: number | null;
  event: LifecycleEvent;
}

export type AgentAction =
  | "resubmit"
  | "resubmit_new_blockhash"
  | "raise_tip"
  | "raise_cu"
  | "widen_slippage"
  | "abort"
  | "hold";

export interface DecisionParams {
  tip_lamports: number;
  cu_limit: number | null;
  refresh_blockhash: boolean;
}

export interface Decision {
  action: AgentAction;
  params: DecisionParams;
  hypotheses: string[];
  chosen_cause: string;
  justification: string;
  confidence: number;
  expected_effect: string;
}

export interface ToolCall {
  tool: string;
  input: unknown;
  result: unknown;
}

export interface GuardrailReport {
  tip_clamped: boolean;
  action_allowed: boolean;
  overrides: string[];
  fell_back_to_default: boolean;
}

export interface DecisionRecord {
  decision_id: string;
  trace_id: string;
  logical_tx_id: string;
  attempt: number;
  at: string;
  trigger: string;
  observations: Record<string, unknown>;
  thinking_summary?: string | null;
  tool_calls: ToolCall[];
  decision: Decision;
  guardrail: GuardrailReport;
  outcome?: unknown;
  model: string;
  request_id?: string | null;
}
