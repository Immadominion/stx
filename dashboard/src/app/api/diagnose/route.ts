// Live transaction autopsy. A faithful TypeScript port of the Rust engine's
// `diagnose` (crates/stx-cli/src/diagnose.rs): same RPC calls, same failure
// taxonomy, same AI prompt. The Rust `stx-server` exposes the identical API for
// self-hosting; this serverless route powers the live dashboard. Read-only.

import { NextRequest, NextResponse } from "next/server";

export const runtime = "nodejs";
export const maxDuration = 60;

const RPC = process.env.HELIUS_RPC_ENDPOINT || process.env.RPC_URL || "";
const ANTHROPIC_KEY = process.env.ANTHROPIC_API_KEY || "";

const AUTOPSY_SYSTEM =
  "You are a senior Solana infrastructure engineer acting as a transaction diagnostician. " +
  "Given the facts about one transaction, explain in 2 to 4 plain sentences what happened and, " +
  "if it failed or never landed, exactly what the developer should change to make it land next time " +
  "(tip size, blockhash freshness, compute limit, targeting a Jito leader). Be concrete. Interpret " +
  "the facts, do not just restate them. No preamble and no markdown headers.";

async function rpc(method: string, params: unknown): Promise<unknown> {
  const r = await fetch(RPC, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const j = await r.json();
  return j?.result;
}

function classify(
  errStr: string | null,
  _cu: number | null,
): { kind: string; evidence: string; confidence: number } | null {
  if (!errStr) return null;
  const e = errStr.toLowerCase();
  if (e.includes("computationalbudgetexceeded") || e.includes("exceeded cus"))
    return { kind: "ComputeExceeded", evidence: "error indicates compute budget exceeded", confidence: 0.95 };
  if (e.includes("blockhashnotfound") || e.includes("blockheightexceeded") || e.includes("transactionexpired"))
    return { kind: "ExpiredBlockhash", evidence: "error indicates blockhash not found / blockheight exceeded", confidence: 0.95 };
  if (e.includes("alreadyprocessed"))
    return { kind: "AlreadyProcessed", evidence: "transaction signature was already processed", confidence: 0.99 };
  if (e.includes("custom") || e.includes("0x1771") || e.includes("slippage"))
    return { kind: "AdverseMarket", evidence: "program returned a custom error (e.g. slippage); likely adverse market", confidence: 0.6 };
  return { kind: "BundleFailed", evidence: "transaction failed on-chain", confidence: 0.5 };
}

export async function GET(req: NextRequest) {
  const sig = (req.nextUrl.searchParams.get("sig") || "").trim();
  if (sig.length < 80 || sig.length > 100 || !/^[a-zA-Z0-9]+$/.test(sig)) {
    return NextResponse.json({ error: "Enter a valid Solana transaction signature." }, { status: 400 });
  }
  if (!RPC) return NextResponse.json({ error: "server not configured" }, { status: 500 });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const d: any = {
    signature: sig, found: false, succeeded: false, slot: null, commitment: null,
    leader: null, fee_lamports: null, compute_units: null, error: null,
    classification: null, log_tail: [], facts: [], headline: "", explanation: null,
  };

  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const statuses = (await rpc("getSignatureStatuses", [[sig], { searchTransactionHistory: true }])) as any;
    const st = statuses?.value?.[0] ?? null;
    if (st) {
      d.found = true;
      d.slot = st.slot;
      d.commitment = st.confirmationStatus ?? null;
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const tx = (await rpc("getTransaction", [sig, { encoding: "json", maxSupportedTransactionVersion: 0, commitment: "confirmed" }])) as any;
    if (tx) {
      d.found = true;
      d.slot = tx.slot;
      const meta = tx.meta;
      if (meta) {
        d.fee_lamports = meta.fee ?? null;
        d.compute_units = meta.computeUnitsConsumed ?? null;
        if (Array.isArray(meta.logMessages)) d.log_tail = meta.logMessages.slice(-4);
        if (meta.err) d.error = JSON.stringify(meta.err);
      }
      d.succeeded = !meta?.err;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const leaders = (await rpc("getSlotLeaders", [tx.slot, 1])) as any;
      d.leader = Array.isArray(leaders) ? (leaders[0] ?? null) : null;
    }
  } catch {
    return NextResponse.json({ error: "RPC lookup failed; try again." }, { status: 502 });
  }

  if (!d.found) {
    d.headline = "Never landed: dropped or expired before inclusion";
    d.facts.push("Not found on-chain (getSignatureStatuses with history search and getTransaction both empty).");
    d.facts.push("A signature that never appears on-chain was dropped before inclusion: the blockhash expired, the tip lost the auction, or the targeted leader was not running Jito.");
  } else if (d.succeeded) {
    d.headline = `Landed and succeeded at slot ${d.slot}${d.commitment ? ` (${d.commitment})` : ""}`;
    d.facts.push(`Landed at slot ${d.slot}.`);
    if (d.leader) d.facts.push(`Block leader: ${d.leader}.`);
    if (d.fee_lamports != null) d.facts.push(`Fee paid: ${d.fee_lamports} lamports.`);
    if (d.compute_units != null) d.facts.push(`Compute units consumed: ${d.compute_units}.`);
  } else {
    const c = classify(d.error, d.compute_units);
    d.classification = c ? { kind: c.kind, evidence: c.evidence, confidence: c.confidence } : null;
    d.headline = `Landed but FAILED: ${c?.kind ?? "Unknown"}`;
    d.facts.push(`Landed at slot ${d.slot} but execution failed.`);
    if (d.error) d.facts.push(`On-chain error: ${d.error}.`);
    if (d.compute_units != null) d.facts.push(`Compute units consumed: ${d.compute_units}.`);
    if (c) d.facts.push(`Classified as ${c.kind} (${c.evidence}), confidence ${Math.round(c.confidence * 100)}%.`);
  }

  if (ANTHROPIC_KEY) {
    try {
      const prompt = `Signature: ${sig}\nFacts:\n- ${d.facts.join("\n- ")}`;
      const r = await fetch("https://api.anthropic.com/v1/messages", {
        method: "POST",
        headers: { "x-api-key": ANTHROPIC_KEY, "anthropic-version": "2023-06-01", "content-type": "application/json" },
        body: JSON.stringify({ model: "claude-opus-4-8", max_tokens: 400, system: AUTOPSY_SYSTEM, messages: [{ role: "user", content: prompt }] }),
      });
      const j = await r.json();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const text = (j?.content as any[])?.find((b) => b?.type === "text")?.text?.trim();
      if (text) d.explanation = text;
    } catch {
      // explanation is best-effort
    }
  }

  return NextResponse.json(d);
}
