# Core implementation questions and binding answers

ADR 0013 is authoritative when older documents disagree.

## Decision index

| Question | Binding answer |
|---|---|
| Runtime? | A plain Rust Cloudflare Worker. No Durable Object requirement. |
| Matching kernel? | `orderbook-rs = 0.10.3`, with `pricelevel = 0.8.4`. |
| Bunting-owned book? | No. `crates/orderbook` is a thin adapter only. |
| Snapshot recovery? | `OrderBookSnapshotPackage` JSON, checksum validation, Workers Cache first, origin fallback. |
| Workers Cache role? | Mandatory immutable snapshot acceleration; never a lock or accepted-command journal. |
| Concurrent commands? | Optimistic expected-version commit in the origin store. |
| Warm isolate state? | Optional cache only. Never required for recovery or ordering. |
| Streaming? | Plain Worker WebSocket, committed sequence cursors, snapshot/reset recovery. |
| Dynamic strategies? | Isolated Dynamic Workers; proposed actions re-enter the ordinary command path. |
| FIX/NBC/RITC/Nautilus? | Protocol and client adapters only; all use the same OrderBook-rs-backed command path. |

## 1. Upstream API usage

Pin:

```toml
orderbook-rs = { version = "=0.10.3", default-features = false }
pricelevel = "=0.8.4"
```

Use:

- `add_limit_order_with_result` for limit orders when the caller needs exactly its own fills;
- `submit_market_order` or the corresponding user-aware API for market orders;
- `cancel_order` and mass-cancel APIs;
- `engage_kill_switch`, `release_kill_switch`, and drain behavior;
- `RiskConfig` and typed `OrderBookError` variants for book-level checks;
- `create_snapshot_package`, `to_json`, `from_json`, `validate`, and `restore_from_snapshot_package`;
- `evict_expired_orders(now_ms)` with an explicit recorded cutoff;
- direct depth, iterator, metric, market-impact, and enriched-snapshot APIs;
- upstream sequencer/replay helpers for recovery and differential tests.

Do not duplicate these implementations inside Bunting.

## 2. Plain Worker request path

```text
request
  -> auth and exact unit conversion
  -> idempotency + expected origin version
  -> Workers Cache snapshot lookup
  -> origin fallback and event-tail replay
  -> OrderBook-rs operation
  -> Bunting canonical events + ledger projection
  -> expected-version origin commit
  -> immutable Workers Cache put
  -> response and stream publication
```

A failed cache put does not change the accepted command. A failed origin commit produces no accepted response or fill publication.

## 3. Cache key and response contract

```text
/v1/orderbooks/{run_id}/{instrument_id}/{event_sequence}/{sha256_checksum}
```

Required response headers:

- `Cache-Control: public, s-maxage=<ttl>, immutable`;
- `ETag: <snapshot checksum>`;
- `X-Bunting-Event-Sequence: <sequence>`;
- `Content-Type: application/json`.

Cache entries contain public order-book state and matching configuration required by the upstream snapshot package. They do not contain credentials or private participant ledger state.

## 4. Origin concurrency

Each mutating command supplies an expected version. The origin commit must atomically:

1. verify current version;
2. append the canonical event batch;
3. write the idempotency result;
4. advance the version;
5. record snapshot metadata when produced.

A conflict returns a typed sequence conflict. Automatic retry is bounded and must not generate a new external order ID or command ID.

## 5. Event translation

Upstream results are translated, not re-decided.

For each call Bunting records sufficient facts to rebuild participant projections:

- submitted and accepted/rejected order identity;
- upstream reject code and normalized Bunting code;
- maker and taker order IDs;
- exact execution price and quantity;
- upstream engine sequence;
- fee amounts;
- resulting cancellation, expiry, and kill-switch behavior;
- represented snapshot checksum and event sequence.

The participant ledger consumes canonical trade and cancellation events. It does not scan the book to infer accounting changes.

## 6. Streaming

The plain Worker can accept WebSockets, but stream recovery is sequence-based rather than isolate-based.

- Snapshot first, then committed updates.
- L2 updates carry absolute resulting quantity; zero removes a level.
- Every frame carries the highest committed Bunting event sequence represented.
- OrderBook-rs `engine_seq` may be included for cross-stream book diagnostics.
- A reconnect requests an event tail or receives `stream.reset` and a current snapshot.
- Public state may coalesce; trades and private execution/account records cannot silently disappear.

## 7. Dynamic Worker strategies

The previous isolation decision remains:

- user code runs in an isolated Dynamic Worker;
- no direct cache, origin, credential, or order capability;
- explicit versioned strategy state;
- bounded CPU, requests, input, output, state, actions, and logs;
- accepted actions return through ordinary auth, risk, OrderBook-rs, ledger, commit, and cache flow;
- replay applies recorded accepted outputs rather than rerunning user Python.

## 8. Old vertical-slice branch

Keep from `feat/deterministic-kernel-vertical-slice`:

- market types and identifiers;
- command and event envelopes;
- ledger and participant risk projections;
- invariant and property-test ideas.

Discard:

- the Bunting-owned `BTreeMap`/arena book;
- the custom crossing loop;
- any checksum based on that obsolete internal book;
- assumptions that a Durable Object is the command owner.
