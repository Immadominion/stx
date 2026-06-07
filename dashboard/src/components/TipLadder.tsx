import type { AttemptInfo } from "@/lib/trace";
import { fmtLamports } from "@/lib/trace";

export function TipLadder({ attempts }: { attempts: AttemptInfo[] }) {
  if (attempts.length === 0) return null;
  const max = Math.max(...attempts.map((a) => a.tip), 1);
  return (
    <div className="space-y-2">
      {attempts.map((a) => {
        const w = Math.max((a.tip / max) * 100, 6);
        const bar =
          a.outcome === "landed"
            ? "bg-st-landed"
            : a.outcome === "failed"
              ? "bg-st-failed/55"
              : a.outcome === "aborted"
                ? "bg-st-failed/35"
                : "bg-st-submitted";
        return (
          <div
            key={a.attempt}
            className="grid grid-cols-[3.5rem_1fr_4rem] items-center gap-3"
          >
            <span className="font-mono text-xs text-ink-2">try {a.attempt}</span>
            <div className="relative h-5 rounded bg-sunk">
              <div
                className={`absolute inset-y-0 left-0 rounded ${bar}`}
                style={{ width: `${w}%` }}
              />
              <span className="absolute inset-y-0 left-2 flex items-center font-mono text-[11px] text-ink">
                {fmtLamports(a.tip)}
              </span>
            </div>
            <span className="text-right font-mono text-xs text-ink-2">
              {a.outcome === "landed" ? "landed" : a.outcome}
            </span>
          </div>
        );
      })}
    </div>
  );
}
