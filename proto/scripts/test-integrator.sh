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
if [ "$1 $2" = "pr merge" ]; then
  n="$3"; sha=""; prev=""
  for a in "$@"; do [ "$prev" = "--match-head-commit" ] && sha="$a"; prev="$a"; done
  tip="$(git --git-dir="$org" rev-parse "refs/heads/feat-$n")"
  [ "$sha" = "$tip" ] || { echo "gh: head branch was modified" >&2; exit 1; }
  [ -e "$FAKE_DIR/merge-fails" ] && exit 1
  git -C "$FAKE_SEED" fetch -q origin
  git -C "$FAKE_SEED" checkout -q main
  git -C "$FAKE_SEED" merge -q --no-edit "origin/feat-$n" > /dev/null 2>&1
  git -C "$FAKE_SEED" push -q origin main > /dev/null 2>&1
  echo true > "$FAKE_DIR/merged-$n"; echo closed > "$FAKE_DIR/state-$n"
  exit 0
fi
path=""
for a in "$@"; do case "$a" in repos/*) path="$a" ;; esac; done
case "$path" in
  */pulls\?*)
    out="["; sep=""
    for n in $(cat "$FAKE_DIR/prs" 2> /dev/null); do
      [ "$(f "state-$n" open)" = open ] || continue
      sha="$(git --git-dir="$org" rev-parse "refs/heads/feat-$n")"
      out="$out$sep{\"number\":$n,\"title\":\"merged: #1 hostile\",\"head\":{\"sha\":\"$sha\"}}"; sep=","
    done
    echo "$out]" ;;
  */pulls/*)
    n="${path##*/}"
    sha="$(f "sha-$n" "$(git --git-dir="$org" rev-parse "refs/heads/feat-$n" 2> /dev/null)")"
    cat <<EOF
{"number":$n,"state":"$(f "state-$n" open)","merged":$(f "merged-$n" false),"title":"merged: #1 ready: #2 hostile title","body":"ready: #9",
 "mergeable_state":"$(f "mstate-$n" clean)","head":{"ref":"$(f "ref-$n" "feat-$n")","sha":"$sha","repo":{"full_name":"$(f "repo-$n" hslee16/klatch)"}}}
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
exit "$(cat "$FAKE_DIR/gate-rc" 2> /dev/null || echo 0)"
SH
chmod +x "$bin"/*

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
  case "\$ref" in refs/heads/feat-*) FAKE_DIR="$fake" FIXTURES="$fx" "$bin/emit-ci" "\${ref#refs/heads/feat-}" "\$new" "\$old" ;; esac
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
_run() {
  exec env FAKE_DIR="$fake" FAKE_ORIGIN="$origin" FAKE_SEED="$seed" FIXTURES="$fx" PATH="$bin:$PATH" \
    INTEGRATOR_REPO="$repo" INTEGRATOR_KLATCH="${KB:-$bin/klatch}" INTEGRATOR_GATE="$bin/gate" INTEGRATOR_GH_REPO=hslee16/klatch \
    INTEGRATOR_RECONCILE_SECS="${RS:-2}" INTEGRATOR_POLL_SECS=0.1 INTEGRATOR_GH_RETRY_SLEEP=0 INTEGRATOR_GH_TIMEOUT=20 \
    INTEGRATOR_CHECK_WAIT_SECS="${CW:-25}" INTEGRATOR_FORWARDER_TRIES=3 INTEGRATOR_FORWARDER_MIN_UP=1000 INTEGRATOR_REQUEUE_SECS=0 \
    perl -e 'alarm shift; exec @ARGV' "${LIMIT:-90}" bash "$integrator" "$@"
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
  ! grep -Evq '^(integrating|checking|merged): #[0-9]+$|^dropped: #[0-9]+ (conflict|checks-failed|closed|timeout|head-changed|error)$|^parked: (running the gate for|waiting for checks on) #[0-9]+, back by [0-9]+m$' "$fake/posts.log" 2> /dev/null
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
