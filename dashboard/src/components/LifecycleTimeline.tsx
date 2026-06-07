import type { Milestone } from "@/lib/trace";
import { fmtDuration, fmtSlot, solscanSlot } from "@/lib/trace";

const DOT: Record<string, string> = {
  drafted: "bg-st-submitted",
  dispatched: "bg-st-submitted",
  landed: "bg-st-landed",
  commitment_confirmed: "bg-st-confirmed",
  commitment_finalized: "bg-st-finalized",
};

export function LifecycleTimeline({ milestones }: { milestones: Milestone[] }) {
  if (milestones.length === 0) return null;
  return (
    <ol className="relative">
      {milestones.map((m, i) => {
        const isLast = i === milestones.length - 1;
        return (
          <li key={m.key} className="relative flex gap-3 pb-5 last:pb-0">
            {!isLast && (
              <span
                className="absolute left-[5px] top-3.5 h-full w-px bg-border"
                aria-hidden
              />
            )}
            <span
              className={`relative z-10 mt-1 size-2.5 shrink-0 rounded-full ring-4 ring-canvas ${DOT[m.key] ?? "bg-st-submitted"}`}
              aria-hidden
            />
            <div className="min-w-0 flex-1">
              <div className="flex items-baseline justify-between gap-3">
                <span className="text-sm font-medium text-ink">{m.label}</span>
                {m.deltaMs !== undefined && (
                  <span className="font-mono text-xs text-ink-2">
                    +{fmtDuration(m.deltaMs)}
                  </span>
                )}
              </div>
              {m.slot !== undefined && (
                <a
                  href={solscanSlot(m.slot)}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="font-mono text-xs text-ink-3 hover:text-brand-ink"
                >
                  slot {fmtSlot(m.slot)}
                </a>
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}
