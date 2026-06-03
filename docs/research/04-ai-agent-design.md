# Autonomous Reasoning Agent for the TX Stack - Research Dossier

> Compiled 2026-06-02. The bounty disqualifies "a simple wrapper that calls functions sequentially without reasoning" and requires "real decisions" + "visible reasoning." This dossier designs an agent that satisfies that, grounded in the bundled `claude-api` skill and Anthropic's agent guidance.

## Bottom line
Give the agent **Failure Reasoning, expressed as an Autonomous Retry loop (observe→reason→decide→act, not hardcoded), with Tip Intelligence as the lever it pulls.** Hardest to fake, latency-safe by construction (runs *after* a failure, off the 400 ms hot path), and produces a rich visible-reasoning trace. **Do NOT give the agent per-slot submission timing** - LLM latency (~1-3 s) ≫ slot time (400 ms) makes it architecturally indefensible.

## 1. Agentic loop with the Claude API

**Workflow vs agent:** a *workflow* calls functions in a fixed order (code-orchestrated) - this is literally the bounty's disqualifier. An *agent* lets the **model decide which tool to call next** based on what it observed (model-orchestrated). Build an agent.

**Manual tool-use loop** on `POST /v1/messages` (manual, not the SDK runner, so you can log/guard each step):
```python
messages = [{"role": "user", "content": observation_payload}]
while True:
    resp = client.messages.create(
        model="claude-opus-4-8", max_tokens=16000,
        thinking={"type": "adaptive", "display": "summarized"},  # visible reasoning
        output_config={"effort": "high"},
        system=POLICY_SYSTEM_PROMPT,   # frozen → cache-friendly
        tools=DECISION_TOOLS, messages=messages)
    log_turn(resp)                      # persist thinking + tool calls
    if resp.stop_reason == "end_turn": break
    messages.append({"role": "assistant", "content": resp.content})
    tool_results = [{"type":"tool_result","tool_use_id":b.id,"content":dispatch(b.name,b.input)}
                    for b in resp.content if b.type == "tool_use"]
    messages.append({"role": "user", "content": tool_results})
```

**Structured/forced decision:** the final decision is a **strict tool call** (`"strict": true`, `additionalProperties:false`), forced via `tool_choice:{"type":"tool","name":"commit_decision"}` once enough evidence is gathered (`auto` during observation).

**Visible reasoning - two channels:**
- (a) **Adaptive thinking** - `thinking={"type":"adaptive","display":"summarized"}`. ⚠ On Opus 4.8/4.7 thinking is **omitted by default** (`display:"omitted"`); you MUST set `"summarized"` or your audit log captures empty blocks. `block.thinking` carries the text; preserve `signature` verbatim if feeding thinking back.
- (b) **Required `reasoning` fields in the decision schema** - durable, structured justification:
```python
COMMIT_DECISION = {
  "name": "commit_decision", "strict": True,
  "input_schema": {"type":"object","properties":{
    "action": {"type":"string","enum":["resubmit","resubmit_new_blockhash","raise_tip","abort","hold"]},
    "tip_lamports": {"type":"integer"},
    "hypotheses": {"type":"array","items":{"type":"string"}},
    "chosen_cause": {"type":"string"},
    "justification": {"type":"string"},   # must cite specific observed values
    "confidence": {"type":"number"},
    "expected_effect": {"type":"string"}},
   "required":["action","tip_lamports","hypotheses","chosen_cause","justification","confidence","expected_effect"],
   "additionalProperties": False}}
```
⚠ Strict schemas do **not** enforce numeric ranges (`minimum`/`maximum`) or string length - validate/clamp `confidence∈[0,1]` and `tip_lamports∈[min,ceiling]` in the deterministic layer.

**Determinism for infra:** Opus 4.8 has **no `temperature`/`top_p`/`top_k`** (sending them = 400). Steer via strict schemas + tight prompt + `effort` (`low`/`medium` routine, `high` for hard post-mortems) + deterministic guardrails. **Prompt caching:** render order is `tools → system → messages`; put immutable policy + tool defs first, volatile observations last; verify via `usage.cache_read_input_tokens`; 4096-token minimum cacheable prefix on Opus 4.8.

## 2. The four candidate decisions - reasoning vs formula

**2.1 Failure Reasoning** - *genuinely reasoning; most defensible.* Same surface error → different root causes → different remedies: a failed swap could be (a) slippage too tight, (b) CU limit too low (ran out before the swap - a *compute* problem wearing a slippage code), (c) priority fee too low (price moved), (d) blockhash expired in transit (refresh, change nothing else), (e) genuinely adverse market (abort - retrying wastes money). Disambiguating requires *combining* error code + CU-consumed-vs-requested + fee-vs-percentile + time-since-submit; remedies differ and are sometimes counterintuitive. A lookup table `0x1771 → widen slippage` would be wrong most of the time it matters. Tools: `get_failure_context`, `get_network_conditions`, `simulate_with_params` (test a hypothesis before committing - the most impressive tool), `get_retry_history`, `commit_decision`.

**2.2 Tip Intelligence** - *real iff framed as multi-factor EV optimization*, formulaic if "tip = 75th percentile." Reason over value-at-risk × urgency × current tip distribution × recent landing elasticity. Strong as the **lever** inside the retry loop; weaker standalone (judges can smell a percentile lookup).

**2.3 Submission Timing** - *do NOT give the agent per-slot timing.* 400 ms slots, ~800 ms usable window, 100-150 ms delays lose fills, LLM latency ~1-3 s → guaranteed to miss the window it's reasoning about. The salvageable version is **timing policy** set ahead of the hot path (e.g. "hold if next 2 leaders are non-Jito and tx isn't urgent"), executed deterministically per-slot - but it overlaps heavily with the other decisions.

**2.4 Autonomous Retry with Fault Injection** - *this is the showcase.* Not a fifth decision; it's the loop that makes Failure Reasoning + Tip Intelligence autonomous, and it's the bounty's own acceptance test ("no hardcoded retry flow"). The fault harness (deliberately stale the blockhash / inject low fee / inject compute exhaustion) lets you **prove** the agent diagnoses rather than scripts: it must detect *which* failure, then choose a **cause-appropriate** remedy (refresh-blockhash-keep-tip vs keep-blockhash-raise-tip vs abort) - avoiding the reflex "failed → bump everything → retry."

**Ranking, hardest-to-fake first:** Failure Reasoning / Autonomous Retry → Tip Intelligence → Timing-as-policy → (per-slot timing: disqualifyingly fake; don't build).

## 3. Clean separation: AI (policy) vs core (mechanism)

```
DETERMINISTIC CORE (works with AI disabled)
  • slot/leader stream • bundle builder • submitter • lifecycle tracker
  • static fallback policy (tip = floor percentile, retry ≤ N w/ blockhash refresh)
        at a decision point ──► AgentClient ──► validated DecisionRecord
        GUARDRAIL VALIDATOR (clamps/rejects, NEVER trusts the LLM)
                    │ async, off hot path
                    ▼  AI BRAIN (Claude): observe→reason→decide, read tools + commit
```
**Fully-functional-without-AI** is both the litmus test for clean separation and a production safety property: if the agent is down/slow/garbage, the core falls back to its static default and keeps landing txs. The AI is a *decision-quality enhancement*, never a liveness dependency.

**Guardrails (deterministic validator on the agent's output):**
- Clamp `tip_lamports` to `[1000, policy_ceiling]` (ceiling = f(value-at-risk)). Log every clamp/override.
- Enforce `max_retries` (agent can *choose* to retry; mechanism caps total).
- Validate `confidence∈[0,1]`; route low-confidence to the conservative default (or human gate).
- Verify preconditions: honor `resubmit_new_blockhash` only if the blockhash **actually** expired (validator re-checks slot age vs 150 - never trusts the LLM's claim) → prevents the double-submit hazard.
- **Bounding a decision doesn't make it not-a-decision:** the agent still chooses *which* action and *what* tip within a meaningful envelope (50th vs 75th percentile is a real landing-probability bet).

**The agent's tools are read-only observation + one commit.** None mutate chain state; `simulate_with_params` is read-only; `refresh_blockhash` returns a hash to the *record*, the *core* decides to use it. Clean trust boundary: LLM proposes, mechanism disposes.

## 4. Visible & auditable reasoning - the DecisionRecord
One record per decision, replayable and human-readable - the single most important artifact for "reasoning is visible":
```jsonc
{ "decision_id","logical_tx_id","attempt","ts","trigger",
  "observations": { "error_code","cu_requested","cu_consumed","tip_paid_lamports",
                    "blockhash_age_slots","tip_floor":{...},"current_slot","leader_is_jito","value_at_risk_usd" },
  "thinking_summary": "...",            // from thinking blocks (display:summarized)
  "hypotheses": [...], "tool_calls": [ {"tool","input","result","ts"} ],
  "chosen_cause","action","params":{...},"justification","confidence","expected_effect",
  "guardrail": { "tip_clamped","action_allowed","overrides":[],"fell_back_to_default" },
  "outcome": { "landed","slot","signature" },   // backfilled after execution
  "model":"claude-opus-4-8","request_id","usage":{...} }
```
Patterns: capture all five layers (observed → reasoned → decided → governed → outcome); tie each record to `response._request_id`; **backfill outcome** to compute agent-vs-default hit rate (strongest answer to "is it actually good?"); **faithfulness caveat** - stated reasoning isn't guaranteed causal, so (a) require justification to cite observed values (checkable), and (b) use `simulate_with_params` as a grounding step (hypothesis validated against a real sim before commit). Render as a one-line-per-decision table + drill-down.

## 5. Avoiding the "sequential wrapper" trap
**Disqualifying:** if/else dressed as AI; single prompt that outputs a number with no observation loop; fixed tool sequence in lockstep; LLM on the 400 ms hot path.
**Qualifying:** multi-step observe→reason→decide→act with **model-chosen** tools (different failures → different tool sequences - that variability *is* the proof); hypothesis testing in-loop (`simulate_with_params` to discriminate); checkable justification citing observed values; **fault injection the agent diagnoses differently each run** (blockhash expiry / fee starvation / compute exhaustion → three different cause-appropriate remedies, not three blanket retries) - this is the demo that wins.

## 6. Cost/latency reality - the key architectural insight
**LLM ~1-3 s ≫ slot 400 ms.** Therefore the agent owns only latency-tolerant decisions: **failure post-mortem** (tx already failed, no clock racing), **retry policy** (a forced wait already exists - you must let the blockhash expire before re-signing), **tip/timing *policy* set ahead of the hot path** (decide strategy for next N slots, cache it, deterministic layer applies per-slot at µs speed). Architect around it: decide policy ahead / execute deterministically in the hot path; cache decisions for recurring contexts + prompt caching; never `await` the LLM inline during a live race; tier the model by urgency. This is the clean rebuttal to "too slow to be real" - the agent is placed where latency is a non-issue *by design*.

## 7. Recommendation
Agent owns: *"a tx failed (or a fault was injected) - diagnose the root cause, choose a cause-appropriate remedy (refresh blockhash / raise CU / adjust tip / widen slippage / abort / hold), justify it; the deterministic core executes the bounded result and reports the outcome back."* Model `claude-opus-4-8`, adaptive thinking summarized, `effort:high` for post-mortems, manual loop, read-only tools + strict `commit_decision`, guardrail validator, DecisionRecord persisted + outcome backfilled. **Demo:** run the fault harness 3× with a different injected failure each time; show different evidence gathered, hypotheses discriminated (incl. a `simulate_with_params` call), and 3 different cause-appropriate remedies; then disable the AI to show the core still runs on its static fallback (clean separation) with a measurably worse landing rate / higher cost (the agent adds real value).

**Sources:** Anthropic *Building Effective Agents* (workflow vs agent; observe→act→reflect). Bundled `claude-api` skill: tool-use.md (manual loop, tool_choice, strict tool use), shared/tool-use-concepts.md (structured-output JSON-schema limits), SKILL.md (thinking/effort), shared/model-migration.md (Opus 4.7/4.8 - thinking omitted by default, no temperature), shared/prompt-caching.md, python/claude-api/README.md (`response._request_id`). Solana: Jito tip_floor, blockhash expiry/retry (Solana retry guide, Helius), slot-timing (~800 ms window). Failure taxonomy: "Why Does My Transaction Fail" (0x1771 slippage dominance; CU-vs-fee disambiguation).
