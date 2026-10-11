#!/usr/bin/env bash
# Gates for the desktop app, which is its own Cargo workspace (ADR 0050).
# The root workspace's seven gates do not build it.
set -euo pipefail
cd "$(dirname "$0")/../apps/bunting-desk"
cargo metadata --locked --format-version 1 --no-deps > /dev/null
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
