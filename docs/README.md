# Documentation status map

Read this before relying on any document under `docs/`. Bunting keeps old
plans, prompts and audits as evidence, but many of them describe designs that
were later replaced (Cloudflare Worker authority, OrderBook-rs as the
production matcher, tRPC, NBC compatibility). A document's status here
overrides anything the document says about itself.

Status meanings:

- **Binding** — instructions an agent must follow.
- **Current** — accurate description of the code or reference evidence today.
- **Target** — accepted direction that is *not yet implemented*; do not
  describe it as done.
- **Historical** — superseded; read only as decision history. Every
  historical document carries a banner at the top.

Last reconciled: 2026-10-10 through slice 21 (ADR 0033 reconciled
statuses as of 2026-10-09).

## Start here

1. [`../AGENTS.md`](../AGENTS.md) and the nearest scoped `AGENTS.md`.
2. [`architecture.md`](architecture.md) — what exists now and what is target.
3. [`research/2026-10-09-exploration-and-next-steps.md`](research/2026-10-09-exploration-and-next-steps.md)
   §8 — owner decisions and the current step order.
4. [`implementation-log/`](implementation-log/) — what each slice actually
   changed and which checks ran.

## Architecture decisions (`adr/`)

ADR status lines are authoritative; read the status line before the decision
text. ADR 0033 reconciles statuses as of 2026-10-09.

| Governing now | Topic |
|---|---|
| 0003 (event-sourcing part) | Event-sourced state |
| 0009, 0010 | Fixed-point numerics, scenario determinism |
| 0011 (protocol semantics part) | Committed-sequence streams, reset, coalescing, backpressure |
| 0018 (as amended) | One authoritative engine; carries ADR 0014's split — participant execution engines never own market state |
| 0017 | Licensing record for the NBC JAR (evidence use only, see 0032) |
| 0021, 0023 | FIX dictionary/profile, concurrent FIX sessions |
| 0022 | Single native venue; Cloudflare read-only publication; no Worker built |
| 0025 + 0028 item 5 | Run archive: version 2 replays every journaled input from genesis or a checkpoint (slice 20) |
| 0026 | Language bindings and FFI lints |
| 0027 | WASIX — **current packaging only**, host still open |
| 0028 | Headless run authority, single ledger, live state, full replay |
| 0029 | Engine-owned deterministic order book |
| 0030 | Continuous admission with a `(release, arrival)` sequencer (replaced 0024 intervals; FIX slice 15, agents slice 18); modes and equalization superseded by 0035 |
| 0031 | FIX + certified Bunting Native Protocol only; app via `bunting-client` — partly implemented (slice 25, see 0040), instructor/admin, run streams and parity test Target |
| 0032 | NBC is reference evidence only; engine surface removed (slice 11) |
| 0033 | Guidance reconciliation and status amendments |
| 0034 | **Historical** — equalized admission; superseded by 0035 |
| 0035 | Latency: real connectivity counts, virtual distance added both ways, no modes (slice 16) |
| 0036 | Public market data: per-venue direct feeds of trades and L2 (slice 21) a consolidated tape from a processor at the hub (slice 22) and order-by-order feeds (slice 23) over the latency map, with per-venue broker identifiers (slice 24); owner decisions 2026-10-10 (data and colocation free) |
| 0037 | One run clock and venue timer (slice 26); **Target:** calendar and session phases, opening and closing auctions, end-of-day marks and multi-day runs |
| 0040 | Bunting Native Protocol v1: fixed binary frames, TLS 1.3 mTLS with a certificate-fingerprint roster, stateless identities, resume by committed sequence, same latency path as FIX (slice 25) |

Superseded or historical ADRs: 0001, 0002, 0004, 0005 and 0006 (Worker
transport details), 0007, 0008, 0012, 0013, 0014 (folded into 0018), 0015,
0016, 0019 (in part), 0020 (in part), 0024 (replaced in slice 15).

## Documents

| Path | Status | Notes |
|---|---|---|
| `AGENTS.md` (this dir) | Binding | Documentation rules |
| `architecture.md` | Binding | Current state and target, labeled separately |
| `deployment.md` | Current | Describes the WASIX packaging that ships today |
| `reference-functionality-audit.md` | Binding for `ref/`/`vendor/` claims | Read before describing any reference |
| `reference-adoption.md` | Binding for dependencies | Read before adding dependencies or copied code |
| `reference-inventory.md` | Current | Submodule pins |
| `implementation-log/` | Current | Per-slice record of changes and checks |
| `plans/2026-10-07-evidence-led-core-roadmap.md` | Current (slice definitions) | Ordering superseded by the 2026-10-09 note §8 |
| `research/` | Current as dated evidence | Point-in-time; later commits may have changed the code |
| `specs/bunting-product-contract.md` | Target | Product boundary; interfaces per ADR 0031 |
| `specs/bunting-fix-competition-profile.md` | Current | FIX profile; admission timing changes with ADR 0030 |
| `specs/bnp-v1.md` | Current | Bunting Native Protocol v1 wire contract (ADR 0040, slice 25) |
| `specs/competition-policies-v1.md` | Current | Implemented competition policies |
| `specs/rit-class-market-simulation.md` | Target | RIT-class feature requirements |
| `specs/rit-tui-parity-matrix.md` | Target | RIT workflow parity for the app/TUI |
| `ports/quarcc-trading-engine.md` | Current | QUARCC port record |
| `ports/ritc-market-making.md` | Current | RITC participant reference |
| `ports/nbc-*.md`, `ports/nbc-*.json`, `ports/nbc-*.tsv` | Historical provenance | NBC is reference only (ADR 0032) |
| `codex-implementation-prompt.md` | Historical | Worker/OrderBook-rs era instructions |
| `core-implementation-questions.md` | Historical | Its "binding answers" are superseded |
| `implementation-pathway.md` | Historical | Worker/OrderBook-rs pathway |
| `core-engine-status-2026-10-07.md` | Historical | Superseded by the independent audit and implementation log |
| `hackathon-base-plan.md` | Historical | Early competition proposal |
| `streamlining-audit.md` | Historical | Early exploration report |
| `joaquin-repository-audit.md` | Historical | OrderBook-rs-era dependency audit |
| `orderbook-rs-example-adoption.md` | Historical | OrderBook-rs is a dev-only oracle |
| `rust-reference-sprint-map.md` | Historical | Early sprint reference map |
| `repository-reorganization.md` | Historical | Completed July move; durable rules live in `AGENTS.md` |
| `plans/` (all except the 2026-10-07 roadmap) | Historical | Completed or superseded plans and worktree handoffs |
| `prompts/` | Historical | Old agent prompts; do not reuse |
| `claude/` | Historical | Archived 2026-07-12 planning session |

## Root-level participant documents

`PROTOCOL.md` (generated by `tools/generate_protocol.py`), `RULES.md`,
`RUNBOOK.md` and `SCORING.md` describe the competition **as currently
implemented** (latency-modeled admission since slice 15, public feeds since
slice 21, consolidated tape since slice 22, order-by-order feeds since slice 23, broker identifiers since slice 24,
Bunting Native Protocol since slice 25; its wire contract is `docs/specs/bnp-v1.md`). Update them in the
same change that implements new behavior (ADR 0031, agent admission), never
ahead of it.

## Keeping this map true

Any change that supersedes a document updates this table and adds the
historical banner in the same commit.
