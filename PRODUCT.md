# Product

## Register

product

## Users

Solana infrastructure developers (people who run trading, MEV, liquidation, or
indexing bots) who need to understand whether and why a transaction landed, and
the bounty judges who need to verify the stack ran on real infrastructure. Their
context: watching live bundle submissions, debugging a failure after the fact,
or reviewing the AI agent's retry decisions. They are technical, but not all of
them live inside Solana's transaction internals, so the interface has to teach
as it shows.

## Product Purpose

Make a Solana transaction's full lifecycle legible. Each transaction or bundle
is treated as a trace: stages (submitted, processed, confirmed, finalized) timed
from validator ground truth, rendered as a calm timeline and a span waterfall,
alongside the dynamic tip behavior, the failure classification, and the AI
agent's reasoning. Everything shown comes from real on-chain runs with
explorer-verifiable slots and signatures. Success is a viewer, even one who is
not deep in Solana, understanding exactly what happened to a transaction and why.

## Brand Personality

Calm, precise, trustworthy. The model is a Stripe payment timeline: an async,
failure-prone, intimidating-sounding process made unhurried and clear. The voice
states what happened in plain terms and shows the evidence. Three words: calm,
precise, legible.

## Anti-references

- The Grafana / Prometheus dashboard wall: dense panels of charts with no
  narrative, everything competing for attention.
- Generic SaaS dashboards: cream or sand backgrounds, uniform rounded cards in a
  grid, pastel gradients, an eyebrow kicker over every section.
- Crypto-neon dark mode: glow, gradient text, purple-on-black, hype.
- Anything that reads as AI-generated: identical card grids, gradient-clipped
  headings, decorative glassmorphism.

## Design Principles

- The transaction is the protagonist. One trace, told as a story, not a soup of
  metrics. The waterfall and timeline lead; aggregates support.
- Ground truth over decoration. Every number is real, sourced from the stream or
  RPC, and traceable to an explorer. No placeholder data, ever.
- Calm under load. An async, retry-heavy, failure-prone process should read as
  unhurried and orderly, not alarming.
- Show the reasoning. The AI agent's decisions are visible and auditable
  (observed, reasoned, decided, outcome), never a black box.
- Legible to an outsider. A backend engineer with no Solana background can follow
  what happened and why.

## Accessibility & Inclusion

WCAG AA contrast for all text and meaningful UI. Status is never conveyed by
color alone: every state carries an icon and a label, and the status palette is
chosen to remain distinguishable for the common color-vision deficiencies.
Reduced motion is honored (the live updates and reveals degrade to instant or
crossfade). Fully keyboard navigable.
