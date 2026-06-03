# Jito Bundles & Submission Infrastructure - Research Dossier

> Compiled 2026-06-02 from primary sources: `docs.jito.wtf/lowlatencytxnsend/`, `github.com/jito-labs/mev-protos` (canonical proto + JSON-RPC HTTP spec), `github.com/jito-labs/jito-ts`, `github.com/jito-labs/jito-rust-rpc`, live `bundles.jito.wtf/api/v1/bundles/tip_floor`, `docs.jito.wtf/lowlatencytxnfeed/` (ShredStream).
>
> Doc-ecosystem note: the **current** authoritative source is `docs.jito.wtf` (no-auth model). The older `jito-labs.gitbook.io/mev/...` searcher pages describe the deprecated auth-keypair era and conflict on several points (notably rate limits). Prefer `docs.jito.wtf`.

## 1. Block Engine endpoints & regional URLs

All endpoints are HTTPS on **port 443**. Base host pattern: `{region.}mainnet.block-engine.jito.wtf`.

| Region | URL |
|---|---|
| Global (auto-routes) | `https://mainnet.block-engine.jito.wtf` |
| Amsterdam | `https://amsterdam.mainnet.block-engine.jito.wtf` |
| Dublin | `https://dublin.mainnet.block-engine.jito.wtf` |
| Frankfurt | `https://frankfurt.mainnet.block-engine.jito.wtf` |
| London | `https://london.mainnet.block-engine.jito.wtf` |
| New York | `https://ny.mainnet.block-engine.jito.wtf` |
| Salt Lake City | `https://slc.mainnet.block-engine.jito.wtf` |
| Singapore | `https://singapore.mainnet.block-engine.jito.wtf` |
| Tokyo | `https://tokyo.mainnet.block-engine.jito.wtf` |

**Testnet:** `https://testnet.block-engine.jito.wtf`, `https://dallas.testnet.block-engine.jito.wtf`, `https://ny.testnet.block-engine.jito.wtf`.
**Devnet:** Jito does **not** run a devnet block engine - bundles only work on **mainnet** and **testnet**.

**API base paths** (HTTP JSON-RPC 2.0, `Content-Type: application/json`, POST):
- Bundles: `/api/v1/bundles` → `sendBundle`, `getBundleStatuses`, `getInflightBundleStatuses`, `getTipAccounts`
- Transactions: `/api/v1/transactions` → `sendTransaction`
- Example: `https://mainnet.block-engine.jito.wtf:443/api/v1/bundles`

**gRPC (jito-ts searcher API):** connects to the same `BLOCK_ENGINE_URL`. The gRPC `SearcherService` exposes `sendBundle`, `getTipAccounts`, `getConnectedLeaders`, `getNextScheduledLeader`, and the streaming `subscribeBundleResults`. Source: `jito-ts/src/sdk/block-engine/searcher.ts`.

## 2. sendBundle - format, limits, semantics

- **Method:** `sendBundle`, params = `[ [tx1, tx2, ...], {"encoding": "base64"} ]`
  - `params[0]`: array of fully-signed transactions as **base64 (recommended)** or **base58 (slow, DEPRECATED)** strings.
  - `params[1].encoding`: optional, `"base64"` | `"base58"`. **Default = `base58`** - always set `base64` explicitly.
- **Max transactions per bundle: 5.** (TS SDK `Bundle.transactionLimit`; `addTransactions` errors if exceeded.)
- **Response:** `bundle_id` = SHA-256 hash of the bundle's tx signatures (string).
- **Atomicity:** all-or-nothing. "If any transaction in a bundle fails, none of the transactions in the bundle will be committed." All txs land in the **same slot/block**.
- **Sequential execution:** "Transactions in a bundle are guaranteed to execute in the order they are listed."
- **Validity / tip requirement:** at least one tx must transfer SOL to one of the 8 tip accounts. The tip can be any instruction (top-level or CPI). **Minimum tip: 1000 lamports.**
- **TS SDK helper:** `Bundle.addTipTx(keypair, tipLamports, tipAccount, recentBlockhash)`.

## 3. Tip accounts

- **Method:** `getTipAccounts` - no params; returns 8 tip account pubkeys. Jito recommends fetching at runtime rather than hardcoding.
- **The 8 tip accounts (verify at runtime):**
  1. `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5`
  2. `HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe`
  3. `Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY`
  4. `ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49`
  5. `DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh`
  6. `ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt`
  7. `DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL`
  8. `3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT`
- **Rotation:** select a tip account at random per bundle to reduce write-lock contention.
- **Placement:** Jito best practice is to include the tip instruction **in the same transaction as your main logic** (with post-tx balance checks), not as a standalone tip tx - standalone tips increase exposure to "uncle bandit" situations. Rust SDK provides `get_random_tip_account()`.

## 4. Tip floor / tip stream API (dynamic tip calculation - NO hardcoded values)

- **REST:** `https://bundles.jito.wtf/api/v1/bundles/tip_floor` → JSON **array** with a single object.
- **WebSocket:** `wss://bundles.jito.wtf/api/v1/bundles/tip_stream`.
- **Live response (fetched 2026-06-02T17:08:35Z):**
  ```json
  [{"time":"2026-06-02T17:08:35+00:00",
    "landed_tips_25th_percentile":0.0000123,
    "landed_tips_50th_percentile":0.00003,
    "landed_tips_75th_percentile":0.0000918625,
    "landed_tips_95th_percentile":0.0005490940000000001,
    "landed_tips_99th_percentile":0.0040696285,
    "ema_landed_tips_50th_percentile":0.000022696327264285852}]
  ```
- **Fields:** `time` (ISO 8601), `landed_tips_{25,50,75,95,99}th_percentile`, `ema_landed_tips_50th_percentile`.
- **UNITS: SOL** (decimal), not lamports. e.g. `0.00003` SOL = 30,000 lamports for the 50th percentile. **`sendBundle` takes the tip in lamports**, so convert: `tip_lamports = ceil(percentile_value * 1e9)`.
- **Landing-probability relationship:** tips are a competitive auction inside the block engine; higher tip → higher bundle-auction priority → higher landing probability. Common pattern: target 50th-75th percentile under normal load, 95th/99th during contention; use `ema_landed_tips_50th_percentile` as a smoothed baseline.

## 5. Bundle status tracking

**`getBundleStatuses`** - on-chain confirmation (like `getSignatureStatuses`):
- Params: `[ [bundle_id, ...] ]`, **max 5**.
- Per bundle: `bundle_id`, `transactions` (base-58 sigs), `slot` (u64), `confirmationStatus` (`processed`|`confirmed`|`finalized`), `err`. Returns context `slot` + `value` array; **`null` if not found / not landed.**

**`getInflightBundleStatuses`** - real-time, **5-minute look-back**:
- Params: `[ [bundle_id, ...] ]`, **max 5**.
- Per bundle: `bundle_id`, `status`, `landed_slot` (u64|null).
- **Status values:** `Invalid` (not in system / 5-min lookback) · `Pending` (not failed/landed/invalid) · `Failed` (all regions marked failed, not forwarded) · `Landed` (on-chain).

**Confirmation pattern:** poll `getInflightBundleStatuses` for fast triage (`Landed`/`Failed`/`Invalid`), then confirm finality with `getBundleStatuses` until `confirmationStatus == confirmed|finalized`. Bundles land within ~1 slot (~400 ms) when the targeted leader is Jito-Solana.

**gRPC streaming alternative - `subscribeBundleResults`** (TS: `searcherClient.onBundleResult(cb, errCb)`). The `BundleResult` proto carries `bundle_id` + a `oneof result` - the **richest** failure signal:
- `Accepted { slot, validator_identity }`
- `Rejected`: `StateAuctionBidRejected {auction_id, simulated_bid_lamports, msg?}` · `WinningBatchBidRejected {...}` · `SimulationFailure {tx_signature, msg?}` · `InternalError {msg}` · `DroppedBundle {msg}`
- `Processed { validator_identity, slot, bundle_index }`
- `Finalized {}`
- `Dropped { reason: DroppedReason }` where `DroppedReason ∈ {BlockhashExpired, PartiallyProcessed, NotFinalized}`

## 6. Leader schedule & timing

- **`getNextScheduledLeader`** (gRPC): "next scheduled leader connected to the block-engine." TS request `{ regions: [] }`. Response: `currentSlot`, `nextLeaderSlot`, `nextLeaderIdentity`. (Underlying proto also carries region, but the TS wrapper drops it.)
- **`getConnectedLeaders`** (gRPC): `{ [validatorIdentity]: SlotList }` - slots where Jito-connected leaders are scheduled.
- **Leader window:** each leader gets **4 consecutive slots** before rotating (Solana protocol).
- **Slot duration:** ~400 ms target.
- **Timing strategy:** compute `slotsUntilLeader = nextLeaderSlot - currentSlot`; submit **just before / during** the connected Jito leader's 4-slot window. Optimal Block-Engine↔validator round-trip is **<50 ms** for reliable in-slot inclusion.

## 7. Rate limits, auth, regions

- **Auth: NOT required for default sends.** "You no longer need an approved auth key for default sends." Optional UUID auth via `x-jito-auth` header or `?uuid=<uuid>` for higher rate-limit tiers.
- **Rate limit (current docs.jito.wtf): 1 request/second/IP/region**, applied **separately per regional endpoint**. Exceeding → **HTTP 429**.
  - ⚠ CONFLICT: legacy GitBook says 5 req/s. Treat **1 req/s per IP per region** as the current default.
- **Multi-region pattern:** because the limit is per-IP-per-region and latency depends on proximity to the active leader, **submit the same bundle to all regional endpoints concurrently** (each region's budget is independent). Send to global + nearest 2-3 regions, or fan out to all.
- **`sendTransaction` extras (non-bundle):** default provides MEV/front-running protection. `?bundleOnly=true` enables revert protection. On success response carries `x-bundle-id` header. Jito recommends a **70/30 priority-fee/tip split** for `sendTransaction` - but for **`sendBundle`, only the Jito tip matters**.

## 8. Bundle-specific failure modes & detection

| Failure | Cause | Detection |
|---|---|---|
| Leader not running Jito | Active leader is non-Jito (no block-engine link) | `getInflightBundleStatuses` stays `Pending` → ages to `Invalid` after 5 min; never `Landed`. Pre-check via `getConnectedLeaders`/`getNextScheduledLeader`. |
| Leader skipped slot | Scheduled leader offline/skipped | Same - never lands; resubmit targeting next Jito leader. |
| Tip too low | Lost the auction | gRPC `Rejected.WinningBatchBidRejected`/`StateAuctionBidRejected` (`simulated_bid_lamports`). HTTP: `Pending`→`Invalid`. Raise tip. |
| Below min tip | Tip < 1000 lamports | Rejected as invalid at submission. |
| Bundle expired | Blockhash expired pre-inclusion | gRPC `Dropped { BlockhashExpired }`. Refresh blockhash, resubmit. |
| Simulation failure | A tx fails simulation | gRPC `Rejected.SimulationFailure { tx_signature, msg }`. |
| Partial / not finalized | Race / fork | `Dropped { PartiallyProcessed }` / `Dropped { NotFinalized }`. |
| Atomic revert | Any tx fails on execution | nothing commits; `getBundleStatuses` → `null`. |

Use gRPC `subscribeBundleResults` for the precise reason; HTTP `getInflightBundleStatuses`/`getBundleStatuses` for the simpler `Pending/Failed/Landed/Invalid` + `confirmationStatus` view.

## 9. ShredStream (brief)

Delivers the **lowest-latency shreds** (block/tx fragments before full assembly) from Jito-connected leaders worldwide via `shredstream-proxy`. Saves **hundreds of ms** vs standard RPC block updates. A **read/observability** path (not submission) - complementary, optional for a lifecycle-tracking stack; access typically requires registering a key.

## Recency / conflict flags
1. **Rate limit:** current = 1 req/s/IP/region; legacy = 5. Use 1.
2. **Auth model changed:** auth keypair no longer required for default sends (older tutorials still assume `AUTH_KEYPAIR_PATH`).
3. **Encoding default** is still base58 while labeled deprecated - always pass `base64`.
4. **`getNextScheduledLeader`** TS wrapper drops region info.
5. **`jito-rust-rpc`** methods: `send_bundle`, `send_txn`, `get_bundle_statuses`, `get_in_flight_bundle_statuses`, `get_tip_accounts`, `get_random_tip_account`; configurable `base_url`. Less actively maintained than `jito-ts`.

**Key URLs:** docs.jito.wtf/lowlatencytxnsend · docs.jito.wtf/lowlatencytxnfeed · github.com/jito-labs/mev-protos (json_rpc/http.md, bundle.proto) · github.com/jito-labs/jito-ts · github.com/jito-labs/jito-rust-rpc · bundles.jito.wtf/api/v1/bundles/tip_floor
