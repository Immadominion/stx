import { faultInjections, overallFunnel, race, traces } from "@/data";
import type { Funnel } from "@/lib/trace";
import { DecisionCard } from "@/components/DecisionCard";
import { Diagnose } from "@/components/Diagnose";
import { RaceCard } from "@/components/RaceCard";
import { TraceCard } from "@/components/TraceCard";
import { ExtLink } from "@/components/ui";

const REPO = "https://github.com/Immadominion/stx";

export default function Home() {
  const agentTrace = traces.find((t) => t.usedAgent);
  return (
    <div className="min-h-dvh">
      <SiteHeader />
      <main className="mx-auto max-w-3xl space-y-14 px-6 py-12">
        <Intro funnel={overallFunnel} />

        <RaceCard naive={race.naive} stx={race.stx} />

        <Diagnose />

        <section className="space-y-5">
          <SectionHead
            title="Transactions"
            desc="Every transaction is a trace: submitted, dispatched, landed, confirmed. Timed from validator ground truth, with the tip and retries that got it there. Slots and signatures link to Solscan."
          />
          {traces.map((t) => (
            <TraceCard key={t.traceId} trace={t} />
          ))}
        </section>

        <section className="space-y-5">
          <SectionHead
            title="The agent's decisions"
            desc="When a submission fails, the agent gathers evidence with read-only tools, tests a hypothesis by simulation, and commits one cause-appropriate remedy, bounded by a deterministic guardrail."
          />
          {agentTrace && agentTrace.decisions.length > 0 && (
            <div className="space-y-3">
              <p className="text-sm text-ink-2">
                Steering the live retry above. The agent learns from each failed
                tip and escalates until the bundle lands.
              </p>
              {agentTrace.decisions.map((d) => (
                <DecisionCard key={d.decision_id} rec={d} />
              ))}
            </div>
          )}
          <div className="space-y-3">
            <p className="text-sm text-ink-2">
              Controlled fault injection: three different failures, three
              different remedies.
            </p>
            {faultInjections.map((d) => (
              <DecisionCard
                key={d.decision_id}
                rec={d}
                heading={faultLabel(d.trigger)}
              />
            ))}
          </div>
        </section>

        <SiteFooter />
      </main>
    </div>
  );
}

function SiteHeader() {
  return (
    <header className="sticky top-0 z-10 border-b border-border bg-canvas">
      <div className="mx-auto flex max-w-3xl items-center justify-between px-6 py-3.5">
        <div className="flex items-baseline gap-2.5">
          <span className="font-mono text-sm font-semibold tracking-tight text-ink">
            stx
          </span>
          <span className="text-xs text-ink-3">control tower</span>
        </div>
        <div className="flex items-center gap-3">
          <span className="inline-flex items-center gap-1.5 rounded-full bg-st-landed/10 px-2.5 py-1 text-xs font-medium text-st-landed-ink">
            <span className="size-1.5 rounded-full bg-st-landed" aria-hidden />
            mainnet
          </span>
          <ExtLink href={REPO} className="text-xs">
            GitHub
          </ExtLink>
        </div>
      </div>
    </header>
  );
}

function Intro({ funnel }: { funnel: Funnel }) {
  return (
    <section className="space-y-5">
      <h1 className="text-balance text-2xl font-semibold tracking-tight text-ink">
        Watch a Solana transaction settle, the way you watch a payment clear.
      </h1>
      <p className="max-w-prose text-pretty text-ink-2">
        stx submits Jito bundles with tips computed from live floor data, tracks
        each one across commitment levels from the Yellowstone stream, classifies
        failures, and lets an AI agent decide how to retry. Everything below is a
        real run on mainnet.
      </p>
      <FunnelBar funnel={funnel} />
    </section>
  );
}

function FunnelBar({ funnel }: { funnel: Funnel }) {
  const stages = [
    { label: "submitted", n: funnel.submitted, color: "bg-st-submitted" },
    { label: "landed", n: funnel.landed, color: "bg-st-landed" },
    { label: "confirmed", n: funnel.confirmed, color: "bg-st-confirmed" },
  ];
  const max = Math.max(funnel.submitted, 1);
  return (
    <div className="rounded-card border border-border bg-canvas p-5">
      <div className="flex items-end justify-between gap-4">
        {stages.map((s) => (
          <div key={s.label} className="flex-1">
            <div className="flex h-16 items-end">
              <div
                className={`w-full rounded-t ${s.color}`}
                style={{ height: `${Math.max((s.n / max) * 100, 8)}%` }}
              />
            </div>
            <div className="mt-2 flex items-baseline justify-between">
              <span className="text-xs text-ink-2">{s.label}</span>
              <span className="font-mono text-sm text-ink">{s.n}</span>
            </div>
          </div>
        ))}
      </div>
      <p className="mt-4 text-xs text-ink-3">
        {funnel.failedAttempts} failed attempts detected, classified, and retried
        to a landing.
      </p>
    </div>
  );
}

function SectionHead({ title, desc }: { title: string; desc: string }) {
  return (
    <div className="space-y-1.5">
      <h2 className="text-lg font-semibold tracking-tight text-ink">{title}</h2>
      <p className="max-w-prose text-pretty text-sm text-ink-2">{desc}</p>
    </div>
  );
}

function SiteFooter() {
  return (
    <footer className="border-t border-border pt-6 text-xs text-ink-3">
      <p>
        stx is open source. The architecture document, the research, and the raw
        lifecycle logs behind this page are in the{" "}
        <ExtLink href={REPO}>repository</ExtLink>.
      </p>
    </footer>
  );
}

function faultLabel(trigger: string): string {
  const map: Record<string, string> = {
    "fault_injected:blockhash_expiry": "Injected: blockhash expiry",
    "fault_injected:fee_starvation": "Injected: fee starvation",
    "fault_injected:compute_exhaustion": "Injected: compute exhaustion",
  };
  return map[trigger] ?? trigger;
}
