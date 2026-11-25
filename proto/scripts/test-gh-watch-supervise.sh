#!/usr/bin/env bash
# Tests for proto/scripts/gh-watch-supervise (#1342). The supervisor runs against a FAKE `klatch` (answers `claims` and
# `stuck` from files, records every call) and a FAKE `gh-watch` (records its arguments; `ensure` writes a live pid file or
# fails). The clock is a file (GHWS_NOW_FILE), so a backoff of minutes is one `--once` pass with a later number in the
# file: nothing here sleeps, nothing waits on a process, no real receiver, forwarder, klatch home or network is touched.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sup="${GHWS_SUP_UNDER_TEST:-$here/gh-watch-supervise}" # the mutation runs point this at a mutated copy
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-ghws-test.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
fake="$work/fake"
od="$work/out"
KLATCH_HOME_DIR="$work/factory-home"
alive=$$ # a pid that is alive for the whole run: this shell
dead="$(sh -c 'echo $$')" # a pid that has ended
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "klatch $* home=${KLATCH_HOME:-unset}" >> "$FAKE_DIR/calls.log"
case "$1" in
  claims) [ -e "$FAKE_DIR/claims-hang" ] && exec /bin/sleep 30; [ -e "$FAKE_DIR/claims-fail" ] && { echo "klatch: store busy" >&2; exit 1; }; cat "$FAKE_DIR/claims.txt" 2> /dev/null; exit 0 ;;
  stuck) [ -e "$FAKE_DIR/stuck-hang" ] && exec /bin/sleep 30; cat "$FAKE_DIR/stuck.txt" 2> /dev/null; exit 0 ;;
esac
echo "unexpected klatch $*" >&2; exit 9
SH
cat > "$bin/gh-watch" <<'SH'
#!/bin/sh
echo "gh-watch $*" >> "$FAKE_DIR/calls.log"
[ "$1" = ensure ] || { echo "unexpected gh-watch $*" >&2; exit 9; }
[ -e "$FAKE_DIR/ensure-fails" ] && exit 3
echo "$(ps -o pgid= -p $$ | tr -d ' ')" >> "$FAKE_DIR/pgid.log"
od=""; while [ $# -gt 0 ]; do [ "$1" = --out-dir ] && od="$2"; shift; done
echo "$FAKE_ALIVE" > "$od/gh-watch.pid"
exit "${FAKE_ENSURE_RC:-0}"
SH
chmod +x "$bin/klatch" "$bin/gh-watch"

now_is() { echo "$1" > "$fake/now"; }
newworld() {
  rm -rf "$fake" "$od"; mkdir -p "$fake" "$od"
  mkdir -p "$KLATCH_HOME_DIR"; : > "$KLATCH_HOME_DIR/rooms.db"
  : > "$fake/calls.log"; : > "$fake/claims.txt"; : > "$fake/stuck.txt"
  now_is 100000
  export FAKE_DIR="$fake" FAKE_ALIVE="$alive" KLATCH_HOME="$work/factory-home" GHWS_NOW_FILE="$fake/now" GHWS_GH_WATCH="$bin/gh-watch"
  export GHWS_OFF_FILE="$work/off" GHWS_POLL_SECS=1 GHWS_MAX_RESTARTS=3 GHWS_WINDOW_SECS=3600 GHWS_BACKOFF_SECS=60 GHWS_EVIDENCE_SECS=180 GHWS_KLATCH_TIMEOUT_SECS=10
  unset GHWS_MAX_TICKS FAKE_ENSURE_RC GHWS_STATE
  rm -f "$work/off"
}
pid_alive() { echo "$alive" > "$od/gh-watch.pid"; }
pid_dead() { echo "$dead" > "$od/gh-watch.pid"; }
claim_live() { printf 'watch:gh\tghwatch.%s\t%ss left\tgh-watch receiver\n' "${2:-$alive}" "${1:-60}" > "$fake/claims.txt"; }
claim_by() { printf "watch:gh\t%s\t%ss left\tgh-watch receiver\n" "$1" "${2:-60}" > "$fake/claims.txt"; }
claim_none() { : > "$fake/claims.txt"; }
args=(--out-dir "$od" --port 18787 --klatch-bin "$bin/klatch" --as-member ghwatch --room factory-status)
tick() { "$sup" --once "${args[@]}" > "$fake/out" 2>&1; }
ensures() { grep -c '^gh-watch ensure' "$fake/calls.log" | tr -d ' '; }
log() { cat "$od/gh-watch.log" 2> /dev/null; }
log_has() { local t; t="$(log)"; grep -q -- "$1" <<< "$t"; } # no pipe: grep -q would SIGPIPE its producer under pipefail (#1357)
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
if log_has 'receiver pid is dead, waiting for the watch:gh claim to lapse'; then ok "it says it is waiting for the lapse"; else bad "wait line: $(log)"; fi
if [ "$(log | grep -c 'waiting for the watch:gh claim to lapse')" -eq 1 ]; then ok "the waiting line is written once (on the change), not on every pass"; else bad "wait line repeated: $(log)"; fi
claim_none; at 100090
if [ "$(ensures)" -eq 1 ] && grep -q '^gh-watch ensure ' "$fake/calls.log"; then ok "the claim lapsed and the pid is dead: gh-watch ensure runs once"; else bad "after lapse ensures=$(ensures) out=$(cat "$fake/out")"; fi
ea="$(grep '^gh-watch ensure' "$fake/calls.log")"
if grep -q -- "--out-dir $od" <<< "$ea" && grep -q -- '--port 18787' <<< "$ea" && grep -q -- "--klatch-bin $bin/klatch" <<< "$ea" && grep -q -- '--room factory-status' <<< "$ea"; then
  ok "ensure gets the same out-dir, port, klatch-bin, member and room the receiver was started with"
else bad "ensure args: $(grep '^gh-watch' "$fake/calls.log")"; fi
at 100095
if log_has 'receiver restarted by ensure' && [ "$(ensures)" -eq 1 ]; then ok "ensure brought the receiver up: logged, and no second ensure"; else bad "restarted line: $(log)"; fi
claim_live 90 "$alive"; at 100100
if log_has 'receiver and watch:gh claim are healthy again'; then ok "the next pass with the claim held again logs it is healthy"; else bad "healthy again: $(log)"; fi

# ---- a lapse needs evidence: a clean stop releases the claim and is not restarted ----
newworld; pid_dead; claim_none; tick; rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ] && log_has 'no lapse of our claim was seen'; then ok "receiver down, no claim and no lapse seen (a clean stop or never started): not restarted, and it says so"; else bad "clean stop: ensures=$(ensures) $(log)"; fi
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
if [ "$(ensures)" -eq 0 ] && log_has 'ALERT watcher supervisor: receiver pid is alive but the watch:gh claim lapsed'; then ok "pid alive but the claim lapsed (a hung receiver): ALERT, no restart, nothing killed"; else bad "hung: ensures=$(ensures) $(log)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_by ghwatch.99999 60; tick
if [ "$(ensures)" -eq 0 ] && log_has 'ALERT watcher supervisor: receiver pid is dead but the watch:gh claim is held by another holder'; then ok "pid dead and the claim held by another receiver (other pid): ALERT, no restart (it is not ours to take)"; else bad "other holder: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; touch "$fake/claims-fail"; tick; rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ] && log_has 'ALERT watcher supervisor: claim state unreadable, not restarting'; then ok "klatch claims failing (store busy, no credential): ALERT, no restart, reports instead"; else bad "unreadable: ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_none; touch "$work/off"; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"; tick
if [ "$(ensures)" -eq 0 ] && log_has 'off file'; then ok "the off file stops restarts"; else bad "off: ensures=$(ensures)"; fi
rm -f "$work/off"; tick
if [ "$(ensures)" -eq 1 ]; then ok "removing the off file lets the next pass restart"; else bad "off removed: ensures=$(ensures)"; fi

# ---- bounded restarts: 3 per hour, backoff 60 s then 120 s then 240 s, then report and stop ----
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"
printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
at 100000
at 100010
if [ "$(ensures)" -eq 1 ]; then ok "ensure fails: the second attempt waits for the backoff (60 s), not retried 10 s later"; else bad "backoff1: $(ensures)"; fi
if log_has 'ensure did not bring the receiver up, next try in 60 s'; then ok "the failed attempt is logged with the wait"; else bad "backoff line: $(log)"; fi
at 100060
if [ "$(ensures)" -eq 2 ]; then ok "after 60 s the second attempt runs"; else bad "attempt2: $(ensures)"; fi
at 100179
if [ "$(ensures)" -eq 2 ]; then ok "the backoff doubles (120 s): 119 s later nothing runs"; else bad "backoff2: $(ensures)"; fi
at 100180
if [ "$(ensures)" -eq 3 ]; then ok "the third attempt runs after 120 s"; else bad "attempt3: $(ensures)"; fi
at 100419
if [ "$(ensures)" -eq 3 ]; then ok "the backoff doubles again (240 s)"; else bad "backoff3: $(ensures)"; fi
at 100420; rc=$?
if [ "$(ensures)" -eq 3 ] && [ "$rc" -eq 3 ] && log_has 'ALERT watcher supervisor: restart limit reached (3 in the last 3600 s), giving up; run gh-watch ensure yourself'; then ok "the fourth restart in the hour is refused: ALERT, exit 3, the human is told to run ensure"; else bad "limit: ensures=$(ensures) rc=$rc $(log)"; fi
at 103000; rc=$?
if [ "$(ensures)" -eq 3 ] && [ "$rc" -eq 3 ]; then ok "it stays stopped while the three attempts are inside the hour"; else bad "still limited: ensures=$(ensures) rc=$rc"; fi
at 103700
if [ "$(ensures)" -eq 4 ]; then ok "an hour after the first attempt the window has room again (attempts older than GHWS_WINDOW_SECS expire)"; else bad "window: ensures=$(ensures)"; fi
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
GHWS_MAX_RESTARTS=1 at 100000; GHWS_MAX_RESTARTS=1 at 100500; rc=$?
if [ "$(ensures)" -eq 1 ] && [ "$rc" -eq 3 ]; then ok "GHWS_MAX_RESTARTS sets the limit (1: one attempt, then stop)"; else bad "limit 1: ensures=$(ensures) rc=$rc"; fi
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
at 100000; pid_dead; claim_none; at 100005
if [ "$(ensures)" -eq 1 ]; then ok "a restart that worked is counted too: a receiver that dies again at once waits for the backoff"; else bad "worked then died: ensures=$(ensures)"; fi

# ---- what it may run, and what text it ever uses ----
newworld; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
# shellcheck disable=SC2016
printf 'watch:gh\t$(touch %s/pwned)\t0s left\treason $(touch %s/pwned2)\n' "$work" "$work" > "$fake/claims.txt"
tick
if [ ! -e "$work/pwned" ] && [ ! -e "$work/pwned2" ] && ! log_has 'pwned'; then ok "claim holder and reason text is data: never run, never logged"; else bad "claim text leaked: $(log)"; fi
bad_lines="$(log | grep -vcE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z (INFO|ALERT) watcher supervisor[: ]' | tr -d ' ')"
if [ "$bad_lines" -eq 0 ] && [ -n "$(log)" ]; then ok "every log line is TIME INFO|ALERT and a fixed template"; else bad "log shape: $(log)"; fi
kcalls="$(grep -E '^klatch ' "$fake/calls.log")"; gcalls="$(grep -E '^gh-watch ' "$fake/calls.log")"
if ! grep -vqE '^klatch (claims|stuck) ' <<< "$kcalls"; then ok "klatch is only ever asked claims and stuck (read only: no post, claim, release, worker or admin)"; else bad "klatch calls: $(grep '^klatch' "$fake/calls.log")"; fi
if ! grep -vqE '^gh-watch ensure ' <<< "$gcalls"; then ok "gh-watch is only ever run as ensure: never serve, forward-command, install-launchd or clean-hooks (the forwarder stays the human's)"; else bad "gh-watch calls: $(grep '^gh-watch' "$fake/calls.log")"; fi
if ! grep -rqE 'launchctl|gh webhook|gh api|install-launchd' "$sup"; then ok "the script never names launchctl, gh webhook, gh api or install-launchd as something to run"; else bad "forbidden command text in the script"; fi

# ---- review of #1608: the evidence must be OUR receiver's (the recorded holder MEMBER.PID), never anyone's ----
st="$od/supervisor"
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none; at 100001
claim_by other.5 4; at 100002
claim_none; at 100010
if [ "$(ensures)" -eq 0 ]; then ok "a clean stop, then another member claims watch:gh and lets it lapse: no restart (its claim is not our crash)"; else bad "foreign claim restarted: ensures=$(ensures) $(log)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none; at 100001
claim_by ghwatch.777 4; at 100002; claim_none; at 100010
if [ "$(ensures)" -eq 0 ]; then ok "a second receiver (same member name, other pid) that held the claim and stopped is not ours either: no restart"; else bad "second receiver restarted: ensures=$(ensures) $(log)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none
printf 'lapsed_claim room factory-status claim watch:gh holder other.5 lapsed 5s\nlapsed_claim room factory-status claim watch:gh holder ghwatch.777 lapsed 5s\n' > "$fake/stuck.txt"; at 100010
if [ "$(ensures)" -eq 0 ]; then ok "a recent lapsed_claim by another holder is not evidence once our holder is recorded"; else bad "foreign lapse restarted: ensures=$(ensures)"; fi
printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.%s lapsed 5s\n' "$alive" > "$fake/stuck.txt"; at 100020
if [ "$(ensures)" -eq 1 ]; then ok "a lapsed_claim by our recorded holder (ghwatch.PID of the receiver that ran) is evidence"; else bad "own lapse not used: ensures=$(ensures)"; fi

# ---- the crash flag goes stale ----
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none
mkdir -p "$st"; printf '90000 ghwatch.%s\n' "$alive" > "$st/crash"; tick
if [ "$(ensures)" -eq 0 ] && [ ! -e "$st/crash" ]; then ok "a crash flag older than GHWS_EVIDENCE_SECS is dropped: no restart"; else bad "stale flag: ensures=$(ensures)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none
printf '99990 ghwatch.%s\n' "$alive" > "$st/crash"; tick
if [ "$(ensures)" -eq 1 ]; then ok "a fresh crash flag of our holder restarts (a --once pass later than the crash still sees it)"; else bad "fresh flag: ensures=$(ensures)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none
printf '99990 other.9\n' > "$st/crash"; tick
if [ "$(ensures)" -eq 0 ]; then ok "a crash flag naming another holder is dropped"; else bad "foreign flag: ensures=$(ensures)"; fi
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_none
printf '100000 ghwatch.%s\n' "$alive" > "$st/crash"
rc=0; GHWS_MAX_TICKS=1 "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$(ensures)" -eq 0 ] && [ ! -e "$st/crash" ]; then ok "a loop run clears old evidence when it starts: a leftover flag never restarts at once"; else bad "leftover flag: ensures=$(ensures)"; fi
newworld; pid_alive; claim_live 60; tick; printf '100000 ghwatch.%s\n' "$alive" > "$st/crash"; tick
if [ ! -e "$st/crash" ]; then ok "a healthy pass removes the crash flag"; else bad "flag survived a healthy pass"; fi
newworld; pid_alive; claim_live 60; tick; printf '100000 ghwatch.%s\n' "$alive" > "$st/crash"; claim_none; tick
if [ ! -e "$st/crash" ]; then ok "a live receiver pid removes the flag even before the claim is back"; else bad "flag survived a live pid"; fi

# ---- ensure runs in its own session ----
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
mypg="$(ps -o pgid= -p $$ | tr -d ' ')"; tick
epg="$(cat "$fake/pgid.log" 2> /dev/null)"
if [ -n "$epg" ] && [ "$epg" != "$mypg" ]; then ok "ensure runs in its own process group, so Ctrl-C to the supervisor's group cannot stop the receiver it starts"; else bad "ensure pgid '$epg' is ours '$mypg'"; fi

# ---- a hung klatch cannot stall it ----
newworld; pid_dead; touch "$fake/claims-hang"; GHWS_KLATCH_TIMEOUT_SECS=1
export GHWS_KLATCH_TIMEOUT_SECS
t0=$SECONDS; tick; rc=$?; took=$((SECONDS - t0))
if [ "$rc" -eq 0 ] && [ "$took" -lt 10 ] && [ "$(ensures)" -eq 0 ] && log_has 'ALERT watcher supervisor: klatch call timed out after 1 s, not restarting'; then ok "klatch claims that hangs is killed after the timeout: ALERT, no restart, the pass returns"; else bad "claims hang: rc=$rc took=${took}s ensures=$(ensures) $(log)"; fi
newworld; pid_dead; claim_none; touch "$fake/stuck-hang"; GHWS_KLATCH_TIMEOUT_SECS=1
export GHWS_KLATCH_TIMEOUT_SECS
t0=$SECONDS; tick; took=$((SECONDS - t0))
if [ "$took" -lt 10 ] && [ "$(ensures)" -eq 0 ] && log_has 'klatch call timed out after 1 s'; then ok "klatch stuck that hangs is killed after the timeout too"; else bad "stuck hang: took=${took}s ensures=$(ensures) $(log)"; fi

# ---- stuck evidence works after a restart too, whatever the poll interval ----
newworld; pid_alive; claim_live 60; tick; pid_dead; claim_live 30; at 100010; claim_none; at 100090
claim_live 90; at 100100
pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.%s lapsed 20s\n' "$alive" > "$fake/stuck.txt"; at 100600
if [ "$(ensures)" -eq 2 ]; then ok "a second crash that was never seen held (a long poll) is restarted from the stuck line, after a first restart"; else bad "second crash missed: ensures=$(ensures) $(log)"; fi

# ---- one supervisor at a time ----
newworld; mkdir -p "$st"; ln -s "$alive" "$st/lock"; pid_dead; claim_none
rc=0; tick || rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 0 ] && ! grep -q '^klatch' "$fake/calls.log" && [ "$(readlink "$st/lock")" = "$alive" ]; then ok "a --once pass while another supervisor holds the lock does nothing and leaves the lock alone"; else bad "lock held: rc=$rc calls=$(cat "$fake/calls.log")"; fi
rc=0; "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 1 ] && grep -q 'another supervisor is running' "$fake/out"; then ok "a second loop supervisor on the same state exits 1"; else bad "second loop rc=$rc $(cat "$fake/out")"; fi
rm -f "$st/lock"; ln -s "$dead" "$st/lock"; tick
if [ ! -e "$st/lock" ] && [ -L "$st/lock" ] ; then bad "lock left after a pass"; elif grep -q '^klatch claims' "$fake/calls.log" && [ ! -L "$st/lock" ]; then ok "a dead holder's lock is taken over, and the lock is released when the pass ends"; else bad "stale lock: $(cat "$fake/calls.log")"; fi

# ---- setup details from the review ----
newworld; rm -f "$KLATCH_HOME_DIR/rooms.db"; rc=0; tick || rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/calls.log" ] && grep -q 'rooms.db' "$fake/out"; then ok "a KLATCH_HOME with no rooms.db is refused (klatch would create a store there): exit 2, nothing run"; else bad "no rooms.db rc=$rc"; fi
newworld; mkdir -p "$work/hh" "$work/real-kl"; rm -f "$work/hh/.klatch"; ln -s "$work/real-kl" "$work/hh/.klatch"; : > "$work/real-kl/rooms.db"
rc=0; HOME="$work/hh" KLATCH_HOME="$work/real-kl" "$sup" --once "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/calls.log" ]; then ok "a home that is the real path of ~/.klatch (itself a symlink) is refused, as the receiver does"; else bad "real path of ~/.klatch rc=$rc"; fi
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"; touch "$fake/ensure-fails"
GHWS_BACKOFF_SECS=08 GHWS_POLL_SECS=08 GHWS_EVIDENCE_SECS=0900 at 100000
GHWS_BACKOFF_SECS=08 at 100007; n7="$(ensures)"; GHWS_BACKOFF_SECS=08 at 100008; n8="$(ensures)"
if [ "$n7" -eq 1 ] && [ "$n8" -eq 2 ]; then ok "numbers with a leading zero (08) are decimal, not octal"; else bad "octal: n7=$n7 n8=$n8 out=$(cat "$fake/out")"; fi
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"; tick
if ! grep -q 'No such file' "$fake/out"; then ok "a fresh state dir prints no No such file error"; else bad "stderr noise: $(cat "$fake/out")"; fi
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"; touch "$fake/ensure-fails"
GHWS_MAX_RESTARTS=1 at 100000; GHWS_MAX_RESTARTS=1 at 100500; GHWS_MAX_RESTARTS=1 at 100600
if [ "$(log | grep -c 'restart limit reached')" -eq 1 ]; then ok "the limit ALERT is written once, not on every --once pass"; else bad "limit alert repeated: $(log)"; fi
# the 600 s cap on the wait: base 400 gives 400 then 800, capped to 600
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"; touch "$fake/ensure-fails"
export GHWS_EVIDENCE_SECS=5000 GHWS_LAPSE_MAX_AGE_SECS=100000 GHWS_BACKOFF_SECS=400
at 100000; at 100399; n1="$(ensures)"; at 100400; n2="$(ensures)"; at 100999; n3="$(ensures)"; at 101000; n4="$(ensures)"
if [ "$n1" -eq 1 ] && [ "$n2" -eq 2 ] && [ "$n3" -eq 2 ] && [ "$n4" -eq 3 ]; then ok "the wait doubles but never passes 600 s (400, then 600 not 800)"; else bad "cap: $n1 $n2 $n3 $n4"; fi
unset GHWS_LAPSE_MAX_AGE_SECS

# ---- loop mode: bounded by GHWS_MAX_TICKS under the test clock, so it never sleeps ----
newworld; pid_dead; claim_none; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
rc=0; GHWS_MAX_TICKS=3 "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 0 ] && [ "$(ensures)" -eq 1 ]; then ok "loop mode runs the bounded number of passes, restarts once, and exits 0"; else bad "loop: rc=$rc ensures=$(ensures) out=$(cat "$fake/out")"; fi
newworld; pid_dead; claim_none; touch "$fake/ensure-fails"; printf 'lapsed_claim room factory-status claim watch:gh holder ghwatch.4242 lapsed 5s\n' > "$fake/stuck.txt"
rc=0; GHWS_BACKOFF_SECS=0 GHWS_MAX_RESTARTS=1 GHWS_MAX_TICKS=50 "$sup" "${args[@]}" > "$fake/out" 2>&1 || rc=$?
if [ "$rc" -eq 3 ] && [ "$(ensures)" -eq 1 ]; then ok "loop mode stops with exit 3 when the restart limit is reached"; else bad "loop limit: rc=$rc ensures=$(ensures)"; fi

if [ "$fails" -eq 0 ]; then echo "all gh-watch-supervise tests passed"; else echo "$fails FAILED"; exit 1; fi
