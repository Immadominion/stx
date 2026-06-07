import type { DecisionRecord, ToolCall } from "@/lib/types";

const ACTION: Record<string, { label: string; tone: string }> = {
  resubmit: { label: "Resubmit", tone: "bg-sunk text-ink-2" },
  resubmit_new_blockhash: {
    label: "Refresh blockhash",
    tone: "bg-st-pending/10 text-st-pending-ink",
  },
  raise_tip: { label: "Raise tip", tone: "bg-st-pending/10 text-st-pending-ink" },
  raise_cu: {
    label: "Raise CU limit",
    tone: "bg-st-pending/10 text-st-pending-ink",
  },
  widen_slippage: {
    label: "Widen slippage",
    tone: "bg-st-pending/10 text-st-pending-ink",
  },
  abort: { label: "Abort", tone: "bg-st-failed/10 text-st-failed-ink" },
  hold: { label: "Hold", tone: "bg-sunk text-ink-2" },
};

export function DecisionCard({
  rec,
  heading,
}: {
  rec: DecisionRecord;
  heading?: string;
}) {
  const d = rec.decision;
  const action = ACTION[d.action] ?? { label: d.action, tone: "bg-sunk text-ink-2" };
  const tools = toolSummary(rec.tool_calls);
  const overrides = rec.guardrail.overrides;

  return (
    <article className="rounded-card border border-border bg-canvas p-5">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-ink">
            {heading ?? `Attempt ${rec.attempt}`}
          </span>
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-medium ${action.tone}`}
          >
            {action.label}
          </span>
        </div>
        <Confidence value={d.confidence} />
      </header>

      <p className="mt-3 text-sm font-medium text-ink">{d.chosen_cause}</p>
      <p className="mt-1.5 text-sm leading-relaxed text-ink-2">
        {d.justification}
      </p>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {Object.entries(rec.observations).map(([k, v]) => (
          <span
            key={k}
            className="rounded bg-sunk px-1.5 py-0.5 font-mono text-[11px] text-ink-2"
          >
            {k} <span className="text-ink">{fmtVal(v)}</span>
          </span>
        ))}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-ink-3">
        {tools && (
          <span>
            evidence gathered: <span className="text-ink-2">{tools}</span>
          </span>
        )}
        {overrides.length > 0 && (
          <span className="text-st-pending-ink">
            guardrail: {overrides.join("; ")}
          </span>
        )}
      </div>

      {rec.thinking_summary && (
        <details className="mt-3">
          <summary className="cursor-pointer select-none text-xs font-medium text-brand-ink">
            show reasoning trace
          </summary>
          <p className="mt-2 whitespace-pre-wrap text-xs leading-relaxed text-ink-2">
            {rec.thinking_summary}
          </p>
        </details>
      )}
    </article>
  );
}

function Confidence({ value }: { value: number }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="font-mono text-xs text-ink-2">conf {value.toFixed(2)}</span>
      <span className="h-1.5 w-12 overflow-hidden rounded-full bg-sunk">
        <span
          className="block h-full rounded-full bg-brand"
          style={{ width: `${Math.round(value * 100)}%` }}
        />
      </span>
    </span>
  );
}

function fmtVal(v: unknown): string {
  if (v === null) return "null";
  if (typeof v === "number") return v.toLocaleString("en-US");
  return String(v);
}

function toolSummary(calls: ToolCall[]): string {
  if (!calls || calls.length === 0) return "";
  const counts = new Map<string, number>();
  for (const c of calls) counts.set(c.tool, (counts.get(c.tool) ?? 0) + 1);
  return [...counts.entries()]
    .map(([t, n]) => (n > 1 ? `${t} ×${n}` : t))
    .join(", ");
}
