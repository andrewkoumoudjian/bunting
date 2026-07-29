#!/bin/sh
set -eu

if ! command -v cargo-wasix >/dev/null 2>&1; then
  echo "cargo-wasix 0.1.28 is required" >&2
  exit 1
fi
if ! command -v wasmer >/dev/null 2>&1; then
  echo "Wasmer 7.2.1 is required" >&2
  exit 1
fi

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo"

cargo +wasix build --locked --release \
  --target wasm32-wasmer-wasi-dl \
  -p bunting-server \
  --bin bunting-server

wasm="$repo/target/wasm32-wasmer-wasi-dl/release/bunting-server.wasm"
compiled="$repo/target/wasm32-wasmer-wasi-dl/release/bunting-server.wasmu"
wasmer validate "$wasm"
wasmer compile --cranelift -o "$compiled" "$wasm"

printf '%s\n' "$wasm" "$compiled"
