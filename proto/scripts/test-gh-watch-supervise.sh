#!/usr/bin/env bash
# Tests for proto/scripts/gh-watch-supervise (#1342). The supervisor runs against a FAKE `klatch` (answers `claims` and
# `stuck` from files, records every call) and a FAKE `gh-watch` (records its arguments; `ensure` writes a live pid file or
# fails). The clock is a file (GHWS_NOW_FILE), so a backoff of minutes is one `--once` pass with a later number in the
# file: nothing here sleeps, nothing waits on a process, no real receiver, forwarder, klatch home or network is touched.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sup="$here/gh-watch-supervise"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-ghws-test.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
fake="$work/fake"
od="$work/out"
alive=$$ # a pid that is alive for the whole run: this shell
dead="$(sh -c 'echo $$')" # a pid that has ended
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "klatch $* home=${KLATCH_HOME:-unset}" >> "$FAKE_DIR/calls.log"
case "$1" in
  claims) [ -e "$FAKE_DIR/claims-fail" ] && { echo "klatch: store busy" >&2; exit 1; }; cat "$FAKE_DIR/claims.txt" 2> /dev/null; exit 0 ;;
  stuck) cat "$FAKE_DIR/stuck.txt" 2> /dev/null; exit 0 ;;
esac
echo "unexpected klatch $*" >&2; exit 9
SH
cat > "$bin/gh-watch" <<'SH'
#!/bin/sh
echo "gh-watch $*" >> "$FAKE_DIR/calls.log"
[ "$1" = ensure ] || { echo "unexpected gh-watch $*" >&2; exit 9; }
[ -e "$FAKE_DIR/ensure-fails" ] && exit 3
od=""; while [ $# -gt 0 ]; do [ "$1" = --out-dir ] && od="$2"; shift; done
echo "$FAKE_ALIVE" > "$od/gh-watch.pid"
exit "${FAKE_ENSURE_RC:-0}"
SH
chmod +x "$bin/klatch" "$bin/gh-watch"

now_is() { echo "$1" > "$fake/now"; }
newworld() {
  rm -rf "$fake" "$od"; mkdir -p "$fake" "$od"
  : > "$fake/calls.log"; : > "$fake/claims.txt"; : > "$fake/stuck.txt"
  now_is 100000
  export FAKE_DIR="$fake" FAKE_ALIVE="$alive" KLATCH_HOME="$work/factory-home" GHWS_NOW_FILE="$fake/now" GHWS_GH_WATCH="$bin/gh-watch"
  export GHWS_OFF_FILE="$work/off" GHWS_POLL_SECS=1 GHWS_MAX_RESTARTS=3 GHWS_WINDOW_SECS=3600 GHWS_BACKOFF_SECS=60 GHWS_GRACE_SECS=150
  unset GHWS_MAX_TICKS FAKE_ENSURE_RC
  rm -f "$work/off"
}
pid_alive() { echo "$alive" > "$od/gh-watch.pid"; }
pid_dead() { echo "$dead" > "$od/gh-watch.pid"; }
claim_live() { printf 'watch:gh\tghwatch.%s\t%ss left\tgh-watch receiver\n' "${2:-$alive}" "${1:-60}" > "$fake/claims.txt"; }
claim_none() { : > "$fake/claims.txt"; }
args=(--out-dir "$od" --port 18787 --klatch-bin "$bin/klatch" --as-member ghwatch --room factory-status)
tick() { "$sup" --once "${args[@]}" > "$fake/out" 2>&1; }
ensures() { grep -c '^gh-watch ensure' "$fake/calls.log" | tr -d ' '; }
log() { cat "$od/gh-watch.log" 2> /dev/null; }
at() { now_is "$1"; tick; }

# ---- usage and setup ----
newworld
if "$sup" --help > "$fake/out" 2>&1 && grep -q "Usage" "$fake/out"; then ok "--help prints usage and exits 0"; else bad "--help"; fi
rc=0; "$sup" --no-such-flag > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 1 ]; then ok "an unknown flag exits 1"; else bad "unknown flag rc=$rc"; fi
rc=0; "$sup" --once --out-dir "$od" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 1 ] && grep -q -- '--klatch-bin' "$fake/out"; then ok "without --klatch-bin there is no claim to read: exit 1 and it says why"; else bad "no klatch-bin rc=$rc"; fi
rc=0; env -u KLATCH_HOME "$sup" --once "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/calls.log" ]; then ok "KLATCH_HOME unset (the live store): refused, exit 2, nothing run"; else bad "no home rc=$rc"; fi
for h in "$work/.klatch" "$work/.KLATCH/x" "$work/a/.Klatch/b"; do
  mkdir -p "$h"; rc=0; KLATCH_HOME="$h" "$sup" --once "${args[@]}" > "$fake/out" 2>&1 || rc=$?
  if [ "$rc" -eq 2 ] && [ ! -s "$fake/calls.log" ]; then ok "KLATCH_HOME with a .klatch component ($h) is the live store: refused, nothing run"; else bad "live home $h rc=$rc"; fi
done
ln -sfn "$work/.klatch" "$work/link-home"
rc=0; KLATCH_HOME="$work/link-home" "$sup" --once "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/calls.log" ]; then ok "a symlink to the live store is refused too"; else bad "symlink home rc=$rc"; fi
rc=0; "$sup" --once --out-dir "$work/.klatch/o" --klatch-bin "$bin/klatch" --as-member g --room r > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -ne 0 ] && [ ! -d "$work/.klatch/o" ]; then ok "an --out-dir inside .klatch is refused"; else bad "out-dir in .klatch rc=$rc"; fi
rc=0; "$sup" --once --out-dir "$od" --klatch-bin "$bin/klatch" --as-member g --room 'bad room;x' > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 1 ]; then ok "a room name that is not plain is refused"; else bad "bad room rc=$rc"; fi
rc=0; GHWS_MAX_RESTARTS=lots tick || rc=$?
if [ "$rc" -eq 1 ]; then ok "a restart limit that is not a number is refused"; else bad "bad limit rc=$rc"; fi

# ---- healthy: nothing to do ----
newworld; pid_alive; claim_live 60
tick; rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ]; then ok "receiver pid alive and the claim held: no restart"; else bad "healthy rc=$rc ensures=$(ensures)"; fi
if grep -q 'claims factory-status home='"$KLATCH_HOME"'$' "$fake/calls.log"; then ok "the claim is read with klatch claims ROOM in the isolated KLATCH_HOME"; else bad "claims call: $(cat "$fake/calls.log")"; fi

# ---- #1601: after a kill -9 the dead claim has not lapsed; wait for the lapse, never a restart storm ----
newworld; pid_alive; claim_live 60; tick
pid_dead
t=100000; i=0
while [ "$i" -lt 12 ]; do t=$((t + 5)); claim_live $((90 - (t - 100000))); at "$t"; i=$((i + 1)); done
if [ "$(ensures)" -eq 0 ]; then ok "pid dead, claim still held (up to 90 s after kill -9): twelve passes in 60 s start nothing (no restart storm)"; else bad "restart storm: $(ensures)"; fi
if log | grep -q 'receiver pid is dead, waiting for the watch:gh claim to lapse'; then ok "it says it is waiting for the lapse"; else bad "wait line: $(log)"; fi
if [ "$(log | grep -c 'waiting for the watch:gh claim to lapse')" -eq 1 ]; then ok "the waiting line is written once (on the change), not on every pass"; else bad "wait line repeated: $(log)"; fi
claim_none; at 100090
if [ "$(ensures)" -eq 1 ] && grep -q '^gh-watch ensure ' "$fake/calls.log"; then ok "the claim lapsed and the pid is dead: gh-watch ensure runs once"; else bad "after lapse ensures=$(ensures) out=$(cat "$fake/out")"; fi
if grep '^gh-watch ensure' "$fake/calls.log" | grep -q -- "--out-dir $od" && grep '^gh-watch ensure' "$fake/calls.log" | grep -q -- '--port 18787' \
  && grep '^gh-watch ensure' "$fake/calls.log" | grep -q -- "--klatch-bin $bin/klatch" && grep '^gh-watch ensure' "$fake/calls.log" | grep -q -- '--room factory-status'; then
  ok "ensure gets the same out-dir, port, klatch-bin, member and room the receiver was started with"
else bad "ensure args: $(grep '^gh-watch' "$fake/calls.log")"; fi
at 100095
if log | grep -q 'receiver restarted by ensure' && [ "$(ensures)" -eq 1 ]; then ok "ensure brought the receiver up: logged, and no second ensure"; else bad "restarted line: $(log)"; fi
claim_live 90 "$alive"; at 100100
if log | grep -q 'receiver and watch:gh claim are healthy again'; then ok "the next pass with the claim held again logs it is healthy"; else bad "healthy again: $(log)"; fi

# ---- a lapse needs evidence: a clean stop releases the claim and is not restarted ----
newworld; pid_dead; claim_none; tick; rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ] && log | grep -q 'no lapse was seen'; then ok "receiver down, no claim and no lapse seen (a clean stop or never started): not restarted, and it says so"; else bad "clean stop: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 120s\n' > "$fake/stuck.txt"; tick
if [ "$(ensures)" -eq 1 ]; then ok "a recent lapsed_claim for watch:gh in klatch stuck is evidence enough on the first pass (the supervisor started after the crash)"; else bad "stuck lapse: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 99999s\n' > "$fake/stuck.txt"; tick
if [ "$(ensures)" -eq 0 ]; then ok "an old lapsed_claim (past GHWS_LAPSE_MAX_AGE_SECS) is not evidence: no restart"; else bad "old lapse restarted"; fi
newworld; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim ask:7 holder bob lapsed 5s\nlapsed_claim room other claim watch:gh holder x.1 lapsed 5s\n' > "$fake/stuck.txt"; tick
if [ "$(ensures)" -eq 0 ]; then ok "a lapse of another claim or in another room is not evidence"; else bad "wrong lapse restarted"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_live 30; at 100005; claim_live 0; at 100010
if [ "$(ensures)" -eq 1 ]; then ok "a claim with 0s left counts as lapsed"; else bad "0s left: ensures=$(ensures) $(log)"; fi

# ---- never kill, never guess: a live pid with a lapsed claim, or an unreadable claim ----
newworld; pid_alive; claim_live 60; tick; claim_none; at 100030
if [ "$(ensures)" -eq 0 ] && log | grep -q 'ALERT watcher supervisor: receiver pid is alive but the watch:gh claim lapsed'; then ok "pid alive but the claim lapsed (a hung receiver): ALERT, no restart, nothing killed"; else bad "hung: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_live 60 99999; tick
now_is 100400; claim_live 60 99999; tick
if [ "$(ensures)" -eq 0 ] && log | grep -q 'ALERT watcher supervisor: receiver pid is dead but the watch:gh claim is still held'; then ok "a claim held past the grace by another holder: ALERT once, no restart (it is not ours to take)"; else bad "other holder: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; touch "$fake/claims-fail"; tick; rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ] && log | grep -q 'ALERT watcher supervisor: claim state unreadable, not restarting'; then ok "klatch claims failing (store busy, no credential): ALERT, no restart, reports instead"; else bad "unreadable: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_none; touch "$work/off"; printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"; tick
if [ "$(ensures)" -eq 0 ] && log | grep -q 'off file'; then ok "the off file stops restarts"; else bad "off: ensures=$(ensures)"; fi
rm -f "$work/off"; tick
if [ "$(ensures)" -eq 1 ]; then ok "removing the off file lets the next pass restart"; else bad "off removed: ensures=$(ensures)"; fi

# ---- bounded restarts: 3 per hour, backoff 60 s then 120 s then 240 s, then report and stop ----
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"
printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
at 100000
at 100010
if [ "$(ensures)" -eq 1 ]; then ok "ensure fails: the second attempt waits for the backoff (60 s), not retried 10 s later"; else bad "backoff1: $(ensures)"; fi
if log | grep -q 'ensure did not bring the receiver up, next try in 60 s'; then ok "the failed attempt is logged with the wait"; else bad "backoff line: $(log)"; fi
at 100060
if [ "$(ensures)" -eq 2 ]; then ok "after 60 s the second attempt runs"; else bad "attempt2: $(ensures)"; fi
at 100179
if [ "$(ensures)" -eq 2 ]; then ok "the backoff doubles (120 s): 119 s later nothing runs"; else bad "backoff2: $(ensures)"; fi
at 100180
if [ "$(ensures)" -eq 3 ]; then ok "the third attempt runs after 120 s"; else bad "attempt3: $(ensures)"; fi
at 100419
if [ "$(ensures)" -eq 3 ]; then ok "the backoff doubles again (240 s)"; else bad "backoff3: $(ensures)"; fi
at 100420; rc=$?
if [ "$(ensures)" -eq 3 ] && [ "$rc" -eq 3 ] && log | grep -q 'ALERT watcher supervisor: restart limit reached (3 in the last 3600 s), giving up; run gh-watch ensure yourself'; then ok "the fourth restart in the hour is refused: ALERT, exit 3, the human is told to run ensure"; else bad "limit: ensures=$(ensures) rc=$rc $(log)"; fi
at 103000; rc=$?
if [ "$(ensures)" -eq 3 ] && [ "$rc" -eq 3 ]; then ok "it stays stopped while the three attempts are inside the hour"; else bad "still limited: ensures=$(ensures) rc=$rc"; fi
at 103700
if [ "$(ensures)" -eq 4 ]; then ok "an hour after the first attempt the window has room again (attempts older than GHWS_WINDOW_SECS expire)"; else bad "window: ensures=$(ensures)"; fi
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"; printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
GHWS_MAX_RESTARTS=1 at 100000; GHWS_MAX_RESTARTS=1 at 100500; rc=$?
if [ "$(ensures)" -eq 1 ] && [ "$rc" -eq 3 ]; then ok "GHWS_MAX_RESTARTS sets the limit (1: one attempt, then stop)"; else bad "limit 1: ensures=$(ensures) rc=$rc"; fi
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
at 100000; pid_dead; claim_none; at 100005
if [ "$(ensures)" -eq 1 ]; then ok "a restart that worked is counted too: a receiver that dies again at once waits for the backoff"; else bad "worked then died: ensures=$(ensures)"; fi

# ---- what it may run, and what text it ever uses ----
newworld; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
# shellcheck disable=SC2016
printf 'watch:gh\t$(touch %s/pwned)\t0s left\treason $(touch %s/pwned2)\n' "$work" "$work" > "$fake/claims.txt"
tick
if [ ! -e "$work/pwned" ] && [ ! -e "$work/pwned2" ] && ! log | grep -q 'pwned'; then ok "claim holder and reason text is data: never run, never logged"; else bad "claim text leaked: $(log)"; fi
bad_lines="$(log | grep -vcE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z (INFO|ALERT) watcher supervisor[: ]' | tr -d ' ')"
if [ "$bad_lines" -eq 0 ] && [ -n "$(log)" ]; then ok "every log line is TIME INFO|ALERT and a fixed template"; else bad "log shape: $(log)"; fi
if ! grep -E '^klatch ' "$fake/calls.log" | grep -vqE '^klatch (claims|stuck) '; then ok "klatch is only ever asked claims and stuck (read only: no post, claim, release, worker or admin)"; else bad "klatch calls: $(grep '^klatch' "$fake/calls.log")"; fi
if ! grep -E '^gh-watch ' "$fake/calls.log" | grep -vqE '^gh-watch ensure '; then ok "gh-watch is only ever run as ensure: never serve, forward-command, install-launchd or clean-hooks (the forwarder stays the human's)"; else bad "gh-watch calls: $(grep '^gh-watch' "$fake/calls.log")"; fi
if ! grep -rqE 'launchctl|gh webhook|gh api|install-launchd' "$sup"; then ok "the script never names launchctl, gh webhook, gh api or install-launchd as something to run"; else bad "forbidden command text in the script"; fi

# ---- loop mode: bounded by GHWS_MAX_TICKS under the test clock, so it never sleeps ----
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
rc=0; GHWS_MAX_TICKS=3 "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 1 ]; then ok "loop mode runs the bounded number of passes, restarts once, and exits 0"; else bad "loop: rc=$rc ensures=$(ensures) out=$(cat "$fake/out")"; fi
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"; printf 'lapsed_claim room factory-status claim watch:gh holder g.1 lapsed 5s\n' > "$fake/stuck.txt"
rc=0; GHWS_BACKOFF_SECS=0 GHWS_MAX_RESTARTS=1 GHWS_MAX_TICKS=50 "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 3 ] && [ "$(ensures)" -eq 1 ]; then ok "loop mode stops with exit 3 when the restart limit is reached"; else bad "loop limit: rc=$rc ensures=$(ensures)"; fi

if [ "$fails" -eq 0 ]; then echo "all gh-watch-supervise tests passed"; else echo "$fails FAILED"; exit 1; fi
