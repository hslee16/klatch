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
if [ "$1" = pr ] && [ "$2" = merge ]; then echo true > "$FAKE_DIR/merged-$3"; exit 0; fi
if [ "$1" = pr ] && [ "$2" = create ]; then
  [ -n "${FAKE_PR_CREATE_FAIL:-}" ] && { echo "gh: could not create" >&2; exit 1; }
  prev=""; for a in "$@"; do [ "$prev" = "--body-file" ] && cp "$a" "$FAKE_DIR/batch-body"; prev="$a"; done
  echo "https://github.com/hslee16/klatch/pull/${FAKE_BATCH_PR:-900}"; exit 0
fi
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
      .head.sha) cat "$FAKE_DIR/head-${path##*/}" 2> /dev/null || cat "$FAKE_DIR/head" ;;
      .body) cat "$FAKE_DIR/body-${path##*/}" 2> /dev/null || true ;;
      .mergeable_state)
        if [ -s "$FAKE_DIR/mstate-${path##*/}" ]; then cat "$FAKE_DIR/mstate-${path##*/}"
        elif [ -s "$FAKE_DIR/mstate-seq" ]; then head -1 "$FAKE_DIR/mstate-seq"; tail -n +2 "$FAKE_DIR/mstate-seq" > "$FAKE_DIR/mstate-seq.t"; mv "$FAKE_DIR/mstate-seq.t" "$FAKE_DIR/mstate-seq"; else cat "$FAKE_DIR/mstate"; fi ;;
      .merged) cat "$FAKE_DIR/merged-${path##*/}" 2> /dev/null || cat "$FAKE_DIR/merged" ;;
    esac ;;
  */compare/*) cat "$FAKE_DIR/behind_by-${path##*...}" 2> /dev/null || cat "$FAKE_DIR/behind_by" 2> /dev/null || echo 0 ;;
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
  rev-parse)
    case "$*" in
      *refs/pr-sweep/*) r="${2##*/}"; cat "$FAKE_DIR/rp-$r" 2> /dev/null || cat "$FAKE_DIR/head-$r" 2> /dev/null || cat "$FAKE_DIR/head" ;;
      *) branch ;;
    esac ;;
  merge-base) [ -n "${FAKE_NOT_ANCESTOR:-}" ] && exit 1; exit 0 ;;
  merge-tree)
    # merge-tree --write-tree --name-only A B: A origin/main -> "main", anything else (HEAD) -> "batch"
    for a in "$@"; do last="$a"; done
    pr="${last##*/}"; case "$4" in origin/main) k=main ;; *) k=batch ;; esac
    f="$(cat "$FAKE_DIR/mt-$k-$pr" 2> /dev/null || true)"
    echo 0123abcd
    if [ -n "$f" ]; then echo "$f" | tr ' ' '\n'; echo; echo "CONFLICT (content)"; exit 1; fi
    exit 0 ;;
  status) [ -n "${FAKE_DIRTY:-}" ] && echo " M file" ;;
  checkout)
    [ -n "${FAKE_CHECKOUT_FAIL:-}" ] && exit 1
    [ "$2" = "-q" ] && shift
    [ "$2" = "-B" ] && echo "$3" > "$FAKE_DIR/branch" ;;
  merge)
    [ "$2" = "--abort" ] && exit 0
    [ "$(branch)" = main ] && echo "MERGE-ON-MAIN" >> "$FAKE_DIR/calls.log"
    case "$*" in *"${FAKE_MERGE_FAIL_REF:-@none@}"*) exit 1 ;; esac
    case "${FAKE_MERGE:-clean}" in clean) exit 0 ;; *) exit 1 ;; esac ;;
  diff) echo "${FAKE_CONFLICT_FILES:-}" | tr ' ' '\n' | sed '/^$/d' ;;
  push)
    case "$*" in
      *alee/batch-*) # the first push opens the batch PR; a later push (main merged in) moves its head to head-900-next
        if [ -e "$FAKE_DIR/batch-pushed" ]; then [ -s "$FAKE_DIR/head-900-next" ] && mv "$FAKE_DIR/head-900-next" "$FAKE_DIR/head-900"; fi
        : > "$FAKE_DIR/batch-pushed" ;;
      *) echo sha-new > "$FAKE_DIR/head"; echo clean > "$FAKE_DIR/mstate" ;;
    esac ;;
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

# --- RESEARCH-NOTES.md is an append-only document like DESIGN.md: a conflict in it alone is resolved by keeping both sides, not reported as a code conflict
newenv
printf '#18   behind               feat-18            #706      integrate main\n' > "$fake/ledger.txt"
FAKE_MERGE=conflict FAKE_CONFLICT_FILES="RESEARCH-NOTES.md" runsweep
if ! grep -q "CODE CONFLICT in: RESEARCH-NOTES.md" "$fake/out"; then ok "a conflict only in RESEARCH-NOTES.md is not a code conflict"; else bad "research notes conflict: $(cat "$fake/out")"; fi

# --- where the time goes (#855): gate, CI wait and bench each leave a timing line; --timing summarises them
newenv
echo behind > "$fake/mstate"
echo "success success" > "$fake/checks-sha-new"
printf '#16   ready                feat-16            #706      merge\n' > "$fake/ledger.txt"
runsweep
tl="$work/tmp$n/pr-sweep-timing.log"
if [ -f "$tl" ] && grep -q ' gate alee/integ-16 [0-9]*s rc=0' "$tl" && grep -q ' wait-green #16 [0-9]*s rc=0' "$tl" && grep -q ' bench main [0-9]*s rc=0' "$tl"; then ok "gate, CI wait and bench each log how long they took"; else bad "timing log: $(cat "$tl" 2>/dev/null)"; fi
printf '2026-11-18T10:00:00Z gate alee/integ-1 120s rc=0\n2026-11-18T10:05:00Z gate alee/integ-2 300s rc=0\n2026-11-18T10:09:00Z bench main 90s rc=0\n' > "$tl"
out=$(env TMPDIR="$work/tmp$n" "$sweep" --timing 2>&1)
if printf '%s\n' "$out" | grep -qE '^gate +2 +420 +210 +300$' && printf '%s\n' "$out" | grep -qE '^bench +1 +90 +90 +90$'; then ok "--timing sums runs, total, average and slowest per step"; else bad "timing summary: $out"; fi
out=$(env TMPDIR="$work/empty-timing" "$sweep" --timing 2>&1)
if printf '%s\n' "$out" | grep -q "no timing log yet"; then ok "--timing says so when there is no log"; else bad "empty timing: $out"; fi

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

# ===== batch mode (#852): several PRs per integration, one gate, one batch PR =====
# batchenv N...: those PRs are ready (green on their own head sha-N, ticket 700+N, body "Closes #700+N"), GitHub marks them
# merged once the batch lands; the batch PR is #900 with head sha-b1, green and clean
batchenv() {
  newenv
  : > "$fake/ledger.txt"
  local p
  for p in "$@"; do
    echo "sha-$p" > "$fake/head-$p"; echo "success success" > "$fake/checks-sha-$p"; echo true > "$fake/merged-$p"
    echo "Closes #$((700 + p))" > "$fake/body-$p"
    printf '#%-4s %-20s %-18s %-9s %s\n' "$p" ready "feat-$p" "#$((700 + p))" "merge, then run the bench" >> "$fake/ledger.txt"
  done
  echo sha-b1 > "$fake/head-900"; echo "success success" > "$fake/checks-sha-b1"; echo clean > "$fake/mstate-900"
}
gitcalls() { grep -c "^git $1" "$log" || true; }

# --- a clean batch of three: ONE gate, ONE batch PR, ONE bench, every PR recorded, announced and its ticket closed
batchenv 11 13 14
echo "Refs #713" > "$fake/body-13"
sed -i.bak 's/ #713 / refs#713 /' "$fake/ledger.txt"
runsweep --batch
if [ "$rc" -eq 0 ] && [ "$(calls '^gh pr create')" -eq 1 ] && grep -q -- '^gh pr create .*--title Batch: #11 #13 #14' "$log" && [ "$(calls '^gate')" -eq 1 ] \
  && [ "$(calls '^bench')" -eq 1 ] && [ "$(calls '^gh pr merge ')" -eq 1 ]; then ok "a clean batch of 3 lands via ONE batch PR, ONE gate and ONE bench"; else bad "clean batch rc=$rc: $(cat "$log") $(cat "$fake/out")"; fi
if grep -q "^gh pr merge 900 --merge --match-head-commit sha-b1$" "$log" && [ "$(calls '^gh pr merge 11')" -eq 0 ]; then ok "the batch PR is merged pinned to its green head; no included PR is merged on its own"; else bad "batch merge: $(cat "$log")"; fi
if [ "$(gitcalls 'checkout -q -B alee/batch-')" -ge 1 ] && [ "$(calls '^git merge .*refs/pr-sweep/11')" -eq 1 ] && [ "$(calls '^git merge .*refs/pr-sweep/13')" -eq 1 ] && [ "$(calls '^git merge .*refs/pr-sweep/14')" -eq 1 ] \
  && [ "$(calls '^git push -q origin alee/batch-[0-9]*$')" -eq 1 ] && grep -q -- "--no-prune origin main" "$log"; then ok "the batch branch is built from origin/main with a real git merge of each head, then pushed once"; else bad "batch branch: $(cat "$log")"; fi
cr=$(line_of '^gh pr create'); gt=$(line_of '^gate'); ps=$(line_of '^git push'); mg=$(line_of '^gh pr merge 900'); bn=$(line_of '^bench'); rc11=$(line_of '^ledger --record-bench 11')
if [ -n "$cr" ] && [ "$gt" -lt "$ps" ] && [ "$ps" -lt "$cr" ] && [ "$cr" -lt "$mg" ] && [ "$mg" -lt "$bn" ] && [ "$bn" -lt "$rc11" ]; then ok "order: gate, push, open the PR, merge, bench, record"; else bad "batch order gt=$gt ps=$ps cr=$cr mg=$mg bn=$bn rc=$rc11"; fi
if [ "$(calls '^ledger --record-bench')" -eq 3 ] && [ "$(calls '^klatch post lobby merged: #')" -eq 3 ] && grep -q '^klatch post lobby merged: #14' "$log"; then ok "every included PR is recorded as benched and announced \"merged: #N\""; else bad "record/announce: $(cat "$log")"; fi
if grep -q "issues/711 -f state=closed" "$log" && grep -q "issues/714 -f state=closed" "$log" && ! grep -q "issues/713 -f state=closed" "$log"; then ok "the tickets the PRs close are closed; a Refs ticket is not"; else bad "tickets: $(cat "$log")"; fi
if grep -q '^Closes #711$' "$fake/batch-body" && grep -q '^Refs #713$' "$fake/batch-body" && grep -q '^Closes #714$' "$fake/batch-body" && tail -1 "$fake/batch-body" | grep -q 'Generated with \[Claude Code\](https://claude.com/claude-code)'; then ok "the batch PR body copies each PR's Closes/Refs lines and ends with the Generated-with line"; else bad "batch body: $(cat "$fake/batch-body")"; fi
if grep -q '^klatch post lobby batch: #900' "$log"; then ok "a lobby summary names the batch PR"; else bad "no summary: $(cat "$log")"; fi

# --- a PR that would conflict in code is dropped BEFORE any merge, reported, posted; never merged
batchenv 11 13 14
echo "proto/klatch/src/lib.rs" > "$fake/mt-main-13"
runsweep --batch
if [ "$(calls '^git merge .*refs/pr-sweep/13')" -eq 0 ] && grep -q '^gh pr create .*--title Batch: #11 #14' "$log" && grep -q "^ledger --post --only 13" "$log" && grep -q "#13.*dropped.*lib.rs" "$fake/out" && [ "$(calls '^gh pr merge 13')" -eq 0 ]; then ok "a code conflict with main drops the PR before it touches the branch; it is reported and posted, never merged"; else bad "code drop: $(cat "$log") $(cat "$fake/out")"; fi

# --- docs-only conflicts are fine: the resolver keeps both sides and the PR stays in the batch
batchenv 11 13 14
echo "DESIGN.md CHANGELOG.md" > "$fake/mt-main-13"
FAKE_MERGE_FAIL_REF=refs/pr-sweep/13 FAKE_CONFLICT_FILES="DESIGN.md" runsweep --batch
if grep -q '^gh pr create .*--title Batch: #11 #13 #14' "$log" && [ "$(calls '^git commit')" -ge 1 ]; then ok "a docs-only conflict does not drop a PR; the keep-both resolver commits the merge"; else bad "docs conflict: $(cat "$log") $(cat "$fake/out")"; fi

# --- two PRs that conflict with each other: the lower number stays, the other is dropped and reported
batchenv 11 13 14
echo "proto/klatch/src/store.rs" > "$fake/mt-batch-13"
runsweep --batch
if [ "$(calls '^git merge .*refs/pr-sweep/13')" -eq 0 ] && [ "$(calls '^git merge .*refs/pr-sweep/11')" -eq 1 ] && grep -q '^gh pr create .*--title Batch: #11 #14' "$log" && grep -q "#13.*dropped.*batch" "$fake/out" && grep -q "^ledger --post --only 13" "$log"; then ok "two mutually conflicting PRs keep the lower number; the higher one is dropped and reported"; else bad "mutual: $(cat "$log") $(cat "$fake/out")"; fi

# --- the trial passed but the real merge conflicts: abort, drop it, go on
batchenv 11 13 14
FAKE_MERGE_FAIL_REF=refs/pr-sweep/13 FAKE_CONFLICT_FILES="proto/klatch/src/lib.rs" runsweep --batch
if [ "$(calls '^git merge --abort')" -ge 1 ] && grep -q '^gh pr create .*--title Batch: #11 #14' "$log" && grep -q "^ledger --post --only 13" "$log"; then ok "a real merge that conflicts after a clean trial is aborted and the PR dropped"; else bad "real conflict: $(cat "$log") $(cat "$fake/out")"; fi

# --- a head that moved since it was seen green is not batched
batchenv 11 13 14
echo sha-moved > "$fake/rp-13"
runsweep --batch
if [ "$(calls '^git merge .*refs/pr-sweep/13')" -eq 0 ] && grep -q '^gh pr create .*--title Batch: #11 #14' "$log"; then ok "a PR whose fetched head differs from the green head is left out"; else bad "moved head: $(cat "$log") $(cat "$fake/out")"; fi

# --- a PR that is not green on its own head is never batched
batchenv 11 13 14
echo "success pending" > "$fake/checks-sha-13"
runsweep --batch
if [ "$(calls '^git merge .*refs/pr-sweep/13')" -eq 0 ] && grep -q '^gh pr create .*--title Batch: #11 #14' "$log" && [ "$(calls '^gh pr merge 13')" -eq 0 ]; then ok "only PRs green on their own head are batched"; else bad "not green: $(cat "$log")"; fi

# --- fewer than 2 survivors: nothing batched, the one-at-a-time path takes over
batchenv 11 13
echo "proto/klatch/src/lib.rs" > "$fake/mt-main-13"
runsweep --batch
if [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^git push')" -eq 0 ] && [ "$(calls '^gh pr merge 11 ')" -eq 1 ] && grep -q "^ledger --post --only 13" "$log" && grep -q "one at a time" "$fake/out"; then ok "fewer than 2 survivors: no batch, the survivor goes the one-at-a-time way, the dropped PR is posted"; else bad "fallback: $(cat "$log") $(cat "$fake/out")"; fi
batchenv 11
runsweep --batch
if [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^gh pr merge 11 ')" -eq 1 ]; then ok "a single candidate is merged the one-at-a-time way"; else bad "single: $(cat "$log")"; fi

# --- gate failure: nothing pushed, no PR, reported; the next pass goes one at a time for that same set
batchenv 11 13 14
FAKE_GATE_FAIL=1 runsweep --batch
if [ "$(calls '^git push')" -eq 0 ] && [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^gh pr merge')" -eq 0 ] && grep -q "batch gate failed" "$fake/out"; then ok "a failing gate on the combined branch pushes nothing, opens nothing and says \"batch gate failed\""; else bad "batch gate fail: $(cat "$log") $(cat "$fake/out")"; fi
for p in 11 13 14; do echo false > "$fake/merged-$p"; done
runsweep --batch
if [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^gh pr merge 11 ')" -eq 1 ] && [ "$(calls '^gh pr merge 13 ')" -eq 1 ] && grep -q "one at a time" "$fake/out"; then ok "the next pass lands that same set one at a time"; else bad "after gate fail: $(cat "$log") $(cat "$fake/out")"; fi

# --- main moved while the batch checks ran: re-merge main into the batch, re-gate, push, wait again, merge the NEW head
batchenv 11 13 14
echo 2 > "$fake/behind_by-sha-b1"
echo sha-b2 > "$fake/head-900-next"; echo "success success" > "$fake/checks-sha-b2"
runsweep --batch
if [ "$(calls '^gate')" -eq 2 ] && [ "$(calls '^git push -q origin alee/batch-')" -eq 2 ] && grep -q "^gh pr merge 900 --merge --match-head-commit sha-b2$" "$log" && ! grep -q -- "--match-head-commit sha-b1" "$log"; then ok "when main moves the batch re-merges main, re-gates, pushes and is merged pinned to the new head"; else bad "main moved: $(cat "$log") $(cat "$fake/out")"; fi
batchenv 11 13 14
echo 2 > "$fake/behind_by-sha-b1"
echo sha-b2 > "$fake/head-900-next"
runsweep --batch
if [ "$(calls '^gh pr merge 900')" -eq 0 ]; then ok "the new batch head is not merged until its checks are green"; else bad "merged unchecked head: $(cat "$log")"; fi
batchenv 11 13 14
echo 2 > "$fake/behind_by-sha-b1"
FAKE_MERGE_FAIL_REF=origin/main FAKE_CONFLICT_FILES="proto/klatch/src/lib.rs" runsweep --batch
if [ "$(calls '^gh pr merge 900')" -eq 0 ] && grep -q "pulls/900 -f state=closed" "$log" && grep -q "^ledger --post --only 11,13,14" "$log"; then ok "if main conflicts with the batch in code the batch PR is closed, nothing merges and the PRs are posted"; else bad "main conflict: $(cat "$log") $(cat "$fake/out")"; fi

# --- the batch checks are not green: never merged
batchenv 11 13 14
echo "success pending" > "$fake/checks-sha-b1"
runsweep --batch
if [ "$(calls '^gh pr merge')" -eq 0 ] && grep -q "waiting for green" "$fake/out"; then ok "a batch PR whose checks are not both green is never merged"; else bad "red batch: $(cat "$log")"; fi

# --- an included PR still open after the batch merged: comment, close via REST, say so; one that is not in main is left alone
batchenv 11 13 14
echo false > "$fake/merged-13"
PR_SWEEP_VERIFY_TRIES=2 runsweep --batch
if grep -q "^git merge-base --is-ancestor sha-13 origin/main" "$log" && grep -q "issues/13/comments .*merged via #900" "$log" && grep -q "^gh api -X PATCH repos/hslee16/klatch/pulls/13 -f state=closed" "$log" && grep -q "#13.*closed" "$fake/out"; then ok "an included PR still open after the merge is verified an ancestor of main, commented \"merged via #900\" and closed"; else bad "close straggler: $(cat "$log") $(cat "$fake/out")"; fi
batchenv 11 13 14
echo false > "$fake/merged-13"
FAKE_NOT_ANCESTOR=1 PR_SWEEP_VERIFY_TRIES=2 runsweep --batch
if ! grep -q "pulls/13 -f state=closed" "$log" && grep -q "^ledger --post --only .*13" "$log"; then ok "a still-open PR whose head is not in main is never closed; it is posted"; else bad "non ancestor: $(cat "$log")"; fi

# --- a failed bench after the batch: nothing recorded, every included PR posted
batchenv 11 13 14
FAKE_BENCH_OUT="11 passed, 1 failed" runsweep --batch
if [ "$(calls 'record-bench')" -eq 0 ] && grep -q "^ledger --post --only 11,13,14" "$log"; then ok "a failed bench after the batch records nothing and posts every included PR"; else bad "batch bench fail: $(cat "$log")"; fi

# --- --max N caps the batch; the lowest numbers go first
batchenv 11 13 14
runsweep --batch --max 2
if grep -q '^gh pr create .*--title Batch: #11 #13 --body-file' "$log" && [ "$(calls '^git merge .*refs/pr-sweep/14')" -eq 0 ]; then ok "--max 2 batches the two lowest PR numbers"; else bad "max: $(cat "$log")"; fi
newenv
runsweep --batch --max 1
r1=$rc
runsweep --max 3
if [ "$r1" -eq 2 ] && [ "$rc" -eq 2 ]; then ok "--max needs a number of at least 2 and --batch"; else bad "max args rc=$r1/$rc"; fi

# --- an open batch PR from an earlier pass blocks a new batch, and its included PRs are left alone
batchenv 11 13
printf '#900  ready                alee/batch-2026111 #711      merge\n' >> "$fake/ledger.txt"
printf 'Batch of PRs\n\n- #11\n- #13\n' > "$fake/body-900"
runsweep --batch
if [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^gh pr merge 11 ')" -eq 0 ] && [ "$(calls '^gh pr merge 13 ')" -eq 0 ] && [ "$(calls '^gh pr merge 900 ')" -eq 1 ]; then ok "an open batch PR blocks a new batch; its included PRs are skipped and the batch PR is merged the ordinary way"; else bad "open batch: $(cat "$log") $(cat "$fake/out")"; fi

# --- dry run: prints the plan (kept and dropped, and why) and changes nothing
batchenv 11 13 14
echo "proto/klatch/src/lib.rs" > "$fake/mt-main-13"
runsweep --batch --dry-run
if [ "$rc" -eq 0 ] && grep -q "would batch #11 #14" "$fake/out" && grep -q "#13 would be dropped.*lib.rs" "$fake/out" && ! grep -q "would merge #11\|would integrate main into #1[134]" "$fake/out" \
  && [ "$(gitcalls 'checkout')" -eq 0 ] && [ "$(gitcalls 'merge ')" -eq 0 ] && [ "$(gitcalls 'push')" -eq 0 ] && [ "$(gitcalls 'commit')" -eq 0 ] && [ "$(gitcalls 'branch')" -eq 0 ] \
  && [ "$(calls '^gh pr create')" -eq 0 ] && [ "$(calls '^gate')" -eq 0 ] && [ "$(calls '^bench')" -eq 0 ] && [ "$(calls 'PATCH')" -eq 0 ] && [ "$(calls '^ledger --post')" -eq 0 ] && [ "$(calls '^klatch post')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "--batch --dry-run prints which PRs would be batched and which dropped and why, and changes nothing"; else bad "batch dry-run rc=$rc: $(cat "$log") $(cat "$fake/out")"; fi
batchenv 11 13 14
echo "proto/klatch/src/store.rs" > "$fake/mt-batch-13"
runsweep --batch --dry-run
if grep -q "would batch #11 #14" "$fake/out" && grep -q "#13 would be dropped.*#11" "$fake/out"; then ok "the dry run names the earlier PR a dropped one conflicts with"; else bad "dry mutual: $(cat "$fake/out")"; fi

# --- every batch run: only alee/batch-* is ever pushed, never main, never forced, never merged on main
cat "$work"/f*/calls.log | grep '^git push' | grep -vE '^git push -q origin (alee/batch-[0-9]+|alee/integ-[0-9]+:feat-[0-9]+)$' > "$work/odd-push.log" || true
if [ ! -s "$work/odd-push.log" ]; then ok "every push is to an alee/batch-* or a PR's own branch (no force, no main)"; else bad "odd pushes: $(cat "$work/odd-push.log")"; fi

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

# --- a conflict on a numbered item that both sides carry with different text is not kept twice (#832)
d="$work/res2"; mkdir -p "$d"
cat > "$d/DESIGN.md" <<'D'
head
29. **a** old
<<<<<<< HEAD
97. **x** original text
=======
97. **x** edited text
>>>>>>> pr
D
( cd "$d" && "$sweep" --resolve-docs DESIGN.md > /dev/null 2>&1 )
if [ "$(grep -c '^97\.' "$d/DESIGN.md")" -le 1 ] || grep -q '^<<<<<<<' "$d/DESIGN.md"; then ok "an item edited on one side and present on the other is not duplicated (left as a conflict)"; else bad "duplicate item: $(cat "$d/DESIGN.md")"; fi
d="$work/res3"; mkdir -p "$d"
cat > "$d/DESIGN.md" <<'D'
head
<<<<<<< HEAD
97. **x** same
=======
98. **y** other
>>>>>>> pr
D
( cd "$d" && "$sweep" --resolve-docs DESIGN.md > /dev/null 2>&1 )
if ! grep -q '^<<<<<<<' "$d/DESIGN.md" && [ "$(grep -c '^9[78]\.' "$d/DESIGN.md")" -eq 2 ]; then ok "two different new items still keep both sides"; else bad "two new items: $(cat "$d/DESIGN.md")"; fi

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
