#!/bin/sh
# Native-versus-WASIX host parity and loopback latency (ADR 0044).
#
# 1. Runs the native server with the durable file origin and built-in
#    agents, drives it with tools/host_load.py, and exports the run's
#    journal as a competition archive.
# 2. Replays that archive with the native and the WASIX build of
#    bunting-rs/examples/replay_archive.rs and requires identical output
#    (command and event counts, journal chain, state hash, scores).
# 3. Times order acknowledgements on the native server (file and memory
#    origin) and the Wasmer-hosted server (memory origin; the WASIX build has
#    no durable origin, see ADR 0044).
#
# Requires: a release build of bunting-server, bunting and the replay_archive
# example; cargo +wasix (toolchain v2026-07-07.3+rust-1.96); wasmer 7.2.1;
# python3. Ports 9880 and 8080 must be free.
#
# usage: tools/host_parity.sh [orders]
set -eu

orders=${1:-2000}
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
release="$repo/target/release"
wasix="$repo/target/wasm32-wasmer-wasi-dl/release"
work=$(mktemp -d)
server_pid=
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT HUP INT TERM

for binary in "$release/bunting-server" "$release/bunting" "$release/examples/replay_archive"; do
  test -x "$binary" || { echo "missing $binary; run cargo build --release first" >&2; exit 1; }
done

if python3 -c 'import socket; socket.create_connection(("127.0.0.1", 8080), timeout=1)' 2>/dev/null; then
  echo "port 8080 is in use; stop the running server first" >&2
  exit 1
fi

cd "$repo"
cargo +wasix build --locked --release --target wasm32-wasmer-wasi-dl \
  -p bunting-server --bin bunting-server -p bunting-rs --example replay_archive >&2

# Writes $work/<name>/local.json with the requested storage kind.
configure() {
  mkdir -p "$work/$1"
  cp apps/bunting-server/config/scenario.json "$work/$1/"
  python3 - "$work/$1/local.json" "$2" <<'EOF'
import json, sys
config = json.load(open("apps/bunting-server/config/local.json"))
# Measure the host, not the participant rate limit.
config["fix"]["max_messages_per_interval"] = 1000000
if sys.argv[2] == "memory":
    config["storage"]["kind"] = "memory"
    config["storage"]["path"] = None
json.dump(config, open(sys.argv[1], "w"))
EOF
}

wait_health() {
  for _ in $(seq 1 80); do
    kill -0 "$server_pid" 2>/dev/null || { cat "$work/$name/server.log" >&2; return 1; }
    if python3 -c 'import urllib.request; urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=1)' 2>/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  echo "server did not become healthy" >&2
  return 1
}

# usage: drive <name> <command...>; prints the load summary as JSON.
drive() {
  name=$1
  shift
  (cd "$work/$name" && exec "$@" local.json >server.log 2>&1) &
  server_pid=$!
  wait_health
  python3 tools/host_load.py 127.0.0.1:9880 "$orders" --json
  kill "$server_pid"
  wait "$server_pid" 2>/dev/null || true
  server_pid=
}

configure native-file file
configure native-memory memory
configure wasix-memory memory

# WASIX runs first: its listener does not set SO_REUSEADDR, so it cannot
# bind while the previous server's connections are in TIME_WAIT.
wasix_memory=$(drive wasix-memory wasmer run "$wasix/bunting-server.wasm" --net \
  --volume "$work/wasix-memory:$work/wasix-memory" --cwd "$work/wasix-memory" --)
native_file=$(drive native-file "$release/bunting-server")
(cd "$work/native-file" && "$release/bunting" export-archive local.json archive.json >&2)
native_memory=$(drive native-memory "$release/bunting-server")

archive_dir="$work/native-file"
native_replay=$("$release/examples/replay_archive" "$archive_dir/archive.json")
wasix_replay=$(wasmer run "$wasix/examples/replay_archive.wasm" \
  --volume "$archive_dir:$archive_dir" -- "$archive_dir/archive.json")

echo "native replay: $native_replay"
echo "wasix replay:  $wasix_replay"
if [ "$native_replay" != "$wasix_replay" ]; then
  echo "PARITY FAILED: native and WASIX replays differ" >&2
  exit 1
fi
echo "parity: identical replay output"
echo "load native file origin:   $native_file"
echo "load native memory origin: $native_memory"
echo "load wasix memory origin:  $wasix_memory"
