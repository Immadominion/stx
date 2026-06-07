# Design

Visual system for the stx Transaction Control Tower. Register: product. Theme:
calm editorial light, in the spirit of a Stripe payment timeline. The mood lives
in the accent and the typography; the surface stays pure white.

## Theme

Physical scene: a developer at a desk in daylight, watching a transaction settle
the way you watch a Stripe payment move from processing to succeeded. Unhurried,
legible, trustworthy. Light, not dark: the lifecycle should feel like reading a
receipt, not staring into an ops console.

Color strategy: Restrained. Pure-white content surface, one olive-lime brand
accent used sparingly, and a separate semantic palette that carries the
transaction lifecycle (the real subject of the page).

## Color (OKLCH)

Surfaces and ink:

```
--bg:            oklch(1 0 0);            /* pure white, content */
--surface:       oklch(0.985 0.001 255);  /* panels, rails, faint cool */
--surface-sunk:  oklch(0.972 0.002 255);  /* wells, code, mono blocks */
--border:        oklch(0.922 0.003 255);
--border-strong: oklch(0.860 0.004 255);
--ink:           oklch(0.23 0.012 260);   /* primary text, ~14:1 on white */
--ink-2:         oklch(0.46 0.010 260);   /* secondary text, AA on white */
--ink-3:         oklch(0.58 0.008 260);   /* captions / large text only */
```

Brand (olive-lime, hue ~118; identity + interaction only, never status):

```
--brand:      oklch(0.74 0.15 118);   /* fills, the live pulse */
--brand-ink:  oklch(0.43 0.13 118);   /* brand-colored text on white, AA */
--brand-wash: oklch(0.965 0.03 118);  /* selected rows, brand tint */
```

Lifecycle status (each ALWAYS paired with an icon + label, never color alone;
chosen for separation under deuteranopia/protanopia). A cool-to-settled
progression for commitment, warm for in-flight, red for failure:

```
--st-submitted: oklch(0.62 0.012 260);  /* neutral gray: built/dispatched */
--st-pending:   oklch(0.72 0.15 70);    /* amber: inflight, awaiting */
--st-landed:    oklch(0.64 0.14 150);   /* green: on-chain */
--st-confirmed: oklch(0.55 0.14 152);   /* deeper green: supermajority */
--st-finalized: oklch(0.52 0.11 215);   /* teal-blue: rooted, settled */
--st-failed:    oklch(0.56 0.20 25);    /* red: failed / aborted */
```

For status text on white, use the same hue at L 0.45-0.50 to hold AA. For filled
badges, use the token as background with white text (verify >= 4.5:1) or a soft
wash (token at L 0.96) with the token-ink as text.

## Typography

Two families, by contrast axis (sans UI + mono data), never more.

- UI: Inter (next/font), weights 400 / 500 / 600. Headings 600, body 400, labels
  and buttons 500.
- Data: Geist Mono (next/font), for slots, signatures, lamports, durations,
  bundle ids. Tabular figures. This is what makes the numbers feel trustworthy.

Fixed rem scale, ratio ~1.2 (product, not fluid):

```
12px .75   13px .8125   14px .875   16px 1   18px 1.125
22px 1.375   28px 1.75   34px 2.125
```

Body 14-16px, ink-2 for secondary. Hero numbers (a slot, a duration) may go to
28-34px in mono. Line length 65-75ch for prose; data can run denser.

## Components

- **Status badge**: icon + label + color dot/fill. The single most-used element;
  one definition, used everywhere. States: submitted, pending, landed, confirmed,
  finalized, failed.
- **Lifecycle timeline** (the hero, Stripe-style): a vertical sequence
  submitted -> processed -> confirmed -> finalized, each node with its timestamp,
  slot, and the delta to the previous stage. Failed/retry branches shown inline.
- **Span waterfall**: horizontal bars per span (tip.decide, dispatch,
  auction.wait, processed, confirmed, finalized), width proportional to duration,
  labeled in mono. The legible, web2-trace view.
- **Tip-vs-attempt**: a small step chart showing the tip escalating across
  attempts against the floor percentiles, with the landing marked.
- **Decision record**: the AI agent's reasoning as observed -> reasoned ->
  decided -> outcome. Reads as a calm explanation, not a log dump.
- **Funnel**: stage drop-off (submitted -> landed -> confirmed -> finalized) as
  proportional bars.
- **Explorer link**: every signature and slot links to Solscan; mono, with an
  external-link affordance.

Each interactive component ships default / hover / focus / active / disabled /
loading (skeleton, not spinner) / empty (teaches the view).

## Layout

App shell: a slim left rail (stx mark + nav: Traces, Funnel, Agent) and a content
column. The default view is a list of recent traces; selecting one opens the
trace-as-story detail (timeline + waterfall + decisions). Responsive is
structural: the rail collapses to a top bar under ~860px; the waterfall scrolls
horizontally on mobile rather than reflowing.

## Motion

150-250ms, ease-out (quart/expo). Motion conveys state only: a stage advancing,
a new trace arriving, the live pulse on a pending state, a subtle number roll on
changing values. No page-load choreography. Every animation has a
`prefers-reduced-motion` path (crossfade or instant).
