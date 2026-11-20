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
  # --allow-dirty: these tests run in whatever checkout they are in, which may have uncommitted changes (#763)
  env PATH="$shim:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_FAIL="$FAKE_FAIL" "$gate" --allow-dirty "$@" > "$work/out" 2>&1
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

# the CI step "the client feature and the bare build compile on their own" is in the gate (#910): a module missing its
# cfg attribute breaks only those builds, so a failure in either must fail the gate and say so
for pat in "--features client" "--no-default-features --locked"; do
  if run_gate "$pat"; then bad "the gate passed although the build '$pat' failed (#910)"; else
    if grep -q "FAILED at step" "$work/out" && grep -q "feature and bare builds" "$work/out" && ! grep -q "GATE PASSED" "$work/out"; then ok "a failing '$pat' build fails the gate at its own step (#910)"; else bad "build step '$pat': $(cat "$work/out")"; fi
  fi
done
run_gate ""
if grep -q -- "build -p klatch --no-default-features --features client --locked" "$work/log" && grep -q -- "build -p klatch --no-default-features --locked" "$work/log"; then ok "the gate runs the client-feature build and the bare build, as CI does (#910)"; else bad "builds missing: $(cat "$work/log")"; fi

run_gate "" --long
if grep -q "cases=5000" "$work/log"; then ok "--long raises the case budget"; else bad "--long: $(cat "$work/log")"; fi
run_gate ""
if grep -q "cases=500" "$work/log"; then ok "the default budget is small"; else bad "default budget: $(cat "$work/log")"; fi

# ---- the gate checks the commit, not the working tree (#763): in a throwaway repository with a commit
g="$work/grepo"; mkdir -p "$g/proto/scripts"; git -C "$g" init -q
cp "$gate" "$g/proto/scripts/gate"
printf 'one\n' > "$g/tracked.txt"
git -C "$g" add tracked.txt proto/scripts/gate
git -C "$g" -c user.name=t -c user.email=t@example.com commit -q -m first
gate_in_g() { # fail-pattern, args...
  FAKE_LOG="$work/log"; : > "$FAKE_LOG"
  FAKE_FAIL="$1"; shift
  env PATH="$shim:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_FAIL="$FAKE_FAIL" "$g/proto/scripts/gate" "$@" > "$work/out" 2>&1
}
printf 'two\n' > "$g/tracked.txt"
gate_in_g "fmt"
rc=$?
if [ "$rc" -ne 0 ] && [ ! -s "$work/log" ] && grep -q "uncommitted" "$work/out" && grep -q "tracked.txt" "$work/out" && grep -q "allow-dirty" "$work/out" && ! grep -q "GATE PASSED" "$work/out"; then ok "a tracked file changed since the commit: the gate refuses, names the file and runs nothing (#763)"; else bad "dirty tree rc=$rc: $(cat "$work/out") log: $(cat "$work/log")"; fi
gate_in_g "fmt" --allow-dirty
if grep -q "fmt" "$work/log" && grep -qi "working tree, not the commit" "$work/out"; then ok "--allow-dirty runs the gate anyway and says it checks the working tree, not the commit"; else bad "allow-dirty: $(cat "$work/out") log: $(cat "$work/log")"; fi
git -C "$g" checkout -q -- tracked.txt
printf 'scratch\n' > "$g/untracked.txt"
gate_in_g "fmt"
if grep -q "fmt" "$work/log" && ! grep -q "uncommitted" "$work/out"; then ok "an untracked file outside proto/, clients/ and release/ does not stop the gate"; else bad "clean tree: $(cat "$work/out") log: $(cat "$work/log")"; fi
# a new source file left out of the commit builds here and fails in CI (#785): refuse untracked, not ignored files in the source trees
rm -f "$g/untracked.txt"; mkdir -p "$g/proto/src" "$g/clients/x" "$g/release"
printf 'fn main() {}\n' > "$g/proto/src/new_mod.rs"
gate_in_g "fmt"
rc=$?
if [ "$rc" -ne 0 ] && [ ! -s "$work/log" ] && grep -q "untracked" "$work/out" && grep -q "proto/src/new_mod.rs" "$work/out" && grep -q "allow-dirty" "$work/out" && ! grep -q "GATE PASSED" "$work/out"; then ok "an untracked file under proto/ is refused, named, and nothing runs (#785)"; else bad "untracked proto rc=$rc: $(cat "$work/out") log: $(cat "$work/log")"; fi
gate_in_g "fmt" --allow-dirty
if grep -q "fmt" "$work/log"; then ok "--allow-dirty runs the gate despite an untracked source file"; else bad "allow-dirty untracked: $(cat "$work/out")"; fi
rm -f "$g/proto/src/new_mod.rs"
for d in clients/x release; do
  printf 'x\n' > "$g/$d/new.txt"
  gate_in_g "fmt"
  if [ ! -s "$work/log" ] && grep -q "$d/new.txt" "$work/out"; then ok "an untracked file under $d is refused too (#785)"; else bad "untracked $d: $(cat "$work/out")"; fi
  rm -f "$g/$d/new.txt"
done
printf 'ignored\n' > "$g/proto/ignored.log"; printf '*.log\n' > "$g/.gitignore"; git -C "$g" add .gitignore; git -C "$g" -c user.name=t -c user.email=t@example.com commit -q -m ignore
gate_in_g "fmt"
if grep -q "fmt" "$work/log" && ! grep -q "untracked" "$work/out"; then ok "an ignored file under proto/ does not stop the gate"; else bad "ignored: $(cat "$work/out")"; fi
printf 'staged\n' > "$g/tracked.txt"; git -C "$g" add tracked.txt
gate_in_g "fmt"
if [ ! -s "$work/log" ] && grep -q "uncommitted" "$work/out"; then ok "a staged but uncommitted change is refused too"; else bad "staged: $(cat "$work/out") log: $(cat "$work/log")"; fi
git -C "$g" reset -q --hard
t0="$work/not-a-repo"; mkdir -p "$t0/proto/scripts"; cp "$gate" "$t0/proto/scripts/gate"
FAKE_LOG="$work/log"; : > "$FAKE_LOG"
if env PATH="$shim:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_FAIL="fmt" GIT_CEILING_DIRECTORIES="$work" "$t0/proto/scripts/gate" > "$work/out" 2>&1; [ -s "$work/log" ] && ! grep -q "uncommitted" "$work/out"; then ok "outside a git checkout (an export) the gate has no commit to compare and runs"; else bad "not a repo: $(cat "$work/out")"; fi

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

named=$(python3 -I -c "
import importlib.machinery, importlib.util, sys
l = importlib.machinery.SourceFileLoader('pl', sys.argv[1]); sp = importlib.util.spec_from_loader('pl', l)
m = importlib.util.module_from_spec(sp); l.exec_module(m)
print(*m.failing([{'name': 'a', 'conclusion': 'success'}, {'name': 'mac', 'conclusion': 'failure', 'url': 'u1'}, {'name': 'b', 'conclusion': 'cancelled', 'url': 'u2'}]), sep=';')
" "$ledger")
if [ "$named" = "mac (failure) u1;b (cancelled) u2" ]; then ok "pr-ledger: names each red or cancelled job with its link"; else bad "pr-ledger: failing jobs listed as '$named'"; fi

# pr-ledger bench stage: a merged PR is done only when benched (or when no bench ledger is in use)
expect_stage '{"merged":true,"ticket_state":"closed","benched":false}' merged-not-benched "merged but not benched is not done"
expect_stage '{"merged":true,"ticket_state":"closed","benched":true}' 'done' "merged and benched is done"
expect_stage '{"merged":true,"ticket_state":"open","benched":true}' merged-open-ticket "benched but the ticket is open still needs the close"
expect_stage '{"merged":true,"ticket_state":"none","benched":false}' merged-not-benched "no ticket and not benched still needs the bench"
if printf '{"merged":true,"benched":false}' | python3 -I "$ledger" --classify | cut -f2 | grep -q "run the bench"; then ok "pr-ledger: merged-not-benched says to run the bench"; else bad "pr-ledger: bench next action"; fi

# the pure helpers: first error line, the post text, change detection, the bench ledger file
plpy() { python3 -I -c "
import importlib.machinery, importlib.util, sys, json
l = importlib.machinery.SourceFileLoader('pl', sys.argv[1]); sp = importlib.util.spec_from_loader('pl', l)
m = importlib.util.module_from_spec(sp); l.exec_module(m)
$1
" "$ledger" "${@:2}"; }
got=$(plpy "print(m.first_error_line('Run cargo test\n\n\`\`\`\ntest a ... ok\nerror[E0425]: cannot find value x\n  --> src/lib.rs:3\n\`\`\`'))")
if [ "$got" = "error[E0425]: cannot find value x" ]; then ok "pr-ledger: first error line picks the error, not the preamble"; else bad "pr-ledger: first error line '$got'"; fi
got=$(plpy "print(repr(m.first_error_line('')), repr(m.first_error_line(None)), m.first_error_line('only this line'))")
if [ "$got" = "'' '' only this line" ]; then ok "pr-ledger: no summary gives nothing, and a plain summary falls back to its first line"; else bad "pr-ledger: error fallback '$got'"; fi
got=$(plpy "print(*m.failing([{'name': 'mac', 'conclusion': 'failure', 'url': 'u1', 'error': 'boom'}]), sep=';')")
if [ "$got" = "mac (failure) u1: boom" ]; then ok "pr-ledger: a failing job shows its first error line"; else bad "pr-ledger: failing with error '$got'"; fi
got=$(plpy "
rows = [({'number': 7, 'ticket': 5}, 'checks-red', 'fix it'), ({'number': 3, 'ticket': None}, 'ready', 'merge')]
print(m.post_text(rows).replace(chr(10), '|'))")
if [ "$got" = "PRs needing a step:|#3 ready (no ticket): merge|#7 checks-red (#5): fix it" ]; then ok "pr-ledger: the post text lists PRs sorted by number"; else bad "pr-ledger: post text '$got'"; fi
got=$(plpy "print(repr(m.post_text([])))")
if [ "$got" = "None" ]; then ok "pr-ledger: nothing needing a step posts nothing"; else bad "pr-ledger: empty post text '$got'"; fi
sf="$work/post-state"
got=$(plpy "
a = m.changed_since_last('x', sys.argv[2]); b = m.changed_since_last('x', sys.argv[2]); c = m.changed_since_last('y', sys.argv[2])
print(a, b, c)" "$sf")
if [ "$got" = "True False True" ]; then ok "pr-ledger: a post happens only when the text changed since the last one"; else bad "pr-ledger: change detection '$got'"; fi
bl="$work/bench.json"
got=$(plpy "
print(m.is_benched(sys.argv[2], 5))
m.record_bench(sys.argv[2], 5); m.record_bench(sys.argv[2], 5); m.record_bench(sys.argv[2], 9)
print(m.is_benched(sys.argv[2], 5), m.is_benched(sys.argv[2], 6), sorted(json.load(open(sys.argv[2]))['benched']))" "$bl")
if [ "$got" = "False
True False [5, 9]" ]; then ok "pr-ledger: the bench ledger records PRs once and answers is-benched"; else bad "pr-ledger: bench ledger '$got'"; fi

# pr-ledger ticket links: only Closes/Fixes/Resolves closes; Refs links without closing; a bare (#N) is nothing (#758)
link_of() { python3 -I -c "
import importlib.machinery, importlib.util, sys
l = importlib.machinery.SourceFileLoader('pl', sys.argv[1]); sp = importlib.util.spec_from_loader('pl', l)
m = importlib.util.module_from_spec(sp); l.exec_module(m)
print(*m.ticket_of({'title': sys.argv[2], 'body': sys.argv[3]}))
" "$ledger" "$1" "$2"; }
expect_link() { # title, body, expected, label
  got=$(link_of "$1" "$2")
  if [ "$got" = "$3" ]; then ok "pr-ledger: $4"; else bad "pr-ledger: $4 (got '$got', wanted '$3')"; fi
}
expect_link "t" "Closes #12" "12 True" "Closes links and closes"
expect_link "t" "fixes #7" "7 True" "Fixes links and closes"
expect_link "t" $'Refs #678\nfound (#733) while here' "678 False" "Refs links without closing and a bare (#N) in prose is not a link"
expect_link "t (#9)" "no keywords" "None False" "a (#N) in the title alone is not a link"
expect_link "t" $'Part of #5\nCloses #6' "6 True" "Closes wins over Part of"

# pr-ledger call savings and failure words (#754, #756): the pull is fetched only for a PR whose checks are all green, and
# a rate limit is told from other failures by its words
got=$(plpy "
g = {'conclusion': 'success'}
print(m.needs_mergeable_state([]), m.needs_mergeable_state([g]), m.needs_mergeable_state([g, {'conclusion': None}]),
      m.needs_mergeable_state([g, {'conclusion': 'failure'}]), m.needs_mergeable_state([g, {'conclusion': 'cancelled'}]),
      m.needs_mergeable_state([g, {'conclusion': 'skipped'}]))")
if [ "$got" = "True True False False False True" ]; then ok "pr-ledger: the pull is wanted when there are no checks (a conflict starts none) or every check has finished and none is bad"; else bad "pr-ledger: needs_mergeable_state '$got'"; fi
got=$(plpy "print(m.rate_limited('gh: API rate limit exceeded for user ID 1. (HTTP 403)'), m.rate_limited('read tcp: operation timed out'), m.rate_limited(None), m.rate_limited('You have exceeded a secondary rate limit'))")
if [ "$got" = "True False False True" ]; then ok "pr-ledger: a rate limit is told from a network failure by its words"; else bad "pr-ledger: rate_limited '$got'"; fi

# the mutants runner refuses a TMPDIR over 60 bytes (#778, #683): a long one breaks the unix socket tests in the baseline
# and the run would report zero mutants. The refusal comes before anything is copied or built.
mutants="$here/mutants"
long_tmp="$work/$(printf 'd%.0s' $(seq 1 70))"
if env TMPDIR="$long_tmp" HOME="$work" PATH="/usr/bin:/bin" "$mutants" gateway-policy > "$work/out" 2>&1; then bad "mutants accepted a TMPDIR of ${#long_tmp} bytes"; else
  if grep -q "TMPDIR is ${#long_tmp} bytes" "$work/out" && grep -q "60" "$work/out"; then ok "mutants refuses a TMPDIR over 60 bytes and says why"; else bad "mutants refusal text: $(cat "$work/out")"; fi
fi
short_tmp="/tmp/$(printf 'd%.0s' $(seq 1 55))"   # exactly 60 bytes
if [ "${#short_tmp}" -ne 60 ]; then bad "test setup: short_tmp is ${#short_tmp} bytes"; fi
env TMPDIR="$short_tmp" HOME="$work" PATH="/usr/bin:/bin" "$mutants" gateway-policy > "$work/out" 2>&1
if ! grep -q "TMPDIR is" "$work/out"; then ok "mutants accepts a TMPDIR of exactly 60 bytes (it goes on to the next check)"; else bad "mutants refused 60 bytes: $(cat "$work/out")"; fi

# the mutants runner refuses to start on a nearly full disk (#683) and gives each module a private short TMPDIR that it
# removes afterwards (a shim cargo-mutants records the TMPDIR it was started with)
if env HOME="$work" PATH="/usr/bin:/bin" MUTANTS_MIN_FREE_GB=999999999 "$mutants" gateway-policy > "$work/out" 2>&1; then bad "mutants started with too little free disk"; else
  if grep -q "GB free" "$work/out" && grep -q "MUTANTS_MIN_FREE_GB" "$work/out"; then ok "mutants refuses to start when the disk is below the free space limit and says why"; else bad "mutants disk refusal text: $(cat "$work/out")"; fi
fi
mshim="$work/mshim"; mkdir -p "$mshim"
cat > "$mshim/cargo-mutants" <<'SH'
#!/bin/sh
echo "$TMPDIR" > "$MSHIM_LOG"
exit 0
SH
chmod +x "$mshim/cargo-mutants"
cp "$mshim/cargo-mutants" "$mshim/cargo"
short="$(mktemp -d /tmp/klt.XXXXXX)"
env HOME="$work" PATH="$mshim:/usr/bin:/bin" TMPDIR="$short" MSHIM_LOG="$work/mlog" MUTANTS_MIN_FREE_GB=0 MUTANTS_DIR="$work/mdir" "$mutants" gateway-policy > "$work/out" 2>&1
seen="$(cat "$work/mlog" 2> /dev/null)"
case "$seen" in
  "$short/klm."*) if [ -e "$seen" ]; then bad "mutants left its private TMPDIR behind: $seen"; else ok "mutants runs each module in a private short TMPDIR and removes it afterwards"; fi ;;
  *) bad "mutants did not set a private TMPDIR (saw '$seen'): $(cat "$work/out")" ;;
esac
rm -rf "$short"

echo
if [ "$fails" -eq 0 ]; then echo "all gate script tests passed"; else echo "$fails gate script test(s) failed"; exit 1; fi
