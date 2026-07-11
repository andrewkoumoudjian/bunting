#!/usr/bin/env bash
# Integration test runner: starts the engine, runs every strategy N times in
# parallel, validates the resulting databases
#
# Usage: bash ./scripts/db_order_debug.sh <client_run_count> <orders_per_client>
#   client_run_count, how many times to run each strategy (stress-test concurrency)
#   orders_per_client, --iterations value passed to each client run
set -euo pipefail

ENGINE_CMD=(./build/engine-cpp/src/trading_engine python_client/config.yaml)
VALIDATOR_CMD=(python3 engine-cpp/tests/scripts/validate_dbs.py .)

CLIENT_RUN_AMT=$1
PYTHON_ORDER_RANGE=$2

CLIENT="python3 python_client/client.py --server localhost:50051"
ENGINE_PID=""

cleanup() {
  if [[ -n "${ENGINE_PID:-}" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    echo "Stopping engine (pid=$ENGINE_PID)…"
    kill -INT "$ENGINE_PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$ENGINE_PID" 2>/dev/null || { wait "$ENGINE_PID" 2>/dev/null || true; return; }
      sleep 0.25
    done
    kill -TERM "$ENGINE_PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$ENGINE_PID" 2>/dev/null || { wait "$ENGINE_PID" 2>/dev/null || true; return; }
      sleep 0.25
    done
    kill -KILL "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

"${ENGINE_CMD[@]}" &
ENGINE_PID=$!

# Wait for the engine to be ready
sleep 1

# Discover how many strategies are configured
NUM_STRATEGIES=$(python3 -c "
import yaml
with open('python_client/config.yaml') as f:
    cfg = yaml.safe_load(f)
print(len(cfg.get('strategies', [])))
")

if (( NUM_STRATEGIES == 0 )); then
  echo "No strategies found in config.yaml" >&2
  exit 1
fi

client_pids=()
client_names=()

for n in $(seq 1 "$NUM_STRATEGIES"); do
  for ((run = 1; run <= CLIENT_RUN_AMT; ++run)); do
    $CLIENT run "$n" --iterations "$PYTHON_ORDER_RANGE" --no-flatten &
    pid=$!
    client_pids+=("$pid")
    client_names+=("strategy $n [run $run]")
  done
done

client_failed=0
for i in "${!client_pids[@]}"; do
  pid="${client_pids[$i]}"
  name="${client_names[$i]}"
  if wait "$pid"; then
    echo "OK   - $name"
  else
    echo "FAIL - $name (exit code $?)"
    client_failed=1
  fi
done

cleanup
trap - EXIT

echo "Running DB validation…"
if "${VALIDATOR_CMD[@]}"; then
  echo "Validation passed"

  mapfile -t db_files < <(find . -type f -name '*.db' | sort)
  if (( ${#db_files[@]} == 0 )); then
    echo "No .db files found"
  else
    echo ""
    echo "DB files:"
    for db in "${db_files[@]}"; do echo "  $db"; done
    echo ""
    read -r -p "Delete all .db files? [y/N] " answer
    if [[ "$answer" =~ ^[Yy]$ ]]; then
      rm -f -- "${db_files[@]}"
      echo "Deleted ${#db_files[@]} database file(s)"
    fi
  fi
else
  echo "Validation failed"
  exit 1
fi

if (( client_failed != 0 )); then
  echo "One or more clients failed"
  exit 1
fi
