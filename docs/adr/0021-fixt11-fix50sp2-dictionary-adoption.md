# ADR 0021: Retain the bounded session core and adopt FIXT.1.1/FIX 5.0 SP2 dictionaries

- Status: Accepted
- Date: 2026-07-16

## Context

The competition profile requires FIXT.1.1 session semantics, FIX 5.0 SP2
application semantics, a FIX Latest Orchestra participant contract, bounded
sans-I/O operation, portable recovery and a Worker-safe dependency graph. ADR
0005 selected project-owned wire/session boundaries but its FIX 4.4 target is
obsolete. The Phase 5.5 spike evaluated maintained RustyFIX first and
FerrumFIX second, without using a `ref/` path dependency.

RustyFIX's complete engine does not pass the gate: the published feature graph
needs downstream repairs for native and Wasm builds, the pending application
queue is unbounded, the session lacks a serializable recovery snapshot and it
does not consume Orchestra. FerrumFIX `fefix 0.7.0` pulls a native
async/database/TLS graph with default features disabled and fails the Wasm
build. In contrast, `rustyfix-dictionary 0.7.4` with only `fixt11` and
`fix50sp2` compiles unchanged for native and Wasm on Rust 1.88.

## Decision

Retain the first-party `simfix-session` state machine because it already has
explicit clocks, bounded journals and pending input, sans-I/O actions, and a
versioned serializable recovery snapshot. Retain `simfix-mapping` as the only
FIX-to-application boundary.

Adopt exact crates.io release `rustyfix-dictionary 0.7.4`, upstream source
commit `2f0ef7830553d482765c14e3c4b32be3432d57b0`, with only the `fixt11` and
`fix50sp2` features. `simfix-wire` loads those standard dictionaries and
validates standard message and field identities. Bunting-owned tags
`10000`-`10020` and message types `U1`-`UC` form a separately generated
Orchestra overlay.

The public profile is `bunting.fixlatest.competition.v1`. Every frame uses
`BeginString(8)=FIXT.1.1`; Logon requires `DefaultApplVerID(1137)=9`, selecting
FIX 5.0 SP2 application semantics. The official FIX Latest Orchestra
repository is normative. RustyFIX's bundled QuickFIX resources are runtime
validation data and are neither copied nor modified by Bunting.

## Consequences

The server, TUI, relay and Worker share one portable session and dictionary
implementation, and durable snapshots remain independent of sockets and host
runtimes. Standard dictionary coverage expands without importing either
candidate's native engine graph. Bunting still owns session conformance and
must maintain differential recovery and external interoperability tests.

## Rejected alternatives

Adopting the full RustyFIX engine is rejected because its released artifact
fails the boundedness, recovery and unmodified portability gates. Adopting
FerrumFIX is rejected because its released dependency graph is unsuitable for
the Worker. Continuing FIX 4.4 is rejected because it contradicts the accepted
competition profile. Copying or modifying a third-party standard dictionary is
rejected because specification-derived data has separate obligations and FIX
Latest Orchestra is the selected normative source.

## Validation

- Build `simfix-wire`, `simfix-session`, server, TUI, relay path and Worker for
  native and `wasm32-unknown-unknown` on Rust 1.88.
- Prove FIXT.1.1 framing, FIX 5.0 SP2 message lookup, exact decimal Price field
  classification, profile Logon, bounded recovery and snapshot restore.
- Run the actual Worker bundle in raw Workerd with the D1 failure-closed gate.
- Complete QuickFIX/J or quickfix-go interoperability in Phase 8.

## Operational impact

Existing FIX 4.4 participants must update BeginString, Logon application
version and the profile identifier together. Session snapshot version 2 remains
valid because the sequencing state shape is unchanged; reconnect emits the new
wire identity.

## Security impact

The upgrade retains bounded frame, field, journal and pending-message limits.
The relay and server reject a Logon whose transport version, application
version, profile, CompIDs or credentials do not match the configured binding.
No new network listener, filesystem store or ambient authority is introduced.

## References

- FIX Trading Community, FIX Orchestra Standard and official FIX
  Orchestrations repository
- `rustyfix-dictionary 0.7.4` from
  `2f0ef7830553d482765c14e3c4b32be3432d57b0`
- `ref/ferrumfix` at `ca2bbe4c6461108646f35f7cc9245bf1848ec368`
- ADR 0005 and ADR 0020
- `docs/reference-functionality-audit.md`
- `docs/reference-adoption.md`
