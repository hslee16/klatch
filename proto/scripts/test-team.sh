#!/usr/bin/env bash
# Tests for proto/scripts/team (#582). The launcher runs against a fake `klatch` (records what it is asked, writes a
# bundle on `worker issue`) and a fake `claude` (records its arguments, environment and directory), in a temp git
# repo and a temp TEAM_DIR. Nothing here touches a real home, keystore or model.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
team="$here/team"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-team-test.XXXXXX")"
cleanup() {
  for p in "$work"/td*/run/*.pid; do
    [ -f "$p" ] && kill -9 "$(cat "$p")" 2> /dev/null
  done
  pkill -9 -f "$work/bin" 2> /dev/null
  rm -rf "$work"
}
trap cleanup EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "$*" >> "$FAKE_DIR/klatch.log"
if [ "$1 $2" = "worker issue" ]; then
  member="$3"
  [ "$member" = "${FAKE_ISSUE_FAIL:-}" ] && { echo "credentials cannot be managed from inside a coding-agent session" >&2; exit 1; }
  while [ $# -gt 0 ]; do [ "$1" = "--out" ] && out="$2"; shift; done
  ( umask 077; echo "member $member" > "$out" )
fi
exit 0
SH
cat > "$bin/claude" <<'SH'
#!/bin/sh
name="$(basename "${KLATCH_IDENTITY_FILE:-none}" .bundle)"
d="$FAKE_DIR/claude-$name"; mkdir -p "$d"
for a in "$@"; do printf '%s\n' "$a"; done > "$d/args"
pwd > "$d/cwd"
printf 'identity=%s\nhome=%s\nname=%s\ndiscover=%s\n' "${KLATCH_IDENTITY_FILE:-}" "${KLATCH_HOME:-}" "${KLATCH_NAME:-}" "${KLATCH_DISCOVER:-}" > "$d/env"
while [ $# -gt 0 ]; do [ "$1" = "--settings" ] && cp "$2" "$d/settings.json"; shift; done
echo '{"type":"system","subtype":"init"}'
[ -n "${FAKE_CLAUDE_SLEEP:-}" ] && /bin/sleep "$FAKE_CLAUDE_SLEEP"
echo '{"type":"result","subtype":"success","total_cost_usd":0.0123,"num_turns":3}'
SH
chmod +x "$bin/klatch" "$bin/claude"

# each scenario gets a throwaway repo with one commit on main, so branches and worktrees never leak between them
n=0
td=""; fake=""; repo=""
newenv() {
  n=$((n + 1)); td="$work/td$n"; fake="$work/fake$n"; repo="$work/repo$n"; mkdir -p "$fake" "$repo"
  git -C "$repo" init -q -b main
  git -C "$repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
}
run_team() {
  env TEAM_KLATCH="$bin/klatch" TEAM_CLAUDE="$bin/claude" TEAM_REPO="$repo" TEAM_DIR="$td" TEAM_BASE=main \
    KLATCH_HOME="$work/home" FAKE_DIR="$fake" "$team" "$@"
}
waitfor() { # a file, for up to about 20 seconds
  local i=0
  while [ ! -e "$1" ] && [ "$i" -lt 100 ]; do /bin/sleep 0.2; i=$((i + 1)); done
  [ -e "$1" ]
}
perm() { stat -f '%Lp' "$1" 2> /dev/null || stat -c '%a' "$1"; }
tasks="$work/tasks.txt"
cat > "$tasks" <<'T'
# name | ticket | prompt
alpha | 11 | Build the alpha thing; open a PR.
beta | 12 | Fix the beta bug and say so in the room.
T

if "$team" > "$work/out" 2>&1; then bad "no subcommand was accepted"; elif [ "$?" -eq 2 ] && grep -q "Usage" "$work/out"; then ok "no subcommand prints usage and exits 2"; else bad "usage: $(cat "$work/out")"; fi
if "$team" frobnicate > "$work/out" 2>&1; then bad "unknown subcommand accepted"; else ok "an unknown subcommand is refused"; fi

# --- start: the happy path
newenv
run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"; waitfor "$td/run/beta.exit"
a="$fake/claude-alpha"
if [ "$rc" -eq 0 ]; then ok "start exits 0 when every worker started"; else bad "start rc=$rc: $(cat "$work/out")"; fi
if grep -q "^worker issue alpha --clearance public --ttl 7200 --parent launcher --out $td/bundles/alpha.bundle$" "$fake/klatch.log"; then ok "a bundle is issued per task with clearance, ttl, parent and an --out path"; else bad "issue args: $(cat "$fake/klatch.log")"; fi
if [ "$(perm "$td/bundles/alpha.bundle")" = 600 ] && [ "$(perm "$td/bundles")" = 700 ]; then ok "the bundle is mode 600 in a 700 directory"; else bad "modes"; fi
if [ -d "$td/worktrees/alpha" ] && [ "$(git -C "$td/worktrees/alpha" branch --show-current)" = "alee/alpha" ]; then ok "each worker gets a worktree on branch alee/<name>"; else bad "worktree"; fi
want_cwd="$(cd "$td/worktrees/alpha" && pwd -P)"
if [ "$(cd "$(cat "$a/cwd")" && pwd -P)" = "$want_cwd" ]; then ok "claude runs in the worker's worktree"; else bad "cwd $(cat "$a/cwd")"; fi
if grep -q "^identity=$td/bundles/alpha.bundle$" "$a/env" && grep -q "^home=$work/home$" "$a/env" && grep -q "^name=$" "$a/env" && grep -q "^discover=$" "$a/env"; then ok "the worker gets the bundle path and KLATCH_HOME, and no KLATCH_NAME or discovery"; else bad "env: $(cat "$a/env")"; fi
args="$(cat "$a/args")"
missing=""
for want in "-p" "--output-format" "stream-json" "--verbose" "--settings" "--max-budget-usd" "2" "--permission-mode" "--setting-sources" "project"; do
  printf '%s\n' "$args" | grep -qx -- "$want" || missing="$missing $want"
done
if [ -z "$missing" ]; then ok "claude is run headless with stream-json, verbose, a budget cap and its own settings"; else bad "claude args missing:$missing"; fi
if printf '%s' "$args" | grep -q "member alpha" && printf '%s' "$args" | grep -q "ticket #11" && printf '%s' "$args" | grep -q "Build the alpha thing" && ! printf '%s' "$args" | grep -q "KLATCH_NAME="; then ok "the brief names the member, the ticket and the task, and has no KLATCH_NAME"; else bad "brief"; fi
pr_missing=""
for want in 'Closes #' 'python3 -I proto/scripts/pr-ledger' "reaches \`ready\`" '--rerun-cancelled' 'Never rerun a real red' 'ready: #' 'release your claims' 'Never merge'; do
  printf '%s' "$args" | grep -qF -- "$want" || pr_missing="$pr_missing [$want]"
done
if [ -z "$pr_missing" ]; then ok "the brief carries the PR rules (Closes #N, poll pr-ledger to ready, rerun cancelled, fix red, post ready, release claims)"; else bad "brief PR rules missing:$pr_missing"; fi
if python3 - "$a/settings.json" "$bin/klatch" <<'PY'
import json, sys
s = json.load(open(sys.argv[1])); k = sys.argv[2]; h = s["hooks"]
need = ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop", "SessionEnd"]
assert all(e in h for e in need), list(h)
for e in need:
    cmds = [x["command"] for m in h[e] for x in m["hooks"]]
    assert any(c.startswith(k + " hook --event " + e) for c in cmds), (e, cmds)
    assert not any("--discover" in c for c in cmds), (e, cmds)
assert "asyncRewake" not in json.dumps(s)
PY
then ok "the settings install the klatch hooks by absolute path, without discovery and without the wake watcher"; else bad "settings: $(cat "$a/settings.json")"; fi
if grep -q "^worker revoke alpha$" "$fake/klatch.log"; then ok "the credential is revoked when the session ends"; else bad "no revoke: $(cat "$fake/klatch.log")"; fi
if grep -q '"type":"result"' "$td/run/alpha.log" && [ "$(cat "$td/run/alpha.exit")" = 0 ]; then ok "stream-json goes to a log per worker and the exit code is recorded"; else bad "log/exit"; fi
if [ -f "$td/run/alpha.pid" ] && [ -f "$td/run/beta.pid" ]; then ok "a pid file per worker"; else bad "pid files"; fi
run_team status > "$work/out" 2>&1
if grep -q "alpha" "$work/out" && grep -q "done" "$work/out" && grep -q "0.0123" "$work/out"; then ok "status shows each worker, whether it finished, and its cost"; else bad "status: $(cat "$work/out")"; fi
run_team logs alpha > "$work/out" 2>&1
if grep -q '"type":"result"' "$work/out"; then ok "logs prints a worker's log"; else bad "logs"; fi
if run_team logs nobody > "$work/out" 2>&1; then bad "logs of an unknown worker succeeded"; else ok "logs of an unknown worker fails"; fi

# --- the wake watcher is opt in
newenv
TEAM_WAKE=1 run_team start "$tasks" > "$work/out" 2>&1; waitfor "$td/run/alpha.exit"
if grep -q '"asyncRewake": *true' "$fake/claude-alpha/settings.json" && grep -q "wake --timeout" "$fake/claude-alpha/settings.json"; then ok "TEAM_WAKE=1 adds the asyncRewake wake watcher"; else bad "wake settings"; fi

# --- the tasks file is checked before anything is issued
newenv
printf 'Bad Name | 1 | x\n' > "$work/t1"; printf 'a | 1 | x\na | 2 | y\n' > "$work/t2"; printf 'a | 1\n' > "$work/t3"; printf 'a | one | x\n' > "$work/t4"
for t in t1 t2 t3 t4; do
  run_team start "$work/$t" > "$work/out" 2>&1; rc=$?
  if [ "$rc" -eq 2 ]; then ok "bad tasks file $t is refused with exit 2"; else bad "$t exit $rc"; fi
done
if [ ! -s "$fake/klatch.log" ]; then ok "nothing is issued when the tasks file is bad"; else bad "issued: $(cat "$fake/klatch.log")"; fi

# --- an issue failure starts no worker for that task and the rest go on
newenv
FAKE_ISSUE_FAIL=alpha run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/beta.exit"
if [ "$rc" -eq 1 ] && [ ! -d "$td/worktrees/alpha" ] && [ ! -d "$fake/claude-alpha" ] && [ -d "$fake/claude-beta" ] && grep -q "alpha" "$work/out"; then ok "a refused issue starts nothing for that task, says why, exits 1, and the others still run"; else bad "issue failure rc=$rc: $(cat "$work/out")"; fi

# --- stop and the watchdog
newenv
printf 'slow | 5 | wait\n' > "$work/ts"
FAKE_CLAUDE_SLEEP=300 run_team start "$work/ts" > "$work/out" 2>&1
waitfor "$td/run/slow.pid"; waitfor "$fake/claude-slow/env"
run_team status > "$work/out" 2>&1
if grep -q "running" "$work/out"; then ok "status shows a live worker as running"; else bad "status running: $(cat "$work/out")"; fi
run_team stop > "$work/out" 2>&1
/bin/sleep 1
if ! kill -0 "$(cat "$td/run/slow.pid")" 2> /dev/null && grep -q "^worker revoke slow$" "$fake/klatch.log"; then ok "stop kills the worker and revokes its credential"; else bad "stop: $(cat "$work/out")"; fi
run_team status > "$work/out" 2>&1
if grep -q "stopped" "$work/out"; then ok "status shows a stopped worker as stopped"; else bad "status stopped: $(cat "$work/out")"; fi
newenv
TEAM_MAX_SECONDS=1 FAKE_CLAUDE_SLEEP=300 run_team start "$work/ts" > "$work/out" 2>&1
if waitfor "$td/run/slow.exit" && [ "$(cat "$td/run/slow.exit")" != 0 ] && grep -q "^worker revoke slow$" "$fake/klatch.log"; then ok "the watchdog kills a worker past TEAM_MAX_SECONDS and revokes it"; else bad "watchdog"; fi
newenv
run_team start "$work/ts" > "$work/out" 2>&1; waitfor "$td/run/slow.exit"
run_team start "$work/ts" > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && [ "$(grep -c '^worker issue slow' "$fake/klatch.log")" -eq 2 ]; then ok "a finished worker can be started again"; else bad "restart rc=$rc: $(cat "$work/out")"; fi
waitfor "$td/run/slow.exit"
run_team stop > /dev/null 2>&1

# --- smoke runs one worker in a throwaway home, never the caller's KLATCH_HOME
newenv
TEAM_KLATCH="$bin/klatch" TEAM_CLAUDE="$bin/claude" TMPDIR="$work" KLATCH_HOME="$work/home" FAKE_DIR="$fake" "$team" smoke > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && grep -q "^home=$work/klatch-team-smoke" "$fake/claude-smoke/env" && ! grep -q "home=$work/home$" "$fake/claude-smoke/env" && grep -q "^worker issue smoke " "$fake/klatch.log"; then ok "smoke uses a throwaway KLATCH_HOME, not the caller's"; else bad "smoke rc=$rc: $(cat "$work/out")"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all team script tests passed"; else echo "$fails team script test(s) failed"; exit 1; fi
