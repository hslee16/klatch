#!/usr/bin/env bash
# Tests for proto/scripts/pr-sweep (#706). The sweep runs against a fake `gh`, a fake `git`, a fake pr-ledger, a fake
# gate and a fake bench, all in a temp dir. Every call is appended to one ordered log, so the tests can say what was
# called, in what order, and what was never called. Nothing here touches the network, a real repo or a real home.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sweep="$here/pr-sweep"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-sweep-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
cat > "$bin/gh" <<'SH'
#!/bin/sh
echo "gh $*" >> "$FAKE_DIR/calls.log"
path=""; jq=""; prev=""
for a in "$@"; do
  case "$a" in repos/*) path="$a" ;; esac
  [ "$prev" = "--jq" ] && jq="$a"
  prev="$a"
done
if [ "$1" = pr ] && [ "$2" = merge ]; then echo true > "$FAKE_DIR/merged"; exit 0; fi
# the rate limit endpoint is free and always answers (#756)
if [ "$2" = rate_limit ]; then
  case "$jq" in
    *remaining*) echo "${FAKE_REMAINING:-5000}" ;;
    *) echo 1999999999 ;;
  esac
  exit 0
fi
# failures on the check-runs endpoint (#754): error, ratelimit, hang, or flaky (the first call of each head fails)
case "$path" in
  */check-runs)
    case "${FAKE_GH_MODE:-}" in
      error) echo "gh: Get \"https://api.github.com/x\": read tcp: operation timed out" >&2; exit 1 ;;
      ratelimit) echo "gh: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1 ;;
      hang) exec /bin/sleep 30 ;;
      flaky)
        if [ ! -e "$FAKE_DIR/flaky-done" ]; then : > "$FAKE_DIR/flaky-done"; echo "gh: connection reset" >&2; exit 1; fi ;;
    esac ;;
esac
case "$*" in
  *"-X PATCH"*"/issues/"*) t="${path##*/}"; echo closed > "$FAKE_DIR/issue-$t"; exit 0 ;;
esac
case "$path" in
  */pulls/*)
    case "$jq" in
      .head.ref) echo "feat-${path##*/}" ;;
      .head.sha) cat "$FAKE_DIR/head" ;;
      .mergeable_state)
        if [ -s "$FAKE_DIR/mstate-seq" ]; then head -1 "$FAKE_DIR/mstate-seq"; tail -n +2 "$FAKE_DIR/mstate-seq" > "$FAKE_DIR/mstate-seq.t"; mv "$FAKE_DIR/mstate-seq.t" "$FAKE_DIR/mstate-seq"; else cat "$FAKE_DIR/mstate"; fi ;;
      .merged) cat "$FAKE_DIR/merged" ;;
    esac ;;
  */compare/*) cat "$FAKE_DIR/behind_by" 2> /dev/null || echo 0 ;;
  */check-runs) sha="${path%/check-runs}"; sha="${sha##*/}"; cat "$FAKE_DIR/checks-$sha" 2> /dev/null || true ;;
  */issues/*) t="${path##*/}"; cat "$FAKE_DIR/issue-$t" 2> /dev/null || echo open ;;
esac
exit 0
SH
cat > "$bin/git" <<'SH'
#!/bin/sh
echo "git $*" >> "$FAKE_DIR/calls.log"
branch() { cat "$FAKE_DIR/branch"; }
case "$1" in
  rev-parse) branch ;;
  status) [ -n "${FAKE_DIRTY:-}" ] && echo " M file" ;;
  checkout)
    [ -n "${FAKE_CHECKOUT_FAIL:-}" ] && exit 1
    [ "$2" = "-q" ] && shift
    [ "$2" = "-B" ] && echo "$3" > "$FAKE_DIR/branch" ;;
  merge)
    [ "$2" = "--abort" ] && exit 0
    [ "$(branch)" = main ] && echo "MERGE-ON-MAIN" >> "$FAKE_DIR/calls.log"
    case "${FAKE_MERGE:-clean}" in clean) exit 0 ;; *) exit 1 ;; esac ;;
  diff) echo "${FAKE_CONFLICT_FILES:-}" | tr ' ' '\n' | sed '/^$/d' ;;
  push) echo sha-new > "$FAKE_DIR/head"; echo clean > "$FAKE_DIR/mstate" ;;
esac
exit 0
SH
cat > "$bin/ledger" <<'SH'
#!/bin/sh
echo "ledger $*" >> "$FAKE_DIR/calls.log"
case "$*" in
  *--rerun-cancelled*|*--record-bench*|*--post*) exit 0 ;;
esac
cat "$FAKE_DIR/ledger.txt"
exit "${FAKE_LEDGER_RC:-0}"
SH
cat > "$bin/gate" <<'SH'
#!/bin/sh
echo "gate" >> "$FAKE_DIR/calls.log"
[ -n "${FAKE_GATE_FAIL:-}" ] && exit 1
exit 0
SH
cat > "$bin/bench" <<'SH'
#!/bin/sh
echo "bench" >> "$FAKE_DIR/calls.log"
echo "${FAKE_BENCH_OUT:-12 passed, 0 failed}"
SH
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "klatch $*" >> "$FAKE_DIR/calls.log"
SH
chmod +x "$bin"/*

n=0
fake=""; log=""
# a fresh fake world per scenario: PR head sha-old, state clean, checks on sha-old as given, issue 706 open
newenv() {
  n=$((n + 1)); fake="$work/f$n"; log="$fake/calls.log"; mkdir -p "$fake" "$work/repo" "$work/tmp$n"
  : > "$log"
  echo main > "$fake/branch"; echo sha-old > "$fake/head"; echo clean > "$fake/mstate"; echo false > "$fake/merged"
  echo "success success" > "$fake/checks-sha-old"
}
runsweep() {
  env PATH="$bin:$PATH" FAKE_DIR="$fake" PR_SWEEP_LEDGER="$bin/ledger" PR_SWEEP_GATE="$bin/gate" PR_SWEEP_BENCH="$bin/bench" PR_SWEEP_KLATCH="$bin/klatch" \
    PR_SWEEP_REPO="$work/repo" TMPDIR="$work/tmp$n" PR_SWEEP_POLL_SECS=0 PR_SWEEP_SETTLE_SECS=0 PR_SWEEP_WAIT_TRIES=3 \
    PR_SWEEP_GH_TIMEOUT="${PR_SWEEP_GH_TIMEOUT:-2}" PR_SWEEP_GH_RETRY_SLEEP=0 \
    "$sweep" "$@" > "$fake/out" 2>&1
  rc=$?
}
calls() { grep -c -- "$1" "$log"; }
line_of() { grep -n -m1 -- "$1" "$log" | cut -d: -f1; }
# --- nothing to do: one summary line, exit 0
newenv
printf '0 tracked, 0 need a step, 0 done\n' > "$fake/ledger.txt"
runsweep
if [ "$rc" -eq 0 ] && [ "$(wc -l < "$fake/out" | tr -d ' ')" -eq 1 ] && grep -q "nothing needs a step" "$fake/out"; then ok "nothing to do exits 0 with a one-line summary"; else bad "nothing to do rc=$rc: $(cat "$fake/out")"; fi

# --- cancelled: rerun, and nothing else
newenv
printf '#11   checks-cancelled     feat-11            #701      rerun the cancelled job\n' > "$fake/ledger.txt"
runsweep
if [ "$rc" -eq 0 ] && grep -q "^ledger --rerun-cancelled 11" "$log" && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^git ')" -eq 0 ]; then ok "checks-cancelled reruns the cancelled jobs and does nothing else"; else bad "cancelled rc=$rc: $(cat "$log")"; fi

# --- red: not touched, not rerun, posted
newenv
printf '#12   checks-red           feat-12            #702      read the failing job\n        build (failure) http://x: error\n' > "$fake/ledger.txt"
runsweep
if [ "$rc" -eq 0 ] && [ "$(calls 'rerun')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^git ')" -eq 0 ] \
  && grep -q "^ledger --post --only 12" "$log"; then ok "checks-red is not touched or rerun, and is posted to the lobby"; else bad "red rc=$rc: $(cat "$log")"; fi

# --- no ticket / stuck / abandoned: untouched and posted together in one message
newenv
cat > "$fake/ledger.txt" <<'L'
#21   ready                feat-21            no ticket merge, then run the bench
#22   checks-running        feat-22            #702      stuck? running for over 3 h: look at the job
#23   abandoned            feat-23            #703      closed without merging
#24   checks-running        feat-24            #704      wait
L
runsweep
if [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^ledger --post')" -eq 1 ] && grep -q "^ledger --post --only 21,22,23 \|^ledger --post --only 21,22,23$" "$log"; then ok "no-ticket, stuck and abandoned PRs are untouched and posted in one message (a plain wait is not)"; else bad "untouched list: $(cat "$log")"; fi

# --- ready: merge only after green, then bench, then record, then close the ticket
newenv
printf '#13   ready                feat-13            #706      merge, then run the bench\n' > "$fake/ledger.txt"
runsweep
m=$(line_of "^gh pr merge 13 --merge"); b=$(line_of "^bench"); r=$(line_of "^ledger --record-bench 13"); c=$(line_of "issues/706")
if [ "$rc" -eq 0 ] && [ -n "$m" ] && [ -n "$b" ] && [ -n "$r" ] && [ "$m" -lt "$b" ] && [ "$b" -lt "$r" ] && [ "$(calls '^git merge')" -eq 0 ]; then ok "ready merges, then benches, then records the bench (no integrate when not behind)"; else bad "ready rc=$rc m=$m b=$b r=$r: $(cat "$log")"; fi
if grep -q -- "--match-head-commit sha-old" "$log"; then ok "the merge is pinned to the head commit that was seen green"; else bad "merge not pinned: $(cat "$log")"; fi
if [ -n "$c" ] && grep -q "^gh api -X PATCH repos/hslee16/klatch/issues/706 -f state=closed" "$log"; then ok "the linked ticket is closed when the merge did not close it"; else bad "ticket not closed: $(cat "$log")"; fi

# --- a merge is announced to the lobby as "merged: #N" (DESIGN item 84), after the merge and not before it
mp=$(line_of '^klatch post lobby merged: #13')
if [ -n "$mp" ] && [ "$m" -lt "$mp" ]; then ok "after the merge the sweep posts \"merged: #13\" to the lobby"; else bad "no merged post after the merge: $(cat "$log")"; fi
if [ "$(calls '^klatch post lobby merged:')" -eq 1 ]; then ok "exactly one merged post per merge"; else bad "merged posts: $(cat "$log")"; fi

# --- ready but the ticket is already closed: no second close
newenv
echo closed > "$fake/issue-706"
printf '#13   ready                feat-13            #706      merge\n' > "$fake/ledger.txt"
runsweep
if [ "$(calls 'pr merge')" -eq 1 ] && [ "$(calls 'PATCH')" -eq 0 ]; then ok "an already closed ticket is left alone"; else bad "closed ticket: $(cat "$log")"; fi

# --- ready in the ledger, but the latest head has a pending, red or missing check: never merged
for state in "success pending" "success failure" ""; do
  newenv
  echo "$state" > "$fake/checks-sha-old"
  printf '#14   ready                feat-14            #706      merge\n' > "$fake/ledger.txt"
  runsweep
  if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^bench')" -eq 0 ]; then ok "never merges when the live checks on the head are \"${state:-none}\""; else bad "merged on \"$state\": $(cat "$log")"; fi
done

# --- ready but the PR turned conflicted: not merged and not integrated
newenv
echo dirty > "$fake/mstate"
printf '#15   ready                feat-15            #706      merge\n' > "$fake/ledger.txt"
runsweep
if [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^git merge')" -eq 0 ]; then ok "a ready PR that is conflicted is neither merged nor integrated"; else bad "ready+dirty: $(cat "$log")"; fi

# --- ready and GitHub says clean, but main has commits the head lacks (#760): integrate first, do not merge the stale head
newenv
printf '#13   ready                feat-13            #706      merge, then run the bench\n' > "$fake/ledger.txt"
echo 2 > "$fake/behind_by"
runsweep
if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^git merge')" -ge 1 ]; then ok "a ready PR that is behind main is integrated, never merged on its stale head"; else bad "stale head rc=$rc: $(cat "$log")"; fi

# --- GitHub says "unknown" right after a merge: ask again, then merge (#793)
newenv
printf '#13   ready                feat-13            #706      merge, then run the bench\n' > "$fake/ledger.txt"
printf 'unknown\nunknown\nclean\n' > "$fake/mstate-seq"
runsweep
if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 1 ]; then ok "an unknown mergeable state is asked again and the PR then merges"; else bad "unknown state rc=$rc: $(cat "$log")"; fi
newenv
printf '#13   ready                feat-13            #706      merge, then run the bench\n' > "$fake/ledger.txt"
for _ in $(seq 1 40); do echo unknown; done > "$fake/mstate-seq"
runsweep
if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && grep -q 'mergeable state is "unknown"' "$fake/out"; then ok "a state that stays unknown is not merged"; else bad "stuck unknown rc=$rc: $(cat "$log")"; fi

# --- ready but behind: integrate on a throwaway branch, push to the PR branch, wait for green on the NEW head, merge
newenv
echo behind > "$fake/mstate"
echo "success success" > "$fake/checks-sha-new"
printf '#16   ready                feat-16            #706      merge\n' > "$fake/ledger.txt"
runsweep
co=$(line_of "^git checkout -q -B alee/integ-16 origin/main"); mg=$(line_of "^git merge"); gt=$(line_of "^gate"); ps=$(line_of "^git push"); mp=$(line_of "^gh pr merge 16")
if [ -n "$co" ] && [ -n "$mg" ] && [ -n "$gt" ] && [ -n "$ps" ] && [ -n "$mp" ] && [ "$co" -lt "$mg" ] && [ "$mg" -lt "$gt" ] && [ "$gt" -lt "$ps" ] && [ "$ps" -lt "$mp" ]; then ok "ready+behind integrates on alee/integ-16, gates, pushes, then merges"; else bad "behind-ready order co=$co mg=$mg gt=$gt ps=$ps mp=$mp: $(cat "$log")"; fi
if grep -q -- "--match-head-commit sha-new" "$log" && grep -q "^git push -q origin alee/integ-16:feat-16$" "$log"; then ok "the push goes to the PR's own branch and the merge is pinned to the new head"; else bad "push/pin: $(cat "$log")"; fi

# --- ready but behind, and the new head has no checks yet: integrated, not merged
newenv
echo behind > "$fake/mstate"
printf '#16   ready                feat-16            #706      merge\n' > "$fake/ledger.txt"
runsweep
if [ "$(calls '^git push')" -eq 1 ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "after integrating, no merge until the new head has green checks"; else bad "new head unchecked: $(cat "$log")"; fi

# --- behind stage: integrate path only
newenv
echo behind > "$fake/mstate"
printf '#17   behind               feat-17            #706      integrate main\n' > "$fake/ledger.txt"
runsweep
if [ "$rc" -eq 0 ] && [ "$(calls '^git merge')" -eq 1 ] && [ "$(calls '^gate')" -eq 1 ] && [ "$(calls '^git push -q origin alee/integ-17:feat-17$')" -eq 1 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^bench')" -eq 0 ]; then ok "behind integrates, runs the gate and pushes to the PR branch; it does not merge"; else bad "behind rc=$rc: $(cat "$log")"; fi

# --- the fetch must not prune: with fetch.prune=true a pruning fetch deletes the PR ref it just made (#764)
if grep -q "^git fetch -q --no-prune origin main +pull/17/head:refs/pr-sweep/17" "$log"; then ok "integrate fetches the PR with --no-prune"; else bad "fetch may prune: $(grep '^git fetch' "$log" | head -2)"; fi

# --- a gate failure stops before the push
newenv
printf '#17   behind               feat-17            #706      integrate main\n' > "$fake/ledger.txt"
FAKE_GATE_FAIL=1 runsweep
if [ "$(calls '^git push')" -eq 0 ] && grep -q "^ledger --post --only 17" "$log"; then ok "a failing gate stops before any push and the PR is posted"; else bad "gate fail: $(cat "$log")"; fi

# --- a code conflict stops, aborts the merge, pushes nothing, and is reported
newenv
printf '#18   conflict             feat-18            #706      integrate main\n' > "$fake/ledger.txt"
FAKE_MERGE=conflict FAKE_CONFLICT_FILES="proto/klatch/src/lib.rs DESIGN.md" runsweep
if [ "$(calls '^git merge --abort')" -eq 1 ] && [ "$(calls '^git push')" -eq 0 ] && [ "$(calls '^gate')" -eq 0 ] && grep -q "^ledger --post --only 18" "$log" && grep -q "CODE CONFLICT" "$fake/out"; then ok "a code conflict aborts the merge, pushes nothing and is reported to the lobby"; else bad "code conflict: $(cat "$log") $(cat "$fake/out")"; fi

# --- never on main: git merge only ever runs on the throwaway branch, even if the checkout fails
newenv
printf '#17   behind               feat-17            #706      integrate main\n' > "$fake/ledger.txt"
FAKE_CHECKOUT_FAIL=1 runsweep
if [ "$(calls '^git merge')" -eq 0 ] && [ "$(calls 'MERGE-ON-MAIN')" -eq 0 ] && [ "$(calls '^git push')" -eq 0 ]; then ok "if the throwaway branch cannot be made, nothing is merged or pushed"; else bad "checkout fail: $(cat "$log")"; fi
newenv
printf '#17   behind               feat-17            #706      integrate main\n' > "$fake/ledger.txt"
FAKE_DIRTY=1 runsweep
if [ "$(calls '^git checkout')" -eq 0 ] && [ "$(calls '^git merge')" -eq 0 ]; then ok "a dirty working tree is never switched or merged"; else bad "dirty tree: $(cat "$log")"; fi

# --- dry run: reads only
newenv
echo behind > "$fake/mstate"
cat > "$fake/ledger.txt" <<'L'
#11   checks-cancelled     feat-11            #701      rerun
#13   ready                feat-13            #706      merge
#17   behind               feat-17            #706      integrate
#12   checks-red           feat-12            #702      fix
L
runsweep --dry-run
if [ "$rc" -eq 0 ] && grep -q "would" "$fake/out" \
  && [ "$(calls 'rerun')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls '^git ')" -eq 0 ] && [ "$(calls 'PATCH')" -eq 0 ] \
  && [ "$(calls '^bench')" -eq 0 ] && [ "$(calls '^gate')" -eq 0 ] && [ "$(calls 'record-bench')" -eq 0 ] && [ "$(calls '^ledger --post')" -eq 0 ] && [ "$(calls '^klatch post')" -eq 0 ]; then ok "--dry-run prints what it would do and changes nothing"; else bad "dry-run rc=$rc: $(cat "$log") $(cat "$fake/out")"; fi

# --- lock: a live holder blocks a second sweep; a dead holder does not; the lock is released
newenv
printf '#13   ready                feat-13            #706      merge\n' > "$fake/ledger.txt"
mkdir -p "$work/tmp$n/pr-sweep.lock"
/bin/sleep 60 &
holder=$!
echo "$holder" > "$work/tmp$n/pr-sweep.lock/pid"
runsweep
if [ "$rc" -eq 0 ] && grep -q "another sweep is running" "$fake/out" && [ ! -s "$log" ]; then ok "a live lock holder stops a second sweep before it does anything"; else bad "lock rc=$rc: $(cat "$fake/out") $(cat "$log")"; fi
kill -9 "$holder" 2> /dev/null
wait "$holder" 2> /dev/null
runsweep
if [ "$(calls 'pr merge')" -eq 1 ] && [ ! -e "$work/tmp$n/pr-sweep.lock" ]; then ok "a dead holder's lock is taken over, and the lock is released afterwards"; else bad "stale lock: $(cat "$fake/out") $(cat "$log")"; fi

# --- a failed gh call is a failed pass, never a clean one (#754)
ready13() { printf '#13   ready                feat-13            #706      merge\n' > "$fake/ledger.txt"; }
newenv; ready13
FAKE_GH_MODE=error runsweep
if [ "$rc" -ne 0 ] && grep -q "call(s) failed, pass incomplete" "$fake/out" && ! grep -q "nothing needs a step" "$fake/out" && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a failing gh call fails the pass (non-zero, \"pass incomplete\", no merge), never \"nothing needs a step\""; else bad "failed call rc=$rc: $(cat "$fake/out")"; fi
if [ "$(calls 'check-runs')" -eq 3 ]; then ok "a failing gh call is tried three times (one try and two retries)"; else bad "retries: $(calls 'check-runs') calls"; fi
newenv; ready13
FAKE_GH_MODE=flaky runsweep
if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 1 ] && ! grep -q "pass incomplete" "$fake/out"; then ok "a call that fails once and then works is retried and the pass is clean"; else bad "flaky rc=$rc: $(cat "$fake/out") $(cat "$log")"; fi
newenv; ready13
start=$SECONDS
PR_SWEEP_GH_TIMEOUT=1 FAKE_GH_MODE=hang runsweep
if [ "$rc" -ne 0 ] && grep -q "pass incomplete" "$fake/out" && [ "$((SECONDS - start))" -lt 20 ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a hung gh call is cut off by the timeout, retried, and fails the pass"; else bad "hang rc=$rc after $((SECONDS - start))s: $(cat "$fake/out")"; fi
newenv; ready13
FAKE_GH_MODE=ratelimit runsweep
if [ "$rc" -ne 0 ] && grep -qi "rate limit" "$fake/out" && grep -q "resets at 2033-" "$fake/out" && grep -q "pass incomplete" "$fake/out" && ! grep -q "checks are not all green" "$fake/out" && [ "$(calls 'check-runs')" -eq 1 ]; then ok "a rate limit is its own failure: not \"not green\", no retry, the reset time is printed"; else bad "rate limit rc=$rc: $(cat "$fake/out") checks=$(calls 'check-runs')"; fi
newenv
printf '0 tracked, 0 need a step, 0 done\n' > "$fake/ledger.txt"
FAKE_LEDGER_RC=1 runsweep
if [ "$rc" -ne 0 ] && grep -q "pass incomplete" "$fake/out" && ! grep -q "nothing needs a step" "$fake/out"; then ok "a ledger that failed makes the pass incomplete even when it listed nothing"; else bad "ledger failed rc=$rc: $(cat "$fake/out")"; fi

# --- a lock held for over 30 minutes by a live process is reported as stuck, and exits non-zero (#754)
newenv; ready13
mkdir -p "$work/tmp$n/pr-sweep.lock"
/bin/sleep 60 &
holder=$!
echo "$holder" > "$work/tmp$n/pr-sweep.lock/pid"
touch -t 202001010000 "$work/tmp$n/pr-sweep.lock"
runsweep
if [ "$rc" -ne 0 ] && grep -q "held the lock for" "$fake/out" && grep -q "kill -9 $holder" "$fake/out" && [ ! -s "$log" ]; then ok "a lock older than 30 minutes with a live pid is reported (with the kill command) and exits non-zero"; else bad "old lock rc=$rc: $(cat "$fake/out")"; fi
kill -9 "$holder" 2> /dev/null
wait "$holder" 2> /dev/null

# --- --budget-floor N: skip with a message when too few API calls are left, read the limit for free (#756)
newenv; ready13
FAKE_REMAINING=100 runsweep --budget-floor 1500
if [ "$rc" -eq 0 ] && grep -q "skipped" "$fake/out" && grep -q "100" "$fake/out" && grep -q "1500" "$fake/out" && [ "$(calls '^ledger')" -eq 0 ] && [ "$(calls 'repos/')" -eq 0 ]; then ok "--budget-floor skips the pass with a message when the budget is below it"; else bad "budget skip rc=$rc: $(cat "$fake/out") $(cat "$log")"; fi
newenv; ready13
FAKE_REMAINING=3000 runsweep --budget-floor 1500
if [ "$rc" -eq 0 ] && [ "$(calls 'pr merge')" -eq 1 ] && ! grep -q "skipped" "$fake/out"; then ok "--budget-floor lets the pass run when enough calls are left"; else bad "budget ok rc=$rc: $(cat "$fake/out")"; fi

# --- across every run above: no force push, no push to main, no environment dump
all="$work/all.log"
cat "$work"/f*/calls.log > "$all"
if ! grep -qE "^git push.*( -f|--force|origin main|:main)" "$all" && ! grep -q "MERGE-ON-MAIN" "$all"; then ok "no run ever force-pushed, pushed main or merged on main"; else bad "forbidden git call: $(grep -E 'force|origin main|:main|MERGE-ON-MAIN' "$all")"; fi
if ! grep -qE "HOME=|PATH=|TMPDIR=" "$work"/f*/out; then ok "the sweep output never prints the environment"; else bad "environment in output"; fi

# --- bench failures are not recorded, and the PR is reported
newenv
printf '#13   ready                feat-13            #706      merge\n' > "$fake/ledger.txt"
FAKE_BENCH_OUT="11 passed, 1 failed" runsweep
if [ "$(calls 'pr merge')" -eq 1 ] && [ "$(calls 'record-bench')" -eq 0 ] && grep -q "^ledger --post --only 13" "$log"; then ok "a failed bench is not recorded and the PR is posted"; else bad "bench fail: $(cat "$log")"; fi

# --- a merged PR not yet benched is benched and recorded; a merged one with an open ticket gets it closed
newenv
cat > "$fake/ledger.txt" <<'L'
#30   merged-not-benched   feat-30            #706      run the bench
#31   merged-open-ticket   feat-31            #710      close the linked ticket
L
runsweep
if [ "$(calls '^bench')" -eq 1 ] && [ "$(calls 'record-bench 30')" -eq 1 ] && grep -q "issues/710 -f state=closed" "$log" && [ "$(calls 'pr merge')" -eq 0 ]; then ok "merged-not-benched is benched and recorded; merged-open-ticket closes the ticket"; else bad "post-merge steps: $(cat "$log")"; fi

# --- the docs-conflict resolver keeps both sides and sorts DESIGN items by number
d="$work/res"; mkdir -p "$d"
cat > "$d/DESIGN.md" <<'D'
head
29. **a** old
<<<<<<< HEAD
31. **c** main
30. **b** main
=======
32. **d** branch
>>>>>>> pr
D
( cd "$d" && "$sweep" --resolve-docs DESIGN.md > /dev/null 2>&1 )
if ! grep -q '^[<>=]\{7\}' "$d/DESIGN.md" && [ "$(grep '^3[0-9]\.' "$d/DESIGN.md" | cut -d. -f1 | tr '\n' ' ')" = "30 31 32 " ]; then ok "the docs resolver keeps both sides and sorts DESIGN items by number"; else bad "resolver: $(cat "$d/DESIGN.md")"; fi

# --- pr-ledger with a fake gh and klatch: --only, the number of API calls, failures, the budget floor (#754, #756)
lbin="$work/lbin"; mkdir -p "$lbin"
now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cat > "$lbin/gh" <<'SH'
#!/bin/sh
echo "$*" >> "$FAKE_DIR/gh.log"
case "$2" in
  rate_limit) echo "{\"resources\":{\"core\":{\"remaining\":${FAKE_REMAINING:-5000},\"reset\":1999999999}}}"; exit 0 ;;
esac
case "$2" in
  */check-runs)
    case "${FAKE_GH_MODE:-}" in
      error) echo "gh: Get x: read tcp: operation timed out" >&2; exit 1 ;;
      ratelimit) echo "gh: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1 ;;
      hang) exec /bin/sleep 30 ;;
      flaky) if [ ! -e "$FAKE_DIR/flaky-done" ]; then : > "$FAKE_DIR/flaky-done"; echo "gh: connection reset" >&2; exit 1; fi ;;
    esac ;;
esac
pr() { # number sha draft body merged_at
  printf '{"number":%s,"head":{"ref":"b%s","sha":"%s"},"state":"open","draft":%s,"title":"t (#9)","body":"%s","updated_at":"2026-11-17T00:00:00Z","merged_at":%s}' "$1" "$1" "$2" "$3" "$4" "$5"
}
case "$2" in
  *pulls?state=open*) echo "[$(pr 5 s5 false '' null),$(pr 6 s6 false '' null),$(pr 7 s7 false 'Closes #9' null),$(pr 8 s8 true '' null)]" ;;
  *pulls?state=closed*) echo "[$(pr 20 s20 false 'Closes #9' "\"$FAKE_NOW\"")]" ;;
  */pulls/5 | */pulls/6) echo '{"mergeable_state":"dirty"}' ;;
  */check-runs)
    case "$2" in
      *s7/check-runs) echo '{"check_runs":[{"id":1,"name":"build","conclusion":"failure","html_url":"u","output":{"summary":"error: boom"}}]}' ;;
      *) echo '{"check_runs":[{"id":1,"name":"build","conclusion":"success","html_url":"u"}]}' ;;
    esac ;;
  */issues/9) echo '{"state":"open"}' ;;
esac
SH
cat > "$lbin/klatch" <<'SH'
#!/bin/sh
echo "$*" >> "$FAKE_DIR/klatch.log"
SH
chmod +x "$lbin/gh" "$lbin/klatch"
runledger() { # args: the real pr-ledger against the fake gh; output in $fake/out, status in rc
  env PATH="$lbin:$PATH" KLATCH="$lbin/klatch" FAKE_DIR="$fake" FAKE_NOW="$now" PR_LEDGER_GH_TIMEOUT="${PR_LEDGER_GH_TIMEOUT:-2}" PR_LEDGER_GH_RETRY_SLEEP=0 \
    python3 -I "$here/pr-ledger" --state-file "$fake/state" "$@" > "$fake/out" 2>&1
  rc=$?
}
ghcalls() { grep -c -- "$1" "$fake/gh.log" 2> /dev/null || true; }

newenv
runledger --post --only 5
if grep -q "#5 " "$fake/klatch.log" 2> /dev/null && ! grep -q "#6 " "$fake/klatch.log"; then ok "pr-ledger --post --only lists just the named PRs"; else bad "--only: $(cat "$fake/out") $(cat "$fake/klatch.log" 2>&1)"; fi

# a plain pass: each stage is right, and the calls are few: both lists once, one check-runs call per open PR that is not a
# draft, the pull itself only when its checks are all green (for mergeable_state), the ticket only for a merged PR
newenv
runledger
if [ "$rc" -eq 0 ] && grep -q "^#5 .*conflict" "$fake/out" && grep -q "^#7 .*checks-red" "$fake/out" && grep -q "^#8 .*draft" "$fake/out" && grep -q "^#20 .*merged-open-ticket" "$fake/out"; then ok "pr-ledger classifies open, red, draft and merged PRs from the lists and the check runs"; else bad "stages rc=$rc: $(cat "$fake/out")"; fi
if [ "$(ghcalls 'pulls?state=open')" -eq 1 ] && [ "$(ghcalls 'pulls?state=closed')" -eq 1 ] && [ "$(ghcalls 'check-runs')" -eq 3 ] \
  && [ "$(ghcalls '/pulls/[0-9]')" -eq 2 ] && [ "$(ghcalls '/issues/')" -eq 1 ]; then ok "pr-ledger makes few calls: lists once, check-runs once per open non-draft PR, the pull only for PRs with green checks, the ticket only for a merged PR"; else bad "call counts: $(cat "$fake/gh.log")"; fi
if [ "$(ghcalls 's20/check-runs')" -eq 0 ] && [ "$(ghcalls 's8/check-runs')" -eq 0 ] && [ "$(ghcalls 'pulls/7')" -eq 0 ] && [ "$(ghcalls 'pulls/8')" -eq 0 ] && [ "$(ghcalls 'pulls/20')" -eq 0 ]; then ok "a merged PR, a draft and a red PR need no check-runs, pull or ticket call they do not use"; else bad "needless calls: $(cat "$fake/gh.log")"; fi

# a failing call: retried, counted, the pass is incomplete and exits non-zero
newenv
FAKE_GH_MODE=error runledger
if [ "$rc" -ne 0 ] && grep -q "call(s) failed, pass incomplete" "$fake/out" && [ "$(ghcalls 'check-runs')" -eq 9 ]; then ok "pr-ledger retries a failing call twice, then says \"N call(s) failed, pass incomplete\" and exits non-zero"; else bad "failed ledger rc=$rc: $(cat "$fake/out") checks=$(ghcalls 'check-runs')"; fi
newenv
FAKE_GH_MODE=error runledger --post
if [ "$rc" -ne 0 ] && ! grep -q "nothing needs a step" "$fake/out" && [ ! -e "$fake/klatch.log" ]; then ok "a failed pass never says \"nothing needs a step\" and posts nothing"; else bad "failed --post rc=$rc: $(cat "$fake/out")"; fi
newenv
FAKE_GH_MODE=flaky runledger
if [ "$rc" -eq 0 ] && ! grep -q "pass incomplete" "$fake/out"; then ok "pr-ledger: a call that fails once and then works leaves the pass clean"; else bad "flaky ledger rc=$rc: $(cat "$fake/out")"; fi
newenv
start=$SECONDS
PR_LEDGER_GH_TIMEOUT=1 FAKE_GH_MODE=hang runledger
if [ "$rc" -ne 0 ] && grep -q "pass incomplete" "$fake/out" && [ "$((SECONDS - start))" -lt 40 ]; then ok "pr-ledger cuts off a hung gh call with a timeout and fails the pass"; else bad "hung ledger rc=$rc after $((SECONDS - start))s: $(cat "$fake/out")"; fi
newenv
FAKE_GH_MODE=ratelimit runledger
if [ "$rc" -ne 0 ] && grep -qi "rate limit" "$fake/out" && grep -q "resets at 2033-" "$fake/out" && grep -q "pass incomplete" "$fake/out" && [ "$(ghcalls 'check-runs')" -eq 1 ]; then ok "pr-ledger: a rate limit stops the pass at once, prints the reset time and exits non-zero"; else bad "ledger rate limit rc=$rc: $(cat "$fake/out") checks=$(ghcalls 'check-runs')"; fi

# --budget-floor N: read the limit (free) and skip with a message when fewer than N calls are left
newenv
FAKE_REMAINING=100 runledger --budget-floor 1500
if [ "$rc" -eq 0 ] && grep -q "skipped" "$fake/out" && grep -q "100" "$fake/out" && [ "$(ghcalls 'repos/')" -eq 0 ] && [ "$(ghcalls 'pulls')" -eq 0 ]; then ok "pr-ledger --budget-floor skips with a message and makes no listing call when the budget is low"; else bad "ledger budget skip rc=$rc: $(cat "$fake/out") $(cat "$fake/gh.log")"; fi
newenv
FAKE_REMAINING=3000 runledger --budget-floor 1500
if [ "$rc" -eq 0 ] && ! grep -q "skipped" "$fake/out" && grep -q "^#5 " "$fake/out"; then ok "pr-ledger --budget-floor lists as usual when enough calls are left"; else bad "ledger budget ok rc=$rc: $(cat "$fake/out")"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all sweep script tests passed"; else echo "$fails sweep script test(s) failed"; exit 1; fi
