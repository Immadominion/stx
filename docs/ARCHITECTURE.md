# stx - Architecture Design Document

> 📄 **Published, separately-hosted version:** https://stx-architecture.vercel.app (the canonical architecture document for the submission). This in-repo file is the source notes.

> A smart Solana transaction stack that treats every transaction as a **distributed trace**: observed in real time from validator ground truth, submitted intelligently via Jito bundles, tracked across commitment levels as a span waterfall, and steered by an AI operator-copilot that owns one real operational decision - *why a transaction failed and what to change before retrying*.
>
> **Working name:** stx (transactions "land" on Solana; a control tower tracks and maximizes landings). Status: design. Target network: **mainnet-beta**. Stack: **Rust core + Next.js dashboard + Claude-powered AI brain**.

---

## 0. The one-paragraph thesis

On Solana, "sending a transaction" is the visible tip of a long pipeline - leader scheduling, TPU ingestion, block production, shred propagation, and three commitment stages. That pipeline is **exactly the shape of a distributed-systems request trace**. stx makes the analogy literal: each transaction/bundle is a **trace** (`trace_id` = signature / bundle-id); each lifecycle stage is a **span** with a real start/end sourced from Yellowstone/Geyser ground truth; the latency deltas the bounty asks for *are* span durations; failure classes are span error tags; retries are linked child traces. The result is a system that is **legible to a web2 engineer on sight** (it looks like Jaeger/Honeycomb/Vercel tracing or a Stripe payment timeline) while being **deep and correct** for a Solana judge (commitment handling becomes visible span boundaries, not a `console.log`).

---

## 1. Design goals & principles

| # | Principle | What it forces in the design |
|---|---|---|
| P1 | **Ground truth, not polling.** | Lifecycle state comes from the Yellowstone stream (data from inside the validator) and Jito bundle-result streams - never from `getSignatureStatuses` polling loops. RPC is a fallback/secondary confirmer only. |
| P2 | **Policy / mechanism separation.** | The deterministic core (stream, build, submit, track, retry) is fully functional with the AI **disabled**. The AI is a decision *brain* consulted at named points; its outputs are validated and bounded before execution. |
| P3 | **The AI lives where latency is free.** | LLM latency (~1-3 s) ≫ slot time (400 ms). The agent owns *post-failure* and *policy* decisions, never per-slot hot-path timing. This is what makes the AI layer defensible rather than theater. |
| P4 | **No hardcoded shortcuts.** | Tips come from live Jito percentile data. Failure classes are derived from real `err`/bundle-result fields. Retry decisions come from the agent (or an explicit, documented fallback policy), never an `if error == X` switch presented as intelligence. |
| P5 | **Everything is an event.** | The core is **event-sourced**: an append-only log of lifecycle events is the source of truth; the FSM state, the dashboard projections, the lifecycle log export, and the trace waterfall are all *derived* from it and fully replayable. |
| P6 | **Correctness is compiler-checked where it can be.** | Rust core: commitment levels, slot statuses, and the failure taxonomy are exhaustive enums; the FSM transitions are total functions; illegal states are unrepresentable. |
| P7 | **Evidence over claims.** | Every run emits explorer-verifiable signatures/bundle-ids and a structured lifecycle log; every AI decision emits an auditable record with the outcome backfilled. |

---

## 2. System architecture

Five layers. Data flows up (observation) and down (action); the event store is the spine.

```
                                   ┌──────────────────────────────────────────────┐
   ┌─────────────────────────┐     │  L5  DASHBOARD  (Next.js · "Control Tower")    │
   │ Solana mainnet-beta     │     │  trace waterfall · lifecycle timeline ·        │
   │  • leaders / slots      │     │  stage funnel · tip↔landing curve ·            │
   │  • Jito Block Engine     │     │  AI reasoning feed · lifecycle log export      │
   │  • Jito tip_floor/stream │     └───────────────▲────────────────────────────────┘
   └───────┬──────────▲───────┘                     │ WebSocket (live) + REST (history)
           │          │                  ┌──────────┴───────────┐
   (gRPC)  │          │ (HTTP/gRPC)       │  L4  GATEWAY / API    │  read-model projections
           ▼          │                   │  (Rust · axum)        │  served from event store
   ┌──────────────────┴───────────────────┴───────────────────────────────────────────┐
   │  L2  CORE TRANSACTION STACK  (Rust · deterministic mechanism - runs without AI)     │
   │                                                                                     │
   │   Ingestor ──► Event Bus ──►  Lifecycle FSM  ──►  Event Store (append-only) ──┐     │
   │  (Yellowstone)               (per-trace state)   (source of truth)           │     │
   │      ▲                              ▲                                         │     │
   │      │           ┌──────────────────┴───────────────┐          Projections ◄─┘     │
   │   Blockhash mgr  │ Submitter (multi-region Jito) ◄─ Bundle builder ◄─ Tip engine    │
   │      │           │ Failure classifier   Retry orchestrator   Static fallback policy │
   │      └───────────┴──────────────────┬───────────────┘                              │
   └──────────────────────────────────── │ ───────────────────────────────────────────┘
                                          │ decision request (async, off hot path)
                                          ▼
   ┌─────────────────────────────────────────────────────────────────────────────────┐
   │  L3  AI BRAIN  (Rust client → Claude · policy)                                     │
   │   observe → reason → decide loop · read-only tools + one strict commit tool        │
   │   GUARDRAIL VALIDATOR (clamps/bounds, never trusts the LLM) · DecisionRecord log    │
   └─────────────────────────────────────────────────────────────────────────────────┘

   L1 = external Solana/Jito infrastructure (top box).  L2 mechanism · L3 policy · L4 serve · L5 surface.
```

A richer, editable version of this diagram ships as [docs/diagrams/architecture.excalidraw](diagrams/architecture.excalidraw).

---

## 3. Key components

### L2 - Core transaction stack (Rust)

**3.1 Stream Ingestor (`ingestor`)** - the eyes.
- Subscribes to Yellowstone gRPC: `slots` (with `interslot_updates` for leader-window timing), `transactions`/`transactions_status` (filtered by the signatures we submit, for stream-native landing confirmation), and `blocks_meta` (for blockhash/blockheight freshness).
- Normalizes raw `SubscribeUpdate`s into internal **observation events** and publishes them to the event bus.
- Owns connection health: ping/pong keepalive, gRPC keepalive args, raised `max_decoding_message_size`, exponential-backoff reconnect with `from_slot` replay + dedup. Backpressure-aware (bounded channels; if a consumer lags we shed or reconnect rather than OOM).

**3.2 Blockhash Manager (`blockhash`)** - freshness authority.
- Maintains a current blockhash fetched at **`confirmed`** (never `finalized`), with its `lastValidBlockHeight`.
- Tracks live block height (from `blocks_meta` stream + RPC fallback) and exposes `is_expired(hash) -> bool` and `slots_remaining(hash)`.
- The retry path consults this; a re-sign is permitted **only** after genuine expiry (`currentBlockHeight > lastValidBlockHeight`).

**3.3 Tip Engine (`tips`)** - dynamic, never hardcoded.
- Polls `bundles.jito.wtf/api/v1/bundles/tip_floor` and subscribes to the `tip_stream` WebSocket; keeps the latest percentile distribution (25/50/75/95/99 + EMA), converting SOL→lamports.
- Exposes `recommend(context) -> TipLamports` for the **static fallback policy** (e.g. target a percentile scaled by congestion), and feeds the same data to the AI brain as an observation. Enforces the 1000-lamport floor.

**3.4 Bundle Builder (`bundle`)** - correct Jito construction.
- Builds a signed bundle (≤5 tx, base64), placing a tip transfer to a **randomly chosen** tip account in the same transaction as the main logic (per Jito best practice). Attaches compute-budget instructions (CU limit from simulation + margin; CU price as priority fee - a *separate* rail from the tip).
- Pure/deterministic given inputs → trivially testable.

**3.5 Submitter (`submit`)** - multi-region dispatch.
- Sends the bundle to the global endpoint + N nearest regions concurrently (per-IP-per-region rate budgets are independent). Optionally subscribes to gRPC `subscribeBundleResults` for the richest accept/reject/drop reasons.
- Records the **dispatch span** (one child span per region attempt) and the returned `bundle_id`.
- Uses `getNextScheduledLeader`/`getConnectedLeaders` to time dispatch into the Jito leader's 4-slot window.

**3.6 Lifecycle FSM + Event Store (`lifecycle`)** - the spine (event-sourced).
- Each transaction/bundle is an aggregate with a total transition function over states: `Drafted → TipDecided → Built → Dispatched → Inflight → Landed(slot) → Processed → Confirmed → Finalized`, plus failure transitions `Expired | FeeTooLow | ComputeExceeded | BundleFailed | Dropped | Aborted`.
- Events (timestamped, slot-stamped) are appended to the store; FSM state and all read models are projections. Span boundaries are computed from event timestamps. Replayable end-to-end.

**3.7 Failure Classifier (`classify`)** - data-derived, not a switch.
- Maps observed signals to the bounty's four named classes + extras, using **real fields**: `meta.err` (`TransactionError`/`InstructionError::ComputationalBudgetExceeded`), blockhash expiry math (`BlockhashNotFound` vs `TransactionExpiredBlockheightExceededError`), Jito bundle results (`Dropped{BlockhashExpired|…}`, `Rejected{…}`), and "never landed" inference from the stream + leader schedule.
- Emits a structured `FailureClass { kind, evidence, confidence }` that the retry path and the AI brain consume.

**3.8 Retry Orchestrator (`retry`)** - bounded, decision-driven.
- On a failure event, gathers context and asks the AI brain for a decision (or, if AI disabled/slow/low-confidence, applies the documented static fallback). Enforces `max_attempts`, backoff, and the blockhash-refresh precondition. Each retry is a **linked child trace**.

**3.9 Static Fallback Policy (`policy`)** - the liveness floor.
- A small, explicit, *documented* rule set (target tip = f(percentile, congestion); refresh blockhash on expiry; raise CU on compute-exceeded; abort after N) that keeps the stack landing transactions when the AI is off. It is the baseline the AI is measured against - never dressed up as "intelligence."

### L3 - AI Brain (Rust client → Claude)

**3.10 Failure-Reasoning / Autonomous-Retry Agent.** See §7.

### L4 - Gateway / API (Rust · axum)

**3.11 Projections & API.** Read models built from the event store (per-trace span tree, stage funnel counts, tip/landing series, decision feed). Serves REST for history + a WebSocket for live push to the dashboard. Optional OTLP exporter so a judge can open the same trace in Jaeger/Honeycomb.

### L5 - Dashboard (Next.js · the "Control Tower")

**3.12 Surface.** Live **trace waterfall** per transaction; Stripe-style **lifecycle timeline**; **stage funnel** (submitted→landed→processed→confirmed→finalized drop-off); **tip-percentile vs landing-probability** chart; **AI reasoning feed** (decision records with drill-down and the post-hoc "was it right?" verdict); one-click **lifecycle log export**. Built to impeccable's standards (see [docs/research/05-differentiation-strategy.md](research/05-differentiation-strategy.md)).

---

## 4. Data flow (end to end)

**Submission path (action, top-down):**
1. A submission intent enters the core → FSM `Drafted`.
2. Tip engine + (optional) AI/policy produce a tip → `TipDecided` (span: `tip.decide`).
3. Bundle builder assembles + signs → `Built` (span: `bundle.build`).
4. Submitter fans out to Jito regions → `Dispatched` → `Inflight` (spans: `dispatch.<region>`, `auction.wait`).

**Observation path (truth, bottom-up):**
5. Ingestor sees the bundle's signature land in a slot via the Yellowstone `transactions` stream → `Landed(slot)` (span edge: `leader.inclusion`).
6. Slot-status stream advances `Processed → Confirmed → Finalized`; each transition closes a span and records the **latency delta** (the README-question-1 signal).
7. On any failure signal, the classifier emits a `FailureClass`; the retry orchestrator consults the AI brain; a decision is validated and either resubmits (new linked trace) or aborts.

Every step appends an event; the gateway projects events into the dashboard's live views in real time.

---

## 5. The trace / span model (the abstraction, precisely)

| Span | Opens on | Closes on | Duration means |
|---|---|---|---|
| `tip.decide` | Drafted | TipDecided | AI/policy decision latency |
| `bundle.build` | TipDecided | Built | local build+sign time |
| `dispatch.<region>` | Built | region ack/`bundle_id` | network RTT to each Block Engine region |
| `auction.wait` | Dispatched | Landed | time from dispatch to inclusion (auction + leader window) |
| `leader.inclusion`→`processed` | Landed | processed update | time to first processed observation |
| `processed`→`confirmed` | processed | confirmed update | **consensus/voting latency = cluster health probe** |
| `confirmed`→`finalized` | confirmed | finalized update | rooting latency (~12.8 s structural floor) |

A retry is a **new trace** with a `links` reference to its parent (OpenTelemetry "span link" semantics), so the waterfall shows the full saga. Optionally exported as real OTLP.

---

## 6. Infrastructure decisions

| Decision | Choice | Rationale |
|---|---|---|
| Core language | **Rust** | First-class `yellowstone-grpc-client` + Jito tooling; exhaustive enum matching on commitment/error types; tokio for the concurrent stream/submit fabric; correctness is compiler-checked (P6). |
| Dashboard | **Next.js + TypeScript + Tailwind/shadcn** | The web2-legible surface; impeccable-grade design. |
| AI integration | **Rust client → Anthropic Messages API** (manual tool-use loop) | Keeps the brain in-process with the core; fine-grained control for logging/guardrails; prompt caching for cost/latency. |
| Network | **mainnet-beta** | Only network with a real Jito auction + realistic tip data + explorer-verifiable slots (judges cross-reference). Tip cost ≈ pennies. (Testnet used for dev/fault-testing.) |
| Streaming provider | Yellowstone gRPC via SolInfra credits (or Helius LaserStream / Triton / QuickNode) | Config-driven `endpoint` + `x-token`; the bounty offers SolInfra credits. |
| RPC | Same provider | Fallback confirmer + `getLatestBlockhash(confirmed)`, `simulateTransaction`, `getRecentPrioritizationFees`. |
| Event store | Embedded append-only log (SQLite/`sled` for dev; Postgres-ready) | Simple, replayable, no external dependency to demo; projections rebuildable. |
| Transport to UI | WebSocket (live) + REST (history) | Push-based, matches the real-time nature; OTLP export optional. |
| Secrets | Keypair + API keys via env / file, never logged | See §9. |

---

## 7. AI agent responsibilities

**Owned decision:** *Failure Reasoning + Autonomous Retry.* When a transaction/bundle fails (in production) or a fault is injected (in the demo), the agent diagnoses the **root cause** and chooses a **cause-appropriate remedy** - not a blanket "bump everything and retry."

**Why this is real reasoning, not a wrapper:** the same surface error has different root causes needing different (sometimes counterintuitive) remedies - a slippage error code can actually be compute exhaustion or fee starvation in disguise; a blockhash expiry needs a refresh and *no* tip change; an adverse market means *abort*, because retrying burns money. Disambiguation requires combining error code + CU-consumed-vs-requested + fee-vs-percentile + blockhash age + retry history, and can be validated mid-loop via `simulate_with_params`. A lookup table would be wrong precisely when it matters.

**Loop (manual tool-use, Claude Opus):**
- `system` = frozen policy prompt (role, the bounded envelope, the decision-schema contract, "diagnose before acting; cite observed values"). Cached.
- **Read-only observation tools:** `get_failure_context`, `get_network_conditions`, `get_tip_floor`, `get_retry_history`, `simulate_with_params` (test a hypothesis against a real simulation before committing).
- **One strict commit tool:** `commit_decision(action, tip_lamports, hypotheses[], chosen_cause, justification, confidence, expected_effect)` - `action ∈ {resubmit, resubmit_new_blockhash, raise_tip, raise_cu, widen_slippage, abort, hold}`.
- **Visible reasoning** via adaptive thinking (`display:"summarized"` - required on Opus 4.8 or thinking is omitted) + the required structured `justification`/`hypotheses`/`confidence` fields.

**Guardrail validator (deterministic; never trusts the LLM):** clamps `tip_lamports` to `[1000, ceiling(value_at_risk)]`; enforces `max_attempts`; validates `confidence∈[0,1]` and routes low-confidence to the conservative default; honors `resubmit_new_blockhash` only if the blockhash *actually* expired (re-checks slot age - prevents double-submit). Every clamp/override is logged.

**Visible-reasoning artifact - `DecisionRecord`:** one per decision, capturing observed → reasoned (thinking summary + hypotheses + tool-call sequence) → decided → governed (guardrail overrides) → outcome (backfilled: landed? slot? signature?) + `request_id` + token usage. Rendered as the dashboard's reasoning feed and exported with the lifecycle log. Lets us compute **agent-vs-fallback landing rate** - the strongest evidence the agent adds value.

**Fault-injection demo (the bounty's acceptance test):** run the harness ≥3× injecting a *different* failure each time (blockhash expiry / fee starvation / compute exhaustion). The agent must gather *different* evidence, discriminate hypotheses (incl. a `simulate_with_params` call), and commit *three different cause-appropriate* remedies. Then disable the AI to show the core still runs on its static fallback (clean separation) with a measurably worse landing rate (real value).

Full design: [docs/research/04-ai-agent-design.md](research/04-ai-agent-design.md).

---

## 8. Failure handling strategy

**Taxonomy & detection** (data-derived - [docs/research/03](research/03-tx-lifecycle-commitment-failures.md)):

| Class | Primary signal | Remedy space |
|---|---|---|
| Expired blockhash | `Dropped{BlockhashExpired}` / `BlockhashNotFound` / blockheight exceeded | refresh blockhash, resubmit (no tip change) |
| Fee/tip too low | never lands + lost auction (`WinningBatchBidRejected`) | raise tip toward higher percentile |
| Compute exceeded | `InstructionError::ComputationalBudgetExceeded` + CU≈limit | raise CU limit (not tip) |
| Bundle failure | inflight `Failed`/`Invalid`, `Rejected{SimulationFailure}` | rebuild/resimulate or abort |
| (extras) | `AlreadyProcessed`, adverse market, `AccountInUse` | dedup / abort / wait |

**Infrastructure resilience:**
- **Stream:** ping/pong keepalive, gRPC keepalive, raised decode size, exponential-backoff reconnect with `from_slot` replay + dedup, bounded channels for backpressure (shed/reconnect, never OOM).
- **Submission:** multi-region fan-out (independent rate budgets), `maxRetries:0` + controlled rebroadcast pattern, leader-window timing.
- **AI:** circuit-breaker → static fallback policy if the brain is down/slow/low-confidence. The stack never blocks on the LLM in a live race.
- **Liveness invariant:** with the AI disabled, the stack still streams, submits, tracks, classifies, and retries.

---

## 9. Security & operations

- **Keypair:** a dedicated low-balance mainnet hot wallet for tips/fees; loaded from env/file; **never logged or sent to the UI**; only the public key and signatures are surfaced.
- **API keys** (Anthropic, RPC/Yellowstone `x-token`): env only; never in client bundles; the dashboard talks only to our gateway.
- **Rate limits:** respect 1 req/s/IP/region per Jito region; the multi-region fan-out is the throughput mechanism, not hammering one region.
- **Blast radius:** tip ceilings and `max_attempts` bound worst-case spend; the guardrail validator is the hard stop on any LLM misbehavior.

---

## 10. README question answers (grounding)

Rigorous, defensible answers are pre-drafted in [docs/research/03-tx-lifecycle-commitment-failures.md §3](research/03-tx-lifecycle-commitment-failures.md) and will be backed by **observations from our own running system** (processed→confirmed deltas under varying conditions; the finalized-blockhash window math; an observed/forced Jito skipped-slot case). Summary:
1. **processed→confirmed delta** = consensus/voting-latency probe → small/steady = healthy fast-voting cluster; large/rising = fork contention, skip rate, congestion.
2. **never `finalized` for a time-sensitive blockhash** = a finalized hash is already ~32 slots/~13 s old, burning ~20% of the ~150-block/~60-90 s window before you sign; use `confirmed`.
3. **Jito leader skips slot** = atomic bundle has no block to land in → not included, no auto-rollover, not retried for you → resubmit (fresh blockhash if expired, likely higher tip) targeting the next Jito leader.

---

## 11. What we expect to learn (honest, to be filled from real runs)

Reserved for the README's operational-observations section: observed stage drop-off, tip overpay vs landing lift, processed→confirmed under congestion, fork risk at `processed`, agent-vs-fallback landing rate, and anything that didn't work. Happy-path-only does not score; this section is where the real operational understanding shows.

---

## Appendix A - Repository layout (planned)

```
stx/
  crates/
    stx-core/        # domain types, FSM, event store, traits
    stx-ingestor/    # Yellowstone gRPC client + normalizer
    stx-jito/        # tip engine, bundle builder, multi-region submitter, bundle-result stream
    stx-agent/       # Claude client, tool loop, guardrails, DecisionRecord
    stx-gateway/     # axum REST + WS, projections, optional OTLP exporter
    stx-cli/         # binary: run / fault-inject / export-log
  dashboard/            # Next.js control tower
  docs/                 # this doc, research dossiers, diagrams, lifecycle logs
```

## Appendix B - Source dossiers
[01 Jito bundles](research/01-jito-bundles.md) · [02 Yellowstone gRPC](research/02-yellowstone-grpc.md) · [03 TX lifecycle/commitment/failures](research/03-tx-lifecycle-commitment-failures.md) · [04 AI agent design](research/04-ai-agent-design.md) · [05 Differentiation strategy](research/05-differentiation-strategy.md)
