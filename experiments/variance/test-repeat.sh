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

if [ "$fails" -eq 0 ]; then echo "all passed"; exit 0; else echo "$fails failed"; exit 1; fi
