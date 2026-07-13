#!/bin/sh
set -eu

EXPECTED_GITLINK=35b8050546679547dc737198ea13aa0ec8ed7db8
EXPECTED_JAR_SHA256=80afc2816970b2538dcaff808008bfebdce5426ac248c074859626605547e254
SUBMODULE=ref/nbc-hft-simulation
JAR="$SUBMODULE/app/exchange-simulator-0.0.1-SNAPSHOT.jar"
OUT=out/nbc-evidence

verify_reference() {
  test "$(git ls-tree HEAD "$SUBMODULE" | awk '{print $3}')" = "$EXPECTED_GITLINK"
  test "$(git -C "$SUBMODULE" rev-parse HEAD)" = "$EXPECTED_GITLINK"
  test "$(shasum -a 256 "$JAR" | awk '{print $1}')" = "$EXPECTED_JAR_SHA256"
}

prepare_output() {
  mkdir -p "$OUT"
  chmod 700 "$OUT"
}
