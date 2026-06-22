import type { Trace } from "@/lib/trace";
import { fmtLamports, solscanSlot, solscanTx, shortId } from "@/lib/trace";
import { ExtLink, StatusBadge } from "./ui";

function Lane({
  title,
  subtitle,
  trace,
  highlight,
}: {
  title: string;
  subtitle: string;
  trace: Trace;
  highlight: boolean;
}) {
  const tips = trace.attempts.map((a) => a.tip);
  return (
    <div
      className={`flex-1 rounded-card border bg-canvas p-5 ${
        highlight ? "border-brand/40" : "border-border"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="font-semibold text-ink">{title}</div>
          <div className="mt-0.5 text-xs text-ink-3">{subtitle}</div>
        </div>
        <StatusBadge status={trace.status} />
      </div>

      <div className="mt-4">
        <div className="mb-1.5 text-xs text-ink-3">tip per attempt (lamports)</div>
        <div className="flex flex-wrap items-center gap-1 font-mono text-xs">
          {tips.map((t, i) => (
            <span key={i} className="inline-flex items-center gap-1">
              {i > 0 && <span className="text-ink-3">→</span>}
              <span className="rounded bg-surface px-2 py-1 text-ink-2">
                {fmtLamports(t)}
              </span>
            </span>
          ))}
        </div>
      </div>

      <dl className="mt-4 grid grid-cols-2 gap-y-3 text-sm">
        <div>
          <dt className="text-xs text-ink-3">attempts</dt>
          <dd className="font-mono text-ink">{trace.attemptCount}</dd>
        </div>
        <div>
          <dt className="text-xs text-ink-3">final tip</dt>
          <dd className="font-mono text-ink">
            {trace.headlineTip ? fmtLamports(trace.headlineTip) : "—"}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-ink-3">outcome</dt>
          <dd className="font-mono text-ink">
            {trace.landedSlot ? "landed" : "never landed"}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-ink-3">landed slot</dt>
          <dd className="font-mono">
            {trace.landedSlot ? (
              <ExtLink href={solscanSlot(trace.landedSlot)}>
                {trace.landedSlot.toLocaleString("en-US")}
              </ExtLink>
            ) : (
              "—"
            )}
          </dd>
        </div>
      </dl>

      {trace.finalSignature && trace.landedSlot && (
        <div className="mt-3 border-t border-border pt-3 text-xs">
          <ExtLink href={solscanTx(trace.finalSignature)} className="font-mono">
            {shortId(trace.finalSignature, 8)}
          </ExtLink>
        </div>
      )}
    </div>
  );
}

export function RaceCard({ naive, stx }: { naive: Trace; stx: Trace }) {
  return (
    <section className="rounded-card border border-border bg-surface p-5">
      <h3 className="text-sm font-semibold text-ink">
        The Race: a naive submit loop vs stx
      </h3>
      <p className="mt-1 max-w-2xl text-sm text-ink-2">
        The same transfer, fired against the same live tip-floor snapshot at the
        same moment. The only variable is the submission strategy. Both lanes are
        confirmed the same way (validator stream cross-checked with RPC), so the
        outcome gap is the engineering, not luck.
      </p>

      <div className="mt-5 flex flex-col gap-4 md:flex-row">
        <Lane
          title="Naive baseline"
          subtitle="fixed median tip · single global endpoint · blind retry"
          trace={naive}
          highlight={false}
        />
        <Lane
          title="stx"
          subtitle="EMA-smart tip · 7-region fan-out · diagnose & escalate"
          trace={stx}
          highlight
        />
      </div>

      <p className="mt-4 max-w-2xl text-sm text-ink-2">
        The naive lane held a flat tip and never won the auction. stx sized the
        tip off the smoothed floor, then escalated toward where landings were
        actually happening and landed
        {stx.landedSlot
          ? ` at slot ${stx.landedSlot.toLocaleString("en-US")}`
          : ""}
        .
      </p>
    </section>
  );
}
