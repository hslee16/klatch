#!/usr/bin/env bash
# Tests for repeat.py (#243), run against a fake agent (a shell script that prints canned result JSON), so no
# Claude Code install is needed and nothing costs money. Cheap enough for the gate; real runs are manual.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repeat="$here/repeat.py"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-repeat.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

# a fake agent: run number k (counted in a file) reports turns 2,4,6,... and cost 0.10,0.20,... in claude -p form
cat > "$work/fake.sh" <<'FAKE'
#!/usr/bin/env bash
n=$(( $(cat "$FAKE_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$FAKE_COUNT"
printf '{"num_turns":%d,"total_cost_usd":0.%d0,"duration_ms":%d000,"is_error":false,"usage":{"input_tokens":%d,"output_tokens":10,"cache_read_input_tokens":5}}\n' \
  $((n*2)) "$n" "$n" $((n*100))
FAKE
chmod +x "$work/fake.sh"
export FAKE_COUNT="$work/count"

out="$(python3 "$repeat" --runs 3 -- "$work/fake.sh" 2>&1)"; code=$?
if [ "$code" -eq 0 ]; then ok "three runs of a fake agent exit 0"; else bad "exit $code: $out"; fi
if [ "$(cat "$work/count")" = 3 ]; then ok "the scenario ran exactly N times"; else bad "ran $(cat "$work/count") times"; fi
case "$out" in *"turns"*"mean 4.00"*"sd 2.00"*"min 2"*"max 6"*) ok "turns: mean, sample sd, min and max";; *) bad "turns line: $out";; esac
case "$out" in *"cost_usd"*"mean 0.20"*) ok "cost mean";; *) bad "cost line: $out";; esac
case "$out" in *"duration_s"*"mean 2.00"*) ok "duration is reported in seconds";; *) bad "duration line: $out";; esac
case "$out" in *"tokens"*"mean 215.00"*) ok "tokens add input, output and cache reads";; *) bad "tokens line: $out";; esac
case "$out" in *"outcome"*"ok 3/3"*) ok "outcomes are counted";; *) bad "outcome line: $out";; esac

# several agents in one run, a failed run, and a JSON file of the aggregates
cat > "$work/multi.sh" <<'FAKE'
#!/usr/bin/env bash
n=$(( $(cat "$FAKE_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$FAKE_COUNT"
if [ "$n" = 2 ]; then s=error; else s=ok; fi
printf '{"agents":{"alice":{"turns":3,"cost_usd":0.5,"tokens":100,"duration_s":10,"outcome":"%s"},"bob":{"turns":%d,"cost_usd":0.1,"tokens":50,"duration_s":5,"outcome":"ok"}}}\n' "$s" "$n"
FAKE
chmod +x "$work/multi.sh"; rm -f "$work/count"
out="$(python3 "$repeat" --runs 3 --json "$work/r.json" -- "$work/multi.sh" 2>&1)"; code=$?
if [ "$code" -eq 0 ]; then ok "a multi-agent scenario exits 0"; else bad "multi exit $code: $out"; fi
case "$out" in *"alice"*"outcome"*"ok 2/3"*"error 1"*) ok "alice: one failed run is counted, not hidden";; *) bad "alice outcome: $out";; esac
case "$out" in *"bob"*"turns"*"mean 2.00"*) ok "each agent gets its own statistics";; *) bad "bob: $out";; esac
if python3 -c "import json; d=json.load(open('$work/r.json')); assert d['runs']==3 and 'alice' in d['agents'] and len(d['agents']['bob']['turns']['values'])==3"; then
  ok "--json writes the raw values per run"; else bad "json file"; fi

# a single run has no spread: say so instead of printing a fake zero
rm -f "$work/count"
out="$(python3 "$repeat" --runs 1 -- "$work/fake.sh" 2>&1)"
case "$out" in *"sd n/a"*) ok "one run reports sd n/a";; *) bad "single run: $out";; esac

# a scenario that prints no JSON fails the run loudly
printf '#!/usr/bin/env bash\necho not json\n' > "$work/junk.sh"; chmod +x "$work/junk.sh"
if python3 "$repeat" --runs 2 -- "$work/junk.sh" > "$work/o" 2>&1; then bad "junk output exited 0"; else ok "unparseable scenario output exits non-zero"; fi
if python3 "$repeat" --runs 0 -- "$work/fake.sh" > "$work/o" 2>&1; then bad "--runs 0 exited 0"; else ok "--runs 0 is rejected"; fi
if python3 "$repeat" --runs 2 > "$work/o" 2>&1; then bad "no command exited 0"; else ok "a missing command is rejected"; fi

# failure count: runs whose verdict is not ok
rm -f "$work/count"
out="$(python3 "$repeat" --runs 3 -- "$work/multi.sh" 2>&1)"
case "$out" in *"failures: 1 of 3"*) ok "failure count line";; *) bad "failures line: $out";; esac
rm -f "$work/count"
out="$(python3 "$repeat" --runs 3 -- "$work/fake.sh" 2>&1)"
case "$out" in *"failures: 0 of 3"*) ok "zero failures reported";; *) bad "zero failures line: $out";; esac

# isolated store: every run gets its own empty KLATCH_HOME and KLATCH_KEY_DIR under a temp dir, removed afterwards
cat > "$work/env.sh" <<'FAKE'
#!/usr/bin/env bash
echo "$KLATCH_HOME" >> "$ENV_LOG"; echo "$KLATCH_KEY_DIR" >> "$ENV_LOG"
[ -z "$(ls -A "$KLATCH_HOME")" ] || echo dirty >> "$ENV_LOG"
touch "$KLATCH_HOME/state"
echo '{"num_turns":1,"total_cost_usd":0.01}'
FAKE
chmod +x "$work/env.sh"; export ENV_LOG="$work/envlog"; : > "$ENV_LOG"
KLATCH_HOME=/should/not/leak python3 "$repeat" --runs 2 -- "$work/env.sh" > "$work/o" 2>&1
if [ "$(sort -u "$ENV_LOG" | wc -l | tr -d ' ')" = 4 ] && ! grep -q -e dirty -e /should/not/leak "$ENV_LOG"; then ok "each run has fresh private store dirs"; else bad "env log: $(cat "$ENV_LOG")"; fi
first="$(head -1 "$ENV_LOG")"
if [ ! -e "$first" ]; then ok "temp stores are removed afterwards"; else bad "store left behind: $first"; fi

# spend cap: stop once total cost reaches the cap (0.10, 0.20, 0.30 ... per run), report why, exit 3
rm -f "$work/count"
out="$(python3 "$repeat" --runs 10 --max-cost 0.35 -- "$work/fake.sh" 2>&1)"; code=$?
if [ "$code" -eq 3 ]; then ok "spend cap stop exits 3"; else bad "cap exit $code: $out"; fi
if [ "$(cat "$work/count")" = 3 ]; then ok "loop stopped once cumulative cost passed the cap"; else bad "ran $(cat "$work/count") times"; fi
case "$out" in *"spend cap"*"0.35"*) ok "report says the cap stopped it";; *) bad "cap message: $out";; esac
case "$out" in *"runs: 3"*) ok "report covers the runs that happened";; *) bad "runs line: $out";; esac

# a run that reports no cost cannot be capped: fail loudly rather than run blind
cat > "$work/nocost.sh" <<'FAKE'
#!/usr/bin/env bash
echo '{"num_turns":1}'
FAKE
chmod +x "$work/nocost.sh"
if python3 "$repeat" --runs 2 --max-cost 1 -- "$work/nocost.sh" > "$work/o" 2>&1; then bad "no-cost run under a cap exited 0"; else ok "a run with no cost under --max-cost fails loudly"; fi

# per-run timeout: the hung scenario and its children are killed, counted as a failure, the loop goes on
cat > "$work/hang.sh" <<'FAKE'
#!/usr/bin/env bash
n=$(( $(cat "$FAKE_COUNT" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$FAKE_COUNT"
if [ "$n" = 1 ]; then
  sleep 300 &
  echo $! > "$HANG_PID"
  wait
fi
echo '{"num_turns":2,"total_cost_usd":0.01}'
FAKE
chmod +x "$work/hang.sh"; rm -f "$work/count"; export HANG_PID="$work/hangpid"
out="$(python3 "$repeat" --runs 2 --timeout 3 -- "$work/hang.sh" 2>&1)"; code=$?
case "$out" in *"failures: 1 of 2"*) ok "timeout counted as a failure, loop continued";; *) bad "timeout report: $out";; esac
case "$out" in *"timeout 1"*) ok "outcome says timeout";; *) bad "timeout outcome: $out";; esac
if [ "$code" -eq 1 ]; then ok "a timed-out run makes the exit non-zero"; else bad "timeout exit $code"; fi
if [ -f "$HANG_PID" ] && ! kill -0 "$(cat "$HANG_PID")" 2>/dev/null; then ok "the hung child was killed"; else bad "hung child still alive"; kill -9 "$(cat "$HANG_PID")" 2>/dev/null; fi

# a crashing scenario no longer aborts the loop
printf '#!/usr/bin/env bash\nexit 7\n' > "$work/crash.sh"; chmod +x "$work/crash.sh"
out="$(python3 "$repeat" --runs 2 -- "$work/crash.sh" 2>&1)"
case "$out" in *"failures: 2 of 2"*) ok "crashes are counted and the loop continues";; *) bad "crash report: $out";; esac

if [ "$fails" -eq 0 ]; then echo "all passed"; exit 0; else echo "$fails failed"; exit 1; fi
