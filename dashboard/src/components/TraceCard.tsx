import type { Trace } from "@/lib/trace";
import { fmtDuration, fmtLamports, shortId, solscanTx } from "@/lib/trace";
import { LifecycleTimeline } from "./LifecycleTimeline";
import { SpanWaterfall } from "./SpanWaterfall";
import { TipLadder } from "./TipLadder";
import { ExtLink, StatusBadge } from "./ui";

export function TraceCard({ trace }: { trace: Trace }) {
  return (
    <section className="rounded-card border border-border bg-canvas">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-5 py-4">
        <div className="flex items-center gap-3">
          <StatusBadge status={trace.status} />
          {trace.finalSignature && (
            <ExtLink
              href={solscanTx(trace.finalSignature)}
              className="font-mono text-sm"
            >
              {shortId(trace.finalSignature, 6)}
            </ExtLink>
          )}
        </div>
        <div className="flex items-center gap-4 font-mono text-xs text-ink-2">
          {trace.headlineTip !== undefined && (
            <span>tip {fmtLamports(trace.headlineTip)}</span>
          )}
          <span>
            {trace.attemptCount}{" "}
            {trace.attemptCount === 1 ? "attempt" : "attempts"}
          </span>
          <span>{fmtDuration(trace.totalMs)}</span>
          <span className="text-ink-3">
            {trace.usedAgent ? "AI-steered" : "policy"}
          </span>
        </div>
      </header>

      <div className="grid gap-8 px-5 py-5 md:grid-cols-2">
        <div>
          <h4 className="mb-3 text-xs font-semibold text-ink-2">Lifecycle</h4>
          <LifecycleTimeline milestones={trace.milestones} />
        </div>
        <div>
          <h4 className="mb-3 text-xs font-semibold text-ink-2">Spans</h4>
          <SpanWaterfall spans={trace.spans} totalMs={trace.totalMs} />
          {trace.spans.length === 0 && (
            <p className="text-sm text-ink-3">No span data.</p>
          )}
        </div>
      </div>

      {trace.attemptCount > 1 && (
        <div className="border-t border-border px-5 py-5">
          <h4 className="mb-3 text-xs font-semibold text-ink-2">
            Tip escalation across attempts
          </h4>
          <TipLadder attempts={trace.attempts} />
        </div>
      )}
    </section>
  );
}
