# Yellowstone gRPC (Dragon's Mouth / Geyser) - Research Dossier

> Compiled 2026-06-02 from `master` of `github.com/rpcpool/yellowstone-grpc` (`yellowstone-grpc-proto/proto/geyser.proto`), Triton docs, and provider docs. Field numbers/enums quoted verbatim from the proto.

## 0. Source map & versions

| Component | Latest version | Notes |
|---|---|---|
| `yellowstone-grpc-geyser` | **13.1.1** (2026-05-29) | Plugin/server. Latest tag built against `solana.alpenglow.rc.1`. |
| `yellowstone-grpc-client` (Rust) | **13.1.0** (2026-05-13) | Built-in auto-reconnect introduced here. |
| `yellowstone-grpc-proto` | **12.4.0** (2026-05-13) | Cuckoo filter additions. |
| `@triton-one/yellowstone-grpc` (npm) | **5.x** | v5.0.0 = napi-rs (native Rust) backend rewrite. |

**Validator compatibility:** the plugin must be ABI-matched to the validator. geyser 13.x → agave 4.0.0-rc.x; 12.x → agave 3.x; 11.x → solana/agave v2.2; per-release tags like `yellowstone-grpc-geyser-1.15.0+solana.1.18.18` map plugin→validator explicitly. **Pin plugin to validator version.**

## 1. `SubscribeRequest` shape

Service: `rpc Subscribe(stream SubscribeRequest) returns (stream SubscribeUpdate)` - **bidirectional stream**. Send a new `SubscribeRequest` any time to *replace* the active filter set.

```protobuf
message SubscribeRequest {
  map<string, SubscribeRequestFilterAccounts>     accounts            = 1;
  map<string, SubscribeRequestFilterSlots>        slots               = 2;
  map<string, SubscribeRequestFilterTransactions> transactions        = 3;
  map<string, SubscribeRequestFilterTransactions> transactions_status = 10;
  map<string, SubscribeRequestFilterBlocks>       blocks              = 4;
  map<string, SubscribeRequestFilterBlocksMeta>   blocks_meta         = 5;
  map<string, SubscribeRequestFilterEntry>        entry               = 8;
  optional CommitmentLevel                        commitment          = 6;
  repeated SubscribeRequestAccountsDataSlice      accounts_data_slice = 7;
  optional SubscribeRequestPing                   ping                = 9;
  optional uint64                                 from_slot           = 11;
}
```

Map **keys are client-chosen filter labels**; matched labels return in `SubscribeUpdate.filters`. Limits: `filter_name_size_limit: 128`, `filter_names_size_limit: 4096`.

### Filter sub-messages (verbatim)
```protobuf
message SubscribeRequestFilterSlots {
  optional bool filter_by_commitment = 1;  // only emit slots at chosen commitment
  optional bool interslot_updates     = 2; // emit FirstShredReceived/Completed/CreatedBank between slots
}
message SubscribeRequestFilterTransactions {   // NOT a oneof - fields independent
  optional bool   vote             = 1;
  optional bool   failed           = 2;
  optional string signature        = 5;
  repeated string account_include  = 3;
  repeated string account_exclude  = 4;
  repeated string account_required = 6;
}
message SubscribeRequestFilterAccounts {
  repeated string account = 2; repeated string owner = 3;
  repeated SubscribeRequestFilterAccountsFilter filters = 4;
  optional bool nonempty_txn_signature = 5;
}
message SubscribeRequestFilterBlocksMeta {}  // empty - broadcast all
```

**Filter logic:** within a filter, *empty = broadcast all*; fields AND; array values OR (except `accounts.filters` which AND).

### `commitment` enum
```protobuf
enum CommitmentLevel { PROCESSED = 0; CONFIRMED = 1; FINALIZED = 2; }
```

### `ping`
Server emits a `Ping` every **15s**; reply by writing a `SubscribeRequest` with **only** `ping = {id}` set (keeps LB-fronted streams alive; does not alter filters).

### `from_slot` (replay)
`optional uint64 from_slot = 11` - on (re)subscribe, server replays buffered updates from that slot, then continues live. Bounded by the server's retained window (see §7). Discover the earliest replayable slot via the unary `SubscribeReplayInfo` RPC (`first_available`).

## 2. Slot updates - `SubscribeUpdateSlot`

```protobuf
message SubscribeUpdateSlot {
  uint64 slot = 1; optional uint64 parent = 2; SlotStatus status = 3; optional string dead_error = 4;
}
enum SlotStatus {
  SLOT_PROCESSED=0; SLOT_CONFIRMED=1; SLOT_FINALIZED=2;
  SLOT_FIRST_SHRED_RECEIVED=3; SLOT_COMPLETED=4; SLOT_CREATED_BANK=5; SLOT_DEAD=6;
}
```
Default emits all statuses. `filter_by_commitment=true` → only your commitment level; `interslot_updates=true` → fine-grained pre-confirmation statuses (`FIRST_SHRED_RECEIVED`, `COMPLETED`, `CREATED_BANK`) - **exactly the signals for slot-progression / leader-window timing**. `dead_error` carries the reason when a bank is dead.

## 3. Transaction subscription - `SubscribeUpdateTransaction`

```protobuf
message SubscribeUpdateTransaction { SubscribeUpdateTransactionInfo transaction = 1; uint64 slot = 2; }
message SubscribeUpdateTransactionInfo {
  bytes signature = 1; bool is_vote = 2;
  solana.storage.ConfirmedBlock.Transaction transaction = 3;
  solana.storage.ConfirmedBlock.TransactionStatusMeta meta = 4;  // err, logs, CU, balances
  uint64 index = 5;
}
```
**`err` and `log_messages` come from `meta`, not top-level.**

**Confirming a specific tx landed via the stream (no RPC polling - bounty requires this):**
1. Open `transactions` filter with `signature: "<sig>"` and `commitment: CONFIRMED` (or `FINALIZED`).
2. On arrival, the tx has landed at/above your commitment; check `transaction.meta.err == None` for success and read `transaction.slot`.
3. For lighter "did it land + did it error", use `transactions_status` (field 10) → `SubscribeUpdateTransactionStatus { slot, signature, is_vote, index, err }` (no full payload).

## 4. Account subscription (brief)

`accounts` filter by `account[]` (OR), `owner[]` (OR), `filters[]` (`datasize`, `memcmp{offset, bytes/base58/base64}`, `token_account_state`, `lamports{eq/ne/lt/gt}` - AND), `nonempty_txn_signature`. Payload `SubscribeUpdateAccountInfo { pubkey, lamports, owner, executable, rent_epoch, data, write_version, txn_signature? }`.

## 5. Block & BlockMeta

```protobuf
message SubscribeUpdateBlockMeta {
  uint64 slot=1; string blockhash=2; Rewards rewards=3; UnixTimestamp block_time=4;
  BlockHeight block_height=5; uint64 parent_slot=6; string parent_blockhash=7;
  uint64 executed_transaction_count=8; uint64 entries_count=9;
}
```
**For blockhash-freshness tracking:** subscribe `blocks_meta` (cheap - no tx/account payload) and read `blockhash` + `block_height` + `slot` per block. Stream-native replacement for `getLatestBlockhash` polling. (Caveat: validator-generated blocks may report `entries_count=0`.)

Unary RPCs on the same service: `GetLatestBlockhash`, `GetBlockHeight`, `GetSlot`, `IsBlockhashValid`, `GetVersion`, `Ping`.

## 6. Commitment semantics in the stream

- Subscriptions emit at the requested `commitment`. `PROCESSED` = earliest but may belong to a slot later dropped/forked.
- Triton pattern: process on `processed` for latency, but **commit only after the slot reaches confirmed/finalized** - buffer events keyed by slot, subscribe to `slots`, release when that slot's `SubscribeUpdateSlot` reaches `SLOT_CONFIRMED`/`SLOT_FINALIZED`.
- A given subscription streams at its chosen level (you don't get one tx re-emitted three times). To observe commitment progression, drive it off the **`slots` stream** status transitions. Slot updates *do* arrive multiple times per slot (processed→confirmed→finalized, + interslot if enabled).

## 7. Reconnection & backpressure

**Client gRPC keepalive (recommended channel args):**
```
grpc.keepalive_time_ms = 30000
grpc.keepalive_timeout_ms = 5000
grpc.keepalive_permit_without_calls = 1   // permit_without_stream
grpc.initial_reconnect_backoff_ms = 1000
grpc.max_reconnect_backoff_ms = 30000
```

**Backpressure / slow consumer:** server buffers per-connection up to `channel_capacity` (default **100,000** msgs). On overflow the **server drops the connection** (lagged consumer); no per-message ack flow control beyond HTTP/2 windows. Mitigation: split high-volume filters across multiple gRPC clients/threads; tune HTTP/2 windows.

**Max message size & compression (server defaults):** `max_decoding_message_size: 4_194_304` (4 MiB) - **every provider tells you to raise the client receive size** (64 MiB-1 GiB), or block/large-account messages silently fail. TS: `grpcMaxDecodingMessageSize: 64*1024*1024`. Rust: `.max_decoding_message_size(1024*1024*1024)`. Compression: gzip + zstd in accept/send.

**Replay window & resume:** `SubscribeReplayInfo.first_available` = earliest replayable slot. `replay_stored_slots` open-source default **150**; Triton production ~**1,000 slots**; Helius LaserStream **24h**. Resume by reconnecting with `from_slot = last_processed_slot - overlap`, then dedup.

**Built-in auto-reconnect (client v13.1.0+):** `ReconnectConfig` defaults - `backoff.initial_interval: 10ms`, `multiplier: 2.0`, `max_retries: 3`, `slot_retention: 250` (~100s). Tracks last slot → reconnects with `from_slot` earlier → `DedupStream` filters already-delivered → falls back to live if outside window.

## 8. Client libraries

### TypeScript - `@triton-one/yellowstone-grpc`
```ts
import Client, { CommitmentLevel, SubscribeRequest } from "@triton-one/yellowstone-grpc";
const client = new Client(endpoint, xToken,
  { grpcMaxDecodingMessageSize: 64 * 1024 * 1024 },
  { enabled: true, backoff: { initialIntervalMs: 100, multiplier: 2, maxRetries: 10 }, slotRetention: 250 });
await client.connect();
const stream = await client.subscribe();          // duplex
stream.on("data", (u) => { /* u.transaction / u.slot / ... top-level oneof */ });
stream.on("error", e => stream.end());
// send/replace filters by WRITING a SubscribeRequest into the stream:
await new Promise((res, rej) => stream.write(req, err => err ? rej(err) : res()));
```
Unary: `client.ping(id)`, `getVersion()`, `getSlot()`, `getBlockHeight()`, `getLatestBlockhash()`, `isBlockhashValid()`, `subscribeReplayInfo()`. **v5.0.0** switched to a napi-rs native backend (documented as no public API break); macOS builds may need `RUSTFLAGS="-Clink-arg=-undefined -Clink-arg=dynamic_lookup"`.

### Rust - `yellowstone-grpc-client`
`GeyserGrpcClient` (tonic). Options: `.tls_config(ClientTlsConfig::new().with_native_roots())`, `.max_decoding_message_size(...)`, `.connect_timeout(...)`, `.http2_adaptive_window(true)`, `.tcp_keepalive(...)`, `.tcp_nodelay()`. Stream: `let (mut tx, mut stream) = client.subscribe_with_request(Some(req)).await?;` then match `msg.update_oneof` over `Account/Slot/Transaction/TransactionStatus/Entry/BlockMeta/Block/Ping/Pong`. Auto-reconnect via `.set_reconnect_config(ReconnectConfig::default())`. **Breaking (client 13.0.0):** concrete `SubscribeRequestSink`/`GeyserStream` replaced `impl Sink`/`impl Stream`; requires Rust 1.94.1 / agave 4.0.0-rc.0.

## 9. Providers & auth

| Provider | Endpoint pattern | Auth |
|---|---|---|
| **Triton One** (Dragon's Mouth, origin) | `https://<...>.rpcpool.com:443` (also :10000) | `x-token` header; ~1,000-slot replay |
| **Helius** (LaserStream, YS-compatible) | `https://laserstream-mainnet-<region>.helius-rpc.com` (9 regions) | API key as `x-token`; **24h replay** |
| **QuickNode** | `<name>.solana-mainnet.quiknode.pro:10000` | `x-token` via gRPC metadata |
| **Shyft** | e.g. `https://grpc.ams.shyft.to` | `x-token` (or IP whitelist) |

(No distinct "SolInfra" Yellowstone product surfaced - the bounty's SolInfra credits likely wrap one of these. Verify endpoint + `x-token` pattern when credentials arrive.)

## Flags
- Replay window varies by provider - use `SubscribeReplayInfo.first_available` at runtime.
- Server default decode = 4 MiB; **always raise client receive size**.
- TS v5 napi rewrite affects deployment (prebuilt binaries / build toolchain on macOS).
- `transactions_status` reuses the Transactions filter but yields the lighter status update - use for landing/err confirmation.
- **Alpenglow**: latest geyser built against `solana.alpenglow.rc.1` - slot-status/consensus semantics will evolve; pin plugin to validator.
