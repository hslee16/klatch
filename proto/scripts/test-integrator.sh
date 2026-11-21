#!/usr/bin/env bash
# Tests for proto/scripts/integrator (#866). The integrator runs against a FAKE `gh` (answers from files, never the
# network; its `webhook forward` replays payload fixtures), a FAKE `klatch` (records every call, keeps a queue file that
# stands for the board) and a fake gate, in a temp dir. The git side is REAL but local: a temp bare "origin", a temp seed
# clone that makes the PR branches, and the temp clone the integrator works in; a post-receive hook in the origin plays
# the part of CI by appending a check_suite fixture to the forwarder's feed. Nothing here touches GitHub, a real repo, a
# real klatch home or a keystore. The payload fixtures in integrator-fixtures/ follow GitHub's documented shapes (with
# the noise fields a real payload has: titles, bodies, logins); they were not recorded from the live repository.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
integrator="$here/integrator"
fx="$here/integrator-fixtures"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-integrator-test.XXXXXX")"
bgpids=""
cleanup() {
  for p in $bgpids; do kill -9 "$p" 2> /dev/null; done
  pkill -9 -f "$work/" 2> /dev/null
  rm -rf "$work"
}
trap cleanup EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

bin="$work/bin"; mkdir -p "$bin"
cat > "$bin/emit-ci" <<'SH'
#!/bin/sh
# emit-ci N NEWSHA OLDSHA: what CI does when a PR branch is pushed, by ci-mode: green red cancelled cancel-old-then-green
# silent-green (green, but no event: the forwarder was down) none (still running)
n=$1; new=$2; old=$3
mode="$(cat "$FAKE_DIR/ci-mode" 2> /dev/null || echo green)"
ev() { sed -e "s/@N@/$n/g" -e "s/@SHA@/$1/g" -e "s/@CONCL@/$2/g" "$FIXTURES/check_suite-pr.json" >> "$FAKE_DIR/feed"; }
case "$mode" in
  green) echo success > "$FAKE_DIR/checks-$new"; ev "$new" success ;;
  silent-green) echo pending > "$FAKE_DIR/checks-$new"; ( /bin/sleep 3; echo success > "$FAKE_DIR/checks-$new" ) > /dev/null 2>&1 & ;;
  red) echo failure > "$FAKE_DIR/checks-$new"; ev "$new" failure ;;
  cancelled) echo cancelled > "$FAKE_DIR/checks-$new"; ev "$new" cancelled ;;
  cancel-old-then-green) ev "$old" cancelled; echo success > "$FAKE_DIR/checks-$new"; ev "$new" success ;;
  none) echo pending > "$FAKE_DIR/checks-$new" ;;
esac
exit 0
SH
cat > "$bin/gh" <<'SH'
#!/bin/sh
echo "gh $*" >> "$FAKE_DIR/calls.log"
org="$FAKE_ORIGIN"
f() { cat "$FAKE_DIR/$1" 2> /dev/null || echo "$2"; }
git() { "$REALGIT" "$@"; }
if [ "$1 $2" = "webhook forward" ]; then
  echo "start $*" >> "$FAKE_DIR/forward.log"
  starts="$(wc -l < "$FAKE_DIR/forward.log" | tr -d ' ')"
  case "$(f forward-mode live)" in
    die) exit 1 ;;
    die-once) [ "$starts" -le 1 ] && exit 1 ;;
  esac
  if [ "$(f forward-mode live)" = drop ]; then
    sleep 600 & tp=$!
  else
    tail -n +1 -f "$FAKE_DIR/feed" & tp=$!
  fi
  trap 'kill $tp 2> /dev/null; echo stopped-by-term >> "$FAKE_DIR/forward.log"; exit 0' TERM INT
  wait $tp
  exit 0
fi
if [ "$1 $2" = "auth status" ]; then
  [ -e "$FAKE_DIR/no-login" ] && { echo "You are not logged into any GitHub hosts" >&2; exit 1; }
  echo "Logged in to github.com account fake"; exit 0
fi
if [ "$1 $2" = "extension list" ]; then
  [ -e "$FAKE_DIR/no-ext" ] || printf 'gh webhook\tcli/gh-webhook\tv1\n'
  exit 0
fi
if [ "$1 $2" = "pr merge" ]; then
  n="$3"; sha=""; prev=""
  for a in "$@"; do [ "$prev" = "--match-head-commit" ] && sha="$a"; prev="$a"; done
  hr="$(f "headref-$n" "feat-$n")"
  tip="$(git --git-dir="$org" rev-parse "refs/heads/$hr")"
  [ "$sha" = "$tip" ] || { echo "gh: head branch was modified" >&2; exit 1; }
  [ -e "$FAKE_DIR/merge-fails" ] && exit 1
  echo "merge $n" >> "$FAKE_DIR/merges.log"
  git -C "$FAKE_SEED" fetch -q origin
  git -C "$FAKE_SEED" checkout -q main
  git -C "$FAKE_SEED" merge -q --no-edit "origin/$hr" > /dev/null 2>&1
  git -C "$FAKE_SEED" push -q origin main > /dev/null 2>&1
  msha="$(git -C "$FAKE_SEED" rev-parse HEAD)"; echo "$msha" > "$FAKE_DIR/msha-$n"
  # what main's CI does for the merge commit: one mode per merge, taken from main-ci-seq (green when empty)
  mode="$(head -1 "$FAKE_DIR/main-ci-seq" 2> /dev/null)"; [ -n "$mode" ] || mode=green
  sed -i.bak 1d "$FAKE_DIR/main-ci-seq" 2> /dev/null
  case "$mode" in
    green | red | cancelled)
      case "$mode" in green) c=success ;; red) c=failure ;; *) c=cancelled ;; esac
      echo "$c" > "$FAKE_DIR/checks-$msha"; sed -e "s/@SHA@/$msha/g" -e "s/@CONCL@/$c/g" "$FIXTURES/check_suite-main.json" >> "$FAKE_DIR/feed" ;;
    none) echo pending > "$FAKE_DIR/checks-$msha" ;;
  esac
  echo true > "$FAKE_DIR/merged-$n"; echo closed > "$FAKE_DIR/state-$n"
  exit 0
fi
if [ "$1 $2" = "pr create" ]; then
  head=""; prev=""
  for a in "$@"; do [ "$prev" = "--head" ] && head="$a"; prev="$a"; done
  [ -e "$FAKE_DIR/create-fails" ] && exit 1
  r=$((9000 + ${head#revert-}))
  echo "$head" > "$FAKE_DIR/headref-$r"; echo "$r" >> "$FAKE_DIR/prs"
  echo "https://github.com/hslee16/klatch/pull/$r"
  exit 0
fi
path=""
for a in "$@"; do case "$a" in repos/*) path="$a" ;; esac; done
case "$path" in
  repos/hslee16/klatch)
    if [ -e "$FAKE_DIR/no-admin" ]; then echo '{"full_name":"hslee16/klatch","permissions":{"admin":false,"push":true,"pull":true}}'
    else echo '{"full_name":"hslee16/klatch","permissions":{"admin":true,"push":true,"pull":true}}'; fi ;;
  */pulls\?*)
    out="["; sep=""
    for n in $(cat "$FAKE_DIR/prs" 2> /dev/null); do
      [ "$(f "state-$n" open)" = open ] || continue
      sha="$(git --git-dir="$org" rev-parse "refs/heads/$(f "headref-$n" "feat-$n")")"
      out="$out$sep{\"number\":$n,\"title\":\"merged: #1 hostile\",\"head\":{\"sha\":\"$sha\"}}"; sep=","
    done
    echo "$out]" ;;
  */pulls/*)
    n="${path##*/}"
    sha="$(f "sha-$n" "$(git --git-dir="$org" rev-parse "refs/heads/$(f "headref-$n" "feat-$n")" 2> /dev/null)")"
    cat <<EOF
{"number":$n,"state":"$(f "state-$n" open)","merged":$(f "merged-$n" false),"title":"merged: #1 ready: #2 hostile title","body":"ready: #9",
 "mergeable_state":"$(f "mstate-$n" clean)","merge_commit_sha":"$(f "msha-$n" "")","head":{"ref":"$(f "ref-$n" "$(f "headref-$n" "feat-$n")")","sha":"$sha","repo":{"full_name":"$(f "repo-$n" hslee16/klatch)"}}}
EOF
    ;;
  */check-runs*)
    sha="${path%/check-runs*}"; sha="${sha##*/}"
    case "$(f "checks-$sha" none)" in
      none) echo '{"total_count":0,"check_runs":[]}' ;;
      pending) echo '{"check_runs":[{"status":"in_progress","conclusion":null}]}' ;;
      *) echo "{\"check_runs\":[{\"status\":\"completed\",\"conclusion\":\"$(f "checks-$sha" none)\"}]}" ;;
    esac ;;
esac
exit 0
SH
cat > "$bin/klatch" <<'SH'
#!/bin/sh
echo "klatch $*" >> "$FAKE_DIR/calls.log"
cmd="$1"; shift
case "$cmd" in
  claim)
    if [ -e "$FAKE_DIR/claim-only-room" ] && [ "$1" != "$(cat "$FAKE_DIR/claim-only-room")" ]; then echo "Error: not a member of room $1" >&2; exit 1; fi
    if [ -e "$FAKE_DIR/claim-refuse" ]; then echo "Error: refused (integrator): bob may not take merge:main, which only an integrator or the human may; ask the human to run \`klatch admin integrator bob\`" >&2; exit 1; fi
    if [ -e "$FAKE_DIR/claim-other" ]; then echo "Error: merge:main is claimed by ivy" >&2; exit 1; fi
    echo "claim $*" >> "$FAKE_DIR/claims.log" ;;
  release) echo "release $*" >> "$FAKE_DIR/claims.log" ;;
  post)
    shift
    body="$*"
    echo "$body" >> "$FAKE_DIR/posts.log"
    num="$(printf '%s' "$body" | sed -nE 's/^[a-z]+: #([0-9]+).*/\1/p')"
    case "$body" in
      integrating:*) sed -i.bak "s/^$num .*/$num integrating/" "$FAKE_DIR/queue" ;;
      checking:*) sed -i.bak "s/^$num .*/$num checking/" "$FAKE_DIR/queue" ;;
      merged:* | dropped:*) sed -i.bak "/^$num /d" "$FAKE_DIR/queue" ;;
    esac ;;
  board)
    echo "[board lobby]"; echo "asks:"; echo "  no open asks"
    if [ -s "$FAKE_DIR/queue" ]; then
      echo "queue (merge:main free):"; i=1
      while read -r n st; do echo "  $i. #$n $st (ready by bob 0m ago)"; i=$((i + 1)); done < "$FAKE_DIR/queue"
      echo "  #99 dropped: code conflict with main (3m ago; \`ready: #99\` enqueues it again)"
      echo "  #98 merged (10m ago)"
    fi
    echo "members:"; echo "  bob last seen 0s ago" ;;
  refusals) cat "$FAKE_DIR/refusals" 2> /dev/null ;;
  wait)
    t=10; while [ $# -gt 0 ]; do [ "$1" = "--timeout" ] && t="$2"; shift; done
    i=0
    while [ "$i" -lt $((t * 10)) ]; do
      if [ -e "$FAKE_DIR/wake" ]; then rm -f "$FAKE_DIR/wake"; echo "[lobby 5 1 bob (unverified)] ready: #7"; exit 0; fi
      /bin/sleep 0.1; i=$((i + 1))
    done ;;
esac
exit 0
SH
cat > "$bin/gate" <<'SH'
#!/bin/sh
echo "gate on $(git rev-parse --abbrev-ref HEAD)" >> "$FAKE_DIR/gate.log"
[ -e "$FAKE_DIR/gate-sleep" ] && /bin/sleep "$(cat "$FAKE_DIR/gate-sleep")"
exit "$(cat "$FAKE_DIR/gate-rc" 2> /dev/null || echo 0)"
SH
cat > "$bin/bench" <<'SH'
#!/bin/sh
echo "bench" >> "$FAKE_DIR/bench.log"
exit "$(cat "$FAKE_DIR/bench-rc" 2> /dev/null || echo 0)"
SH
cat > "$bin/git" <<'SH'
#!/bin/sh
# logs every push (the real git does the work), so a test can say where a push went
case " $* " in *" push "*) echo "git $*" >> "$FAKE_DIR/calls.log" ;; esac
exec "$REALGIT" "$@"
SH
chmod +x "$bin"/*
realgit="$(command -v git)"

n=0
W=""; fake=""; origin=""; seed=""; repo=""
g() { git -C "$seed" "$@"; }
newworld() {
  n=$((n + 1)); W="$work/w$n"; fake="$W/fake"; origin="$W/origin.git"; seed="$W/seed"; repo="$W/repo"
  mkdir -p "$fake"
  git init -q --bare -b main "$origin"
  git init -q -b main "$seed"; g config user.email t@example.com; g config user.name t
  printf 'base\n' > "$seed/README.md"
  printf 'l1\nl2\nl3\nl4\nl5\n' > "$seed/code.txt"
  printf 'intro\n1. **One**\n2. **Two**\n' > "$seed/DESIGN.md"
  g add -A; g commit -q -m base
  g remote add origin "$origin"; g push -q origin main
  cat > "$origin/hooks/post-receive" <<EOF
#!/bin/sh
while read old new ref; do
  case "\$ref" in
    refs/heads/feat-*) FAKE_DIR="$fake" FIXTURES="$fx" "$bin/emit-ci" "\${ref#refs/heads/feat-}" "\$new" "\$old" ;;
    refs/heads/revert-*) FAKE_DIR="$fake" FIXTURES="$fx" "$bin/emit-ci" "\$((9000 + \${ref#refs/heads/revert-}))" "\$new" "\$old" ;;
  esac
done
EOF
  chmod +x "$origin/hooks/post-receive"
  git clone -q "$origin" "$repo" 2> /dev/null
  git -C "$repo" config user.email integrator@example.com; git -C "$repo" config user.name integrator
  : > "$fake/feed"; : > "$fake/queue"; : > "$fake/prs"; echo green > "$fake/ci-mode"; echo live > "$fake/forward-mode"
}
sha_of() { git --git-dir="$origin" rev-parse "refs/heads/$1"; }
mkpr() { # number file content [message]: a PR branch from the current main, with the file replaced; queued as waiting
  local num=$1 file=$2 content=$3 mode
  mode="$(cat "$fake/ci-mode")"; echo green > "$fake/ci-mode"
  g checkout -q main; g checkout -q -B "feat-$num"
  printf '%b' "$content" > "$seed/$file"
  g add -A; g commit -q -m "pr $num"
  g push -q origin "feat-$num" 2> /dev/null
  git --git-dir="$origin" update-ref "refs/pull/$num/head" "$(sha_of "feat-$num")"
  echo "$mode" > "$fake/ci-mode"
  echo "$num" >> "$fake/prs"; echo "$num waiting" >> "$fake/queue"
}
advance() { # file content: main moves on (a merge by someone else)
  g checkout -q main
  printf '%b' "$2" > "$seed/$1"
  g add -A; g commit -q -m "main moves"; g push -q origin main 2> /dev/null
}
# the integrator, with a fake world: tiny waits, never the real gh, git remote or klatch
# A process started in the background by a non-interactive shell (a CI job, a gate run as a background task) ignores
# SIGINT, and a shell script cannot trap a signal that was ignored on entry: reset the dispositions so the signal
# cases test the integrator, not the caller (#866).
_run() {
  # shellcheck disable=SC2016 # the single quotes are on purpose: perl, not the shell, expands $SIG
  exec env FAKE_DIR="$fake" FAKE_ORIGIN="$origin" FAKE_SEED="$seed" FIXTURES="$fx" PATH="$bin:$PATH" REALGIT="$realgit" INTEGRATOR_BENCH="$bin/bench" INTEGRATOR_VERIFY_WAIT_SECS="${VW:-}" INTEGRATOR_ROOM="${ROOM:-}" INTEGRATOR_MAX_HOLD_SECS="${MHS:-}" INTEGRATOR_MAX_HOLD_MINUTES="${MHM:-}" \
    INTEGRATOR_REPO="$repo" INTEGRATOR_KLATCH="${KB:-$bin/klatch}" INTEGRATOR_GATE="$bin/gate" INTEGRATOR_GH_REPO=hslee16/klatch \
    INTEGRATOR_RECONCILE_SECS="${RS:-2}" INTEGRATOR_POLL_SECS=0.1 INTEGRATOR_GH_RETRY_SLEEP=0 INTEGRATOR_GH_TIMEOUT=20 \
    INTEGRATOR_CHECK_WAIT_SECS="${CW:-25}" INTEGRATOR_FORWARDER_TRIES=3 INTEGRATOR_FORWARDER_MIN_UP=1000 INTEGRATOR_REQUEUE_SECS=0 \
    perl -e '$SIG{INT} = "DEFAULT"; $SIG{QUIT} = "DEFAULT"; $SIG{TERM} = "DEFAULT"; alarm shift; exec @ARGV' "${LIMIT:-90}" bash "$integrator" "$@"
}
runint() { ( _run "$@" ); }
ipid=""
# shellcheck disable=SC2120,SC2119
runint_bg() { # the subshell execs the integrator, so $! is the integrator itself and a signal reaches it
  ( _run "$@" ) > "$fake/out" 2>&1 &
  ipid=$!; bgpids="$bgpids $ipid"
}
waitfor() { # file, extended regex, seconds
  local i=0
  while [ "$i" -lt $(($3 * 10)) ]; do
    grep -Eq "$2" "$1" 2> /dev/null && return 0
    /bin/sleep 0.1; i=$((i + 1))
  done
  return 1
}
posts() { cat "$fake/posts.log" 2> /dev/null | paste -sd '|' -; }
calls() { grep -c "$1" "$fake/calls.log" 2> /dev/null || true; }
fixed_posts() { # every word posted is one of the fixed templates
  ! grep -Evq '^(integrating|checking|merged): #[0-9]+$|^reverted: #[0-9]+$|^released: held merge:main [0-9]+m with no merged word, waiting for a new ready:$|^halted: main (red|not verified) after #[0-9]+$|^dropped: #[0-9]+ (conflict|checks-failed|closed|timeout|head-changed|error)$|^parked: (running the gate for|waiting for checks on) #[0-9]+, back by [0-9]+m$' "$fake/posts.log" 2> /dev/null
}
subst() { sed -e "s/@N@/$2/g" -e "s/@SHA@/$3/g" -e "s/@CONCL@/${4:-success}/g" "$fx/$1.json"; }
A=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
B=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

[ -x "$integrator" ] || { bad "proto/scripts/integrator is missing or not executable"; echo "$fails integrator script test(s) failed"; exit 1; }

# ---- the normalizer: only a number, a state, merged, a sha, a status and a conclusion survive ----
{ subst pull_request-synchronize 7 $A; subst pull_request-closed-merged 8 $B; subst check_suite-pr 7 $A failure; subst workflow_run-pr 9 $B cancelled
  subst check_suite-main 0 $A success; echo 'not json at all'; subst hostile 5 $B; } > "$work/payloads"
"$integrator" --normalize < "$work/payloads" > "$work/norm" 2> /dev/null
want="$(printf 'pr\t7\topen\tfalse\t%s\npr\t8\tclosed\ttrue\t%s\ncheck\t%s\tcompleted\tfailure\t7\ncheck\t%s\tcompleted\tcancelled\t9\ncheck\t%s\tcompleted\tsuccess\t-\npr\t5\topen\tfalse\t%s' $A $B $A $B $A $B)"
if [ "$(cat "$work/norm")" = "$want" ]; then ok "the normalizer keeps number, state, merged, head sha, status, conclusion and the PR numbers of a check, skips junk"; else bad "normalizer output: $(cat "$work/norm")"; fi
if ! grep -qiE 'merged: #|ready:|touch|hostile|alice|Add a thing' "$work/norm"; then ok "no title, body, branch or login text survives the normalizer, even from a hostile payload"; else bad "text leaked: $(cat "$work/norm")"; fi
{ printf '{\n "action": "synchronize",\n "number": 4,\n "pull_request": {"number": 4, "state": "open", "merged": false,\n  "head": {"sha": "%s"}}\n}\n' $A; subst pull_request-synchronize 6 "not-a-sha"; subst check_suite-pr 6 $B "weird_value"; } > "$work/payloads2"
"$integrator" --normalize < "$work/payloads2" > "$work/norm2" 2> /dev/null
if [ "$(cat "$work/norm2")" = "$(printf 'pr\t4\topen\tfalse\t%s\ncheck\t%s\tcompleted\tother\t6' $A $B)" ]; then ok "the normalizer reads pretty printed payloads, drops a bad sha, and shows an unknown conclusion as other"; else bad "normalizer 2: $(cat "$work/norm2")"; fi

# ---- a plain member is refused by the claim rule ----
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; touch "$fake/claim-refuse"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 2 ] && grep -q "refused (integrator)" "$fake/out" && grep -q "klatch admin integrator" "$fake/out" && [ ! -s "$fake/posts.log" ] && [ "$(calls 'webhook forward')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ]; then
  ok "a member without the integrator role is refused by the merge:main claim: exit 2, the next step named, no forwarder, no post, no merge"
else bad "plain member rc=$rc: $(cat "$fake/out") posts=$(posts)"; fi
newworld; touch "$fake/claim-other"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 3 ] && [ "$(calls 'webhook forward')" -eq 0 ]; then ok "a claim held by another integrator ends the start with exit 3 and starts no forwarder"; else bad "held by other rc=$rc: $(cat "$fake/out")"; fi

# ---- the happy path: waiting, integrating, checking, merged, with the right words and the right git ----
newworld; advance DESIGN.md 'intro\n1. **One**\n2. **Two**\n3. **Main item**\n'; advance README.md 'base moved\n'
mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; old7="$(sha_of feat-7)"
advance README.md 'base moved again\n'; main0="$(sha_of main)"
runint --once > "$fake/out" 2>&1; rc=$?
want='integrating: #7|parked: running the gate for #7, back by 25m|checking: #7|parked: waiting for checks on #7, back by 25m|merged: #7'
if [ "$rc" -eq 0 ] && [ "$(posts)" = "$want" ]; then ok "a queued PR flows integrating, gate (parked), checking, wait for checks (parked), merged, with fixed words"; else bad "happy path rc=$rc posts=$(posts) out=$(tail -5 "$fake/out")"; fi
new7="$(sha_of feat-7)"
if [ "$new7" != "$old7" ] && git --git-dir="$origin" merge-base --is-ancestor "$old7" "$new7" && git --git-dir="$origin" merge-base --is-ancestor "$main0" "$new7"; then ok "main was merged into the PR branch as a fast-forward push (the old head is an ancestor; no rebase, no force)"; else bad "branch history"; fi
if grep -q "gate on integrator/7" "$fake/gate.log" 2> /dev/null && [ "$(wc -l < "$fake/gate.log" | tr -d ' ')" -eq 1 ]; then ok "the gate ran once, on the merged branch"; else bad "gate log: $(cat "$fake/gate.log" 2> /dev/null)"; fi
if grep -q "gh pr merge 7 --repo hslee16/klatch --merge --match-head-commit $new7" "$fake/calls.log"; then ok "the merge is pinned to the head sha the checks were green on"; else bad "merge call: $(grep 'pr merge' "$fake/calls.log")"; fi
if grep -q "webhook forward --repo hslee16/klatch --events pull_request,check_suite,workflow_run" "$fake/calls.log"; then ok "the forwarder is started as gh webhook forward for the three event types"; else bad "forward call"; fi
if ! grep -q "push.*--force\|push -f" "$fake/calls.log"; then ok "no force push is ever asked for"; else bad "force"; fi
if [ "$(grep -c '^claim' "$fake/claims.log")" -ge 3 ] && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ] && grep -q "claim lobby merge:main --ttl 600" "$fake/claims.log"; then ok "the claim is taken with the lease, renewed while it works and released at the end"; else bad "claims: $(cat "$fake/claims.log")"; fi
if fixed_posts; then ok "every posted word matches a fixed template"; else bad "posts: $(posts)"; fi
if [ -z "$(git -C "$repo" status --porcelain)" ]; then ok "the integrator's checkout is left clean"; else bad "dirty checkout"; fi

# ---- the board lines that are not in the queue (dropped, merged) are never taken for work ----
if [ "$(grep -c 'dropped: #99\|#98' "$fake/posts.log")" -eq 0 ]; then ok "dropped and merged lines of the board are not read as queue entries"; else bad "ended entries read"; fi

# ---- a docs conflict is resolved, keeping both sides ----
newworld; mkpr 7 DESIGN.md 'intro\n1. **One**\n2. **Two**\n3. **From the PR**\n'
advance DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **From main**\n'
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$' && git --git-dir="$origin" show main:DESIGN.md | grep -q "From the PR" && git --git-dir="$origin" show main:DESIGN.md | grep -q "From main" && ! git --git-dir="$origin" show main:DESIGN.md | grep -q '<<<<<<<'; then ok "a docs conflict keeps both sides (as pr-sweep does) and the PR is merged"; else bad "docs conflict rc=$rc posts=$(posts)"; fi

# ---- a code conflict drops the PR with the fixed reason, before anything is touched ----
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; before="$(sha_of feat-7)"
advance code.txt 'l1\nl2\nMAIN\nl4\nl5\n'
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && [ "$(posts)" = 'dropped: #7 conflict' ] && [ "$(sha_of feat-7)" = "$before" ] && [ ! -e "$fake/gate.log" ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a code conflict posts dropped: #7 conflict only; no integrating word, no gate, no push, no merge"; else bad "conflict rc=$rc posts=$(posts)"; fi

# ---- a failed gate drops the PR and pushes nothing ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; before="$(sha_of feat-7)"; echo 1 > "$fake/gate-rc"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'dropped: #7 checks-failed$' && ! posts | grep -q 'checking' && [ "$(sha_of feat-7)" = "$before" ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a failed gate posts dropped: #7 checks-failed, pushes nothing, merges nothing"; else bad "gate fail rc=$rc posts=$(posts)"; fi

# ---- red checks drop the PR ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo red > "$fake/ci-mode"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'checking: #7|' && posts | grep -q 'dropped: #7 checks-failed$' && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a failed check suite on the current head posts dropped: #7 checks-failed and merges nothing"; else bad "red rc=$rc posts=$(posts)"; fi

# ---- a cancelled check is a failure only for the current head ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo cancel-old-then-green > "$fake/ci-mode"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$' && ! posts | grep -q dropped; then ok "a cancelled check for the superseded head sha is ignored; the green one on the current head merges"; else bad "cancel old rc=$rc posts=$(posts)"; fi
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo cancelled > "$fake/ci-mode"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'dropped: #7 checks-failed$' && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a cancelled check on the current head is a failure"; else bad "cancel current rc=$rc posts=$(posts)"; fi

# ---- a check suite on main with no PR does not touch the queue ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'
subst check_suite-main 0 cccccccccccccccccccccccccccccccccccccccc failure >> "$fake/feed"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$' && ! posts | grep -q dropped; then ok "a failed check suite on main with no PR is not a queue event"; else bad "main suite rc=$rc posts=$(posts)"; fi

# ---- a dropped forwarder: events are lost, the reconcile finds the green head ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo silent-green > "$fake/ci-mode"; echo drop > "$fake/forward-mode"
RS=1 runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$'; then ok "with every event lost, the periodic read of the head's check runs finds the green head and merges"; else bad "lost events rc=$rc posts=$(posts) out=$(tail -3 "$fake/out")"; fi
if [ "$(calls 'pulls?state=open')" -ge 2 ]; then ok "the open PR list is read at start and again on the reconcile interval"; else bad "reconcile reads: $(calls 'pulls?state=open')"; fi

# ---- a forwarder that dies once is restarted and reconciled; one that cannot run ends the process, claim released ----
newworld; echo die-once > "$fake/forward-mode"
runint_bg; waitfor "$fake/forward.log" 'start.*start|start' 10; /bin/sleep 5
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
if [ "$(grep -c '^start' "$fake/forward.log")" -ge 2 ] && [ "$(calls 'pulls?state=open')" -ge 2 ] && [ "$rc" -eq 0 ]; then ok "a forwarder that died is started again and the open PR list is read again"; else bad "restart: starts=$(grep -c '^start' "$fake/forward.log") reads=$(calls 'pulls?state=open') rc=$rc $(cat "$fake/out")"; fi
newworld; echo die > "$fake/forward-mode"
runint > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 4 ] && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ] && [ "$(grep -c '^start' "$fake/forward.log")" -le 4 ]; then ok "a forwarder that cannot be kept running ends the process with exit 4 and the claim released (it never holds it silently)"; else bad "dead forwarder rc=$rc starts=$(grep -c '^start' "$fake/forward.log") claims=$(cat "$fake/claims.log")"; fi

# ---- woken by a ready word; the claim is renewed while it works; a signal ends it cleanly ----
newworld; advance README.md 'moved\n'
RS=40 runint_bg
waitfor "$fake/calls.log" 'klatch wait lobby --only ready: --timeout 40' 15 && started=1 || started=0
mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'
touch "$fake/wake"
if waitfor "$fake/posts.log" 'merged: #7' 25; then ok "a ready word wakes the idle integrator (a 40 s wait, finished in under 25 s) and the PR is merged"; else bad "wake: posts=$(posts) started=$started out=$(tail -3 "$fake/out")"; fi
if [ "$started" -eq 1 ] && [ "$(calls 'webhook forward')" -eq 1 ] && [ "$(calls 'klatch board')" -ge 1 ]; then ok "it waits in klatch wait for ready: words and reads the queue from the board"; else bad "wait/board"; fi
claims_before="$(grep -c '^claim' "$fake/claims.log")"
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
if [ "$rc" -eq 0 ] && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ] && [ "$claims_before" -ge 3 ]; then ok "a TERM ends the process with exit 0 and releases merge:main"; else bad "term rc=$rc claims=$(cat "$fake/claims.log")"; fi
if grep -q stopped-by-term "$fake/forward.log"; then ok "the forwarder is stopped with TERM (so gh removes its temporary webhook), not killed"; else bad "forwarder not stopped by TERM: $(cat "$fake/forward.log")"; fi

# ---- a signal that bash loses is sent again; a signal during a long wait is acted on at once (#1099) ----
# bash 3.2 loses a trapped signal that lands while it expands a command substitution; the integrator runs its loop in a
# child and a small parent sends TERM again every second. INTEGRATOR_TEST_DROP_TERMS=2 makes the worker ignore two signals.
newworld; advance README.md 'moved\n'
RS=40 INTEGRATOR_TEST_DROP_TERMS=2 runint_bg
waitfor "$fake/calls.log" 'klatch wait lobby --only ready: --timeout 40' 15 && started=1 || started=0
t0=$SECONDS
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
took=$((SECONDS - t0))
if [ "$started" -eq 1 ] && [ "$rc" -eq 0 ] && [ "$took" -le 10 ] && grep -q stopped-by-term "$fake/forward.log"; then ok "a TERM that the process lost twice is sent again by the supervisor and still stops it (exit 0, in $took s, forwarder stopped)"; else bad "lost term started=$started rc=$rc took=$took s out=$(tail -3 "$fake/out")"; fi
newworld; advance README.md 'moved\n'
RS=40 runint_bg
waitfor "$fake/calls.log" 'klatch wait lobby --only ready: --timeout 40' 15 && started=1 || started=0
t0=$SECONDS
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
took=$((SECONDS - t0))
if [ "$started" -eq 1 ] && [ "$rc" -eq 0 ] && [ "$took" -le 3 ]; then ok "a TERM during a 40 s klatch wait stops the integrator in under 3 s (it did in $took s)"; else bad "term in a long wait started=$started rc=$rc took=$took s"; fi
newworld; advance README.md 'moved\n'
RS=2 runint_bg
waitfor "$fake/calls.log" 'klatch wait lobby --only ready: --timeout 2' 15 && started=1 || started=0
wpid="$(pgrep -P "$ipid" | head -1)"
kill -9 "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null
gone=0; for _ in $(seq 1 100); do if [ -n "$wpid" ] && ! kill -0 "$wpid" 2> /dev/null; then gone=1; break; fi; /bin/sleep 0.2; done
[ "$gone" -eq 0 ] && [ -n "$wpid" ] && kill -9 "$wpid" 2> /dev/null
if [ "$started" -eq 1 ] && [ "$gone" -eq 1 ]; then ok "when the supervisor is killed (kill -9) the worker notices and stops by itself"; else bad "orphan worker started=$started wpid=$wpid gone=$gone"; fi

# ---- a signal in the middle of a PR (waiting for checks) ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo none > "$fake/ci-mode"
runint_bg
waitfor "$fake/posts.log" 'checking: #7' 30 && inflight=1 || inflight=0
kill -INT "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null; rc=$?
if [ "$inflight" -eq 1 ] && [ "$rc" -eq 0 ] && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ] && [ "$(calls 'pr merge')" -eq 0 ] && grep -q stopped-by-term "$fake/forward.log"; then ok "a signal while waiting for checks releases the claim, stops the forwarder and merges nothing"; else bad "signal mid PR rc=$rc posts=$(posts)"; fi

# ---- the PR was closed while it waited ----
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'; echo none > "$fake/ci-mode"
runint_bg
waitfor "$fake/posts.log" 'checking: #7' 30
echo closed > "$fake/state-7"; subst pull_request-closed-unmerged 7 "$(sha_of feat-7)" >> "$fake/feed"
waitfor "$fake/posts.log" 'dropped: #7' 20; kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null
if posts | grep -q 'dropped: #7 closed$' && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a PR closed while it waits is dropped with the reason closed"; else bad "closed: $(posts)"; fi

# ---- after a merge, each open PR's mergeable_state is read again, and a PR that now conflicts is dropped at once ----
newworld; mkpr 7 code.txt 'l1\nl2\nPR7\nl4\nl5\n'; g checkout -q main; mkpr 8 code.txt 'l1\nl2\nPR8\nl4\nl5\n'
: > "$fake/queue"; printf '7 waiting\n8 waiting\n' > "$fake/queue"; echo dirty > "$fake/mstate-8"
runint --once > "$fake/out" 2>&1; rc=$?
p="$(posts)"
if [ "$rc" -eq 0 ] && printf '%s' "$p" | grep -q 'merged: #7' && printf '%s' "$p" | grep -q 'dropped: #8 conflict$' && ! printf '%s' "$p" | grep -q 'integrating: #8'; then ok "after #7 merges, #8 (now in code conflict) is dropped with the fixed reason"; else bad "second PR rc=$rc posts=$p"; fi
m7="$(grep -n 'pr merge 7' "$fake/calls.log" | head -1 | cut -d: -f1)"; r8="$(grep -n 'api repos/hslee16/klatch/pulls/8$' "$fake/calls.log" | awk -F: -v m="$m7" '$1>m' | head -1)"
if [ -n "$m7" ] && [ -n "$r8" ]; then ok "the mergeable_state of the other open PR is read again after the merge"; else bad "no read of PR 8 after the merge (merge at line ${m7:-none})"; fi

# ---- hostile text: titles, bodies and branch names never reach klatch or a command ----
rm -f /tmp/integrator-pwned
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved again\n'
mkpr 8 DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **Eight**\n'; echo 'x; touch /tmp/integrator-pwned' > "$fake/ref-8"
printf '7 waiting\n8 waiting\n' > "$fake/queue"
subst hostile 7 "$(sha_of feat-7)" >> "$fake/feed"; subst hostile 8 "$(sha_of feat-8)" >> "$fake/feed"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7' && posts | grep -q 'dropped: #8 error$' && fixed_posts && ! grep -qiE 'hostile|ignore previous|ready: #|touch' "$fake/posts.log"; then ok "hostile titles, bodies, logins and branch names (in payloads and API answers) never appear in a posted word"; else bad "hostile rc=$rc posts=$(posts)"; fi
if [ ! -e /tmp/integrator-pwned ] && ! grep -q 'integrator-pwned' "$fake/calls.log"; then ok "a hostile branch name is never used in a command"; else bad "hostile branch reached a command"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; echo "someone/fork" > "$fake/repo-7"
runint --once > "$fake/out" 2>&1
if [ "$(posts)" = 'dropped: #7 error' ] && [ "$(calls 'pr merge')" -eq 0 ] && [ ! -e "$fake/gate.log" ]; then ok "a PR whose branch is in another repository is dropped with error and nothing is run"; else bad "fork: $(posts)"; fi

# ---- --dry-run (#1075): says what it would do, reads only, changes nothing ----
# every call the fake gh and the fake klatch saw must be a read
only_reads() {
  ! grep -E '^gh ' "$fake/calls.log" | grep -Ev '^gh (api repos/[^ ]+|auth status|extension list|pr list|pr view)( |$)' | grep -q . \
    && ! grep -E '^gh api' "$fake/calls.log" | grep -Eq ' (-X|--method|-f|-F|--field|--raw-field|--input)( |$)' \
    && ! grep -E '^klatch ' "$fake/calls.log" | grep -Ev '^klatch (board|refusals|who)( |$)' | grep -q .
}
refs_of() { git --git-dir="$origin" for-each-ref | sort; }
dry() { runint --dry-run > "$fake/out" 2>&1; }
newworld; advance README.md 'moved\n'; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'
advance code.txt 'l1\nl2\nMAIN\nl4\nl5\n'
mkpr 9 DESIGN.md 'intro\n1. **One**\n2. **Two**\n3. **Nine**\n'
advance README.md 'moved again\n'
printf '9 waiting\n7 waiting\n' > "$fake/queue"
refs0="$(refs_of)"; head0="$(git -C "$repo" rev-parse HEAD)"
dry; rc=$?
o="$fake/out"
lineof() { grep -n -m1 -F -- "$1" "$o" | cut -d: -f1; }
l9="$(lineof 'would post: integrating: #9')"; lm9="$(lineof 'would post: merged: #9')"; l7="$(lineof 'would post: dropped: #7 conflict')"
if [ "$rc" -eq 0 ] && [ -n "$l9" ] && [ -n "$lm9" ] && [ -n "$l7" ] && [ "$l9" -lt "$lm9" ] && [ "$lm9" -lt "$l7" ]; then ok "a dry run with two queued PRs prints the would-do lines for each, in queue order"; else bad "dry run rc=$rc: $(cat "$o")"; fi
want_order="$(grep -F 'would post:' "$o" | sed 's/^.*would post: //; s/  (.*//' | paste -sd '|' -)"
if [ "$want_order" = 'integrating: #9|parked: running the gate for #9, back by 25m|checking: #9|parked: waiting for checks on #9, back by 25m|merged: #9|dropped: #7 conflict' ]; then ok "the would-post words are the fixed templates, in the order a real run posts them"; else bad "would-post words: $want_order"; fi
if grep -q 'would take the merge:main claim' "$o" && grep -q 'would start gh webhook forward --repo hslee16/klatch --events pull_request,check_suite,workflow_run' "$o" && grep -q 'would run gh pr merge 9 --merge --match-head-commit' "$o" && grep -q 'would run the gate' "$o" && grep -q 'would push' "$o"; then ok "it says it would take the claim, start the forwarder, run the gate, push and merge"; else bad "would lines: $(cat "$o")"; fi
if only_reads; then ok "the fake gh and the fake klatch saw only read calls"; else bad "a write call: $(cat "$fake/calls.log")"; fi
if [ "$(calls 'klatch post')" -eq 0 ] && [ "$(calls 'klatch claim')" -eq 0 ] && [ "$(calls 'klatch release')" -eq 0 ] && [ ! -e "$fake/posts.log" ] && [ ! -e "$fake/claims.log" ] \
  && [ "$(calls 'git push')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 0 ] && [ "$(calls 'webhook')" -eq 0 ] && [ ! -e "$fake/gate.log" ] && [ ! -e "$fake/forward.log" ]; then ok "no post, claim, release, push, merge, gate or webhook happened"; else bad "something changed: $(cat "$fake/calls.log")"; fi
if [ "$(refs_of)" = "$refs0" ]; then ok "the fake git remote is unchanged"; else bad "remote changed"; fi
if [ "$(git -C "$repo" rev-parse HEAD)" = "$head0" ] && [ "$(git -C "$repo" rev-parse --abbrev-ref HEAD)" = main ] && [ -z "$(git -C "$repo" status --porcelain)" ] && ! git -C "$repo" for-each-ref refs/heads | grep -q -v 'refs/heads/main$'; then ok "the clone is untouched: same head, on main, clean, no new branch"; else bad "clone changed"; fi
if [ "$(tail -1 "$o")" = 'dry run: nothing was changed.' ]; then ok "the last line says nothing was changed"; else bad "last line: $(tail -1 "$o")"; fi
for k in role 'gh login' 'gh permission' 'webhook extension' clone gate 'merge:main'; do
  grep -Eq "^setup: $k: ok" "$o" || bad "setup line for $k is not ok: $(grep "^setup: $k" "$o")"
done
if ! grep -q 'PROBLEM' "$o"; then ok "a healthy setup reports every check as ok"; else bad "unexpected problem: $(grep PROBLEM "$o")"; fi

# a missing role (a recorded refusal by the integrator rule), a missing login and a missing permission are named problems
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; printf '30s ago\tbob\tlobby\tclaim\tintegrator\n' > "$fake/refusals"
dry; rc=$?
if [ "$rc" -eq 1 ] && grep -q '^setup: role: PROBLEM no-integrator-role: .*klatch admin integrator' "$fake/out" && [ "$(tail -1 "$fake/out")" = 'dry run: nothing was changed.' ] && only_reads; then ok "a member that was refused the claim is reported as a named role problem with the next step; still read only"; else bad "role problem rc=$rc: $(cat "$fake/out")"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; touch "$fake/no-login"
dry; rc=$?
if [ "$rc" -eq 1 ] && grep -q '^setup: gh login: PROBLEM no-gh-login: .*gh auth login' "$fake/out" && only_reads; then ok "no gh login is a named problem with the next step"; else bad "login problem rc=$rc: $(cat "$fake/out")"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; touch "$fake/no-admin" "$fake/no-ext"
dry; rc=$?
if [ "$rc" -eq 1 ] && grep -q '^setup: gh permission: PROBLEM no-admin: ' "$fake/out" && grep -q '^setup: webhook extension: PROBLEM no-webhook-extension: .*gh extension install cli/gh-webhook' "$fake/out"; then ok "a login without admin, and a missing webhook extension, are named problems"; else bad "permission problem rc=$rc: $(cat "$fake/out")"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; echo dirt > "$repo/untracked.txt"; git -C "$repo" checkout -q -b other
dry; rc=$?
if [ "$rc" -eq 1 ] && grep -q '^setup: clone: PROBLEM dirty-clone: ' "$fake/out" && grep -q '^setup: clone: PROBLEM wrong-branch: ' "$fake/out"; then ok "a dirty clone and a clone on the wrong branch are named problems"; else bad "clone problems rc=$rc: $(cat "$fake/out")"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'
mv "$bin/gate" "$bin/gate.off"; dry; rc=$?; mv "$bin/gate.off" "$bin/gate"
if [ "$rc" -eq 1 ] && grep -q '^setup: gate: PROBLEM no-gate: ' "$fake/out"; then ok "a missing gate script is a named problem"; else bad "gate problem rc=$rc: $(cat "$fake/out")"; fi

# hostile text never appears in the output
rm -f /tmp/integrator-pwned
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; mkpr 8 DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **Eight**\n'; echo 'x; touch /tmp/integrator-pwned' > "$fake/ref-8"
printf '7 waiting\n8 waiting\n' > "$fake/queue"
dry; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'would post: dropped: #8 error' "$fake/out" && ! grep -qiE 'hostile|ignore previous|ready: #|touch|integrator-pwned' "$fake/out" && [ ! -e /tmp/integrator-pwned ]; then ok "a hostile PR title, body and branch name never appear in the dry run output"; else bad "hostile in dry run rc=$rc: $(cat "$fake/out")"; fi
newworld; dry; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'queue is empty' "$fake/out" && [ "$(tail -1 "$fake/out")" = 'dry run: nothing was changed.' ]; then ok "an empty queue is said so"; else bad "empty queue rc=$rc: $(cat "$fake/out")"; fi

# ---- --merge-without-update (#867): off by default; merge a PR whose merge-tree against current main is clean ----
mwu() { runint --once --merge-without-update "$@" > "$fake/out" 2>&1; }
pushes() { grep -E '^git .*push ' "$fake/calls.log" 2> /dev/null; }
no_leak() { ! grep -qiE 'hostile|ignore previous|ready: #|touch|integrator-pwned' "$fake/posts.log" "$fake/calls.log" 2> /dev/null; }
clean_world() { newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved\n'; }

# the default mode never skips the update, never reads main's verdict and never reverts
clean_world; old7="$(sha_of feat-7)"; echo red > "$fake/main-ci-seq"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && [ "$(sha_of feat-7)" != "$old7" ] && [ -s "$fake/gate.log" ] && ! posts | grep -q 'reverted\|halted' && ! pushes | grep -q revert-; then ok "without the flag the up-to-date flow runs as before: main merged into the branch, the gate run, a red main is not acted on"; else bad "default mode rc=$rc posts=$(posts)"; fi

# a clean merge-tree merges without updating the branch
clean_world; old7="$(sha_of feat-7)"
mwu; rc=$?
if [ "$rc" -eq 0 ] && [ "$(posts)" = 'integrating: #7|checking: #7|parked: waiting for checks on #7, back by 25m|merged: #7' ]; then ok "a clean merge-tree merges with no main merge, no gate and no push: integrating, checking, parked, merged, and nothing extra after a green main"; else bad "clean merge rc=$rc posts=$(posts) out=$(tail -3 "$fake/out")"; fi
if [ "$(sha_of feat-7)" = "$old7" ] && [ ! -e "$fake/gate.log" ] && [ -z "$(pushes)" ]; then ok "the PR branch was not touched (same head), the gate did not run and nothing was pushed"; else bad "branch changed or push: $(pushes)"; fi
if grep -q "gh pr merge 7 --repo hslee16/klatch --merge --match-head-commit $old7" "$fake/calls.log"; then ok "the merge is pinned to the PR's current head with --match-head-commit"; else bad "merge call: $(grep 'pr merge' "$fake/calls.log")"; fi
if fixed_posts && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ]; then ok "every word is a fixed template and the claim is released"; else bad "words or claim"; fi

# a code conflict still drops
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance code.txt 'l1\nl2\nMAIN\nl4\nl5\n'
mwu; rc=$?
if [ "$rc" -eq 0 ] && [ "$(posts)" = 'dropped: #7 conflict' ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "with the flag a code conflict still posts dropped: #7 conflict and merges nothing"; else bad "mwu conflict rc=$rc posts=$(posts)"; fi

# a docs-only conflict is not a clean merge-tree: it takes the update path (both sides kept)
newworld; mkpr 7 DESIGN.md 'intro\n1. **One**\n2. **Two**\n3. **From the PR**\n'; advance DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **From main**\n'
mwu; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$' && [ -s "$fake/gate.log" ] && git --git-dir="$origin" show main:DESIGN.md | grep -q "From the PR" && git --git-dir="$origin" show main:DESIGN.md | grep -q "From main"; then ok "a docs-only conflict is not clean, so it still takes the update flow and keeps both sides"; else bad "mwu docs rc=$rc posts=$(posts)"; fi

# red checks on the PR head: not merged
clean_world; mv "$fake/ci-mode" "$fake/ci-mode.x"; echo red > "$fake/ci-mode"; git -C "$seed" checkout -q feat-7; printf 'l1\nl2\nPR2\nl4\nl5\n' > "$seed/code.txt"; git -C "$seed" commit -q -am more; git -C "$seed" push -q origin feat-7 2> /dev/null
mwu; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'dropped: #7 checks-failed$' && [ "$(calls 'pr merge')" -eq 0 ]; then ok "with the flag a failed check suite on the current head still drops the PR and merges nothing"; else bad "mwu red pr rc=$rc posts=$(posts)"; fi

# a green main from another sha does not matter; a red main from ANOTHER sha is not reverted by this run
clean_world
subst check_suite-main 0 cccccccccccccccccccccccccccccccccccccccc failure >> "$fake/feed"
mwu; rc=$?
if [ "$rc" -eq 0 ] && [ "$(posts)" = 'integrating: #7|checking: #7|parked: waiting for checks on #7, back by 25m|merged: #7' ] && ! pushes | grep -q revert- && [ "$(calls 'pr create')" -eq 0 ]; then ok "a red main from ANOTHER sha is not this run's merge: no revert, no word, no branch"; else bad "other sha rc=$rc posts=$(posts)"; fi

# red main attributed to the merge: the revert goes through a branch and a PR, never a push to main
clean_world; printf 'red\ngreen\n' > "$fake/main-ci-seq"
mwu; rc=$?
want='integrating: #7|checking: #7|parked: waiting for checks on #7, back by 25m|merged: #7|parked: waiting for checks on #9007, back by 25m|reverted: #7'
if [ "$rc" -eq 0 ] && [ "$(posts)" = "$want" ]; then ok "a red main after the merge reverts it and posts reverted: #7 (the revert PR's own checks are waited for)"; else bad "red main rc=$rc posts=$(posts) out=$(tail -4 "$fake/out")"; fi
l_push="$(grep -n 'push .*revert-7' "$fake/calls.log" | head -1 | cut -d: -f1)"; l_create="$(grep -n '^gh pr create' "$fake/calls.log" | head -1 | cut -d: -f1)"; l_merge="$(grep -n '^gh pr merge 9007 ' "$fake/calls.log" | head -1 | cut -d: -f1)"
if [ -n "$l_push" ] && [ -n "$l_create" ] && [ -n "$l_merge" ] && [ "$l_push" -lt "$l_create" ] && [ "$l_create" -lt "$l_merge" ]; then ok "the order is: push the branch revert-7, create the PR, then merge the revert PR"; else bad "order push=$l_push create=$l_create merge=$l_merge: $(cat "$fake/calls.log")"; fi
if pushes | grep -q 'origin revert-7:refs/heads/revert-7' && ! pushes | grep -Eq 'main|--force| -f '; then ok "the only push is to the remote branch revert-7 (never main, never forced)"; else bad "pushes: $(pushes)"; fi
if grep -q "gh pr create --repo hslee16/klatch --base main --head revert-7 --title Revert PR #7 (main went red)" "$fake/calls.log" && grep -q "gh pr merge 9007 --repo hslee16/klatch --merge --match-head-commit $(sha_of revert-7)" "$fake/calls.log"; then ok "the revert PR has a fixed title and is merged pinned to its current head"; else bad "create/merge calls: $(grep -E 'pr (create|merge)' "$fake/calls.log")"; fi
if [ "$(git --git-dir="$origin" show main:code.txt)" = "$(printf 'l1\nl2\nl3\nl4\nl5')" ] && git --git-dir="$origin" log -1 --format=%s main | grep -q '^Revert PR #7'; then ok "main ends with the PR's change undone by the merged revert"; else bad "main content: $(git --git-dir="$origin" show main:code.txt)"; fi
if no_leak && fixed_posts; then ok "no PR title or body text is in a word, a command or the revert"; else bad "leak"; fi

# hostile text in the PR: the revert commit message and PR text come from a fixed template
git --git-dir="$origin" log -1 --format=%B revert-7 > "$work/revert-msg"
if ! grep -qiE 'hostile|ignore previous|feat-7|pr 7$' "$work/revert-msg" && grep -q 'Revert PR #7 (main went red)' "$work/revert-msg"; then ok "the revert commit message is the fixed template (no branch name, title or body of the reverted PR)"; else bad "revert message: $(cat "$work/revert-msg")"; fi

# a revert that fails halts: distinct exit code, a fixed word, no further merge
clean_world; mkpr 8 DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **Eight**\n'; printf '7 waiting\n8 waiting\n' > "$fake/queue"
printf 'red\n' > "$fake/main-ci-seq"; touch "$fake/create-fails"
mwu; rc=$?
if [ "$rc" -eq 5 ] && posts | grep -q 'halted: main red after #7$' && ! posts | grep -q 'reverted: #7' && [ "$(calls 'pr merge')" -eq 1 ] && [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ]; then ok "a revert that cannot be made halts: exit 5, halted: main red after #7, PR 8 not merged, the claim released"; else bad "halt rc=$rc posts=$(posts) merges=$(calls 'pr merge')"; fi
if ! posts | grep -q '#8' && fixed_posts && no_leak; then ok "nothing was posted for the next PR after the halt, and every word is fixed"; else bad "after halt: $(posts)"; fi

# a revert whose own checks fail halts too (never a second revert, never a merge of the red revert)
clean_world; mkpr 8 DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **Eight**\n'; printf '7 waiting\n8 waiting\n' > "$fake/queue"
printf 'red\n' > "$fake/main-ci-seq"; echo red > "$fake/ci-mode"
mwu; rc=$?
if [ "$rc" -eq 5 ] && posts | grep -q 'halted: main red after #7$' && [ "$(calls 'pr merge 9007')" -eq 0 ] && [ "$(calls 'pr merge')" -eq 1 ] && [ "$(pushes | grep -c revert-)" -eq 1 ]; then ok "a revert PR with red checks is not merged: halted, exit 5, one revert branch only, PR 8 untouched"; else bad "red revert rc=$rc posts=$(posts)"; fi

# a revert that merges but leaves main red halts (no loop)
clean_world; printf 'red\nred\n' > "$fake/main-ci-seq"
mwu; rc=$?
if [ "$rc" -eq 5 ] && posts | grep -q 'halted: main red after #7$' && ! posts | grep -q 'reverted: #7' && [ "$(pushes | grep -c revert-)" -eq 1 ]; then ok "main still red after the revert halts the process and never reverts again"; else bad "still red rc=$rc posts=$(posts)"; fi

# main never reports: the merge is not verified, so the process stops rather than merge on
clean_world; mkpr 8 DESIGN.md 'intro\n1. **One**\n2. **Two**\n4. **Eight**\n'; printf '7 waiting\n8 waiting\n' > "$fake/queue"; echo none > "$fake/main-ci-seq"
VW=3 mwu; rc=$?
if [ "$rc" -eq 5 ] && posts | grep -q 'halted: main not verified after #7$' && [ "$(calls 'pr merge')" -eq 1 ] && ! pushes | grep -q revert-; then ok "a main that is never verified halts (exit 5) without reverting a merge it cannot show was bad"; else bad "unverified rc=$rc posts=$(posts)"; fi

# a cancelled run on main is not a failure to revert on
clean_world; echo cancelled > "$fake/main-ci-seq"
VW=3 mwu; rc=$?
if [ "$rc" -eq 5 ] && posts | grep -q 'halted: main not verified after #7$' && ! pushes | grep -q revert-; then ok "a cancelled check on main is not a failure: no revert, the process halts as unverified"; else bad "cancelled main rc=$rc posts=$(posts)"; fi

# --verify-gate runs the gate on the merge commit of main; --verify-bench runs the bench
clean_world
mwu --verify-gate --verify-bench; rc=$?
if [ "$rc" -eq 0 ] && grep -q 'gate on HEAD' "$fake/gate.log" && [ "$(wc -l < "$fake/gate.log" | tr -d ' ')" -eq 1 ] && [ "$(wc -l < "$fake/bench.log" | tr -d ' ')" -eq 1 ] && posts | grep -q 'parked: running the gate for #7' && posts | grep -q 'merged: #7|' && ! posts | grep -q 'reverted\|halted'; then ok "--verify-gate and --verify-bench run the gate (on main's merge commit) and the bench once after the merge, parked first"; else bad "verify flags rc=$rc posts=$(posts) gate=$(cat "$fake/gate.log" 2> /dev/null) bench=$(cat "$fake/bench.log" 2>&1) out=$(tail -4 "$fake/out")"; fi
clean_world; echo 1 > "$fake/gate-rc"
mwu --verify-gate; rc=$?
if [ "$rc" -eq 5 ] && pushes | grep -q 'revert-7' && posts | grep -q 'halted: main red after #7$'; then ok "a red local gate on main triggers the revert path (here the gate stays red, so it halts)"; else bad "red gate rc=$rc posts=$(posts)"; fi
clean_world
mwu; rc=$?
if [ ! -e "$fake/gate.log" ] && [ ! -e "$fake/bench.log" ]; then ok "without the verify flags neither the gate nor the bench runs after a merge"; else bad "gate or bench ran"; fi

# the verify flags need the mode
newworld
runint --once --verify-gate > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'merge-without-update' "$fake/out"; then ok "--verify-gate and --verify-bench without --merge-without-update are a usage error"; else bad "verify without mode rc=$rc"; fi

# the dry run describes the mode's plan and changes nothing
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance README.md 'moved\n'
refs0="$(refs_of)"
runint --dry-run --merge-without-update --verify-gate > "$fake/out" 2>&1; rc=$?
o="$fake/out"
if [ "$rc" -eq 0 ] && grep -q 'merge without update' "$o" && grep -q 'would skip merging main into the branch' "$o" && grep -q 'would verify main' "$o" && grep -q 'would run the gate on main' "$o" && grep -q 'would revert' "$o" && grep -q 'branch revert-7' "$o" && grep -q 'never a push to main' "$o" && ! grep -q 'would push the merged branch' "$o"; then ok "the dry run says it would skip the update, verify main, run the gate on main and revert through a branch and a PR"; else bad "dry run mwu rc=$rc: $(cat "$o")"; fi
want_order="$(grep -F 'would post:' "$o" | sed 's/^.*would post: //; s/  (.*//' | paste -sd '|' -)"
if [ "$want_order" = 'integrating: #7|checking: #7|parked: waiting for checks on #7, back by 25m|merged: #7|parked: running the gate for #7, back by 25m|reverted: #7' ] && only_reads && [ "$(refs_of)" = "$refs0" ] && [ ! -e "$fake/posts.log" ] && [ -z "$(pushes)" ] && no_leak; then ok "the dry run's words are the real templates, in order, and it is still read only"; else bad "dry mwu words: $want_order"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; advance code.txt 'l1\nl2\nMAIN\nl4\nl5\n'
runint --dry-run --merge-without-update > "$fake/out" 2>&1
if grep -q 'would post: dropped: #7 conflict' "$fake/out"; then ok "the dry run of the mode still reports a conflict as dropped"; else bad "dry mwu conflict: $(cat "$fake/out")"; fi

# ---- merge:main is one room, the lobby; the helper holds it in its configured room or does not run (#1055) ----
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; echo lobby > "$fake/claim-only-room"
ROOM=ops runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 3 ] && grep -q "room ops" "$fake/out" && grep -q "the lobby" "$fake/out" && [ "$(calls 'webhook forward')" -eq 0 ] && [ ! -s "$fake/posts.log" ] && [ "$(calls 'pr merge')" -eq 0 ]; then ok "a helper that cannot hold merge:main in its configured room (not the lobby) refuses to run: exit 3, the room and the lobby named, no forwarder, no post"; else bad "wrong room rc=$rc: $(cat "$fake/out")"; fi
ROOM=ops runint --once > "$fake/out" 2>&1
if grep -q "claim ops merge:main" "$fake/claims.log" 2> /dev/null || grep -q "klatch claim ops merge:main" "$fake/calls.log"; then ok "the claim is asked for in INTEGRATOR_ROOM"; else bad "claim room: $(cat "$fake/calls.log")"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; echo lobby > "$fake/claim-only-room"
runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 0 ] && posts | grep -q 'merged: #7$' && ! grep -v '^claim lobby\|^release lobby' "$fake/claims.log" | grep -q .; then ok "by default the room is the lobby: every claim and release is for lobby and the PR merges"; else bad "lobby default rc=$rc claims=$(cat "$fake/claims.log")"; fi
newworld
ROOM='bad room;x' runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 1 ] && grep -q "INTEGRATOR_ROOM" "$fake/out" && [ ! -s "$fake/claims.log" ]; then ok "a room name that is not a plain name is a usage error before any claim"; else bad "bad room rc=$rc: $(cat "$fake/out")"; fi

# ---- a hold with no merged word is released by the helper itself, and taken again only by a fresh ready: (#1054) ----
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'; mkpr 8 README.md 'eight\n'
echo 1 > "$fake/gate-rc"; echo 2 > "$fake/gate-sleep"
MHS=1 RS=30 runint_bg
if waitfor "$fake/posts.log" '^released: held merge:main' 40; then ok "a hold with no merged word for the limit is released with a visible word"; else bad "no release: posts=$(posts) out=$(tail -3 "$fake/out")"; fi
rel_at="$(grep -c . "$fake/posts.log")"
/bin/sleep 3
if [ "$(tail -1 "$fake/claims.log" | cut -d' ' -f1)" = release ] && [ "$(grep -c . "$fake/posts.log")" -eq "$rel_at" ]; then ok "after releasing, the claim is not taken again while the queue is unchanged (a stuck loop cannot hold forever)"; else bad "retook: posts=$(posts) claims=$(tail -3 "$fake/claims.log")"; fi
echo 0 > "$fake/gate-rc"; rm -f "$fake/gate-sleep"; mkpr 9 NINE.md 'nine\n'; touch "$fake/wake"
if waitfor "$fake/posts.log" 'merged: #9' 40; then ok "a fresh ready: word makes it take the claim again and carry on"; else bad "no resume: posts=$(posts)"; fi
kill -TERM "$ipid" 2> /dev/null; wait "$ipid" 2> /dev/null
if fixed_posts; then ok "the release word is a fixed template of an integer"; else bad "posts: $(posts)"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'
MHS=1 runint --once > "$fake/out" 2>&1
if posts | grep -q 'merged: #7$' && ! posts | grep -q '^released'; then ok "a merged word resets the hold clock: a working integrator is not released"; else bad "released while merging: $(posts)"; fi
newworld; mkpr 7 code.txt 'l1\nl2\nPR\nl4\nl5\n'
MHM=abc runint --once > "$fake/out" 2>&1; rc=$?
if [ "$rc" -eq 1 ] && grep -q "INTEGRATOR_MAX_HOLD_MINUTES" "$fake/out"; then ok "INTEGRATOR_MAX_HOLD_MINUTES must be a whole number"; else bad "bad hold knob rc=$rc: $(cat "$fake/out")"; fi

# ---- a real klatch refuses a plain member (only when a built binary exists; temp home and key dir) ----
kb="${INTEGRATOR_TEST_KLATCH:-$here/../target/debug/klatch}"
if [ -x "$kb" ]; then
  newworld; kh="$W/khome"; mkdir -p "$kh/keys"
  KLATCH_HOME="$kh/home" KLATCH_KEY_DIR="$kh/keys" KLATCH_NAME=bob "$kb" join lobby > /dev/null 2>&1
  KLATCH_HOME="$kh/home" KLATCH_KEY_DIR="$kh/keys" KLATCH_NAME=bob KB="$kb" runint --once > "$fake/out" 2>&1; rc=$?
  if [ "$rc" -eq 2 ] && grep -q "refused (integrator)" "$fake/out"; then ok "a real klatch (temp home) refuses a plain member's merge:main claim and the helper exits 2"; else bad "real klatch rc=$rc: $(cat "$fake/out")"; fi
else
  ok "(skipped) no klatch binary at $kb: the real refusal check needs a built binary"
fi

echo
if [ "$fails" -eq 0 ]; then echo "all integrator script tests passed"; else echo "$fails integrator script test(s) failed"; exit 1; fi
