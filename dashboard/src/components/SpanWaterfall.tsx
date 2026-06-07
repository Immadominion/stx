import type { Span } from "@/lib/trace";
import { fmtDuration } from "@/lib/trace";

const SPAN_COLOR: Record<string, string> = {
  "tip.decide": "bg-brand",
  "bundle.build": "bg-st-submitted",
  dispatch: "bg-st-submitted",
  "auction.wait": "bg-st-pending",
  confirm: "bg-st-confirmed",
  finalize: "bg-st-finalized",
};

export function SpanWaterfall({
  spans,
  totalMs,
}: {
  spans: Span[];
  totalMs: number;
}) {
  if (spans.length === 0) return null;
  const max = Math.max(
    totalMs,
    ...spans.map((s) => s.offsetMs + s.durationMs),
    1,
  );
  return (
    <div className="space-y-1.5">
      {spans.map((s) => {
        const left = (s.offsetMs / max) * 100;
        const width = Math.max((s.durationMs / max) * 100, 0.6);
        return (
          <div
            key={s.name}
            className="grid grid-cols-[8.5rem_1fr_4.5rem] items-center gap-3"
          >
            <span className="truncate font-mono text-xs text-ink-2">
              {s.name}
            </span>
            <div className="relative h-3 rounded-sm bg-sunk">
              <div
                className={`absolute inset-y-0 rounded-sm ${SPAN_COLOR[s.name] ?? "bg-st-submitted"}`}
                style={{ left: `${left}%`, width: `${width}%` }}
              />
            </div>
            <span className="text-right font-mono text-xs text-ink-2">
              {fmtDuration(s.durationMs)}
            </span>
          </div>
        );
      })}
    </div>
  );
}
