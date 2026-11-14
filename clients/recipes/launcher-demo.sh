#!/usr/bin/env bash
# A launcher's whole flow against the real binary, with no model: open a run, issue three workers, have them hand
# work to each other and contend for a path, release it by reporting the work item done, and end the run with an
# encrypted archive. Every step is checked; the script exits non-zero at the first one that is not true.
#
#   KLATCH_BIN=/path/to/klatch clients/recipes/launcher-demo.sh
#
# A launcher is a script or a factory, never a coding-agent session and never a worker; that is how the gateway tells
# it apart from an agent, so the demo runs with no harness registry in its home.
set -euo pipefail

BIN="${KLATCH_BIN:-$(cd "$(dirname "$0")/../.." && pwd)/proto/target/debug/klatch}"
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/klatch-demo.XXXXXX")"
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/home" "$ROOT/fake-home" "$ROOT/bundles"

# the launcher: no name, no harness registry, the key directory only a launcher has
launcher() { env -i PATH="$PATH" HOME="$ROOT/fake-home" KLATCH_HOME="$ROOT/home" KLATCH_KEY_DIR="$ROOT/keys" "$BIN" "$@"; }
# a worker: only the path of its key bundle, no name, no key directory
worker() { local n="$1"; shift; env -i PATH="$PATH" HOME="$ROOT/fake-home" KLATCH_HOME="$ROOT/home" KLATCH_IDENTITY_FILE="$ROOT/bundles/w-$n.bundle" "$BIN" "$@"; }
step() { printf '  ok  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1" >&2; exit 1; }
expect() { grep -q -- "$2" <<<"$1" || fail "$3 (wanted '$2', got: $1)"; }

echo "launcher demo with $BIN"
launcher run create demo >/dev/null;                                         step "opened run demo"
launcher worker issue --count 3 --prefix w --clearance internal --parent factory --ttl 600 \
  --out-dir "$ROOT/bundles" >/dev/null;                                      step "issued workers w-1 to w-3 in one call"

worker 1 post --kind work-order --payload '{"item":"wo:1","files":["src/a.rs"]}' run.demo.main "w-1 hands w-2 a work order" >/dev/null
seen="$(worker 2 read run.demo.main --peek 2>/dev/null)"
expect "$seen" "w-1 hands w-2" "w-2 did not see the handoff"
expect "$seen" "handoff work-order" "the handoff lost its kind"
expect "$seen" '"item":"wo:1"' "the handoff lost its payload"
case "$seen" in *"(unverified)"*) fail "a credentialed worker's message was marked unverified";; esac
step "a handoff travels with its kind and payload, from a verified sender"

worker 1 claim run.demo.main src/a.rs --anchor wo:1 >/dev/null
if worker 2 claim run.demo.main src/a.rs >/dev/null 2>&1; then fail "two workers held one path"; fi
step "a contested path has one holder"

out="$(launcher anchor done wo:1)"
expect "$out" "released 1" "the work item was not released"
worker 2 claim run.demo.main src/a.rs >/dev/null;                            step "reporting the item done freed the path for the next worker"

launcher run end demo --archive "$ROOT/demo.archive" >/dev/null
if grep -q "w-1 hands w-2" "$ROOT/demo.archive"; then fail "the archive is readable"; fi
step "the run ended and its archive is encrypted"
refused="$(worker 3 post run.demo.main "late" 2>&1 || true)"
expect "$refused" "not open" "a torn-down run came back"
step "its rooms are gone and cannot come back"

launcher worker revoke w-3 >/dev/null
revoked="$(worker 3 post lobby "after revoke" 2>&1 || true)"
expect "$revoked" "revoked" "a revoked worker still posted"
step "a revoked worker is stopped at once"
echo "PASS"
