#!/bin/sh
set -eu
. "$(dirname "$0")/common.sh"
verify_reference
prepare_output

entries="$OUT/archive-entries.txt"
hashes="$OUT/archive-entry-sha256.tsv"
unzip -Z1 "$JAR" > "$entries"
entry_count=$(wc -l < "$entries" | tr -d ' ')
test "$entry_count" -le 10000
if LC_ALL=C grep -Eq '(^/|(^|/)\.\.(/|$)|\\)' "$entries"; then
  echo "unsafe archive path" >&2
  exit 1
fi

: > "$hashes"
while IFS= read -r entry; do
  case "$entry" in
    */) continue ;;
  esac
  hash=$(unzip -p "$JAR" "$entry" | shasum -a 256 | awk '{print $1}')
  printf '%s\t%s\n' "$hash" "$entry" >> "$hashes"
done < "$entries"

{
  printf 'jar_sha256\t%s\n' "$EXPECTED_JAR_SHA256"
  printf 'gitlink\t%s\n' "$EXPECTED_GITLINK"
  printf 'entry_count\t%s\n' "$entry_count"
  printf 'application_class_count\t%s\n' "$(grep -Ec '^BOOT-INF/classes/ca/mc/exchange_simulator/.*\.class$' "$entries")"
  printf 'bundled_library_count\t%s\n' "$(grep -Ec '^BOOT-INF/lib/.*\.jar$' "$entries")"
  printf 'java_version\t%s\n' "$(java -version 2>&1 | head -1)"
  printf 'javap_version\t%s\n' "$(javap -version 2>&1 | head -1)"
  printf 'jar_tool_version\t%s\n' "$(jar --version 2>&1 | head -1)"
  printf 'inventory_command\ttools/nbc-evidence/inventory.sh\n'
} > "$OUT/inventory-summary.tsv"
