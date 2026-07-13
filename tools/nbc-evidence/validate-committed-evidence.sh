#!/bin/sh
set -eu
. "$(dirname "$0")/common.sh"
verify_reference

for json in \
  docs/ports/nbc-evidence-manifest.v1.json \
  tests/fixtures/nbc/external-contract-manifest.v1.json \
  tests/fixtures/nbc/runtime/scenario-list.v1.json; do
  jq -e . "$json" >/dev/null
done

class_count=0
while IFS="$(printf '\t')" read -r expected entry role disposition; do
  test "$expected" = sha256 && continue
  actual=$(unzip -p "$JAR" "$entry" | shasum -a 256 | awk '{print $1}')
  test "$actual" = "$expected"
  case "$entry" in
    BOOT-INF/classes/ca/mc/exchange_simulator/*.class|BOOT-INF/classes/ca/mc/exchange_simulator/**/*.class)
      class_count=$((class_count + 1)) ;;
  esac
  test -n "$role"
  test -n "$disposition"
done < docs/ports/nbc-jar-inventory.v1.tsv
test "$class_count" -eq 40

test "$(jq -r '.jar_sha256' tests/fixtures/nbc/runtime/scenario-list.v1.json)" = "$EXPECTED_JAR_SHA256"
test "$(jq -r '.gitlink' tests/fixtures/nbc/runtime/scenario-list.v1.json)" = "$EXPECTED_GITLINK"
test "$(jq -r '.response.status' tests/fixtures/nbc/runtime/scenario-list.v1.json)" -eq 200
