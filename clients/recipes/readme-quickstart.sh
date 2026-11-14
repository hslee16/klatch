#!/usr/bin/env bash
# Runs the shell quickstart in README.md against the real binary (the fenced block after "Without an agent"), so the
# README cannot drift from what klatch does. A line whose comment says "refused" must fail; every other line must pass.
#
#   KLATCH_BIN=/path/to/klatch clients/recipes/readme-quickstart.sh [README.md]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
README="${1:-$ROOT/README.md}"
export BIN="${KLATCH_BIN:-$ROOT/proto/target/debug/klatch}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/klatch-readme.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export KLATCH_HOME="$WORK/home" KLATCH_KEY_DIR="$WORK/keys"

# the first fenced block after the line "Without an agent"
block="$(awk '/^Without an agent/ { seen = 1 } seen && /^```/ { n++; next } seen && n == 1 { print } n == 2 { exit }' "$README")"
[ -n "$block" ] || { echo "FAIL  no quickstart block found in $README" >&2; exit 1; }

count=0
while IFS= read -r line; do
  [ -n "$line" ] || continue
  count=$((count + 1))
  cmd="${line%%#*}"
  cmd="${cmd//klatch /\"\$BIN\" }"
  if [[ "$line" == *refused* ]]; then
    if eval "$cmd" >/dev/null 2>&1; then echo "FAIL  expected a refusal: $line" >&2; exit 1; fi
  else
    eval "$cmd" >/dev/null 2>&1 || { echo "FAIL  a quickstart line failed: $line" >&2; exit 1; }
  fi
  printf '  ok  %s\n' "${line%%#*}"
done <<<"$block"
[ "$count" -ge 8 ] || { echo "FAIL  only $count quickstart lines found" >&2; exit 1; }
echo "PASS ($count lines)"
