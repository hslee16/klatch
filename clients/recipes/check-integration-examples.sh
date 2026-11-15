#!/usr/bin/env bash
# Runs the examples of INTEGRATION.md against the real binary (#101), so the guide cannot drift from what klatch does.
# No network and no model: a throwaway home, two members, a few seconds.
#
#   KLATCH_BIN=/path/to/klatch clients/recipes/check-integration-examples.sh
#
# It checks that each example (a) reads the history another member left, (b) posts and gets a seq back, (c) resumes with
# `from_seq` and sees only its own post, (d) shows `verified` when started with a worker key bundle, and that the guide
# contains every example file verbatim, so what a reader copies is what was run.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${KLATCH_BIN:-$ROOT/proto/target/debug/klatch}"
EX="$ROOT/clients/examples"
GUIDE="$ROOT/INTEGRATION.md"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/klatch-guide.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export KLATCH_HOME="$WORK/home" KLATCH_KEY_DIR="$WORK/keys" KLATCH_BIN="$BIN"
unset KLATCH_IDENTITY_FILE KLATCH_DISCOVER KLATCH_NAME

fail() { echo "FAIL  $*" >&2; exit 1; }
expect() { # expect <output> <text> <what>
  case "$1" in *"$2"*) printf '  ok  %s\n' "$3" ;; *) printf '%s\n' "$1" >&2; fail "$3 (wanted: $2)" ;; esac
}

[ -f "$GUIDE" ] || fail "no $GUIDE"
KLATCH_NAME=alice "$BIN" join lobby > /dev/null 2>&1
KLATCH_NAME=alice "$BIN" post lobby "hello from alice" > /dev/null

run_example() { # run_example <label> <member> <command...>
  local label="$1" member="$2"
  shift 2
  local out
  out="$(KLATCH_NAME="$member" "$@" lobby 2>&1)" || { printf '%s\n' "$out" >&2; fail "$label exited with an error"; }
  expect "$out" "history: alice: hello from alice" "$label reads the history"
  expect "$out" "posted: seq" "$label posts"
  expect "$out" "resumed: $member: hello from $member" "$label resumes and sees only its own post"
}

[ -f "$EX/room_bot.py" ] || fail "no $EX/room_bot.py"
run_example "python" bot-py python3 -I "$EX/room_bot.py"

if command -v node > /dev/null 2>&1 && node --experimental-strip-types -e 0 > /dev/null 2>&1; then
  [ -f "$EX/room_bot.ts" ] || fail "no $EX/room_bot.ts"
  run_example "typescript" bot-ts node --no-warnings --experimental-strip-types "$EX/room_bot.ts"
else
  echo "  skip typescript (needs Node 22.6 or newer)"
fi

# authentication: a launcher issues a bundle, the member is told by the path only, and its posts read as verified
# (a home with no harness registry: a launcher is never itself a coding-agent session, and issuing refuses inside one)
mkdir "$WORK/fakehome"
HOME="$WORK/fakehome" "$BIN" worker issue w-guide --clearance internal --ttl 600 --out "$WORK/w-guide.bundle" > /dev/null 2>&1 \
  || fail "worker issue failed"
out="$(KLATCH_IDENTITY_FILE="$WORK/w-guide.bundle" python3 -I "$EX/room_bot.py" lobby 2>&1)" \
  || { printf '%s\n' "$out" >&2; fail "the bundle run exited with an error"; }
expect "$out" "member: w-guide (verified)" "a key bundle makes the member verified"

# the guide quotes each example whole
for f in "$EX"/room_bot.py "$EX"/room_bot.ts; do
  python3 -I - "$GUIDE" "$f" << 'PY' || fail "INTEGRATION.md does not contain $(basename "$f") verbatim"
import sys
guide, example = (open(p, encoding="utf-8").read() for p in sys.argv[1:3])
sys.exit(0 if example in guide else 1)
PY
  printf '  ok  the guide contains %s verbatim\n' "$(basename "$f")"
done
echo "integration examples ok"
