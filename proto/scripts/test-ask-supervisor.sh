#!/usr/bin/env bash
# Tests for proto/scripts/ask-supervisor (#1170, part 2 of #671). The supervisor runs against a FAKE `klatch` (answers
# `asks` and `could-answer` from files, records every call) and a FAKE launcher (records its environment, then ends at
# once or stays until told to). No real claude, no credential, no real klatch home, no network. The waits are bounded
# polls of a file, never a long sleep, so nothing here leaves a `/bin/sleep 3600` behind (#1101) and a loaded machine
# only makes a wait longer (#1142).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sup="$here/ask-supervisor"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-asksup-test.XXXXXX")"
bgpids=""
cleanup() {
  for p in $bgpids; do kill -9 "$p" 2> /dev/null; done
  pkill -9 -f "$work/" 2> /dev/null
  rm -rf "$work"
}
trap cleanup EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
fake="$work/fake"; mkdir -p "$fake"
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "klatch $*" >> "$FAKE_DIR/calls.log"
case "$1" in
  asks) [ -e "$FAKE_DIR/asks-fail" ] && exit 1; cat "$FAKE_DIR/asks.txt" 2> /dev/null; exit 0 ;;
  could-answer)
    [ -e "$FAKE_DIR/could-fail" ] && { echo "klatch: admin-scope" >&2; exit 1; }
    if grep -qx "$3 $4" "$FAKE_DIR/yes.txt" 2> /dev/null; then echo yes; else echo no; echo "clearance: $4 is not cleared for the room"; fi
    exit 0 ;;
esac
echo "unexpected klatch $*" >&2; exit 9
SH
cat > "$bin/launch" <<'SH'
#!/bin/sh
# one line per start: ask member active, plus anything that must never reach a helper
echo "$ASKSUP_ASK $ASKSUP_MEMBER active=$ASKSUP_ACTIVE room=$ASKSUP_ROOM text=${ASKSUP_TEXT:-none}" >> "$FAKE_DIR/launches.log"
# #1177: a launcher reports what the helper really cost by writing ASKSUP_COST_FILE (the test picks the bytes)
[ -n "${ASKSUP_COST_FILE:-}" ] && echo "cost-file-set" >> "$FAKE_DIR/costenv.log"
[ -n "${ASKSUP_COST_FILE:-}" ] && [ -e "$FAKE_DIR/cost-value" ] && cp "$FAKE_DIR/cost-value" "$ASKSUP_COST_FILE"
[ -e "$FAKE_DIR/launch-fails" ] && exit 3
[ -e "$FAKE_DIR/launch-quick" ] && exit 0
while [ ! -e "$FAKE_DIR/stop-$ASKSUP_MEMBER" ]; do /bin/sleep 0.2; done
exit 0
SH
chmod +x "$bin/klatch" "$bin/launch"

newworld() {
  rm -rf "$fake"; mkdir -p "$fake"
  : > "$fake/asks.txt"; : > "$fake/yes.txt"; : > "$fake/calls.log"; : > "$fake/launches.log"
  touch "$fake/launch-quick"
  rm -rf "$work/state"
}
ask() { printf '#%s %s unclaimed %ss: %s\n' "$1" "$2" "${3:-600}" "${4:-what is the plan}" >> "$fake/asks.txt"; }
yes_for() { printf '%s %s\n' "$1" "$2" >> "$fake/yes.txt"; }
launches() { grep -c '' "$fake/launches.log" | tr -d ' '; }
base_env() {
  export FAKE_DIR="$fake" ASKSUP_KLATCH="$bin/klatch" ASKSUP_LAUNCH="$bin/launch" ASKSUP_ROOM=lobby ASKSUP_CANDIDATES="helper1 helper2"
  export ASKSUP_STATE="$work/state" ASKSUP_POLL_SECS=1 ASKSUP_PAUSE_SECS=0 ASKSUP_MIN_AGE_SECS=0 ASKSUP_MAX_HELPERS=1
  export ASKSUP_START_USD=1 ASKSUP_MAX_TOTAL_USD=100 ASKSUP_HELPER_SECS=3600 ASKSUP_OFF_FILE="$work/off"
  unset ASKSUP_ACTIVE ASKSUP_TEST_DROP_TERMS
  rm -f "$work/off"
}
run_once() { "$sup" --once > "$fake/out" 2>&1; }
run_bg() { "$sup" > "$fake/out" 2>&1 & ipid=$!; bgpids="$bgpids $ipid"; }
waitfor() { # file, pattern, seconds (default 60): a loaded machine starts things late
  local i=0 max=$(( ${3:-60} * 5 ))
  while [ "$i" -lt "$max" ]; do grep -q -- "$2" "$1" 2> /dev/null && return 0; /bin/sleep 0.2; i=$((i + 1)); done
  return 1
}
base_env

# ---- usage and setup ----
newworld; base_env
if "$sup" --help > "$fake/out" 2>&1 && grep -q "Usage" "$fake/out"; then ok "--help prints usage and exits 0"; else bad "--help"; fi
"$sup" --no-such-flag > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 1 ]; then ok "an unknown flag exits 1"; else bad "unknown flag rc=$rc"; fi
rc=0; ASKSUP_CANDIDATES="" run_once || rc=$?
if [ "$rc" -eq 1 ] && grep -q "ASKSUP_CANDIDATES" "$fake/out"; then ok "no candidate helpers configured: exit 1 and it says which setting"; else bad "no candidates rc=$rc $(cat "$fake/out")"; fi
rc=0; ASKSUP_LAUNCH="" run_once || rc=$?
if [ "$rc" -eq 1 ] && grep -q "ASKSUP_LAUNCH" "$fake/out"; then ok "no launch command and not --dry-run: exit 1"; else bad "no launch rc=$rc"; fi
rc=0; ASKSUP_ROOM='bad room;x' run_once || rc=$?
if [ "$rc" -eq 1 ]; then ok "a room name that is not plain is refused"; else bad "bad room rc=$rc"; fi
rc=0; ASKSUP_CANDIDATES='ok; touch /tmp/x' run_once || rc=$?
if [ "$rc" -eq 1 ]; then ok "a candidate name that is not plain is refused"; else bad "bad candidate rc=$rc"; fi
rc=0; ASKSUP_MAX_TOTAL_USD=lots run_once || rc=$?
if [ "$rc" -eq 1 ]; then ok "a spend cap that is not a number is refused"; else bad "bad spend rc=$rc"; fi
newworld; ask 5 alice; yes_for 5 helper1
rc=0; ASKSUP_ACTIVE=1 run_once || rc=$?
if [ "$rc" -eq 2 ] && [ "$(launches)" -eq 0 ] && ! grep -q '^klatch' "$fake/calls.log"; then ok "run from inside a started helper (ASKSUP_ACTIVE=1) it refuses, exit 2, and touches nothing: a helper never starts a helper"; else bad "active rc=$rc"; fi

# ---- the happy path and what a helper is given ----
newworld
# shellcheck disable=SC2016
ask 5 alice 600 'run $(touch '"$work"'/pwned) now'
yes_for 5 helper1
run_once; rc=$?
if [ "$rc" -eq 0 ] && [ "$(launches)" -eq 1 ] && grep -q '^5 helper1 active=1 room=lobby text=none$' "$fake/launches.log"; then ok "an unanswered ask with a yes from could-answer starts that helper (ASKSUP_ASK, ASKSUP_MEMBER, ASKSUP_ROOM, ASKSUP_ACTIVE=1)"; else bad "happy rc=$rc launches=$(cat "$fake/launches.log") out=$(cat "$fake/out")"; fi
if [ ! -e "$work/pwned" ] && ! grep -q 'pwned' "$fake/launches.log"; then ok "the ask text is data: never run and never passed to the helper"; else bad "ask text leaked or ran"; fi
if grep -q '^klatch could-answer lobby 5 helper1$' "$fake/calls.log"; then ok "the check is the read-only core command: klatch could-answer ROOM ASK MEMBER"; else bad "check call: $(cat "$fake/calls.log")"; fi
if ! grep -qE '^klatch (post|claim|worker|admin|release)' "$fake/calls.log"; then ok "it calls nothing that posts, claims or issues a credential"; else bad "forbidden call: $(cat "$fake/calls.log")"; fi

# ---- the first candidate that could answer is chosen; a no starts nobody ----
newworld; ask 5 alice; yes_for 5 helper2
run_once
if [ "$(launches)" -eq 1 ] && grep -q '^5 helper2 ' "$fake/launches.log" && grep -q 'could-answer lobby 5 helper1' "$fake/calls.log"; then ok "helper1 fails the check, helper2 passes: helper2 is started"; else bad "candidate pick: $(cat "$fake/launches.log")"; fi
newworld; ask 5 alice
run_once
if [ "$(launches)" -eq 0 ] && grep -q 'clearance' "$fake/out"; then ok "no candidate passes: nothing is started and the reason the check gave is shown"; else bad "all no: $(cat "$fake/launches.log") $(cat "$fake/out")"; fi
newworld; ask 5 alice; yes_for 5 helper1; touch "$fake/could-fail"
run_once
if [ "$(launches)" -eq 0 ]; then ok "a could-answer call that fails (for example not the human) starts nobody"; else bad "could-fail started"; fi
newworld; ask 5 alice; yes_for 5 helper1; touch "$fake/asks-fail"
run_once; rc=$?
if [ "$rc" -eq 0 ] && [ "$(launches)" -eq 0 ]; then ok "an asks call that fails starts nobody and is not fatal"; else bad "asks-fail rc=$rc"; fi

# ---- which asks are eligible ----
newworld; printf '#5 alice claimed 600s: x\nask cycle: #1 -> #2; bob yields\n' > "$fake/asks.txt"; ask 6 alice 10; yes_for 5 helper1; yes_for 6 helper1
ASKSUP_MIN_AGE_SECS=60 run_once
if [ "$(launches)" -eq 0 ]; then ok "a claimed ask and an ask younger than ASKSUP_MIN_AGE_SECS are left to the room"; else bad "eligibility: $(cat "$fake/launches.log")"; fi
newworld; ask 5 helper2; yes_for 5 helper1
run_once
if [ "$(launches)" -eq 0 ]; then ok "an ask posted by one of the helper identities never starts a helper (no helper-starts-helper)"; else bad "helper ask started"; fi

# ---- caps ----
newworld; ask 5 alice; ask 6 alice; yes_for 5 helper1; yes_for 6 helper2
ASKSUP_MAX_HELPERS=1 ASKSUP_PAUSE_SECS=0 run_once
n1="$(launches)"
newworld; ask 5 alice; ask 6 alice; yes_for 5 helper1; yes_for 6 helper2
ASKSUP_MAX_HELPERS=2 ASKSUP_PAUSE_SECS=0 run_once
n2="$(launches)"
if [ "$n1" -eq 1 ] && [ "$n2" -eq 2 ]; then ok "ASKSUP_MAX_HELPERS bounds helpers at once (1 gives one start, 2 gives two)"; else bad "max helpers n1=$n1 n2=$n2"; fi
newworld; ask 5 alice; ask 6 alice; yes_for 5 helper1; yes_for 6 helper2
ASKSUP_MAX_HELPERS=2 ASKSUP_PAUSE_SECS=100 run_once
if [ "$(launches)" -eq 1 ]; then ok "ASKSUP_PAUSE_SECS: a second helper is not started within the pause of the first"; else bad "pause: $(launches)"; fi
newworld; ask 5 alice; ask 6 alice; yes_for 5 helper1; yes_for 6 helper2
ASKSUP_MAX_HELPERS=5 ASKSUP_START_USD=2 ASKSUP_MAX_TOTAL_USD=3 run_once
if [ "$(launches)" -eq 1 ] && grep -qi 'spend' "$fake/out"; then ok "ASKSUP_MAX_TOTAL_USD: a start that would pass the cap (2 + 2 > 3) is refused, and it says so"; else bad "spend cap: $(launches) $(cat "$fake/out")"; fi
ASKSUP_MAX_HELPERS=5 ASKSUP_START_USD=2 ASKSUP_MAX_TOTAL_USD=3 run_once
if [ "$(launches)" -eq 1 ]; then ok "the spend is kept in ASKSUP_STATE, so a restarted supervisor still counts it"; else bad "spend persisted: $(launches)"; fi
newworld; ask 5 alice; yes_for 5 helper1; touch "$work/off"
run_once
if [ "$(launches)" -eq 0 ] && grep -qi 'off' "$fake/out"; then ok "the off switch (ASKSUP_OFF_FILE exists): nothing is started"; else bad "off switch: $(launches)"; fi
rm -f "$work/off"; run_once
if [ "$(launches)" -eq 1 ]; then ok "removing the off file lets the next pass start the helper"; else bad "off removed: $(launches)"; fi

# ---- the measured cost (#1177): the launcher reports it in ASKSUP_COST_FILE, the fixed charge is the fallback ----
spent_of() { cat "$work/state/spent" 2> /dev/null || echo none; }
same_num() { awk -v a="$1" -v b="$2" 'BEGIN { exit !(a == b) }'; } # 0.5 equals 0.50
newworld; ask 5 alice; yes_for 5 helper1; printf '0.5\n' > "$fake/cost-value"
ASKSUP_START_USD=2 run_once
if grep -q cost-file-set "$fake/costenv.log" 2> /dev/null && same_num "$(spent_of)" 0.5; then ok "the helper gets ASKSUP_COST_FILE; the cost it wrote (0.5) replaces the fixed charge (2) in the total"; else bad "measured cost: spent=$(spent_of) env=$(cat "$fake/costenv.log" 2> /dev/null)"; fi
newworld; ask 5 alice; yes_for 5 helper1; printf '7.25' > "$fake/cost-value"
ASKSUP_START_USD=2 ASKSUP_MAX_TOTAL_USD=100 run_once
if same_num "$(spent_of)" 7.25; then ok "a measured cost above the fixed charge is charged too (no trailing newline needed)"; else bad "higher cost: spent=$(spent_of)"; fi
newworld; ask 5 alice; yes_for 5 helper1
ASKSUP_START_USD=2 run_once
if same_num "$(spent_of)" 2; then ok "no cost file written: the fixed per-start charge stays"; else bad "absent cost: spent=$(spent_of)"; fi
# shellcheck disable=SC2016 # the dollar sign is literal junk
for junk in 'lots' '-3' '1e9' '5 USD; touch pwned' '0.5.5' '' '$(touch pwned)' '1000000000000000000000000'; do
  newworld; ask 5 alice; yes_for 5 helper1; printf '%s\n' "$junk" > "$fake/cost-value"
  ASKSUP_START_USD=2 run_once
  if same_num "$(spent_of)" 2 && [ ! -e pwned ] && [ ! -e "$work/pwned" ]; then ok "an unparsable cost ('$junk') is ignored: the fixed charge stays"; else bad "junk cost '$junk': spent=$(spent_of)"; fi
done
newworld; ask 5 alice; yes_for 5 helper1; printf '0.5\nignored text after the number\n' > "$fake/cost-value"
ASKSUP_START_USD=2 run_once
if same_num "$(spent_of)" 2; then ok "a cost file with text after the number is not trusted: the fixed charge stays"; else bad "multi-line cost: spent=$(spent_of)"; fi
newworld; ask 5 alice; yes_for 5 helper1; printf '99999\n' > "$fake/cost-value"
ASKSUP_START_USD=2 ASKSUP_MAX_TOTAL_USD=10 run_once
if same_num "$(spent_of)" 10; then ok "a reported cost is capped at ASKSUP_MAX_TOTAL_USD (99999 counts as 10)"; else bad "capped cost: spent=$(spent_of)"; fi
# the cap reflects reality: with a fixed charge of 2 and a cap of 3 only one start fits; measured at 0.5 a second does
newworld; ask 5 alice; ask 6 alice; yes_for 5 helper1; yes_for 6 helper2; printf '0.5\n' > "$fake/cost-value"
export ASKSUP_MAX_HELPERS=5 ASKSUP_START_USD=2 ASKSUP_MAX_TOTAL_USD=3
run_once; run_once
n1="$(launches)"
printf '3\n' > "$fake/cost-value"; ask 7 alice; yes_for 7 helper1; yes_for 7 helper2
run_once; run_once
n2="$(launches)"
if [ "$n1" -eq 2 ] && [ "$n2" -eq 3 ]; then ok "measured costs free budget the fixed charge would hold (a second start fits at 0.5, not at 2)"; else bad "cap with measured: n1=$n1 n2=$n2 spent=$(spent_of)"; fi
ask 8 alice; yes_for 8 helper1
run_once
if [ "$(launches)" -eq 3 ] && grep -qi 'spend cap' "$fake/out"; then ok "once the measured total reaches the cap no more helpers are started, and it says so"; else bad "after cap: $(launches) spent=$(spent_of) $(cat "$fake/out")"; fi
base_env

# ---- dry run ----
newworld; ask 5 alice; yes_for 5 helper1
ASKSUP_LAUNCH="" "$sup" --dry-run > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'would start helper1 for ask #5' "$fake/out" && [ "$(launches)" -eq 0 ] && [ ! -e "$work/state" ]; then ok "--dry-run prints what it would start, starts nothing and writes no state"; else bad "dry run rc=$rc $(cat "$fake/out")"; fi

# ---- one helper per ask, a retry only after the first ended ----
newworld; rm -f "$fake/launch-quick"; ask 5 alice; yes_for 5 helper1; yes_for 5 helper2
ASKSUP_MAX_HELPERS=3 ASKSUP_MAX_PER_ASK=2 run_bg
waitfor "$fake/launches.log" '^5 helper1' 60 && s=1 || s=0
/bin/sleep 3
if [ "$s" -eq 1 ] && [ "$(launches)" -eq 1 ]; then ok "while the helper for an ask is running, no second helper is started for it (3 passes later: still one)"; else bad "second helper while running s=$s launches=$(launches)"; fi
touch "$fake/stop-helper1"
waitfor "$fake/launches.log" '^5 helper2' 60 && r=1 || r=0
touch "$fake/stop-helper2"
/bin/sleep 3
if [ "$r" -eq 1 ] && [ "$(launches)" -eq 2 ]; then ok "after the first helper ended with the ask still open, one more is started (a different identity), up to ASKSUP_MAX_PER_ASK"; else bad "retry r=$r launches=$(launches)"; fi
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null

newworld; rm -f "$fake/launch-quick"; touch "$fake/launch-fails"; ask 5 alice; yes_for 5 helper1
ASKSUP_MAX_PER_ASK=1 run_bg
waitfor "$fake/launches.log" '^5 helper1' 60 && s=1 || s=0
/bin/sleep 3
if [ "$s" -eq 1 ] && [ "$(launches)" -eq 1 ]; then ok "a helper that fails to start still counts: with ASKSUP_MAX_PER_ASK=1 the ask is not tried again"; else bad "failed launch retried: $(launches)"; fi
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null

# ---- the time limit per helper ----
newworld; rm -f "$fake/launch-quick"; ask 5 alice; yes_for 5 helper1
ASKSUP_HELPER_SECS=2 run_bg
waitfor "$fake/launches.log" '^5 helper1' 60 && s=1 || s=0
hp="$(pgrep -f "$bin/launch" | sed -n 1p)"
gone=0; for _ in $(seq 1 100); do if [ -n "$hp" ] && ! kill -0 "$hp" 2> /dev/null; then gone=1; break; fi; /bin/sleep 0.2; done
if [ "$s" -eq 1 ] && [ "$gone" -eq 1 ] && waitfor "$fake/out" 'time limit' 5; then ok "a helper that runs past ASKSUP_HELPER_SECS is killed (kill -9) and the supervisor says so"; else bad "time limit s=$s gone=$gone hp=$hp out=$(tail -3 "$fake/out")"; fi
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null

# ---- signals ----
newworld; rm -f "$fake/launch-quick"; ask 5 alice; yes_for 5 helper1
run_bg
waitfor "$fake/launches.log" '^5 helper1' 60 && s=1 || s=0
hp="$(pgrep -f "$bin/launch" | sed -n 1p)"
t0=$SECONDS
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
took=$((SECONDS - t0))
gone=1; [ -n "$hp" ] && kill -0 "$hp" 2> /dev/null && gone=0
if [ "$s" -eq 1 ] && [ "$rc" -eq 0 ] && [ "$took" -le 10 ] && [ "$gone" -eq 1 ]; then ok "a TERM stops the supervisor with exit 0 in $took s and ends the helper it started"; else bad "term s=$s rc=$rc took=$took gone=$gone"; fi
newworld; rm -f "$fake/launch-quick"; ask 5 alice; yes_for 5 helper1
ASKSUP_TEST_DROP_TERMS=2 run_bg
waitfor "$fake/launches.log" '^5 helper1' 60 && s=1 || s=0
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
if [ "$s" -eq 1 ] && [ "$rc" -eq 0 ]; then ok "a TERM the worker lost twice is sent again by the small parent and still stops it (#1099)"; else bad "lost term s=$s rc=$rc"; fi
newworld; ASKSUP_POLL_SECS=30 run_bg
waitfor "$fake/calls.log" 'klatch asks' 60 && s=1 || s=0
t0=$SECONDS
kill -HUP "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
took=$((SECONDS - t0))
if [ "$s" -eq 1 ] && [ "$rc" -eq 0 ] && [ "$took" -le 5 ]; then ok "a signal during a 30 s pause between polls stops it at once ($took s)"; else bad "signal in pause s=$s rc=$rc took=$took"; fi
newworld; ASKSUP_POLL_SECS=30 run_bg
waitfor "$fake/calls.log" 'klatch asks' 60 && s=1 || s=0
wpid="$(pgrep -P "$ipid" | sed -n 1p)"
kill -9 "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null
gone=0; for _ in $(seq 1 100); do if [ -n "$wpid" ] && ! kill -0 "$wpid" 2> /dev/null; then gone=1; break; fi; /bin/sleep 0.2; done
[ "$gone" -eq 0 ] && [ -n "$wpid" ] && kill -9 "$wpid" 2> /dev/null
if [ "$s" -eq 1 ] && [ "$gone" -eq 1 ]; then ok "when the supervisor is killed (kill -9) the worker notices and stops by itself"; else bad "orphan worker s=$s wpid=$wpid gone=$gone"; fi

# ---- nothing is left behind (#1101): no long sleep from this test's processes ----
if pgrep -f "/bin/sleep 3600" > /dev/null 2>&1 && pgrep -fl "/bin/sleep 3600" | grep "$work" > /dev/null; then bad "a /bin/sleep 3600 outlived the run"; else ok "no /bin/sleep 3600 was started by these runs (#1101)"; fi
/bin/sleep 1
leftover="$(pgrep -f "$work/bin/launch" 2> /dev/null | wc -l | tr -d ' ')"
if [ "$leftover" -eq 0 ]; then ok "no helper process is left running after the runs"; else bad "leftover helpers: $leftover"; fi

echo
if [ "$fails" -eq 0 ]; then echo "ask-supervisor tests: all passed"; else echo "ask-supervisor tests: $fails FAILED"; exit 1; fi
