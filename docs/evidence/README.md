# Evidence

Real artifacts from `stx` running against mainnet (Solinfra RPC + gRPC, Jito Frankfurt block engine, Anthropic Opus 4.8). Slots and signatures are explorer-verifiable.

## AI agent: three injected failures, three different remedies

`stx fault-inject <scenario>` runs the real agent against a controlled failure. The agent gathers evidence with read-only tools, tests hypotheses with live simulation, and commits one cause-appropriate decision. The point: the remedies differ, which is what separates reasoning from a fixed retry script.

| File | Injected failure | Agent's remedy |
|---|---|---|
| `agent-blockhash-expiry.json` | expired blockhash | `resubmit_new_blockhash` (refresh, tip unchanged) |
| `agent-fee-starvation.json` | tip below floor | `raise_tip` to p75 (CU unchanged) |
| `agent-compute-exhaustion.json` | CU at the ceiling | `raise_cu` to 260k (tip unchanged) |

Each record contains the observations, the full reasoning trace, every tool call (including the simulations), the committed decision, and the guardrail verdict.

## AI agent steering a real retry

`agent-live-decisions.json` is a live mainnet run with `--use-agent`. The bundle did not land at p50 or p75. Using the retry history, the agent recognised that the already-tried tips kept getting dropped and escalated toward p95, where the bundle landed. It also self-corrected between attempts (attempt 1 guessed blockhash staleness; attempt 2, seeing the history, switched to fee starvation).

- Landed: slot **424863529**, confirmed from the Yellowstone stream.
- `lifecycle-agent-run.json` is the append-only event log for that run.
- tx: https://solscan.io/tx/2qkVdddLBRUgG4bBJeA8HCzTpASC46rHjuYxLw59UJvZtuvXagDiR1awzsiFVqZ3YTCWtJfgYGYTErFWaHpQXLsF

## Deterministic fallback (AI disabled)

`lifecycle-fallback-run.json` shows the stack landing with the AI off, using only the deterministic escalation policy: two failures, then a landing at p95.

- Landed: slot **424777434**.
- tx: https://solscan.io/tx/22ySyfKac78WGJudeUPy7Jt4x52G8YwVTPhLCHKTbe2kJVA6YHauFwuAAEjUCoue5yg9SERAVsRMzS7QkVUQgCon

## Note on an early bug (kept honest)

The first live runs double-submitted: confirmation was opened after submit (racing a sub-second landing) and trusted Jito's `getInflightBundleStatuses`, which returned `Invalid` for bundles that had actually landed. The fix was to subscribe to the signature before submitting and cross-check the authoritative RPC `getSignatureStatuses`. Confirmation now never false-negatives, so the loop never resubmits a landed transaction.
