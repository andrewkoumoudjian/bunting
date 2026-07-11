# Bunting architecture

## 1. Purpose

Bunting is a deterministic stock-market simulation platform built primarily in Rust for Cloudflare Workers. It supports configurable market scenarios, multiple simultaneous traders, native APIs, NautilusTrader, FIX 4.4, a browser UI, deterministic replay and isolated user-submitted Python strategies.

The platform is a simulator and education/research environment. It does not claim colocated exchange latency and does not route real-money orders unless a future separately reviewed gateway is added.

## 2. Architectural principles

1. **Pure kernel:** matching, risk, ledger, logical clock and scenario state are platform-neutral Rust.
2. **Single run authority:** one `MarketRun` Durable Object owns the total order of one run.
3. **Event-sourced truth:** accepted canonical events and snapshots reconstruct all authoritative state.
4. **Fixed-point correctness:** no floating-point price, quantity, cash, fee, risk or score arithmetic.
5. **Protocol boundaries:** HTTP, native WebSocket, legacy NBC, FIX and Nautilus translate into the same commands and events.
6. **Deterministic scenarios:** published configuration, seeds, algorithms and administrator interventions are immutable and replayable.
7. **Least privilege:** user strategies receive no direct storage, secret or unrestricted network capability.
8. **Bounded resources:** queues, frames, books, histories, strategy outputs and batches have explicit limits.
9. **Reference code is evidence:** all ports are audited, attributed, tested and adapted rather than copied blindly.
10. **Agile primitives:** start with one correct vertical slice and add order types, agents and scale only behind stable interfaces.

The binding decisions are recorded in `docs/adr/`.

## 3. System topology

```text
Browser UI ───────────────┐
Native Rust/Python SDK ───┼── HTTPS/WSS ──> Rust Edge API Worker
NautilusTrader adapter ───┤                         │
FIX application ─TCP─> FIX bridge ─WSS──────────────┘
                                                   │
                                                   ▼
                                      MarketRun Durable Object
                             authentication context already resolved
                                      command sequencing and state
                                                   │
                   ┌───────────────────────────────┼────────────────────────────┐
                   ▼                               ▼                            ▼
          Pure Rust market kernel       Durable Object SQLite         WebSocket fan-out
          matching/risk/ledger/clock    events/snapshots/FIX          native + FIX streams
                   │                               │
                   └──────── canonical events ─────┘
                                                   │
                       ┌───────────────────────────┼───────────────────────────┐
                       ▼                           ▼                           ▼
                      D1                          R2                         Queues
              global control plane        replays/source/exports       derived idempotent jobs
                                                                              │
                                                                      Analytics/logging

User Python source ─> Rust API ─service binding─> TypeScript loader ─> isolated Python Worker
                                                              proposed actions ─> Rust validation
```

## 4. Repository structure and ownership

### Core crates

- `market-types`: identifiers and checked fixed-point primitives.
- `market-events`: canonical commands, facts, envelopes, versioning and causation.
- `orderbook`: deterministic price levels, FIFO and order indexing.
- `matching-engine`: validated order execution and event generation.
- `risk-engine`: pre-trade and run-level risk decisions.
- `ledger`: cash, positions, fees and P&L projections.
- `simulation-clock`: logical time and clock modes.
- `scenario-schema`: canonical scenario documents and validation.
- `scenario-engine`: schedules, conditions and scenario lifecycle.
- `agent-models`: deterministic background market participants.
- `scoring`: versioned derived competition metrics.
- `protocol-native`: HTTP/WebSocket wire schemas.
- `protocol-legacy-nbc`: compatibility with the imported simulator contract.
- `simfix-wire`: FIX framing, dictionaries and serialization.
- `simfix-session`: transport-independent FIX session state machine.
- `simfix-mapping`: FIX application messages to canonical commands/events.
- `replay-format`: snapshot and event archive formats.
- `test-fixtures`: small deterministic fixtures and golden messages.

### Worker adapters

- `edge-api`: authentication, authorization, validation, coarse rate limits, safe caching and Durable Object routing.
- `market-run-do`: authoritative live run adapter, SQLite persistence, alarms and WebSocket fan-out.
- `export-consumer`: Queue consumer for replay and result artifacts.
- `analytics-consumer`: derived telemetry only.

### Services

- `strategy-loader`: the smallest possible TypeScript boundary around the Cloudflare Worker Loader API.

### Clients

- `rust-sdk`: typed native API client.
- `fix-bridge`: standard local FIX/TCP to Bunting FIX-over-WSS.
- `python-sdk`: typed API client and strategy tooling.
- `nautilus-adapter`: current NautilusTrader adapter layout.

## 5. Core domain and numerics

Correctness-critical values use explicit units:

```rust
PriceTicks(i64)
QuantityLots(i64)
MoneyMinor(i128)
LogicalTimeNs(u64)
EventSequence(u64)
BasisPoints(i64)
```

An instrument defines currency, tick size, lot size, display scale and permitted price range. External decimals are accepted as strings and converted exactly. Invalid tick or lot values are rejected; there is no silent rounding unless an endpoint explicitly declares a versioned rounding rule.

Checked arithmetic is mandatory. Multiplication uses widened intermediates. Serialization avoids unsafe JSON integer ranges by using decimal strings or validated bounded integers.

Statistical agents may use floating point internally only when needed. They validate finite values, clamp them and quantize to fixed-point commands through a versioned boundary. Floating point never determines book equality, priority, fills, cash, positions or final score totals.

## 6. Order book and matching

The initial book implements price-time priority:

- bids ordered by descending price;
- asks ordered by ascending price;
- FIFO at each price level;
- generational/slab storage for stable order references;
- direct client/engine order lookup;
- explicit remaining quantity;
- exact total level quantity.

Initial order operations:

- market;
- limit;
- immediate-or-cancel;
- fill-or-kill;
- post-only;
- cancel;
- cancel/replace;
- cancel all/kill switch.

Initial self-trade prevention is configurable and starts with cancel-newest.

Matching returns events rather than performing storage or transport calls. Core invariants include no overfill, conservation of quantity, correct FIFO, post-only non-crossing, all-or-none FOK behavior and terminal cancellation semantics.

## 7. Commands and canonical events

Example commands:

```text
SubmitOrder
CancelOrder
ReplaceOrder
ParticipantReady
AdvanceClock
InjectScenarioEvent
PauseRun
ResumeRun
HaltInstrument
ResumeInstrument
EndRun
```

Example events:

```text
OrderReceived
OrderAccepted
OrderRejected
OrderRested
OrderReduced
OrderCanceled
OrderExpired
TradeExecuted
PositionChanged
BalanceChanged
RiskLimitBreached
InstrumentHalted
InstrumentResumed
SimulationAdvanced
ScenarioEventTriggered
RunPaused
RunResumed
RunCompleted
```

Each event envelope includes schema version, run ID, event ID, sequence, logical time, wall-time metadata, actor, correlation ID, causation sequence and typed payload.

Events are facts. Transport-specific fields remain at protocol boundaries. Event versions are never mutated silently; migrations or compatibility readers are explicit.

## 8. Authoritative command pipeline

The `MarketRun` object processes one logical command at a time:

1. receive authenticated actor and command;
2. validate size and schema;
3. verify tenant, run, participant and scopes;
4. check idempotency key and command hash;
5. validate run and instrument status;
6. run exact participant throttles and pre-trade risk;
7. execute deterministic kernel transition;
8. atomically persist event batch and idempotency result;
9. apply events to memory;
10. acknowledge private result;
11. publish private and public projections;
12. enqueue idempotent derived work.

An order is not accepted until its event batch is durable. Failed persistence produces no accepted acknowledgement or public execution.

## 9. Durable Object lifecycle

Object identity is derived from tenant and run ID. The object owns all instruments in the run initially. Instrument sharding is deferred until measured limits require it.

On activation:

1. load compact metadata;
2. load the newest compatible checksummed snapshot;
3. replay events after the snapshot sequence;
4. restore scheduled events, FIX sessions and indexes;
5. verify state checksum/invariants;
6. accept new commands.

Snapshots include books, order indexes, ledgers, risk views, agent state, PRNG state, logical clock, scoring state and pinned version hashes. A snapshot is an optimization, not a separate truth source.

WebSocket hibernation stores only small connection attachments: identity, channel subscriptions, last acknowledged sequence and FIX session identifier. Canonical state remains in SQLite.

Accelerated work uses bounded event batches. Alarms schedule the next wall-time wake-up but do not define logical order. Alarm handlers are idempotent because retries are possible.

## 10. Durable Object SQLite model

Initial tables:

```sql
metadata(key PRIMARY KEY, value BLOB)
events(sequence PRIMARY KEY, event_id UNIQUE, logical_time_ns, event_type, actor_id, correlation_id, payload)
snapshots(sequence PRIMARY KEY, state_version, state_blob, checksum, created_at)
idempotency(participant_id, idempotency_key, command_hash, result_sequence, result_blob, PRIMARY KEY(...))
scheduled_events(scheduled_id PRIMARY KEY, due_logical_time_ns, priority, payload, processed_sequence)
fix_sessions(session_id PRIMARY KEY, sender_comp_id, target_comp_id, expected_incoming_seq, next_outgoing_seq, logged_on, heartbeat_seconds, state_blob)
fix_outbound_journal(session_id, msg_seq_num, sending_time, message, PRIMARY KEY(...))
```

Schema migrations are versioned and exercised against realistic historical fixtures.

## 11. Simulation clocks

### Lockstep

Publish a snapshot, collect participant `READY`/legacy `DONE`, expire a deterministic decision deadline, apply queued commands and advance. This is the fairness-oriented competition and teaching mode.

### Paced

The runtime maps logical batches to wall time using a pacing multiplier. Network arrival becomes an explicit command arrival order, while agents and scenario events use logical schedules.

### Accelerated

Process logical events as fast as allowed in bounded activations. Yield, checkpoint and schedule continuation before runtime limits are approached.

Given identical participant commands, the clock modes produce the same logical outcome; only delivery timing differs.

## 12. Scenario system

Scenarios move through:

```text
draft -> validated -> canonicalized -> published immutable version -> run
```

A published version contains all materialized defaults and hashes. A run pins scenario, engine, agent, scoring, random derivation and quantization versions.

Configuration groups:

- metadata;
- instruments;
- clock;
- microstructure and latency;
- fees;
- risk;
- agent populations;
- scheduled events;
- bounded conditional events;
- scoring.

Initial agents are fundamental, noise, momentum, mean-reversion, inventory market maker, institutional execution, liquidity withdrawal, forced liquidation, news shock and spiking behavior ported from the Java scenario concepts.

Each agent has isolated deterministic random streams derived from run seed, agent ID, instrument ID, stream name and derivation version. Agents propose commands and never bypass risk or matching.

Wirefilter is a conditional candidate for administrator rule expressions only after Wasm compilation, size, latency, memory and security tests. The first implementation can use a smaller project-owned expression AST.

## 13. Native API and streaming

Control-plane endpoints cover instruments, scenarios, runs, joining, results and replay. Trading endpoints cover orders, positions, account, snapshots and event recovery. Administrative endpoints cover scenario versions, pause, resume, step, event injection, halts and run completion.

The main WebSocket supports channel subscriptions:

```text
run.status
instrument.status:<instrument>
book.l1:<instrument>
book.l2:<instrument>
trades:<instrument>
bars:<instrument>:<interval>
orders.private
executions.private
positions.private
account.private
risk.private
strategy.logs
```

Every message has a protocol version, run ID, event/projection sequence, logical time, channel, type and payload.

Clients receive a snapshot followed by deltas. They detect gaps and request resume or resnapshot. Output queues are bounded. Public L1 can coalesce to the latest value; private executions cannot be dropped silently. Slow consumers receive a warning and are disconnected with recovery metadata if limits persist.

JSON is the first correct protocol. A compact binary codec is feature-gated after compatibility fixtures and profiling.

## 14. FIX architecture

The Worker cannot accept inbound raw TCP directly. Bunting therefore preserves FIX 4.4 messages and session semantics over a binary WebSocket subprotocol:

```text
Sec-WebSocket-Protocol: bunting.fix44.v1
```

One binary frame contains one complete raw tag-value FIX message with SOH delimiters. The run Durable Object owns session sequence, heartbeat, resend and message journal state.

`bunting-fix-bridge` is a native Rust client package and binary. It exposes local FIX/TCP acceptor and initiator modes, incrementally frames TCP bytes, tunnels exact complete messages over WSS and writes returned bytes unchanged. It uses bounded buffers, authenticated WSS, explicit reconnect and diagnostics.

The project adapts a pinned subset of IronFix behind project-owned wire/session traits. Fixer and another independent implementation are conformance peers. No complete native FIX engine is placed in the Worker graph.

## 15. NautilusTrader

The Bunting adapter follows the pinned Nautilus developer guide:

- Rust HTTP client;
- Rust WebSocket client and parsing;
- PyO3 layer;
- Python `InstrumentProvider`;
- live market-data client;
- live execution client;
- configuration and factories;
- startup and reconnect reconciliation.

Nautilus uses the native Bunting API rather than FIX. Mappings cover instruments, quote ticks, trade ticks, L2 deltas, bars, order events, fills, positions and account state.

## 16. User Python strategies

Source is immutable and addressed by content hash in R2. D1 stores ownership and metadata. The Rust API invokes a minimal TypeScript Loader service through a service binding. The loader starts/reuses an isolated Python Dynamic Worker with denied global egress and explicit limits.

The Python wrapper receives sanitized batches and explicit state. It returns state, proposed actions and bounded logs. The Rust caller validates actions and passes them through the normal command pipeline.

No strategy receives reusable credentials, direct storage, a Durable Object stub, Queues or unrestricted network access. Runtime/source/SDK versions and callback inputs/outputs are recorded.

## 17. Cloudflare product responsibilities

| Primitive | Responsibility |
|---|---|
| Durable Object memory | hot active-run state and connection indexes |
| Durable Object SQLite | authoritative run events, snapshots, idempotency, schedules and FIX state |
| D1 | tenants, users, scenario/run/strategy directories and completed-result indexes |
| R2 | replay archives, immutable strategy source, exports and large diagnostics |
| KV | non-authoritative read-heavy metadata tolerant of delayed propagation |
| Workers Cache | public immutable/versioned GET responses and static assets |
| Queues | idempotent derived export, aggregation, notification and compaction |
| Analytics/logging | operational telemetry, never financial truth |

Cache is not used for live orders, private account data or WebSocket state. Exact participant order limits are enforced in the run object; edge rate limiting is only a coarse abuse layer.

## 18. Performance model

Optimize after correctness and measurement.

Initial practices:

- small dependency graph and disabled default features;
- release LTO and size inspection for Wasm;
- fixed-point compact domain types;
- no allocation-heavy JSON in matching internals;
- slab/generational order storage;
- batch SQLite event inserts;
- snapshot cadence based on event count and byte size;
- no D1/KV/R2 reads in the matching hot path;
- bounded public-data coalescing;
- immediate private acknowledgements after durability;
- independent derived Queue work;
- bounded accelerated batches;
- stable ordered collections only where output order matters;
- native benchmarks for the pure kernel and Worker integration load tests.

Pingora, quiche, foundations and other native Cloudflare service repositories are references only. They are not Worker dependencies because they assume native operating-system services.

## 19. Security model

- tenant and participant identity is attached to every command;
- scopes separate data, order, strategy and administrator capabilities;
- reusable credentials are not placed in WebSocket URLs;
- all untrusted payloads have byte, count and nesting limits;
- public/private channels are distinct;
- idempotency prevents accidental duplicate commands;
- strategy Workers receive denied egress and no secrets;
- reference/vendor licenses and patches are tracked;
- protocol parsers are fuzzed;
- admin interventions are canonical events;
- logs redact credentials and sensitive headers;
- resource exhaustion produces explicit disconnect/recovery rather than unbounded growth.

## 20. Observability

Record command validation, risk, matching, persistence, fan-out, snapshot and recovery latency; orders/trades/events per second; WebSocket queue sizes; slow consumers; FIX session/resend activity; strategy CPU/failures; alarm delay; Queue retries; and logical-to-wall pacing lag.

Include tenant, run, participant, instrument, correlation, sequence, strategy and FIX session identifiers where backend cardinality supports them. Canonical events remain the audit source.

## 21. Testing and verification

### Pure kernel

- unit and property tests;
- price-time priority;
- no overfills;
- quantity and cash conservation;
- cancel/post-only/FOK invariants;
- checked arithmetic;
- deterministic seed/command replay;
- snapshot equivalence.

### FIX

- BodyLength and CheckSum;
- malformed fields and groups;
- partial and combined TCP reads;
- logon, logout, heartbeat, test request;
- sequence gaps, resend, gap fill and duplicates;
- bridge reconnect and ordering;
- independent implementation interoperability.

### Worker

- Durable Object routing and reconstruction;
- SQLite migrations;
- binary WebSocket frames and hibernation attachments;
- alarm retries;
- Queue duplicate delivery;
- cache safety;
- strategy egress denial and limits.

### Nautilus

- instrument discovery;
- book synchronization and recovery;
- order lifecycle;
- fills and account reconciliation.

## 22. Reference-porting map

### C++ engine

Port concepts into separate crates: order management becomes command processing and canonical events; position keeper becomes ledger; risk interfaces become risk engine; journal/order store become event persistence projections; execution gateways become protocol/runtime adapters.

Do not port native threads, mutexes, gRPC, filesystem SQLite or races caused by asynchronous broker identifier mapping.

### Java simulator assets

Port scenario parameters and agent concepts through a canonical schema. Add validation, fixed-point boundaries, independent random streams and replay tests.

### Rust RITC implementation

Extract API models and separately reviewed agent-model formulas where useful. Keep blocking HTTP, native sleeps, Tokio and raw floating-point market state out of the Worker kernel.

### External submodules

Use `docs/reference-inventory.md` as the authoritative role and pin list.

## 23. Delivery path

1. **Bootstrap:** ADRs, references, workspace, fixed-point and event primitives.
2. **Kernel vertical slice:** one instrument, limit/market/cancel, ledger, risk, deterministic replay CLI.
3. **Worker vertical slice:** edge Worker, one run Durable Object, SQLite, REST and native WSS.
4. **FIX:** shared wire/session crates, Worker WSS endpoint and native TCP bridge.
5. **Scenarios:** schema, agents, clock modes, admin controls and scoring.
6. **Nautilus:** Rust/Python adapter and reconciliation.
7. **Dynamic Python:** loader service, isolated callbacks, quotas and editor.
8. **Hardening:** load, fuzz, chaos/recovery, observability and security review.

The first deployable release remains deliberately small: one venue, a bounded number of instruments per run, price-time matching, basic risk, deterministic scenarios and reliable protocol adapters. New complexity is introduced only behind measured, tested primitives.
