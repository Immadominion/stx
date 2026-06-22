# Demo script (2 to 3 minutes)

A tight, honest walkthrough. The spine is the Race: prove the smart part is smart,
then show *why* it's smart, then show the surface that makes it legible. Every
number on screen is real and on-chain checkable.

Record terminal + browser. Keep talking; let the screen do the proving.

---

## 0. Hook (15s)

> "Sending a Solana transaction is easy. Getting it to land when the network is
> busy, knowing the instant it did, and reacting the right way when it doesn't,
> is the hard part. This is stx, a control tower for that whole journey. Let me
> show you the difference it makes, live on mainnet."

Have the dashboard open at **stx-alpha.vercel.app**, scrolled to The Race.

---

## 1. The Race (45s) — the proof

> "Same transfer, fired two ways against the exact same tip-floor snapshot at the
> same moment. Left is the naive way most people do it: a fixed median tip, one
> endpoint, blind retry. Right is stx."

Point at the two tip ladders.

> "Watch the tip. The naive lane holds a flat tip and keeps losing the auction.
> stx sizes the tip off the smoothed floor, then escalates toward where landings
> are actually happening, and lands."

Read the result out loud:

> "Naive: never landed. stx: landed, finalized. Same floor, same instant. The
> only difference is the strategy. And you can click the slot to check it on
> Solscan, this is real."

Click the stx slot link → Solscan, show it's a real finalized block.

> "And this isn't one lucky run. Across six races back to back under contention,
> the naive way landed zero. stx landed all six, every one finalized on-chain."

(Evidence: docs/evidence/races/README.md — naive 0/6, stx 6/6.)

Optional live version (riskier, more impressive): run it in front of them:

```
stx race --timeout 25
```

---

## 2. Why it's smart (45s) — the agent

> "When a submission fails, stx doesn't just retry and hope. It diagnoses why."

Run a fault-injection live (no real money, instant):

```
stx fault-inject fee-starvation
```

> "It gathered evidence, ran a simulation to rule out compute and blockhash
> problems, concluded the tip was the issue, and committed exactly one remedy,
> raise the tip. Throw a different failure at it..."

```
stx fault-inject blockhash-expiry
```

> "...and it refreshes the blockhash instead. Same loop, different diagnosis,
> different fix. Raising the tip on a blockhash problem does nothing, and that's
> the kind of mistake a blind retry makes. Every decision is recorded, with the
> reasoning and a deterministic guardrail bounding it."

Cut to the dashboard "agent's decisions" section to show a real recorded decision.

---

## 3. The surface (30s) — legibility

> "And the whole life of every transaction is a trace you can read, the way you'd
> read a payment clearing. Submitted, dispatched, landed, confirmed, finalized,
> with the real time between each stage, measured from the validator stream."

Scroll the dashboard trace: the spans, the commitment ladder deltas, the region
fan-out, the Solscan links.

---

## 4. Close (15s)

> "It runs on mainnet today, it's open source, and everything you saw, the
> landings, the agent's reasoning, the race, is in the repo as raw logs you can
> verify. stx-alpha.vercel.app."

Show the GitHub link and the live URL.

---

## One-take cheat sheet (commands in order)

```
# terminal, env sourced (set -a; . ./.env.local; set +a)
stx tip-floor                 # show the live floor you're sizing against
stx race --timeout 25         # the race (or use the pre-recorded one on the dashboard)
stx fault-inject fee-starvation
stx fault-inject blockhash-expiry
stx leaders                   # optional: live leader schedule
```

Browser tabs: stx-alpha.vercel.app, the repo, a Solscan slot from the race.

## The 15-second cut (for sharing)

Just section 1. The flat tip ladder next to the climbing one, "naive never
landed, stx landed, same floor," click the slot. That clip is the whole pitch.
