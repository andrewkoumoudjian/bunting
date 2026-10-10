# ADR 0033: Guidance reconciliation after the owned book and owner decisions

- Status: Accepted (2026-10-09) by the repository owner's direction in the
  2026-10-09 exploration session (answers recorded in
  [the exploration note §8](../research/2026-10-09-exploration-and-next-steps.md#8-owner-decisions-2026-10-09-and-revised-plan),
  followed by approval of proposed ADRs 0030–0032 and a request that repository
  guidance stop agents from following superseded paths).
- Amended by: [ADR 0044](0044-native-server-binary-host.md) (2026-10-10)
  makes the host decision that decision 3 deferred; decision 3 and the ADR
  0027 row below are superseded by it.
- Date: 2026-10-09
- Accepts: ADR 0030, ADR 0031, ADR 0032.
- Amends status of: ADR 0007, 0012, 0016, 0018, 0019, 0020, 0022, 0024, 0027.

## Context

By `main@1d857d1` the code had moved past several accepted ADRs and binding
documents:

- ADR 0029 replaced OrderBook-rs with the engine-owned book, but ADR 0019's
  status, `docs/architecture.md`, `docs/core-implementation-questions.md`
  ("binding answers") and `docs/codex-implementation-prompt.md`
  ("non-negotiable") still told agents to use OrderBook-rs and not to own a
  book.
- Commit `eed8e00` deleted `apps/bunting-worker`, D1 origin, Workers Cache and
  Worker FIX session objects, but ADR 0016 and 0020 remained "Accepted" and
  several scoped `AGENTS.md` files still described Worker, cache and Durable
  Object behavior.
- ADR 0007 and ADR 0012 define hosted user-strategy execution through
  Cloudflare Dynamic Workers and Queues feeding a run object; ADR 0022 removed
  every Cloudflare command path, so those designs cannot run.
- The owner gave direction on fairness, interfaces, NBC and hosting
  (2026-10-09) that proposed ADRs 0030–0032 encode.

An agent obeying the repository's own precedence rules would follow the stale
text. This ADR records which decisions govern so status lines and guidance can
point at one place.

## Decision

1. **Accept ADRs 0030, 0031 and 0032.** They describe the target; none is
   implemented yet. Guidance must say "target, not implemented" until the
   implementation log records each slice.
2. **Status amendments** (status lines updated to point here; decision text
   left as history):

| ADR | New status | Reason |
|---|---|---|
| 0007 Dynamic Worker loader | Superseded by ADR 0022 and this ADR | No Cloudflare command path exists. Contestant strategies run on the contestant's side and connect over FIX or BNP (ADR 0031). A future hosted-strategy sandbox needs a new ADR. |
| 0012 Asynchronous strategy dispatch | Superseded by ADR 0022 and this ADR | Same as 0007; depends on a Cloudflare run object and Queues. |
| 0016 Native Rust tRPC Worker | Superseded by ADR 0020, 0022 and 0031 | Worker deleted in `eed8e00`; app-facing protocol is BNP. |
| 0018 Unified engine | Accepted; matcher clause superseded by ADR 0029; NBC-compatibility clauses superseded by ADR 0032 | Single-engine authority remains binding. |
| 0019 Engine owns OrderBook-rs | Superseded in part by ADR 0029 | Package ownership of matching remains; OrderBook-rs is a dev-only oracle. |
| 0020 Transport-neutral engine, outbound FIX | Accepted for the transport-neutral engine boundary; Worker browser transport and outbound FIX superseded by ADR 0022, 0023 and 0031 | Inbound FIX acceptor and BNP are the only interfaces. |
| 0022 Native venue, publication Worker | Accepted; the "transitional Worker" clause is complete (`eed8e00`); no Worker is currently built | |
| 0024 Discrete matching intervals | Superseded by ADR 0030 (target) | The interval writer remains the implemented behavior until the ADR 0030 sequencer lands. |
| 0027 Wasmer WASI runtime | Accepted as the **current** server packaging only; not a binding long-term host | Owner direction: WASIX is not required if there are better ways to run the binary anywhere. |

3. **Host direction.** The server must stay buildable and testable as a native
   binary. Do not add dependencies or code paths that only work under WASIX.
   Native static binaries and an OCI image are the preferred distribution
   candidates; the final host selection is made by a later ADR using
   measured native-versus-WASIX data (exploration note Step 2). The engine and
   protocol packages keep the `wasm32-unknown-unknown` gate.
4. **Documentation status map.** [`docs/README.md`](../README.md) classifies
   every document as binding, current, target or historical. Historical
   documents carry a "do not follow" banner. Agents read the map before
   relying on any document under `docs/`.

## Consequences

- Agents get one consistent answer: owned book, single native venue, FIX + BNP
  only, NBC as reference evidence, latency-modeled continuous admission as the
  target, host undecided but native-capable.
- Historical documents remain in the repository as evidence but no longer
  read as instructions.
- Implemented behavior (interval writer, WASIX release packaging, NBC module)
  still exists until the corresponding implementation slices remove or
  replace it; guidance must not describe those removals as done.

## Rejected alternatives

- **Rewrite old ADR decision text:** violates the ADR rule to amend with new
  ADRs and destroys decision history.
- **Delete historical documents:** loses provenance that the evidence
  discipline relies on; banners are enough.
- **Leave statuses and fix only `AGENTS.md`:** agents also read ADR status
  lines and docs directly; contradictions would persist.

## Validation

- Every ADR in the table above has a status line naming this ADR.
- `docs/README.md` lists every file under `docs/` (excluding `docs/adr/` and
  `docs/research/`, which have their own rules) with a status.
- Outside historical documents, ADR history and research notes, no document
  instructs agents to use OrderBook-rs in production, add Worker/D1/Workers
  Cache/Durable Object behavior, preserve NBC compatibility, or treat WASIX as
  mandatory.

## Operational impact

None at runtime. Release packaging is unchanged until the host ADR.

## Security impact

None at runtime. Removing contradictory guidance reduces the risk of an agent
re-introducing a second authority (Worker command path) that ADR 0022 removed
for security and correctness reasons.

## References

- [ADR 0022](0022-native-competition-venue-and-publication-worker.md), [ADR 0029](0029-bunting-owned-deterministic-order-book.md), [ADR 0030](0030-proposed-latency-modeled-continuous-admission.md), [ADR 0031](0031-proposed-bunting-native-client-protocol.md), [ADR 0032](0032-proposed-nbc-reference-only.md)
- [Exploration note, 2026-10-09](../research/2026-10-09-exploration-and-next-steps.md)
