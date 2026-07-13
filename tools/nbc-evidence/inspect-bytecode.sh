#!/bin/sh
set -eu
. "$(dirname "$0")/common.sh"
verify_reference
prepare_output
"$(dirname "$0")/inventory.sh"

classes='ca.mc.exchange_simulator.scenario.ScenarioConfig
ca.mc.exchange_simulator.core.OrderBook
ca.mc.exchange_simulator.core.Order
ca.mc.exchange_simulator.core.CancelOrder
ca.mc.exchange_simulator.core.Fill
ca.mc.exchange_simulator.core.DeterministicRandom
ca.mc.exchange_simulator.simulator.EventManager
ca.mc.exchange_simulator.simulator.SimulationContext
ca.mc.exchange_simulator.simulator.SimulationEngine
ca.mc.exchange_simulator.simulator.SimulationEvent
ca.mc.exchange_simulator.websocket.MarketDataHandler
ca.mc.exchange_simulator.websocket.OrderEntryHandler
ca.mc.exchange_simulator.service.MetricsCalculator'

rm -rf "$OUT/classes"
mkdir -p "$OUT/classes"
printf '%s\n' "$classes" | while IFS= read -r class; do
  entry="BOOT-INF/classes/$(printf '%s' "$class" | tr . /).class"
  unzip -p "$JAR" "$entry" > "$OUT/classes/${class##*.}.class"
  test "$(wc -c < "$OUT/classes/${class##*.}.class")" -le 1048576
  javap -classpath "$OUT/classes" -p -c -s "$OUT/classes/${class##*.}.class" \
    > "$OUT/${class##*.}.javap.txt"
done

for resource in application.yaml application.yml schema.sql \
  scenarios/flash.json scenarios/hft_dominated.json \
  scenarios/mini_flash_crash.json scenarios/normal_market.json \
  scenarios/stressed_market.json; do
  unzip -p "$JAR" "BOOT-INF/classes/$resource" > "$OUT/$(basename "$resource").resource"
  test "$(wc -c < "$OUT/$(basename "$resource").resource")" -le 1048576
done
