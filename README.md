# stx

A smart Solana transaction stack that treats every transaction as a distributed trace.

Sending a transaction on Solana is only the visible tip of a long pipeline: leader scheduling, TPU ingestion, block production, shred propagation, and three commitment stages. stx treats each transaction or bundle as a trace, where every lifecycle stage is a span timed from validator ground truth. It streams live slot and leader data over Yellowstone gRPC, submits Jito bundles with tips computed from live tip-floor data, tracks the lifecycle across processed, confirmed and finalized, classifies failures, and retries automatically. An AI agent owns one real decision: diagnosing why a transaction failed and what to change before retrying.

* Architecture document: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
* Architecture diagram: [docs/diagrams/architecture.excalidraw.json](docs/diagrams/architecture.excalidraw.json)
* Research notes: [docs/research/](docs/research/)

## Status

The core stack and the AI layer are built and tested. 64 tests pass across the library crates (`cargo test --workspace`). The CLI is verified live against the Jito API.

| Crate | Responsibility | Tests |
|---|---|---|
| `stx-core` | Event-sourced lifecycle FSM, commitment ladder, failure taxonomy, span model, decision records, in-memory event store, span and funnel projections, deterministic fallback policy | 22 |
| `stx-jito` | Live tip-floor engine, bundle builder (solana-sdk 4), Jito Block Engine JSON-RPC client with multi-region fan-out, Solana RPC client, data-derived failure classifier | 23 |
| `stx-ingestor` | Yellowstone gRPC: slot and signature subscriptions, observation normalizer, ping keep-alive, auto-reconnect | 5 |
| `stx-agent` | AI tool-use reasoning loop, deterministic guardrail validator, auditable decision records, fault-injection scenarios | 14 |
| `stx-cli` | Runnable binary: live tip data, the AI fault-injection demo, and (with credentials) live bundle submission | n/a |

One principle runs through all of it. The deterministic core works fully with the AI disabled, falling back to an explicit policy, so the AI is never a liveness dependency. It only improves decision quality at named decision points, and it sits where model latency (about 1 to 3 seconds) is free (post-failure reasoning), never on the 400ms hot path.

## Run it

```bash
cargo build --release        # builds the `stx` binary

# Live Jito tip floor (no credentials needed)
./target/release/stx tip-floor

# The 8 Jito tip accounts
./target/release/stx tip-accounts

# Run the AI agent against an injected failure (needs ANTHROPIC_API_KEY).
# It gathers evidence with read-only tools, tests a hypothesis with a
# simulation, then commits a cause-appropriate decision, bounded by the
# guardrail and printed as an auditable decision record.
export ANTHROPIC_API_KEY=...
./target/release/stx fault-inject blockhash-expiry
./target/release/stx fault-inject fee-starvation
./target/release/stx fault-inject compute-exhaustion
```

The three `fault-inject` scenarios are the acceptance test for real reasoning. The agent must gather different evidence and commit three different cause-appropriate remedies (refresh the blockhash and keep the tip, raise the tip, raise the compute limit), not three blanket retries.

Live bundle submission and the dashboard wire the Yellowstone, RPC and keypair credentials into the lifecycle tracker. The deterministic pieces they compose are already built and tested above.

## The three questions

### 1. What does the delta between processed_at and confirmed_at tell you about network health?

It measures how long the cluster took to reach optimistic supermajority (over 2/3 of stake) confirmation on the block containing your transaction. That is the latency of the consensus voting round-trip, not raw execution. Execution (processed) happens the instant the leader includes the transaction; confirmation requires a supermajority of stake to replay the block and land votes covering that slot. So the delta is a direct probe of vote propagation and consensus speed.

A small, stable delta (roughly half a second to a second, one or two slots) means a healthy cluster: votes flowing fast, little fork contention, validators on schedule. A large or growing delta means the cluster is struggling to agree. The usual causes are fork contention (a processed transaction can sit on a minority fork and only confirm once over 2/3 of stake votes through that slot), a high skip rate (missed leader slots delay the vote chain), and vote or replay lag under congestion. We will augment this with measured deltas from the live runs.

### 2. Why should you never use finalized commitment when fetching a blockhash for a time-sensitive transaction?

A recent blockhash is valid only while it stays within about the last 150 blocks (roughly 60 to 90 seconds). getLatestBlockhash returns lastValidBlockHeight as the current block height plus 150.

A finalized blockhash is already about 32 slots (around 13 seconds) old by definition, because finalized means rooted with 31 or more confirmed blocks on top. And getLatestBlockhash defaults to finalized commitment, which is a trap. Starting from a finalized blockhash burns roughly 20 percent of the validity window before you even sign, which shrinks usable retry time and raises the odds of expiry. That shows up as BlockhashNotFound (rejected at submit) or TransactionExpiredBlockheightExceededError (never landed in time). To keep the window large you fetch the freshest blockhash. We use confirmed: fresh but on the canonical chain, so it is not a minority-fork hash that vanishes. stx fetches blockhashes at confirmed in [stx-jito/src/solrpc.rs](crates/stx-jito/src/solrpc.rs).

### 3. What happens to your bundle if the Jito leader skips their slot?

Bundles are only processed by a Jito-Solana leader, so a bundle is effectively targeted at an upcoming Jito leader's slots, and it is atomic (all transactions execute in the same slot, in order; if any fails, none commit; max 5 transactions).

If that leader skips their slot (no block produced), the bundle has no block to land in, so it is simply not included. It does not roll over to the next leader and it is not retried for you. getInflightBundleStatuses reports it as Pending and then ages out to Failed or Invalid; it never reaches Landed. You must resubmit it yourself, targeting the next Jito leader window, and if the blockhash expired meanwhile you rebuild with a fresh one and usually a higher tip, since inclusion is a tip auction. stx's retry path does exactly this, and the classifier separates this "never landed" case from explicit errors. (A truly skipped slot is not the same as an uncled block: if a leader did build a block that the supermajority then skips and someone rebroadcasts the individual transactions, those re-enter the normal banking stage, which does not respect bundle atomicity.)

## Roadmap

Built and tested: the full component library and the AI reasoning layer, plus a live-verified CLI. Remaining: the end-to-end submit orchestrator (RPC, bundle builder, multi-region submit, stream confirmation, agent or fallback retry), the dashboard (trace waterfall, lifecycle timeline, stage funnel, tip versus landing curve, live reasoning feed), and the live mainnet runs (10 or more bundle submissions including failures, explorer-verifiable, producing the lifecycle log).

## License

MIT
