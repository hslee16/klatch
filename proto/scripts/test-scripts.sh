#!/usr/bin/env bash
# Tests for proto/scripts/gate and proto/scripts/install-git-hook (#403). The gate is driven with a fake `cargo` that
# records what it was asked to run and fails on demand, so the order of the steps, stopping at the first failure and
# the replay hint are checked without a real build.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
gate="$here/gate"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-gate-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

# a fake cargo: logs "args | seed | cases" and fails when the arguments contain $FAKE_FAIL
shim="$work/shim"; mkdir -p "$shim"
cat > "$shim/cargo" <<'SH'
#!/bin/sh
echo "$* | seed=${HEGEL_SEED:-} cases=${HEGEL_TEST_CASES:-}" >> "$FAKE_LOG"
case "$*" in *"$FAKE_FAIL"*) [ -n "$FAKE_FAIL" ] && { echo "fake failure in: $*"; exit 1; } ;; esac
exit 0
SH
chmod +x "$shim/cargo"
run_gate() { # fail-pattern, args...
  FAKE_LOG="$work/log"; : > "$FAKE_LOG"
  FAKE_FAIL="$1"; shift
  env PATH="$shim:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_FAIL="$FAKE_FAIL" "$gate" "$@" > "$work/out" 2>&1
}

# the seed the gate must derive from this commit
sha="$(git -C "$repo" rev-parse --short=8 HEAD)"
want_seed=$((16#$sha))

if "$gate" --help > "$work/out" 2>&1 && grep -q "Usage" "$work/out"; then ok "--help prints usage and exits 0"; else bad "--help"; fi
if "$gate" --no-such-flag > "$work/out" 2>&1; then bad "an unknown flag was accepted"; elif [ "$?" -eq 2 ]; then ok "an unknown flag exits 2"; else bad "unknown flag exit code"; fi

if run_gate "clippy" ; then bad "the gate passed although clippy failed"; else
  if grep -q "fmt" "$work/log" && grep -q "clippy" "$work/log" && ! grep -q "test --workspace" "$work/log" && grep -q "FAILED at step 2" "$work/out" && ! grep -q "GATE PASSED" "$work/out"; then ok "it stops at the first failing step (clippy, step 2) and runs nothing after it"; else bad "stop at first failure: $(cat "$work/log") // $(cat "$work/out")"; fi
fi

if run_gate "--test properties"; then bad "the gate passed although a property failed"; else
  if grep -q "FAILED at step" "$work/out" && grep -q "HEGEL_SEED=$want_seed" "$work/out" && grep -q "reproduce_failure" "$work/out"; then ok "a failing property prints the seed to replay it (derived from the commit) and mentions the blob"; else bad "replay hint: $(cat "$work/out")"; fi
fi

run_gate ""
if grep -q "GATE PASSED" "$work/out" && grep -q "seed=$want_seed" "$work/log"; then ok "with everything passing it says so, and every cargo step ran with the commit's seed"; else bad "pass: $(cat "$work/out")"; fi
steps=$(grep -c "^" "$work/log")
order_ok=1
prev=0
for pat in "fmt" "clippy" "test --workspace" "deny" "test-clock" "--test properties"; do
  n=$(grep -n -- "$pat" "$work/log" | head -1 | cut -d: -f1)
  if [ -z "${n:-}" ] || [ "$n" -le "$prev" ]; then order_ok=0; echo "    step '$pat' out of order or missing (line ${n:-none}, after $prev)"; fi
  prev=${n:-$prev}
done
if [ "$order_ok" -eq 1 ]; then ok "the steps run in order: fmt, clippy, tests, deny, test-clock, properties ($steps cargo calls)"; else bad "step order"; fi

run_gate "" --long
if grep -q "cases=5000" "$work/log"; then ok "--long raises the case budget"; else bad "--long: $(cat "$work/log")"; fi
run_gate ""
if grep -q "cases=500" "$work/log"; then ok "the default budget is small"; else bad "default budget: $(cat "$work/log")"; fi

# ---- the pre-push hook installer, in a throwaway repository -------------------------------------------------------
t="$work/repo"; mkdir -p "$t/proto/scripts"; git -C "$t" init -q
cp "$here/gate" "$here/install-git-hook" "$t/proto/scripts/"
hook="$t/.git/hooks/pre-push"
if "$t/proto/scripts/install-git-hook" > "$work/out" 2>&1 && [ -x "$hook" ] && grep -q "klatch gate" "$hook"; then ok "the installer writes an executable pre-push hook"; else bad "install: $(cat "$work/out")"; fi
before=$(cat "$hook")
if "$t/proto/scripts/install-git-hook" > "$work/out" 2>&1 && [ "$(cat "$hook")" = "$before" ]; then ok "installing twice changes nothing"; else bad "second install"; fi
"$t/proto/scripts/install-git-hook" --uninstall > "$work/out" 2>&1
if [ ! -e "$hook" ]; then ok "--uninstall removes the hook it wrote"; else bad "uninstall left the hook"; fi
printf '#!/bin/sh\necho mine\n' > "$hook"; chmod +x "$hook"
if "$t/proto/scripts/install-git-hook" > "$work/out" 2>&1; then bad "overwrote a hook it did not write"; elif grep -q "mine" "$hook" && grep -qi "already" "$work/out"; then ok "it refuses to overwrite a hook it did not write, and says so"; else bad "foreign hook: $(cat "$work/out")"; fi
"$t/proto/scripts/install-git-hook" --uninstall > "$work/out" 2>&1
if grep -q "mine" "$hook"; then ok "--uninstall leaves a hook it did not write"; else bad "uninstall removed a foreign hook"; fi

# pr-ledger: every PR gets exactly one stage and a next step (classification only, no network)
ledger="$here/pr-ledger"
stage_of() { printf '%s' "$1" | python3 -I "$ledger" --classify | cut -f1; }
expect_stage() { # json, expected, label
  got=$(stage_of "$1" 2>&1)
  if [ "$got" = "$2" ]; then ok "pr-ledger: $3"; else bad "pr-ledger: $3 (got '$got', wanted '$2')"; fi
}
G='{"name":"t","conclusion":"success"}'
expect_stage '{"checks":['"$G"'],"mergeable_state":"clean"}' ready "green and clean is ready to merge"
expect_stage '{"checks":['"$G"',{"name":"m","conclusion":null}]}' checks-running "an unfinished check is running"
expect_stage '{"checks":[{"name":"m","conclusion":null}],"idle_hours":9}' checks-running "a long-running check is still running (flagged stuck in the next step)"
expect_stage '{"checks":['"$G"',{"name":"m","conclusion":"cancelled"}]}' checks-cancelled "a cancelled job is not a failure"
expect_stage '{"checks":['"$G"',{"name":"m","conclusion":"failure"}]}' checks-red "a failed job is red"
expect_stage '{"checks":['"$G"',{"name":"m","conclusion":"failure"},{"name":"n","conclusion":"cancelled"}]}' checks-red "red wins over cancelled"
expect_stage '{"checks":['"$G"'],"mergeable_state":"dirty"}' conflict "a conflict needs an integrate"
expect_stage '{"checks":['"$G"'],"mergeable_state":"behind"}' behind "behind main needs an integrate"
expect_stage '{"checks":[]}' no-checks "no checks reported is flagged"
expect_stage '{"draft":true,"checks":['"$G"']}' draft "a draft is a draft"
expect_stage '{"merged":true,"ticket_state":"open"}' merged-open-ticket "merged with its ticket still open is not finished"
expect_stage '{"merged":true,"ticket_state":"closed"}' 'done' "merged and ticket closed is done"
expect_stage '{"merged":true,"ticket_state":"none"}' 'done' "merged with no linked ticket is done"
expect_stage '{"closed":true,"merged":false}' abandoned "closed without merging is abandoned"
if printf '{"idle_hours":9,"checks":[{"name":"m","conclusion":null}]}' | python3 -I "$ledger" --classify | cut -f2 | grep -q stuck; then ok "pr-ledger: a long-running check says it may be stuck"; else bad "pr-ledger: stuck hint missing"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all gate script tests passed"; else echo "$fails gate script test(s) failed"; exit 1; fi
