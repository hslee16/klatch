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
# a sentinel watchdog length no other process on the machine uses (random per run, so two runs at once do not see each
# other's watchdogs, #1142), so the leak check at the end sees only this test's (#1101)
sentinel=$((3100 + RANDOM % 4000))
export TEAM_MAX_SECONDS=$sentinel
# the relaunch (#1011) is off for the older scenarios, which count credentials; its own scenarios turn it on
export TEAM_MAX_RESTARTS=0 TEAM_RESTART_PAUSE_SECS=0
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
if [ "$1" = read ]; then
  m="$(basename "${KLATCH_IDENTITY_FILE:-none}" .bundle)"
  u="$(cat "$FAKE_DIR/unread-$m" 2> /dev/null || echo 0)"
  case " $* " in
    *" --count "*) echo "$u" ;;
    *" --peek "*) i=0; while [ "$i" -lt "$u" ]; do i=$((i + 1)); echo "[$i 2026-11-21 bob] hello mail $i"; done ;;
  esac
fi
if [ "$1" = claims ]; then
  [ -n "${FAKE_CLAIMS_FAIL:-}" ] && { echo "no such room" >&2; exit 1; }
  [ -n "${FAKE_CLAIMS:-}" ] && cat "$FAKE_CLAIMS"
  echo "claims-as=${KLATCH_NAME:-} room=$2" >> "$FAKE_DIR/claims.env"
fi
exit 0
SH
cat > "$bin/claude" <<'SH'
#!/bin/sh
name="$(basename "${KLATCH_IDENTITY_FILE:-none}" .bundle)"
d="$FAKE_DIR/claude-$name"; mkdir -p "$d"
for a in "$@"; do printf '%s\n' "$a"; done > "$d/args.tmp"; mv "$d/args.tmp" "$d/args" # whole or absent, never half written
pwd > "$d/cwd"
date +%s >> "$d/starts"
k="$(wc -l < "$d/starts" | tr -d ' ')"
prev=""
for a in "$@"; do [ "$prev" = "-p" ] && printf '%s' "$a" > "$d/prompt.$k"; prev="$a"; done
[ -n "${FAKE_CLAUDE_DRAIN:-}" ] && [ "$k" -ge 2 ] && rm -f "$FAKE_DIR/unread-$name"
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
waitfor() { # a file, for up to about 2 minutes (a loaded machine starts the background worker late: 20 s was too tight, #1099)
  local i=0
  while [ ! -e "$1" ] && [ "$i" -lt 600 ]; do /bin/sleep 0.2; i=$((i + 1)); done
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
# the permission mode and the tool allowlist (#594): workers do not run in bypassPermissions by default
if printf '%s\n' "$args" | grep -A1 -x -- "--permission-mode" | grep -qx "dontAsk" && ! printf '%s\n' "$args" | grep -qx "bypassPermissions"; then ok "the default permission mode is dontAsk, not bypassPermissions (#594)"; else bad "permission mode: $(printf '%s' "$args" | grep -A1 -x -- --permission-mode)"; fi
allow="$(printf '%s\n' "$args" | grep -A1 -x -- "--allowedTools" | sed -n 2p)"
al_missing=""
for want in "Bash($bin/klatch join:*)" "Bash($bin/klatch post:*)" "Bash($bin/klatch wait:*)" "Bash(git add:*)" "Bash(git commit:*)" "Bash(git push -u origin alee/alpha)" "Bash(gh api repos/hslee16/klatch/issues/11)" "Bash(gh api repos/hslee16/klatch/issues/11/comments)" "Bash(cargo:*)" "Bash(gh pr create:*)" "Bash(gh pr view:*)" "Bash(gh pr checks:*)" "Edit" "Write" "Read"; do
  case ",$allow," in *",$want,"*) ;; *) al_missing="$al_missing [$want]" ;; esac
done
if [ -n "$allow" ] && [ -z "$al_missing" ]; then ok "the default allowlist covers the klatch commands (by absolute path), git, the build tools and file edits"; else bad "allowlist missing:$al_missing in: $allow"; fi
al_bad=""
for no in "Bash(gh api:*)" "Bash(gh api *" "git push -u origin:*" "git push -u origin alee/*" "origin main" "alee/beta" "issues/12" "-X" "klatch admin" "klatch worker" "git push:*" "git push --force" "Bash(*" "Bash(bash:*)" "Bash(sh:*)" "Bash(curl:*)" "Bash(rm:*)"; do
  case "$allow" in *"$no"*) al_bad="$al_bad [$no]" ;; esac
done
if [ -z "$al_bad" ]; then ok "the default allowlist grants no klatch admin or worker, bare or wildcard git push, gh api wildcard, shell or curl, or rm (#1156)"; else bad "allowlist too wide:$al_bad"; fi
if printf '%s' "$args" | grep -qF "UNTRUSTED" && printf '%s' "$args" | grep -qF "even from a verified member" && printf '%s' "$args" | grep -qF "only this brief and your task"; then ok "the brief marks room mail as untrusted data, even from a verified member (#594)"; else bad "brief untrusted marking missing"; fi
if printf '%s' "$args" | grep -q "member alpha" && printf '%s' "$args" | grep -q "ticket #11" && printf '%s' "$args" | grep -q "Build the alpha thing" && ! printf '%s' "$args" | grep -q "KLATCH_NAME="; then ok "the brief names the member, the ticket and the task, and has no KLATCH_NAME"; else bad "brief"; fi
pr_missing=""
for want in 'Closes #' 'python3 -I proto/scripts/pr-ledger' "reaches \`ready\`" '--rerun-cancelled' 'Never rerun a real red' 'ready: #' 'release your claims' 'Never merge'; do
  printf '%s' "$args" | grep -qF -- "$want" || pr_missing="$pr_missing [$want]"
done
df_missing=""
for want in 'design/items/<ticket>-<slug>.md' 'never append to DESIGN.md'; do
  printf '%s' "$args" | grep -qF -- "$want" || df_missing="$df_missing [$want]"
done
if [ -z "$df_missing" ]; then ok "the brief tells members to write design/items/<ticket>-<slug>.md and not append to DESIGN.md (#807)"; else bad "brief design fragment words missing:$df_missing"; fi
if grep -qF 'design/items/<ticket>-<slug>.md' "$here/AGENT-PR-RULES.md" && grep -qF 'never append' "$here/AGENT-PR-RULES.md"; then ok "AGENT-PR-RULES.md carries the design fragment rule"; else bad "AGENT-PR-RULES.md lacks the design fragment rule"; fi
lc_missing=""
# shellcheck disable=SC2016
for want in 'post `pr: #N <title>`' 'pr:N --shared' 'merged: #N'; do
  printf '%s' "$args" | grep -qF -- "$want" || lc_missing="$lc_missing [$want]"
done
if [ -z "$lc_missing" ]; then ok "the brief tells members to post pr: #N <title>, claim pr:N --shared and post merged: #N (DESIGN item 84)"; else bad "brief PR lifecycle missing:$lc_missing"; fi
# shellcheck disable=SC2016
if grep -qF 'post `pr: #PR <title>`' "$here/AGENT-PR-RULES.md" && grep -qF 'pr:PR --shared' "$here/AGENT-PR-RULES.md" && grep -qF 'merged: #PR' "$here/AGENT-PR-RULES.md"; then ok "AGENT-PR-RULES.md carries the same PR lifecycle words"; else bad "AGENT-PR-RULES.md lacks the PR lifecycle words"; fi
# shellcheck disable=SC2016
for want in 'at most once every 3 minutes' '--budget-floor' '--timeout 180'; do
  printf '%s' "$args" | grep -qF -- "$want" || pr_missing="$pr_missing [$want]"
  grep -qF -- "$want" "$here/AGENT-PR-RULES.md" || pr_missing="$pr_missing [rules: $want]"
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

# --- the permission mode and allowlist can be overridden, and only on request (#594)
newenv
TEAM_PERMISSION_MODE=bypassPermissions run_team start "$tasks" > "$work/out" 2>&1; waitfor "$td/run/alpha.exit"
if printf '%s\n' "$(cat "$fake/claude-alpha/args")" | grep -A1 -x -- "--permission-mode" | grep -qx "bypassPermissions"; then ok "TEAM_PERMISSION_MODE=bypassPermissions restores bypassPermissions when asked"; else bad "bypass override: $(cat "$work/out")"; fi
newenv
TEAM_ALLOWED_TOOLS="Read,Bash(echo:*)" run_team start "$tasks" > "$work/out" 2>&1; waitfor "$td/run/alpha.exit"
if [ "$(grep -A1 -x -- "--allowedTools" "$fake/claude-alpha/args" | sed -n 2p)" = "Read,Bash(echo:*)" ] && grep -A1 -x -- "--permission-mode" "$fake/claude-alpha/args" | grep -qx "dontAsk"; then ok "TEAM_ALLOWED_TOOLS replaces the allowlist and the mode stays dontAsk"; else bad "allow override: $(cat "$fake/claude-alpha/args" | grep -A1 allowed)"; fi
newenv
TEAM_PERMISSION_MODE='bad mode' run_team start "$tasks" > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/klatch.log" ]; then ok "an unknown TEAM_PERMISSION_MODE is refused with exit 2 before anything is issued"; else bad "bad mode rc=$rc"; fi

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

# --- TEAM_MAX_TOTAL_USD: a team-wide budget, checked before anything is issued (#593)
newenv
TEAM_MAX_TOTAL_USD=3 run_team start "$tasks" > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ] && grep -q "TEAM_MAX_TOTAL_USD" "$work/out" && [ ! -s "$fake/klatch.log" ] && [ ! -d "$td/worktrees/alpha" ]; then ok "two workers at 2 USD each are refused under a 3 USD team budget, exit 2, nothing issued"; else bad "team budget refusal rc=$rc: $(cat "$work/out")"; fi
newenv
TEAM_MAX_TOTAL_USD=4 run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"; waitfor "$td/run/beta.exit"
if [ "$rc" -eq 0 ] && [ -d "$fake/claude-alpha" ] && [ -d "$fake/claude-beta" ]; then ok "a team budget that covers every worker's cap lets the team start"; else bad "team budget ok rc=$rc: $(cat "$work/out")"; fi
printf 'gamma | 13 | Do gamma.\n' > "$work/tg"
TEAM_MAX_TOTAL_USD=2.01 run_team start "$work/tg" > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ] && grep -q "already spent" "$work/out" && [ ! -d "$fake/claude-gamma" ]; then ok "money already spent by earlier workers counts against the team budget"; else bad "team budget spent rc=$rc: $(cat "$work/out")"; fi
TEAM_MAX_TOTAL_USD=2.5 run_team start "$work/tg" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/gamma.exit"
if [ "$rc" -eq 0 ] && [ -d "$fake/claude-gamma" ]; then ok "the same launch fits under a larger team budget"; else bad "team budget fits rc=$rc: $(cat "$work/out")"; fi
newenv
TEAM_MAX_TOTAL_USD=abc run_team start "$tasks" > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ] && [ ! -s "$fake/klatch.log" ]; then ok "a non-numeric TEAM_MAX_TOTAL_USD is refused with exit 2"; else bad "bad total rc=$rc"; fi

# --- a ticket someone else already holds on the board is not launched (#593)
newenv
printf 'ticket #11\tsomeone\t3000s left\tworking on it\nsrc/x.rs\tother\t100s left\t\n' > "$fake/claims.txt"
FAKE_CLAIMS="$fake/claims.txt" run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/beta.exit"
if [ "$rc" -eq 1 ] && grep -q "ticket #11 is already claimed by someone" "$work/out" && ! grep -q "^worker issue alpha" "$fake/klatch.log" && [ ! -d "$fake/claude-alpha" ] && [ -d "$fake/claude-beta" ]; then ok "a task whose ticket is claimed by another member starts nothing, says who holds it, and the rest still run"; else bad "claimed ticket rc=$rc: $(cat "$work/out")"; fi
if grep -q "^claims-as=launcher room=lobby$" "$fake/claims.env"; then ok "the board is read as the launcher in the team room"; else bad "claims env: $(cat "$fake/claims.env" 2> /dev/null)"; fi
newenv
printf 'ticket #11\talpha\t3000s left\tmine\nticket #110\tsomeone\t10s left\t\n' > "$fake/claims.txt"
FAKE_CLAIMS="$fake/claims.txt" run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"
if [ "$rc" -eq 0 ] && [ -d "$fake/claude-alpha" ]; then ok "a ticket held by the worker's own name, or a longer number that shares a prefix, does not block"; else bad "own claim rc=$rc: $(cat "$work/out")"; fi
newenv
FAKE_CLAIMS_FAIL=1 run_team start "$tasks" > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"
if [ "$rc" -eq 0 ] && [ -d "$fake/claude-alpha" ] && grep -q "could not read the claims" "$work/out"; then ok "a board that cannot be read warns and does not block the launch"; else bad "claims fail rc=$rc: $(cat "$work/out")"; fi
newenv
printf 'nt | - | No ticket.\n' > "$work/tn"
FAKE_CLAIMS="$fake/claims.txt" run_team start "$work/tn" > "$work/out" 2>&1; waitfor "$td/run/nt.exit"
if [ ! -f "$fake/claims.env" ]; then ok "a task with no ticket does not read the board"; else bad "read the board for no ticket"; fi

# --- relaunch on unread mail (#1011, #591): `team resume NAME` and the runner that calls it when a worker ends
newenv
printf 'alpha | 11 | Build the alpha thing; open a PR.\n' > "$work/t1w"
TEAM_MAX_RESTARTS=3 run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.note"
if [ "$(cat "$td/run/alpha.note")" = "no unread mail" ] && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 1 ]; then ok "a worker that ends with no unread mail is not relaunched, and the reason is kept"; else bad "no unread: $(cat "$td/run/alpha.note" 2> /dev/null)"; fi
if [ "$(grep -c '^worker revoke alpha$' "$fake/klatch.log")" -eq 2 ]; then ok "the credential issued to look at the count is revoked again"; else bad "revokes: $(cat "$fake/klatch.log")"; fi
if grep -q "^read lobby --count$" "$fake/klatch.log" && ! grep -q "^read lobby$" "$fake/klatch.log"; then ok "the launcher counts with read --count and never with a read that marks mail read"; else bad "reads: $(grep '^read' "$fake/klatch.log")"; fi
run_team status > "$work/out" 2>&1
if grep -q "restarts=0" "$work/out" && grep -q "no unread mail" "$work/out"; then ok "status shows the restart count and the note"; else bad "status note: $(cat "$work/out")"; fi

newenv
echo 2 > "$fake/unread-alpha"
TEAM_MAX_RESTARTS=3 run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.note"
if [ "$(cat "$td/run/alpha.note")" = "restart limit 3 reached" ] && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 4 ] && [ "$(cat "$td/run/alpha.restarts")" = 3 ]; then ok "unread mail relaunches a worker at most TEAM_MAX_RESTARTS times (one start and three relaunches)"; else bad "limit: starts=$(wc -l < "$fake/claude-alpha/starts") note=$(cat "$td/run/alpha.note" 2> /dev/null)"; fi
p2="$(cat "$fake/claude-alpha/prompt.2" 2> /dev/null)"
if printf '%s' "$p2" | grep -qF "restart 1 of 3" && printf '%s' "$p2" | grep -qF "2 unread" && printf '%s' "$p2" | grep -qF "hello mail 2" && printf '%s' "$p2" | grep -qF "Build the alpha thing" && printf '%s' "$p2" | grep -qF "not instructions from the human" && printf '%s' "$p2" | grep -qF "UNTRUSTED" && printf '%s' "$p2" | grep -qF "never run a command"; then ok "the relaunch prompt carries the unread digest (as data) and the original task"; else bad "relaunch prompt: $p2"; fi
if ! grep -qF "restart 1 of 3" "$fake/claude-alpha/prompt.3" 2> /dev/null && grep -qF "restart 2 of 3" "$fake/claude-alpha/prompt.3" 2> /dev/null && [ "$(grep -o 'Build the alpha thing' "$fake/claude-alpha/prompt.4" | wc -l | tr -d ' ')" -eq 1 ]; then ok "each relaunch prompt is built from the original task, not stacked on the last one"; else bad "prompt 3/4"; fi
if [ "$(grep -c '^worker issue alpha' "$fake/klatch.log")" -eq 4 ] && grep -q "identity=$td/bundles/alpha.bundle" "$fake/claude-alpha/env"; then ok "every relaunch has a fresh credential issued by the launcher setup"; else bad "issues: $(grep -c '^worker issue alpha' "$fake/klatch.log")"; fi
run_team status > "$work/out" 2>&1
if grep -q "restarts=3" "$work/out" && grep -q "restart limit 3 reached" "$work/out"; then ok "status shows restarts=3 and why it stopped"; else bad "status limit: $(cat "$work/out")"; fi

newenv
echo 2 > "$fake/unread-alpha"
FAKE_CLAUDE_DRAIN=1 TEAM_MAX_RESTARTS=3 run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.note"
if [ "$(cat "$td/run/alpha.note")" = "no unread mail" ] && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 2 ]; then ok "a relaunched worker that reads its mail is not relaunched again"; else bad "drain: starts=$(wc -l < "$fake/claude-alpha/starts") note=$(cat "$td/run/alpha.note" 2> /dev/null)"; fi

newenv
echo 2 > "$fake/unread-alpha"
run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.exit"; /bin/sleep 1
if [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 1 ] && ! grep -q "^read " "$fake/klatch.log" && [ ! -f "$td/run/alpha.note" ]; then ok "TEAM_MAX_RESTARTS=0 turns the relaunch off: no count is read, nothing is relaunched"; else bad "off: $(grep '^read' "$fake/klatch.log")"; fi
run_team resume alpha > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"
if [ "$rc" -eq 1 ] && grep -q "restart limit 0 reached" "$work/out"; then ok "resume under TEAM_MAX_RESTARTS=0 refuses and says why"; else bad "resume at 0: rc=$rc $(cat "$work/out")"; fi
FAKE_CLAUDE_DRAIN=1 TEAM_MAX_RESTARTS=3 run_team resume alpha > "$work/out" 2>&1; rc=$?
waitfor "$td/run/alpha.exit"; waitfor "$td/run/alpha.note"
if [ "$rc" -eq 0 ] && grep -q "resumed alpha (restart 1 of 3, 2 unread" "$work/out" && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 2 ] && grep -qF "hello mail 1" "$fake/claude-alpha/prompt.2"; then ok "team resume NAME relaunches a worker with unread mail, digest as the prompt"; else bad "manual resume rc=$rc: $(cat "$work/out")"; fi
rm -f "$fake/unread-alpha"
TEAM_MAX_RESTARTS=3 run_team resume alpha > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && grep -q "no unread mail" "$work/out" && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 2 ]; then ok "team resume with nothing unread relaunches nothing and exits 0"; else bad "resume nothing rc=$rc: $(cat "$work/out")"; fi
run_team resume nobody > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ]; then ok "resume of an unknown worker exits 2"; else bad "resume unknown rc=$rc"; fi

newenv
echo 2 > "$fake/unread-slow"
FAKE_CLAUDE_SLEEP=300 TEAM_MAX_RESTARTS=3 run_team start "$work/ts" > "$work/out" 2>&1
waitfor "$td/run/slow.pid"; waitfor "$fake/claude-slow/env"
TEAM_MAX_RESTARTS=3 run_team resume slow > "$work/out" 2>&1; rc=$?
if [ "$rc" -ne 0 ] && grep -q "already running" "$work/out"; then ok "resume refuses a worker that is still running"; else bad "resume running rc=$rc: $(cat "$work/out")"; fi
run_team stop > /dev/null 2>&1
TEAM_MAX_RESTARTS=3 run_team resume slow > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 1 ] && grep -q "stopped by hand" "$work/out" && [ "$(wc -l < "$fake/claude-slow/starts" | tr -d ' ')" -eq 1 ]; then ok "a worker stopped with team stop is never relaunched, whatever its unread count"; else bad "resume stopped rc=$rc: $(cat "$work/out")"; fi

newenv
echo 2 > "$fake/unread-alpha"
TEAM_MAX_TOTAL_USD=2.01 TEAM_MAX_RESTARTS=3 run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.note"
if grep -q "^team budget" "$td/run/alpha.note" && [ "$(wc -l < "$fake/claude-alpha/starts" | tr -d ' ')" -eq 1 ]; then ok "a relaunch that would pass TEAM_MAX_TOTAL_USD is refused, counting what the ended run spent"; else bad "cap: $(cat "$td/run/alpha.note" 2> /dev/null)"; fi

newenv
echo 2 > "$fake/unread-alpha"
TEAM_RESTART_PAUSE_SECS=2 TEAM_MAX_RESTARTS=1 run_team start "$work/t1w" > "$work/out" 2>&1
waitfor "$td/run/alpha.note"
t1="$(sed -n 1p "$fake/claude-alpha/starts")"; t2="$(sed -n 2p "$fake/claude-alpha/starts")"
if [ -n "$t2" ] && [ $((t2 - t1)) -ge 2 ]; then ok "TEAM_RESTART_PAUSE_SECS separates a start from its relaunch"; else bad "pause: $t1 $t2"; fi
newenv
TEAM_MAX_RESTARTS=x run_team resume alpha > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ]; then ok "resume with no such worker exits 2 even with a bad TEAM_MAX_RESTARTS"; else bad "bad max rc=$rc"; fi

# --- smoke runs one worker in a throwaway home, never the caller's KLATCH_HOME
newenv
TEAM_KLATCH="$bin/klatch" TEAM_CLAUDE="$bin/claude" TMPDIR="$work" KLATCH_HOME="$work/home" FAKE_DIR="$fake" "$team" smoke > "$work/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && grep -q "^home=$work/klatch-team-smoke" "$fake/claude-smoke/env" && ! grep -q "home=$work/home$" "$fake/claude-smoke/env" && grep -q "^worker issue smoke " "$fake/klatch.log"; then ok "smoke uses a throwaway KLATCH_HOME, not the caller's"; else bad "smoke rc=$rc: $(cat "$work/out")"; fi

# --- every worker run above left its watchdog behind a while: none may outlive the run (#1101)
/bin/sleep 1
leaked="$(pgrep -f "^/bin/sleep $sentinel\$" | wc -l | tr -d ' ')"
if [ "$leaked" -eq 0 ]; then ok "no watchdog sleep outlives a finished or stopped worker"; else bad "$leaked watchdog sleep(s) left running"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all team script tests passed"; else echo "$fails team script test(s) failed"; exit 1; fi
