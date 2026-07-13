#!/bin/sh
set -eu
. "$(dirname "$0")/common.sh"
verify_reference
prepare_output

runtime="$OUT/runtime"
rm -rf "$runtime"
mkdir -p "$runtime/home" "$runtime/tmp"
mkdir -p "$runtime/src/main/resources/scenarios"
chmod 700 "$runtime" "$runtime/home" "$runtime/tmp"
runtime_path="$PWD/$runtime"
log="$runtime_path/startup.log"
status="$runtime_path/http-status.tsv"
: > "$status"

jar_path="$PWD/$JAR"
for scenario in flash hft_dominated mini_flash_crash normal_market stressed_market; do
  unzip -p "$JAR" "BOOT-INF/classes/scenarios/$scenario.json" \
    > "$runtime/src/main/resources/scenarios/$scenario.json"
done

cd "$runtime"
env -i PATH=/usr/bin:/bin HOME="$runtime_path/home" TMPDIR="$runtime_path/tmp" \
  java -jar "$jar_path" --server.address=127.0.0.1 --server.port=18080 \
  --spring.datasource.url="jdbc:sqlite:$runtime_path/simulator.db" \
  > "$log" 2>&1 &
pid=$!
cleanup() {
  kill "$pid" 2>/dev/null || true
  sleep 1
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

attempt=0
while test "$attempt" -lt 45; do
  if ! kill -0 "$pid" 2>/dev/null; then
    break
  fi
  code=$(curl --noproxy '*' --silent --output replays.body \
    --write-out '%{http_code}' --max-time 1 http://127.0.0.1:18080/api/replays || true)
  if test "$code" != 000; then
    printf 'GET /api/replays\t%s\n' "$code" >> "$status"
    break
  fi
  attempt=$((attempt + 1))
  sleep 1
done

head -c 65536 startup.log > startup.bounded.log
mv startup.bounded.log startup.log
if test -f replays.body; then
  head -c 65536 replays.body > replays.bounded.body
  mv replays.bounded.body replays.body
fi
