#!/usr/bin/env bash
# dev.sh, start the engine and run test clients in one command.
#
# Usage:
#   ./scripts/dev.sh                          # all strategies, paper trading, sequential
#   ./scripts/dev.sh --gateway alpaca         # override gateway for every strategy
#   ./scripts/dev.sh --strategy 1             # single strategy
#   ./scripts/dev.sh --parallel               # run all strategies concurrently
#   ./scripts/dev.sh --iterations 10          # fewer signal cycles
#   ./scripts/dev.sh --build                  # rebuild engine before starting
#   ./scripts/dev.sh --validate               # run DB validation after clients finish
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ---- defaults ----------------------------------------------------------------
GATEWAY=""
STRATEGY=""
ITERATIONS=""
PARALLEL=false
BUILD=false
VALIDATE=false
SERVER="localhost:50051"
PORT=50051
CONFIG="$REPO_ROOT/python_client/config.yaml"

# ---- parse args --------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case $1 in
    --gateway)    GATEWAY="$2";    shift 2 ;;
    --strategy)   STRATEGY="$2";   shift 2 ;;
    --iterations) ITERATIONS="$2"; shift 2 ;;
    --parallel)   PARALLEL=true;   shift   ;;
    --build)      BUILD=true;      shift   ;;
    --validate)   VALIDATE=true;   shift   ;;
    --config)     CONFIG="$2";     shift 2 ;;
    -h|--help)
      sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

ENGINE_BIN="$REPO_ROOT/build/engine-cpp/src/trading_engine"
CLIENT="python3 $REPO_ROOT/python_client/client.py --server $SERVER --config $CONFIG"

# ---- build -------------------------------------------------------------------
if [[ "$BUILD" == true ]]; then
  echo "Building engine…"
  cmake --build "$REPO_ROOT/build" -j
fi

if [[ ! -x "$ENGINE_BIN" ]]; then
  echo "error: engine not found at $ENGINE_BIN" >&2
  echo "       Run with --build, or: cmake -S . -B build && cmake --build build -j" >&2
  exit 1
fi

# ---- optional gateway override (writes a temp config for the engine) ---------
TMPCONFIG=""
ENGINE_CONFIG="$CONFIG"
if [[ -n "$GATEWAY" ]]; then
  TMPCONFIG=$(mktemp /tmp/quarcc_XXXXXX.yaml)
  $CLIENT gen-config --gateway "$GATEWAY" > "$TMPCONFIG"
  ENGINE_CONFIG="$TMPCONFIG"
  echo "Gateway overridden → $GATEWAY"
fi

# ---- cleanup -----------------------------------------------------------------
ENGINE_PID=""

cleanup() {
  if [[ -n "${ENGINE_PID:-}" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    echo "Stopping engine (pid=$ENGINE_PID)…"
    kill -INT "$ENGINE_PID" 2>/dev/null || true
    for _ in {1..20}; do
      kill -0 "$ENGINE_PID" 2>/dev/null || { wait "$ENGINE_PID" 2>/dev/null || true; break; }
      sleep 0.25
    done
    kill -TERM "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  [[ -n "$TMPCONFIG" ]] && rm -f "$TMPCONFIG"
}
trap cleanup EXIT

# ---- start engine ------------------------------------------------------------
echo "Starting engine → $ENGINE_CONFIG"
"$ENGINE_BIN" "$ENGINE_CONFIG" &
ENGINE_PID=$!

sleep 3

# ---- run clients -------------------------------------------------------------
EXTRA=""
[[ -n "$ITERATIONS" ]] && EXTRA="$EXTRA --iterations $ITERATIONS"
[[ "$PARALLEL" == true ]] && EXTRA="$EXTRA --parallel"

if [[ -n "$STRATEGY" ]]; then
  $CLIENT run "$STRATEGY" $EXTRA
else
  $CLIENT run-all $EXTRA
fi

# ---- final positions ---------------------------------------------------------
echo ""
echo "--- positions ---"
$CLIENT positions

# ---- optional validation -----------------------------------------------------
if [[ "$VALIDATE" == true ]]; then
  echo ""
  echo "Running DB validation…"
  python3 "$REPO_ROOT/engine-cpp/tests/scripts/validate_dbs.py" "$REPO_ROOT"
fi
