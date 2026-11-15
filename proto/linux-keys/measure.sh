#!/usr/bin/env bash
# Latency of the key fetch and of a hook, in a test container (#384). Run by in-container.sh with --bench. $1 is the
# variant: with `bus` it measures the Secret Service and the key file side by side; with anything else, the key file.
# Release build, a store of 300 messages in 5 rooms, 150 runs of each command one after another as a hook runs.
set -euo pipefail
variant="$1"
runs=150

cd /src/proto
cargo build -q --release --locked -p klatch
bin="$CARGO_TARGET_DIR/release/klatch"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export HOME="$work/home" KLATCH_HOME="$work/rooms" XDG_STATE_HOME="$work/state" KLATCH_NAME=alice
mkdir -p "$HOME"

# p50 and p99 in milliseconds of `runs` runs of the command (microsecond clock, no extra process per run)
measure() {
  local label="$1"
  shift
  local i s e
  for ((i = 0; i < runs; i++)); do
    s="${EPOCHREALTIME/./}"
    "$@" > /dev/null 2>&1 || true
    e="${EPOCHREALTIME/./}"
    echo $((e - s))
  done | sort -n | awk -v label="$label" '{a[NR]=$1} END {
    printf "%-46s p50 %6.1f ms   p99 %6.1f ms   max %6.1f ms\n", label, a[int(NR*0.5)+1]/1000, a[int(NR*0.99)+1]/1000, a[NR]/1000 }'
}

fill() {
  for r in 1 2 3 4 5; do
    "$bin" join "room$r" > /dev/null
    for ((m = 0; m < 60; m++)); do "$bin" post "room$r" "message $m of room $r, some ordinary text" > /dev/null; done
    "$bin" read "room$r" > /dev/null 2>&1
  done
}

hooked() { "$bin" hook --event PostToolUse; }

echo "--- latency, $variant ($runs runs each; the budget is 50 ms at p99 over the hook's own work)"
if [ "$variant" = bus ]; then
  # the store is made under the Secret Service (no KLATCH_KEY_DIR), so every command fetches its key from the keyring
  unset KLATCH_KEY_DIR
  fill
  measure "secret-tool lookup alone" secret-tool lookup service klatch name probe
  measure "hook, key in the Secret Service" hooked
  measure "who, key in the Secret Service" "$bin" who lobby
  measure "post, key in the Secret Service" "$bin" post room1 hello
fi

# the same store content under a key file, in a store of its own
unset DBUS_SESSION_BUS_ADDRESS
export KLATCH_HOME="$work/rooms-file" XDG_STATE_HOME="$work/state-file"
fill
measure "hook, key in a file" hooked
measure "who, key in a file" "$bin" who lobby
measure "post, key in a file" "$bin" post room1 hello
