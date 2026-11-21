#!/usr/bin/env bash
# Tests for the dm adoption measurement (#1194), against a canned store, a fake klatch and a fake claude, so nothing
# costs money and no Claude Code install is needed. Cheap enough for the gate; real runs are the human's choice.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repeat="$here/../variance/repeat.py"
metric="$here/metric.py"
run="$here/run.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-dmadopt.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

# ---- the metric on a canned store: pair alpha/helper moved to a dm, pair carol/dave stayed in the lobby ----
cat > "$work/canned.txt" <<'EOF'
[lobby]
[1 1000 alpha] seeking: how do I slugify a title
[2 1001 helper] moved: details in dm with alpha
[3 1002 helper] answer to #1: x
[4 1003 alpha] closed #1 ok
[ops]
[1 1000 carol] seeking: which port
[2 1001 dave] need-info #1: tcp or udp
[3 1002 carol] info #1: tcp
[4 1003 dave] answer to #1: 8080
[dm.alpha.helper]
[1 1010 helper] q1
[2 1020 alpha] a1
[3 1030 helper] q2
[4 1040 alpha] a2
EOF
out="$(python3 "$metric" --history "$work/canned.txt" 2>&1)"; code=$?
if [ "$code" -eq 0 ]; then ok "the metric exits 0 on a canned store"; else bad "metric exit $code: $out"; fi
if [ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" = 1 ]; then ok "the metric prints one line"; else bad "metric lines: $out"; fi
chk() { python3 -c "import json,sys; d=json.loads(sys.argv[1]); e=d['agents']['exchange']; sys.exit(0 if ($2) else 1)" "$out"; }
if chk x "e['adoption']==0.5"; then ok "one exchange moved and one did not: adoption 0.5"; else bad "adoption: $out"; fi
if chk x "e['exchanges']==2 and e['moved']==1"; then ok "two exchanges, one moved"; else bad "counts: $out"; fi
if chk x "e['lobby_messages']==4 and e['dm_messages']==4"; then ok "lobby and dm message counts"; else bad "message counts: $out"; fi
if chk x "e['outcome']=='ok'"; then ok "the exchange outcome is ok"; else bad "outcome: $out"; fi
# reply latency: gaps between consecutive messages by different members in a dm room (10, 10, 10 s)
if chk x "e['reply_latency_s']==10"; then ok "mean reply latency inside the dm room"; else bad "latency: $out"; fi
if chk x "e['lobby_reply_latency_s']==1"; then ok "mean reply latency in the lobby"; else bad "lobby latency: $out"; fi

# an ask-N working room counts as moved too
sed -e 's/^\[dm.alpha.helper\]/[ask-2]/' "$work/canned.txt" > "$work/askroom.txt"
out="$(python3 "$metric" --history "$work/askroom.txt" 2>&1)"
if chk x "e['adoption']==0.5"; then ok "an ask-N room counts as moved"; else bad "ask room: $out"; fi
# a dm below the minimum size does not count; --min-dm raises the bar
out="$(python3 "$metric" --history "$work/canned.txt" --min-dm 5 2>&1)"
if chk x "e['adoption']==0.0 and e['moved']==0"; then ok "a dm under --min-dm messages is not a move"; else bad "min-dm: $out"; fi
# a dm with one member only (the other never posted, name does not list it) is not a two-member exchange
cat > "$work/lonely.txt" <<'EOF'
[lobby]
[1 1000 alpha] hello
[2 1001 helper] hi
[3 1002 alpha] more
[dm.alpha.helper]
[1 1010 helper] q1
[2 1020 helper] q2
[3 1030 helper] q3
EOF
out="$(python3 "$metric" --history "$work/lonely.txt" 2>&1)"
if chk x "e['adoption']==1.0"; then ok "a dm named for both members counts when both are members"; else bad "named dm: $out"; fi
# no exchange of three messages at all: not a verdict, a failed run
cat > "$work/quiet.txt" <<'EOF'
[lobby]
[1 1000 alpha] hello
[2 1001 helper] hi
EOF
out="$(python3 "$metric" --history "$work/quiet.txt" 2>&1)"
if chk x "e['outcome']=='no-exchange' and e['adoption'] is None"; then ok "no exchange of 3 messages: outcome no-exchange"; else bad "quiet: $out"; fi
# a stray unverified label and the paging line do not break parsing
cat > "$work/noisy.txt" <<'EOF'
[lobby]
[1 1000 alpha (unverified)] a
[2 1001 helper] b
[3 1002 alpha] c
[7 more messages not shown; run `klatch history lobby --since 3`]
EOF
out="$(python3 "$metric" --history "$work/noisy.txt" 2>&1)"
if chk x "e['exchanges']==1 and e['lobby_messages']==3"; then ok "labels and paging lines are tolerated"; else bad "noisy: $out"; fi
# agent result files add turns, cost and duration per agent
printf '{"num_turns":9,"total_cost_usd":0.0065,"duration_ms":42000,"is_error":false,"usage":{"input_tokens":100,"output_tokens":10}}\n' > "$work/alpha.json"
printf '{"num_turns":16,"total_cost_usd":0.0086,"duration_ms":50000,"is_error":true}\n' > "$work/helper.json"
out="$(python3 "$metric" --history "$work/canned.txt" --result alpha="$work/alpha.json" --result helper="$work/helper.json" 2>&1)"
if python3 -c "
import json,sys
d=json.loads(sys.argv[1])['agents']
a,h=d['alpha'],d['helper']
sys.exit(0 if a['turns']==9 and abs(a['cost_usd']-0.0065)<1e-9 and a['duration_s']==42 and a['tokens']==110 and a['outcome']=='ok' and h['outcome']=='error' else 1)" "$out"; then
  ok "turns, cost, duration and outcome per agent"; else bad "agents: $out"; fi
# a missing result file is a failed agent, not a crash
out="$(python3 "$metric" --history "$work/canned.txt" --result alpha="$work/nope.json" 2>&1)"
if python3 -c "import json,sys; sys.exit(0 if json.loads(sys.argv[1])['agents']['alpha']['outcome']=='no-result' else 1)" "$out"; then ok "a missing agent result is outcome no-result"; else bad "missing result: $out"; fi
if python3 "$metric" > "$work/o" 2>&1; then bad "no input exited 0"; else ok "no input is rejected"; fi

# ---- a fake klatch and a fake claude drive the whole scenario ----
cat > "$work/klatch" <<'FAKE'
#!/usr/bin/env bash
# fake klatch: rooms are files under $KLATCH_HOME/rooms, one line "seq ts sender body" per message
store="$KLATCH_HOME/rooms"; mkdir -p "$store"
post() { # room sender text
  n=$(( $(wc -l < "$store/$1" 2>/dev/null || echo 0) + 1 ))
  echo "$n $((1000 + n * 10)) $2 $3" >> "$store/$1"
}
rows() { while read -r seq ts who body; do echo "[$seq $ts $who] $body"; done < "$store/$1"; }
case "$1" in
  init|join) exit 0 ;;
  post) post "$2" "$KLATCH_NAME" "$3" ;;
  dm) peer="$2"; room="dm.$(printf '%s\n%s\n' "$KLATCH_NAME" "$peer" | sort | paste -sd. -)"; post "$room" "$KLATCH_NAME" "$3"
 ;;
  history)
    shift
    if [ -z "${1:-}" ] || [ "${1#--}" != "$1" ]; then
      for f in "$store"/*; do [ -e "$f" ] || continue; echo "[$(basename "$f")]"; rows "$(basename "$f")"; done
    else
      [ -e "$store/$1" ] && rows "$1"
    fi ;;
  *) echo "fake klatch: unknown $1" >&2; exit 2 ;;
esac
FAKE
cat > "$work/claude" <<'FAKE'
#!/usr/bin/env bash
# fake claude -p: alpha and helper play a 2-question exchange, in the lobby (FAKE_MODE=stay) or in a dm (move)
k="$KLATCH_BIN"
echo "$KLATCH_NAME $*" >> "$FAKE_LOG"
case "$KLATCH_NAME:$FAKE_MODE" in
  alpha:stay) "$k" post lobby "seeking: slug"; "$k" post lobby "info #1: underscore"; "$k" post lobby "info #1: 40" ;;
  helper:stay) "$k" post lobby "need-info #1: separator"; "$k" post lobby "need-info #1: length"; "$k" post lobby "answer to #1: ok" ;;
  alpha:move) "$k" post lobby "seeking: slug"; "$k" dm helper "a1"; "$k" dm helper "a2" ;;
  helper:move) "$k" post lobby "moved: dm with alpha"; "$k" dm alpha "q1"; "$k" dm alpha "q2" ;;
esac
printf '{"num_turns":3,"total_cost_usd":0.01,"duration_ms":2000,"is_error":false,"usage":{"input_tokens":10,"output_tokens":5}}\n'
FAKE
chmod +x "$work/klatch" "$work/claude"
export FAKE_LOG="$work/claude.log"
scen() { # mode [extra args]
  mode=$1; shift
  FAKE_MODE=$mode KLATCH_BIN="$work/klatch" CLAUDE_BIN="$work/claude" bash "$run" "$@"
}
kh="$work/home1"; kk="$work/keys1"; mkdir -p "$kh" "$kk"
out="$(KLATCH_HOME="$kh" KLATCH_KEY_DIR="$kk" scen stay 2>"$work/err")"; code=$?
if [ "$code" -eq 0 ]; then ok "the scenario exits 0 with fake agents"; else bad "scenario exit $code: $out $(cat "$work/err")"; fi
last="$(printf '%s\n' "$out" | tail -1)"
if python3 -c "import json,sys; e=json.loads(sys.argv[1])['agents']['exchange']; sys.exit(0 if e['adoption']==0.0 and e['exchanges']==1 else 1)" "$last"; then ok "an exchange kept in the lobby has adoption 0"; else bad "stay result: $last"; fi
if python3 -c "import json,sys; d=json.loads(sys.argv[1])['agents']; sys.exit(0 if d['alpha']['turns']==3 and d['helper']['cost_usd']==0.01 else 1)" "$last"; then ok "the result carries each agent's turns and cost"; else bad "agents: $last"; fi
out="$(KLATCH_HOME="$work/home2" KLATCH_KEY_DIR="$work/keys2" scen move 2>/dev/null | tail -1)"
if python3 -c "import json,sys; e=json.loads(sys.argv[1])['agents']['exchange']; sys.exit(0 if e['adoption']==1.0 and e['dm_messages']==4 and e['lobby_messages']==2 else 1)" "$out"; then ok "an exchange that moved to a dm has adoption 1"; else bad "move result: $out"; fi
if grep -q -- '--model haiku' "$FAKE_LOG" && grep -q -- '--max-budget-usd' "$FAKE_LOG"; then ok "default model haiku and a per-session budget cap"; else bad "claude args: $(head -2 "$FAKE_LOG")"; fi

# arms: the arm is recorded; the prompts differ only where the arm says so
: > "$FAKE_LOG"
out="$(KLATCH_HOME="$work/home3" KLATCH_KEY_DIR="$work/keys3" scen stay --arm private 2>/dev/null | tail -1)"
if python3 -c "import json,sys; sys.exit(0 if json.loads(sys.argv[1])['arm']=='private' else 1)" "$out"; then ok "the arm is recorded in the result"; else bad "arm: $out"; fi
a_base="$(KLATCH_HOME="$work/home4" KLATCH_KEY_DIR="$work/keys4" scen stay --arm baseline >/dev/null 2>&1; grep '^alpha ' "$FAKE_LOG" | tail -1)"
: > "$FAKE_LOG"
a_priv="$(KLATCH_HOME="$work/home5" KLATCH_KEY_DIR="$work/keys5" scen stay --arm private >/dev/null 2>&1; grep '^alpha ' "$FAKE_LOG" | tail -1)"
if [ -n "$a_base" ] && [ "$a_base" != "$a_priv" ]; then ok "the private arm changes the asker prompt"; else bad "arm prompts equal"; fi
case "$a_base" in *"klatch dm"*|*" dm "*) bad "the baseline prompt names dm";; *) ok "the baseline prompt never names dm";; esac
if scen stay --arm bogus > "$work/o" 2>&1; then bad "an unknown arm exited 0"; else ok "an unknown arm is rejected"; fi

# isolation: refuses the live store, and makes private dirs when the harness gave none
if KLATCH_HOME="$HOME/.klatch" KLATCH_KEY_DIR="$work/k" scen stay > "$work/o" 2>&1; then bad "the live ~/.klatch was accepted"; else ok "refuses KLATCH_HOME=\$HOME/.klatch"; fi
# shellcheck disable=SC2016
out="$(env -u KLATCH_HOME -u KLATCH_KEY_DIR bash -c 'FAKE_MODE=stay KLATCH_BIN="$1" CLAUDE_BIN="$2" bash "$3"' _ "$work/klatch" "$work/claude" "$run" 2>/dev/null | tail -1)"
if python3 -c "import json,sys; json.loads(sys.argv[1])" "$out" 2>/dev/null; then ok "without KLATCH_HOME the scenario makes its own private store"; else bad "self isolation: $out"; fi
if [ ! -e "$HOME/.klatch/rooms" ] || ! grep -q 'seeking: slug' "$HOME/.klatch/rooms/lobby" 2>/dev/null; then ok "nothing was written to the live store"; else bad "live store touched"; fi

# ---- the aggregate: repeat.py with N runs reports the adoption share and its spread ----
cat > "$work/alt.sh" <<'FAKE'
#!/usr/bin/env bash
# alternate stay, move, stay, move by run counter, through the real scenario script
n=$(( $(cat "$ALT_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$ALT_COUNT"
if [ $((n % 2)) = 1 ]; then export FAKE_MODE=stay; else export FAKE_MODE=move; fi
exec bash "$RUN_SH"
FAKE
chmod +x "$work/alt.sh"
export ALT_COUNT="$work/altcount" RUN_SH="$run" KLATCH_BIN="$work/klatch" CLAUDE_BIN="$work/claude"
out="$(python3 "$repeat" --runs 4 --timeout 60 --json "$work/agg.json" -- "$work/alt.sh" 2>&1)"; code=$?
if [ "$code" -eq 0 ]; then ok "repeat.py runs the scenario N times"; else bad "repeat exit $code: $out"; fi
case "$out" in *"agent exchange"*"adoption"*"mean 0.50"*"sd 0.58"*) ok "the aggregate reports adoption share 0.50 with sd 0.58";; *) bad "aggregate: $out";; esac
case "$out" in *"failures: 0 of 4"*) ok "no failed runs";; *) bad "failures: $out";; esac
if python3 -c "import json; d=json.load(open('$work/agg.json')); a=d['agents']['exchange']['adoption']; assert a['values']==[0.0,1.0,0.0,1.0]"; then ok "--json keeps the per-run adoption values"; else bad "agg.json"; fi
# a run with no exchange is a failure in the aggregate, not a silent zero
cat > "$work/none.sh" <<'FAKE'
#!/usr/bin/env bash
exec bash "$RUN_SH"
FAKE
chmod +x "$work/none.sh"
out="$(FAKE_MODE=none python3 "$repeat" --runs 2 -- "$work/none.sh" 2>&1)"
case "$out" in *"failures: 2 of 2"*) ok "runs with no exchange count as failures";; *) bad "none: $out";; esac

if [ "$fails" -eq 0 ]; then echo "all passed"; exit 0; else echo "$fails failed"; exit 1; fi
