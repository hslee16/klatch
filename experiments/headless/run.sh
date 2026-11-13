#!/bin/bash
# Real-session check for headless Claude Code (#285): does every hook fire, does the session join the lobby at
# start and leave at the end, and does a nested child never mark its parent? Costs a few cents per run; run it on
# demand (the human's choice), not in CI. It uses a scratch project and a scratch store, never your real ones.
#
#   experiments/headless/run.sh [--klatch PATH] [--runs N] [--default-sources] [--nested] [--json FILE]
#
# --default-sources  load the user's own Claude Code settings and plugins too (the situation in which #284 showed up)
# --nested           also run a parent session that starts a child `claude -p` and checks the parent was not marked
set -u
KLATCH="${HOME}/.local/share/klatch/klatch"; RUNS=3; SOURCES="--setting-sources local"; NESTED=0; JSON=""
while [ $# -gt 0 ]; do
  case "$1" in
    --klatch) KLATCH="$2"; shift 2;;
    --runs) RUNS="$2"; shift 2;;
    --default-sources) SOURCES=""; shift;;
    --nested) NESTED=1; shift;;
    --json) JSON="$2"; shift 2;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done
[ -x "$KLATCH" ] || { echo "no klatch binary at $KLATCH (use --klatch PATH)" >&2; exit 2; }
command -v claude >/dev/null || { echo "claude is not installed" >&2; exit 2; }

WORK=$(mktemp -d /private/tmp/klatch-headless.XXXXXX 2>/dev/null || mktemp -d)
mkdir -p "$WORK/proj" "$WORK/home"
export KLATCH_HOME="$WORK/home"
(cd "$WORK/proj" && "$KLATCH" init --scope project --yes >/dev/null) || { echo "init failed" >&2; exit 2; }

members() { KLATCH_NAME=checker "$KLATCH" who lobby --all 2>/dev/null | awk -F'\t' '{print $1}' | grep -v '^checker$' | sort; }
fails=0; total=0; cost=0; results=()
record() { # name pass detail
  total=$((total+1)); if [ "$2" = 1 ]; then echo "pass  $1"; else echo "FAIL  $1: $3"; fails=$((fails+1)); fi
  results+=("{\"check\":\"$1\",\"passed\":$([ "$2" = 1 ] && echo true || echo false),\"detail\":\"$3\"}")
}
add_cost() { c=$(python3 -c "import json,sys;print(json.load(open('$1')).get('total_cost_usd',0))" 2>/dev/null || echo 0); cost=$(python3 -c "print($cost+$c)"); }

for i in $(seq 1 "$RUNS"); do
  before=$(members)
  out="$WORK/run$i.json"
  (cd "$WORK/proj" && claude -p "Run ls once with the Bash tool, then reply with the single word done." \
    $SOURCES --permission-mode acceptEdits --allowedTools "Bash(ls:*)" --output-format json < /dev/null > "$out" 2>/dev/null)
  sleep 2; add_cost "$out"
  new=$(comm -13 <(echo "$before") <(members) | head -1)
  if [ -z "$new" ]; then record "run $i: the session joined the lobby" 0 "no new member appeared"; continue; fi
  record "run $i: the session joined the lobby ($new)" 1 ""
  hooks=$(KLATCH_NAME=checker "$KLATCH" hooks --of "$new")
  for ev in SessionStart UserPromptSubmit PostToolUse SessionEnd; do
    if echo "$hooks" | grep -q "^$ev"; then record "run $i: $ev fired" 1 ""; else record "run $i: $ev fired" 0 "not recorded"; fi
  done
  if KLATCH_NAME=checker "$KLATCH" who lobby --all | grep "^$new" | grep -q "status: done"; then
    record "run $i: marked done at the end" 1 ""
  else record "run $i: marked done at the end" 0 "no done status"; fi
done

if [ "$NESTED" = 1 ]; then
  before=$(members)
  out="$WORK/nested.json"
  prompt="Step 1: with the Bash tool run exactly: claude -p 'say hi' --permission-mode acceptEdits --output-format json. Step 2: with the Bash tool run exactly: $KLATCH --discover who lobby --all. Reply with the full output of step 2 and nothing else."
  (cd "$WORK/proj" && claude -p "$prompt" $SOURCES --permission-mode acceptEdits \
    --allowedTools "Bash(claude:*)" "Bash($KLATCH:*)" --output-format json < /dev/null > "$out" 2>/dev/null)
  sleep 2; add_cost "$out"
  mid=$(python3 -c "import json;print(json.load(open('$out')).get('result',''))" 2>/dev/null)
  after=$(members); added=$(comm -13 <(echo "$before") <(echo "$after") | wc -l | tr -d ' ')
  if [ "$added" -ge 2 ]; then record "nested: parent and child are two members" 1 ""; else record "nested: parent and child are two members" 0 "only $added new"; fi
  # The parent is the new member whose SessionStart ran longest ago. In the listing the parent itself took after
  # the child ended, the child must be done (its own end) and the parent must not be.
  parent=""; child=""; oldest=-1
  for m in $(comm -13 <(echo "$before") <(echo "$after")); do
    age=$(KLATCH_NAME=checker "$KLATCH" hooks --of "$m" | awk '$1=="SessionStart"{print $4}' | tr -d 's')
    if [ -n "$age" ] && [ "$age" -gt "$oldest" ]; then
      [ -n "$parent" ] && child="$parent"; parent="$m"; oldest="$age"
    else child="$m"; fi
  done
  pline=$(echo "$mid" | grep "^$parent" | head -1); cline=$(echo "$mid" | grep "^$child" | head -1)
  if [ -n "$parent" ] && [ -n "$child" ] && echo "$cline" | grep -q "status: done" && ! echo "$pline" | grep -q "status: done"; then
    record "nested: the child's end marked the child done and did not mark its parent" 1 ""
  else
    record "nested: the child's end marked the child done and did not mark its parent" 0 "parent=$parent [$pline] child=$child [$cline]"
  fi
fi

echo; echo "$((total-fails)) of $total checks passed; cost about \$$cost; claude $(claude --version 2>/dev/null | head -1)"
if [ -n "$JSON" ]; then
  printf '{"claude":"%s","runs":%s,"sources":"%s","nested":%s,"cost_usd":%s,"checks":[%s]}\n' \
    "$(claude --version 2>/dev/null | head -1)" "$RUNS" "${SOURCES:-default}" "$NESTED" "$cost" "$(IFS=,; echo "${results[*]}")" > "$JSON"
fi
rm -rf "$WORK"
[ "$fails" = 0 ]
