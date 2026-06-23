"use client";

import { useState } from "react";
import { solscanSlot, solscanTx } from "@/lib/trace";
import { ExtLink, StatusBadge } from "./ui";

type Diagnosis = {
  signature: string;
  found: boolean;
  succeeded: boolean;
  slot: number | null;
  commitment: string | null;
  leader: string | null;
  fee_lamports: number | null;
  compute_units: number | null;
  error: string | null;
  classification: { kind: string; evidence: string; confidence: number } | null;
  facts: string[];
  headline: string;
  explanation: string | null;
};

const EXAMPLES = [
  {
    label: "a failed swap",
    sig: "5W2QENRJEAgyr6RwksqqbgvhFKsFGNEwwCoDVgBmcx3ZionaH4dK5mLnHiQB68njBcQ3FArpTV8nBDTmMGotZK8v",
  },
  {
    label: "a dropped tx",
    sig: "4nzZ78L1V3hPGS1dCSuS5EGiebHFQDcK8dNp4c9UUfF6FNjpGo6AGmhooLbiEHJggNgwWwNw5aypbYCyXg3F111a",
  },
];

export function Diagnose() {
  const [sig, setSig] = useState("");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<Diagnosis | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(value: string) {
    const s = value.trim();
    if (!s) return;
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const r = await fetch(`/api/diagnose?sig=${encodeURIComponent(s)}`);
      const j = await r.json();
      if (!r.ok) setError(j.error || "diagnosis failed");
      else setResult(j);
    } catch {
      setError("request failed; try again");
    }
    setLoading(false);
  }

  const status = result
    ? result.succeeded
      ? "finalized"
      : result.found
        ? "failed"
        : "aborted"
    : "pending";

  return (
    <section className="rounded-card border border-border bg-surface p-5">
      <h3 className="text-sm font-semibold text-ink">The Transaction ER</h3>
      <p className="mt-1 max-w-2xl text-sm text-ink-2">
        Paste any Solana transaction signature. stx fetches what actually happened
        on-chain, classifies it with the same taxonomy the submit loop uses, and
        the AI explains why it landed or died and what to change. Your wallet says
        a transaction &ldquo;failed&rdquo;; this tells you why.
      </p>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          run(sig);
        }}
        className="mt-4 flex flex-col gap-2 sm:flex-row"
      >
        <input
          value={sig}
          onChange={(e) => setSig(e.target.value)}
          placeholder="transaction signature"
          spellCheck={false}
          className="flex-1 rounded-md border border-border bg-canvas px-3 py-2 font-mono text-sm text-ink placeholder:text-ink-3 focus:border-brand focus:outline-none"
        />
        <button
          type="submit"
          disabled={loading || !sig.trim()}
          className="rounded-md bg-ink px-4 py-2 text-sm font-medium text-canvas transition-opacity disabled:opacity-40"
        >
          {loading ? "diagnosing…" : "Diagnose"}
        </button>
      </form>

      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-ink-3">
        <span>try:</span>
        {EXAMPLES.map((ex) => (
          <button
            key={ex.sig}
            onClick={() => {
              setSig(ex.sig);
              run(ex.sig);
            }}
            className="rounded border border-border px-2 py-0.5 text-ink-2 hover:border-brand"
          >
            {ex.label}
          </button>
        ))}
      </div>

      {error && <p className="mt-3 text-sm text-st-failed-ink">{error}</p>}

      {result && (
        <div className="mt-4 rounded-card border border-border bg-canvas p-4">
          <div className="flex flex-wrap items-center gap-3">
            <StatusBadge status={status} />
            <span className="text-sm font-medium text-ink">{result.headline}</span>
          </div>
          {result.explanation && (
            <p className="mt-3 text-sm leading-relaxed text-ink-2">
              {result.explanation}
            </p>
          )}
          <dl className="mt-3 flex flex-wrap gap-x-5 gap-y-1 font-mono text-xs text-ink-3">
            {result.slot != null && (
              <span>
                slot{" "}
                <ExtLink href={solscanSlot(result.slot)}>
                  {result.slot.toLocaleString("en-US")}
                </ExtLink>
              </span>
            )}
            {result.leader && <span>leader {result.leader.slice(0, 8)}…</span>}
            {result.fee_lamports != null && <span>fee {result.fee_lamports}</span>}
            {result.compute_units != null && <span>CU {result.compute_units}</span>}
          </dl>
          <div className="mt-3">
            <ExtLink href={solscanTx(result.signature)} className="font-mono text-xs">
              view on Solscan
            </ExtLink>
          </div>
        </div>
      )}
    </section>
  );
}
