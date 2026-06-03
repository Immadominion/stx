# Solana TX Lifecycle, Commitment, Blockhash & Failure Classification - Research Dossier

> Compiled 2026-06-02 from solana.com/docs, docs.anza.xyz, docs.rs (solana-sdk), and reputable engineering blogs (Helius, QuickNode, Chainstack). The three README-question answers (§3) are the rigorous, defensible versions.

## 1. Transaction journey through the network

Solana has **no traditional mempool**. Clients/RPC nodes forward transactions directly to the scheduled **leader** (Gulf Stream), because the leader schedule is known in advance.

**Clock primitives:** slot = **400 ms**; each leader gets **4 consecutive slots (1.6 s)** before rotation; the **leader schedule is computed once per epoch (~2 days)**, stake-weighted; **Proof of History** is a VDF "clock" giving cryptographic event ordering.

**Gulf Stream (client → leader):** forwards txs to the current/next leader(s) ahead of time (mirror image of Turbine).

**TPU (leader-side ingestion):** **Fetch → SigVerify → Banking → PoH → Broadcast.**
- *Fetch:* inbound packets over **QUIC** (replaced raw UDP), batched.
- *SigVerify:* batch Ed25519 verify + dedup.
- *Banking:* schedules + **executes** txs against the "bank" via Sealevel parallel execution; txs locking the same writable accounts can't run in parallel.
- *PoH:* hashes executed entries into the PoH chain.
- *Broadcast:* serializes entries into shreds, streams to cluster.

**Shreds & propagation:** entries split into **shreds (≤1280 bytes)**, grouped into FEC batches (**32 data + 32 coding = 64**). **Turbine** propagates shreds through a stake-weighted tree; non-leaders ingest via **TVU**. As validators replay & **vote**, the tx moves **processed → confirmed → finalized**.

## 2. Commitment levels - exact definitions

| Level | Definition | Reversible? |
|---|---|---|
| **processed** | Processed by a leader, included in the most recent block the node knows; may be on a **minority fork**, may lack votes, can be dropped. | Yes, easily |
| **confirmed** | Block voted on by a **supermajority (≥⅔ / >66% of stake)** - "optimistic confirmation." | Only under slashing |
| **finalized** | ≥⅔ stake **plus ≥31 further confirmed blocks built on top** (max lockout, rooted). | No (practically) |

**Optimistic confirmation (Anza, verbatim):** a block reaching optimistic confirmation "will not be reverted unless at least one validator is slashed."
**Rooting:** Tower BFT lockouts double (1,2,4,…, capped at 32 votes). 32 slots ≈ **12.8 s**.
**Typical wall-clock (Helius measured):** processed→confirmed **~0.5-1 s**; confirmed→finalized **~10-20 s** (~13 s typical), driven by ~12.8 s / 32-slot rooting.
**Usage:** `confirmed` = recommended production default; `finalized` for high-value/irreversible flows; `processed` = UI optimism/testing.

## 3. The three README questions - rigorous answers

### (a) Delta between `processed_at` and `confirmed_at` → what it says about network health
Measures **how long the cluster took to reach optimistic/supermajority (≥⅔ stake) confirmation** on the block containing your tx - i.e. the latency of the voting/consensus round-trip, not execution time. Execution (processed) is instant when the leader includes the tx; confirmation requires a supermajority to replay the block and land votes including that slot. So the delta is a direct probe of **vote propagation + consensus speed at submission time**.
- **Small, stable (~0.5-1 s, one-two slots):** healthy cluster - votes flowing fast, little fork contention, validators on schedule.
- **Large/growing:** cluster struggling to reach consensus. Inflators: **fork contention** (your block landed on a fork not yet holding supermajority), **high skip rate** (leaders missing slots delays the vote chain), **vote/replay lag & congestion** (heavy CU load, bandwidth saturation → late votes), **network turbulence** (Turbine packet loss, restarts).
- In short: a real-time **consensus-health gauge** - small/steady = fast-voting healthy cluster; large/rising = voting lag, fork contention, elevated skip rate, or congestion.

### (b) Why never use `finalized` commitment when fetching a blockhash for a time-sensitive tx
A `recentBlockhash` is valid only while it's within the **last ~150 blocks** (leader checks the 151 most recent). `getLatestBlockhash` returns `lastValidBlockHeight = currentBlockHeight + 150`; the window is **~150 blocks ≈ 60-90 s** (Solana docs cite "~1 min 19 s").
A **`finalized` blockhash is already ~32 slots / ~12-13 s old** by definition. And `getLatestBlockhash`'s **default commitment is `finalized`** - a trap. Starting from a finalized blockhash **burns ~31-32 of your ~150 blocks (~20%+, ~13 s) before you even sign**, drastically shrinking usable lifetime and raising the odds of expiry → `BlockhashNotFound` (leader/RPC no longer has it) or `TransactionExpiredBlockheightExceededError` (never landed before `lastValidBlockHeight`). To **maximize the window**, fetch the freshest blockhash - `confirmed` is the sweet spot (fresh, but on the canonical chain so not a minority-fork hash that vanishes; `processed` is freshest but can be on a dropped fork).

### (c) What happens to your bundle if the Jito leader skips their slot
1. **Bundles are only processed by a Jito-Solana leader** - they're effectively targeted at an upcoming Jito leader's slots; a bundle can be set to expire after the next Jito-Solana leader.
2. **Atomic, same slot:** all txs execute in one slot; cannot cross slot boundaries; if any fails, none commit; max 5 txs; sequential.
3. **If that Jito leader skips/misses their slot** (no block produced), the bundle has **no block to land in** → simply **not included**. It does **not** auto-roll-over to the next leader and is **not** retried for you. `getInflightBundleStatuses` shows `Pending` then ages to `Failed`/`Invalid`; never `Landed`.
4. **You must resubmit yourself**, targeting the next Jito leader window - and if the blockhash expired meanwhile, **rebuild with a fresh blockhash** (and typically a **higher tip**, since inclusion is a tip auction).
5. **Caveat (uncled blocks):** if a Jito leader *did* build a block with your bundle but the block gets skipped by the supermajority (an "uncle"), and someone rebroadcasts the individual txs, those hit the **normal banking stage which ignores bundle atomicity / revert protection** → partial/unprotected execution possible. The clean "skipped slot" case (no block) just fails the whole bundle.

## 4. Blockhash mechanics
- **150-block validity** (measured in **blocks, not slots**; block height < slot height due to skips). Leader checks the 151 most recent.
- **`getLatestBlockhash`** → `blockhash`, `lastValidBlockHeight` (= current height + 150). Params: `commitment` (default **`finalized`** - override to `confirmed`!), `minContextSlot` (guards against a lagging RPC behind an LB serving a stale hash).
- **Expiry check:** store `lastValidBlockHeight`; poll `getBlockHeight`; once `currentBlockHeight > lastValidBlockHeight`, the hash is dead - stop rebroadcasting, only *then* re-sign with a fresh blockhash.
- **`isBlockhashValid`** RPC - whether a given blockhash is still valid.
- **Durable nonces (no expiry):** nonce account; first instruction `AdvanceNonceAccount`; the stored nonce replaces `recentBlockhash`; never expires; advancing prevents replay. For offline signing / multisig / custody.

## 5. Failure classification - exact error shapes
Top-level rejections use **`TransactionError`**; per-instruction failures wrap **`InstructionError`** in `TransactionError::InstructionError(index, InstructionError)`.

**Expired/missing blockhash - two DISTINCT failures (don't conflate):**
- **`BlockhashNotFound`** - a **`sendTransaction` (preflight/submit) error**, returned synchronously: RPC/leader doesn't recognize the blockhash (expired, never on this fork, or the simulating RPC is behind the issuing RPC). `TransactionError::BlockhashNotFound`.
- **`TransactionExpiredBlockheightExceededError`** - a **web3.js `confirmTransaction` client-side error**: the tx **never landed** before block height passed `lastValidBlockHeight`. Accepted for sending, but never included. → "submitted fine, but the window closed."

**Fee too low / insufficient priority - silent drop (no error enum):** the leader just **doesn't include** the tx; dropped without an on-chain failure. Validators increasingly rate-limit/block low-fee sources (Helius requires total fees > **10,000 lamports** for staked routing). Observed only as a tx that never reaches `confirmed` and eventually expires.

**Compute exceeded:** `InstructionError::ComputationalBudgetExceeded` (exact name; NOT "ComputeBudgetExceeded"), inside `TransactionError::InstructionError(i, ComputationalBudgetExceeded)`. Log: `Program <id> consumed 200000 of 200000 compute units` then `Program failed to complete: exceeded maximum number of instructions allowed`. State reverts; fee still charged.

**Program-defined:** `InstructionError::Custom(u32)` (Anchor `#[error_code]` → starts at 6000); also `ProgramFailedToComplete`, `ProgramFailedToCompile`.

**Bundle failures (Jito):** separate from `TransactionError`. `getInflightBundleStatuses` → `Invalid`/`Pending`/`Failed`/`Landed`; `getBundleStatuses` → `processed`/`confirmed`/`finalized` for landed; gRPC `BundleResult.Dropped{BlockhashExpired|PartiallyProcessed|NotFinalized}` and `Rejected{...}`.

**Other common `TransactionError`:** `AccountInUse`, `AlreadyProcessed` (same signature reprocessed - common on over-eager retries), `InsufficientFundsForFee`, `InsufficientFundsForRent{account_index}`, `AccountNotFound`, `TooManyAccountLocks`, `WouldExceedMaxBlockCostLimit`, `MaxLoadedAccountsDataSizeExceeded`, `AddressLookupTableNotFound`, etc. Instruction-level: `InstructionError::InsufficientFunds`.

**Simulation:** `simulateTransaction` returns `err` (a `TransactionError`) + `logs` + `unitsConsumed` without landing. `replaceRecentBlockhash: true` lets the node substitute a valid blockhash. Preflight in `sendTransaction` runs this same simulation.

## 6. Retry strategy best practices
- **RPC default:** forwards to leaders **every ~2 s** until finalized or blockhash expiry (150 blocks / ~79 s), to current + next leader.
- **`maxRetries: 0` + custom rebroadcast** of the **same signed tx** every ~2 s until confirmed/expired = full control (advanced pattern).
- **`skipPreflight: false`** by default (catches errors before broadcast). `true` only for lowest-latency flows you've validated.
- **Refresh blockhash / re-sign ONLY after expiry** (`currentBlockHeight > lastValidBlockHeight`) - re-signing early risks landing two valid copies.
- **Stream-based confirmation beats polling:** `signatureSubscribe`/`slotSubscribe` (or Geyser) gives push-based status with far lower latency and fewer RPC calls; polling risks rate limits + stale reads (mitigate with `minContextSlot`).
- **Staked connections (SWQoS):** stake-weighted send improves landing during congestion.

## 7. Priority fees & compute budget (SEPARATE from Jito tips)
- **`ComputeBudgetProgram`:** `SetComputeUnitLimit(u32)` (default 200k/instruction, max **1,400,000** CU) + `SetComputeUnitPrice(u64)` in **micro-lamports/CU** (1 lamport = 1,000,000 micro-lamports).
- **Priority fee = `ceil(cu_price * cu_limit / 1e6)` lamports**, 100% to validator. Computed off the **requested** CU limit, not consumed - set limit to simulated usage + ~10%.
- **Base fee:** 5,000 lamports/signature (50% burned).
- **`getRecentPrioritizationFees`:** samples `{slot, prioritizationFee}` (micro-lamports/CU) over up to 150 blocks; optional `addresses[]` (≤128) for fees of txs locking those writable accounts.
- **Jito tips are a separate rail:** out-of-protocol SOL transfer to a tip account (min 1000 lamports), selected by Jito's off-chain auction (highest tip wins), routed privately (front-run protection). Priority fees buy ordering in the in-protocol scheduler; tips buy the bundle auction. Can combine, distinct recipients/logic.

## Key precision notes
- `getLatestBlockhash` default commitment is **`finalized`** - the single most common foot-gun (README answer b). Always override to `confirmed` for time-sensitive sends.
- Validity = **150 blocks ≈ 60-90 s**, blocks not slots.
- Compute-exceeded enum = **`ComputationalBudgetExceeded`**.
- Skipped Jito slot ≠ uncled block: skipped = whole atomic bundle fails (resubmit); uncle = rebroadcast txs can lose atomicity/revert-protection.

**Sources:** docs.anza.xyz/consensus/commitments · /proposals/optimistic_confirmation · /implemented-proposals/durable-tx-nonces · solana.com/docs/core/transactions/{retry,durable-nonces} · /core/fees · /rpc/http/{getlatestblockhash,getrecentprioritizationfees,isblockhashvalid} · docs.rs/solana-sdk TransactionError · docs.rs/solana-transaction InstructionError · docs.jito.wtf/lowlatencytxnsend · Helius blogs (commitment-levels, blockhash-errors, how-to-land-transactions, gulf-stream, executive-overview) · QuickNode Jito bundles · Chainstack (expiry, compute-budget).
