# Reference inventory

References are commit-pinned research and provenance. Production manifests use released crates.

## Production implementation references

| Path | Upstream pin | License | Status |
|---|---|---|---|
| `ref/orderbook-rs` | `575de34260b0fce346372074b6b938df058693a8` | MIT | production crate `0.10.3` |
| `ref/pricelevel` | `a5b61671391295783d0e35ba68fdf4a9702dee60` | MIT | production transitive crate `0.8.4` |
| `ref/workers-rs` | `5f2d6c9192377451d43910098738624474196364` | Apache-2.0 | production Worker/Cache runtime |

The production server runtime is not mirrored under `ref/`. ADR 0027 pins
Wasmer `7.2.1` at `c14032594b893b40e9b71456d504cf55c141c8f6`,
cargo-wasix `0.1.28` at `b2d0e1c874fc6ac5dbaf71715b12c6809104767f`,
and WASIX Rust toolchain `v2026-07-07.3+rust-1.96`.

## Joaquín ecosystem references

| Path | Pin | Role |
|---|---|---|
| `ref/option-chain-orderbook` | `19e8e45bf122c3ebe3e1784f73e04adba2781ea6` | future options hierarchy built on OrderBook-rs |
| `ref/market-maker-rs` | repository gitlink | market-making formulas and strategy decomposition |
| `ref/ironsbe` | `cf365e4815c04ff31acd81568952e9ff477c6d89` | future SBE codec/schema/codegen candidate |
| `ref/fauxchange` | `293bdc52bedc816f76da5db106f44535e4438593` | design intent only; no implementation exists |
| `ref/ironfix` | repository gitlink | FIX codec candidate |

## Independent oracles and contracts

- `ref/liquibook`: matching behavior.
- `ref/exchange-core`: accounting, risk, atomicity, and state hashes.
- `ref/quickfixj`, `ref/fixer`, `ref/ferrumfix`: FIX conformance and layering.
- `ref/nautilus-trader`: adapter contract.
- `ref/barter-rs`: OMS/risk/audit architecture.
- `ref/abides` and NBC assets: scheduler, agents, and scenarios.

## Authorized translation source

- `ref/nbc-hft-simulation` at `35b8050546679547dc737198ea13aa0ec8ed7db8`: contains the selected NBC JAR authorized by ADR 0017 for inspection, Rust translation and redistribution; exact JAR hash and authority are recorded in `docs/ports/nbc-evidence-manifest.v1.json`.

## Deregistered published-crate mirrors

The following gitlinks were removed on 2026-07-28 because they carried no unique evidence beyond their published crates and upstream repositories. Their exact evidence identities remain recorded in `docs/reference-functionality-audit.md`: `cqrs`, `nexosim`, `wirefilter`, `slotmap`, `intrusive-rs`, `rand`, `postcard`, and `proptest`.

The superseded tRPC `11.18.0` fixture oracle was removed at the same time. Its historical source identity remains in archived ADR and plan text, but it is no longer an active conformance input or toolchain dependency.

See `docs/reference-adoption.md` and `docs/joaquin-repository-audit.md` for binding decisions.
