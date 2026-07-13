# Reference inventory

References are commit-pinned research and provenance. Production manifests use released crates.

## Production implementation references

| Path | Upstream pin | License | Status |
|---|---|---|---|
| `ref/orderbook-rs` | `575de34260b0fce346372074b6b938df058693a8` | MIT | production crate `0.10.3` |
| `ref/pricelevel` | `a5b61671391295783d0e35ba68fdf4a9702dee60` | MIT | production transitive crate `0.8.4` |
| `ref/workers-rs` | `5f2d6c9192377451d43910098738624474196364` | Apache-2.0 | production Worker/Cache runtime |

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
- `ref/nexosim`, `ref/abides`, NBC assets: scheduler, agents, and scenarios.

## Authorized translation source

- `ref/nbc-hft-simulation` at `35b8050546679547dc737198ea13aa0ec8ed7db8`: contains the selected NBC JAR authorized by ADR 0017 for inspection, Rust translation and redistribution; exact JAR hash and authority are recorded in `docs/ports/nbc-evidence-manifest.v1.json`.

## Pending protocol conformance intake

- tRPC `11.18.0`, source git head `6aec1578a899df50a17e4e78d5512a099b574c18`, MIT: planned development-only wire oracle under ADR 0016; Sprint 0 must complete the source/manifests/entrypoint audit before adoption.

## Historical implementation candidates

`slotmap` and `intrusive-rs` were added for a planned custom book. ADR 0013 makes that implementation obsolete; retain them only as general data-structure references.

See `docs/reference-adoption.md` and `docs/joaquin-repository-audit.md` for binding decisions.
