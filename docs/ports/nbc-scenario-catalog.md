# NBC scenario source catalog and provenance

This catalog is the first artifact in the NBC port. It records the imported scenario inputs before any algorithm or Java runtime is ported.

The files remain reference evidence. This document does not declare them canonical Bunting scenarios and does not resolve their license or the meaning of every numeric parameter.

## Source locations

Primary source tree:

```text
ref/nbc_engine/app/src/main/resources/scenarios/
```

A duplicate copy exists under:

```text
ref/ritc_mm/app/src/main/resources/scenarios/
```

The five verified pairs have identical Git blob SHAs. `ref/nbc_engine` is treated as the provenance root; the duplicate under `ref/ritc_mm` must not become a second scenario lineage.

## Verified source inventory

| Scenario | Source file | Git blob SHA | Seed | Steps | Step interval | Agent counts and distinguishing behavior |
|---|---|---:|---:|---:|---:|---|
| Normal market | `normal_market.json` | `900d45268ff311784bdc27d43b25dd0b4a1c52d4` | 12345 | 36,000 | 100 ms | fundamental 30; long momentum 15; short momentum 15; noise 30; market maker 5; includes `initialSpread` 0.50 |
| Flash crash | `flash.json` | `3514de6212931ab62a8ede4d4d7fb58800cfdded` | 12346 | 54,000 | 100 ms | fundamental 30; long momentum 30; short momentum 30; noise 30; market maker 20; institutional seller 1 |
| Mini flash crash | `mini_flash_crash.json` | `114da4e279c4d7c39631995b031d3675325ec3f8` | 12347 | 36,000 | 100 ms | fundamental 30; long momentum 30; short momentum 30; noise 30; market maker 5; spiking agents 2 |
| Stressed market | `stressed_market.json` | `3e3a7bc42fcf6084a0cc92027421b0b1078a2a1c` | 12348 | 36,000 | 100 ms | fundamental 20; long momentum 25; short momentum 25; noise 20; market maker 3; negative drift and higher volatility |
| HFT dominated | `hft_dominated.json` | `a2ca2f52f75e02c8e14c28ea110de73614cb4e45` | 12349 | 36,000 | 100 ms | fundamental 10; long momentum 10; short momentum 50; noise 20; market maker 8 |

All five specify an initial fundamental value of `1000.0` and tick size `0.25`. These decimal spellings are source data only. Canonical Bunting files must express exact decimal strings and validated integer units.

## Parameter vocabulary

### Scenario-level fields

- `id`
- `name`
- `description`
- `seed`
- `durationSteps`
- `stepIntervalMs`
- `marketConfig`
- `traders`
- `specialEvents`

### Market fields observed

- `initialFundamentalValue`
- `tickSize`
- `initialSpread` in the normal scenario only
- `fundamentalDrift`
- `fundamentalVolatility`

### Agent families and fields observed

#### Fundamental

- `count`
- `kappa1`
- `kappa2`
- `interval`

#### Momentum, long-term and short-term

- `count`
- `decayRate`
- `chi`
- `psi`
- `cancelRate`

#### Noise

- `count`
- `eta`
- `kappa`
- `cancelRate`

#### Market maker

- `count`
- `gamma`
- `delta`
- `maxEdge`
- `inventoryLimit`
- `safeInventory`
- `restPeriod`

#### Institutional seller

- `count`
- `initialInventory`
- `percentageOfVolume`
- `startTime`
- `orderInterval`

#### Spiking

- `count`
- `spikeLength`
- `activationProbability`
- `orderVolume`

## Unresolved semantics

The following must not be guessed during transcription:

- whether drift and volatility are per step, per second or calibrated to another horizon;
- whether `interval`, `restPeriod`, `orderInterval` and `spikeLength` are steps, milliseconds or another simulation unit;
- the formulas and units represented by `kappa1`, `kappa2`, `chi`, `psi`, `eta`, `kappa`, `gamma`, `delta` and `maxEdge`;
- whether `percentageOfVolume` means a percentage, integer weight or schedule parameter;
- the timezone and relationship to logical simulation start for `startTime`;
- cancellation selection and queue semantics;
- random distribution families used by each agent;
- whether the normal scenario's parameter scale is intentionally different from the other four scenarios.

A source value may be preserved in a `legacy_parameters` map while its semantic type remains unresolved, but unresolved values cannot drive canonical agents.

## Canonical target locations

```text
scenarios/nbc/
  README.md
  source-manifest.json
  normal-market.v1.json
  flash-crash.v1.json
  mini-flash-crash.v1.json
  stressed-market.v1.json
  hft-dominated.v1.json
```

The target files are not added until the schema and translation-provenance gates below pass.

Parsing and behavior belong in:

```text
packages/bunting-engine/      production scenario, agent, clock, scoring and DONE behavior
packages/nbc-market-engine/   transitional provenance and differential oracle during integration
packages/market-types/        shared checked units and identities where semantics match
packages/market-events/       shared canonical envelopes where semantics match
schemas/nbc/                  versioned translated configuration/protocol schemas
tests/conformance/nbc/        selected-JAR versus Rust fixtures
apps/trpc-api/                native Rust tRPC profile/configuration boundary
```

## Required provenance fields in canonical scenarios

Each transcribed scenario must include:

- canonical scenario ID and version;
- canonical schema version;
- source repository path;
- source Git blob SHA;
- source scenario ID and name;
- transcription timestamp and authoring commit;
- explicit unit mapping for every converted field;
- PRNG algorithm and derivation version;
- a list of unresolved or redesigned parameters;
- a statement of whether each agent implementation is recovered, literature-derived or Bunting-designed.

## Transcription sequence

1. Verify ADR 0017 authority and record source/JAR provenance for imported scenario data.
2. Define strict `scenario-schema` types with `deny_unknown_fields` behavior.
3. Define exact logical-time, tick, lot and probability representations.
4. Create `source-manifest.json` containing only provenance and hashes.
5. Transcribe `normal_market` first without enabling agents.
6. Validate all static values and unit conversions.
7. Implement and enable fundamental and noise agents behind explicit model versions.
8. Add market maker, momentum, institutional and spiking agents incrementally.
9. Transcribe the remaining four scenarios only when every referenced model version exists.
10. Record distributional comparison results without claiming Java runtime equivalence.

## Verification requirements

- The imported file still hashes to the recorded Git blob SHA.
- The duplicate file under `ref/ritc_mm` has the same blob SHA.
- Canonical exact decimals round-trip to expected ticks/lots without floating authority.
- Unknown fields and invalid distributions reject.
- Same canonical scenario version, seed and commands produce identical event bytes and state hashes.
- Adding an unrelated agent does not alter another agent's random stream.
- Snapshot/restore preserves pending scheduled items and every PRNG stream.
- Legacy scenario aliases resolve only in `protocol-legacy-nbc`.

## Current status

- Source files inventoried: complete for five scenario families.
- Duplicate hashes verified: complete for the five listed files.
- External evidence manifest: complete in `docs/ports/nbc-evidence-manifest.v1.json`.
- External contract fixture manifest: complete for documentation-derived and client-corroborated cases; no black-box traces are recorded.
- Translation/redistribution authority: complete under ADR 0017; original upstream license metadata remains unresolved and recorded.
- Unit semantics: pending.
- Canonical schema: not implemented.
- Canonical scenario JSON: intentionally not translated yet.
- Agent algorithms: not implemented.
