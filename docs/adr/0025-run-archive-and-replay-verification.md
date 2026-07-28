# ADR 0025: Run archive and replay verification

- Status: Accepted
- Date: 2026-07-28
- Extends: ADR 0010

## Context

ADR 0010 requires deterministic simulation, but a competition result also needs a portable record proving which scenario, seeds, commands and engine version produced the final state. The existing golden full-run fixture demonstrates the shape of a deterministic record but is test data rather than an operator-owned archive contract.

Scoring directly from live mutable state would make disputes depend on an unavailable process or database. A replayable immutable archive makes the result independently verifiable.

## Decision

Every completed or halted competition round emits a versioned immutable archive containing the scenario identifier and checksum, engine and archive schema versions, named seed set, roster identifiers, ordered accepted-command stream with arrival sequence, canonical event stream, final state checksum and artifact checksums.

`bunting replay <archive>` reconstructs the run without wall-clock or network input and exits successfully only when canonical bytes and checksums match. `bunting score <archive>` is a pure function of the verified archive. `bunting judge <archives>` produces deterministic machine-readable and human-readable leaderboards.

Archives are append-complete records: recovery may resume writing an interrupted round, but published archives are immutable and content-addressed.

## Consequences

Determinism becomes an enforced compatibility contract rather than a property asserted only by unit tests. Engine or schema changes that intentionally change canonical output require a version transition and preserved reader for supported archives.

Archives may contain private participant activity, so public publication must use an explicitly redacted projection while the authoritative archive remains access-controlled.

## Rejected alternatives

### Score directly from the live origin store

Rejected because a mutable store does not provide a portable, independently verifiable dispute record.

### Archive snapshots without commands and events

Rejected because a final snapshot proves neither command ordering nor replay determinism.

### Treat the golden fixture as the archive format

Rejected because a test fixture lacks versioning, provenance and operational completion semantics; it is the seed for the format, not the contract itself.

## Validation

- replay of the canonical golden archive is byte-identical;
- two replays on supported hosts produce identical canonical event and state checksums;
- scoring reads only a verified archive;
- tampering with scenario, command, event or checksum data fails closed;
- halt and restart produce one valid archive without duplicated accepted commands;
- public projections contain no private participant fields.

## Operational impact

Round close, emergency halt and recovery workflows must finalize or resume the archive before results are published. Retention and publication policies key artifacts by checksum.

## Security impact

Archive parsing is bounded and versioned. Private archives require access control, while public leaderboards and snapshots are derived outputs that cannot be used as authority inputs.

## References

- [`0010-deterministic-simulation.md`](0010-deterministic-simulation.md)
- [`../plans/competition-platform-reorganization.md`](../plans/competition-platform-reorganization.md)
- [`../../tests/goldens/competition-full-run.v1.json`](../../tests/goldens/competition-full-run.v1.json)
