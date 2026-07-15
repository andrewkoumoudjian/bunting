# Product contract worktree handoff

## Branch and base

- Branch: `codex/product-contract`
- Base: `origin/main` at `9c53967943d6d3b556a40b7fce9f6edc8ab3b3b3`

## Decisions made

- One sans-I/O Rust application service surrounds the unified `bunting-engine`;
  transports call it in process and never own market truth.
- Native deployments may accept FIX/TCP or FIX/TLS. Cloudflare accepts no raw
  TCP and initiates an outbound bidirectional FIX session to an external
  acceptor/session relay.
- Verified actor roles and deny-by-default public/participant/team/instructor/
  administrator audiences govern every projection. Built-in agents are
  participant-scoped service identities using QUARCC.
- Published scenarios are immutable; runs pin all behavior versions and use
  logical time, optimistic versions, commit-before-ack and cursor-based reset.
- `bunting.fix44.competition.v1` is the complete participant interface: every
  competition-visible fact and participant action must be available through
  FIX alone.
- Hidden RIT formulas remain unresolved and require named, versioned Bunting
  policies. Static proprietary evidence supplies clean-room surfaces only.

## Files changed

- `docs/specs/bunting-product-contract.md`
- `docs/specs/bunting-fix-competition-profile.md`
- `docs/specs/rit-tui-parity-matrix.md`
- `schemas/product/bunting.product.v1.json`
- `schemas/fix/bunting.fix44.competition.v1.json`
- `packages/bunting-api-contract/src/lib.rs`
- `docs/plans/corrected-bunting-implementation-plan.md`
- `docs/plans/worktree-handoffs/product-contract.md`

## Interfaces downstream lanes must use

- Product version: `bunting.product.v1` / Rust
  `PRODUCT_CONTRACT_VERSION`.
- FIX version: `bunting.fix44.competition.v1` / Rust
  `FIX_COMPETITION_PROFILE_VERSION`.
- Rust identity boundary: `ActorRole`, `ActorIdentity`, `Audience` and
  `audience_allows` in `bunting-api-contract`.
- Bunting FIX extension range and exact message inventory in
  `schemas/fix/bunting.fix44.competition.v1.json`.
- Every application mutation carries authenticated actor, command/correlation
  ID, expected run version and logical time; every output carries audience and
  committed sequence.
- Ratatui lanes implement rows in `rit-tui-parity-matrix.md` over the FIX or
  transport-neutral service contract; they do not add UI-only competition
  capabilities.

## Unresolved questions

- Exact RIT matching/validation/timing, accounting/valuation/risk/scoring,
  tender/news/agent, OTC, asset/facility, product/settlement and recovery
  formulas remain unresolved.
- Exact RIT status/null/error, throttle/delay, RTD refresh and high-topic-count
  behavior remain unresolved.
- External FIX gateway product/hosting, TLS trust model and two independent
  interoperability acceptors remain implementation decisions within this
  topology.
- Standard FIX component/group selections for each versioned `U*` payload must
  be finalized as each feature is implemented; generic unversioned JSON is
  prohibited.

## Verification actually run

All commands ran from the product-contract worktree with
`CARGO_TARGET_DIR=/Users/andrewkoumoudjian/Documents/QUARCC/bunting/target`:

- `cargo metadata --locked --format-version 1 --no-deps` — passed.
- `cargo fmt --all --check` — passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` — passed.
- `cargo test --locked --workspace` — passed.
- `cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'` — passed and printed `orderbook-rs v0.10.3`.
- `cargo check --locked --workspace --target wasm32-unknown-unknown` — passed.
- `git diff --check` — passed.
- Focused JSON/schema/audience checks: `jq empty` on both new schemas and
  `cargo test --locked -p bunting-api-contract -p simfix-wire` — passed.
