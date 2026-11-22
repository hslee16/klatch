#!/usr/bin/env bash
# Tests for proto/scripts/gate and proto/scripts/install-git-hook (#403). The gate is driven with a fake `cargo` that
# records what it was asked to run and fails on demand, so the order of the steps, stopping at the first failure and
# the replay hint are checked without a real build.
set -uo pipefail
here=/Users/hosanglee/klatch-wt-1331/proto/scripts
repo="$(cd "$here/../.." && pwd)"
gate="$here/gate"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-gate-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

# a fake cargo: logs "args | seed | cases" and fails when the arguments contain $FAKE_FAIL. The one exception is the
# plain `build -p klatch --locked --manifest-path ...` that the gate's release and shell checks make so the README
# recipes have a klatch binary (#982): it is passed to the real cargo, because a fake cannot produce the binary and a
# fresh worktree has none (the main checkout only passed because an old build was lying in proto/target)
real_cargo="$(command -v cargo || true)"
shim="$work/shim"; mkdir -p "$shim"
cat > "$shim/cargo" <<'SH'
#!/bin/sh
echo "$* | seed=${HEGEL_SEED:-} cases=${HEGEL_TEST_CASES:-}" >> "$FAKE_LOG"
case "$*" in "build -p klatch --locked --manifest-path "*) [ -n "$REAL_CARGO" ] && exec "$REAL_CARGO" "$@" > /dev/null 2>&1 ;; esac
# the licence notices check (#1034) reads the real dependency tree, so metadata, about and tree go to the real cargo (#1306)
case "$1" in metadata | about | tree) [ -n "$REAL_CARGO" ] && exec "$REAL_CARGO" "$@" ;; esac
case "$*" in *"$FAKE_FAIL"*) [ -n "$FAKE_FAIL" ] && { echo "fake failure in: $*"; exit 1; } ;; esac
exit 0
SH
chmod +x "$shim/cargo"
run_gate() { # fail-pattern, args...
  FAKE_LOG="$work/log"; : > "$FAKE_LOG"
  FAKE_FAIL="$1"; shift
  # --allow-dirty: these tests run in whatever checkout they are in, which may have uncommitted changes (#763)
  env PATH="$shim:$PATH" FAKE_LOG="$FAKE_LOG" FAKE_FAIL="$FAKE_FAIL" REAL_CARGO="$real_cargo" "$gate" --allow-dirty "$@" > "$work/out" 2>&1
}

# the workspace tests use nextest, then the doctests, as CI does (#1331); without nextest the gate says so and uses cargo test
run_gate ""
if grep -q "^nextest run --workspace --locked" "$work/log" && grep -q "^test --workspace --doc --locked" "$work/log" && ! grep -q "^test --workspace --locked" "$work/log"; then ok "with nextest the gate runs nextest and the doctests, as CI does (#1331)"; else bad "nextest step: $(cat "$work/log")"; fi
run_gate "nextest --version"
if grep -q "GATE PASSED" "$work/out" && grep -q "cargo-nextest is not installed" "$work/out" && grep -q "^test --workspace --locked" "$work/log" && ! grep -q "^nextest run" "$work/log"; then ok "without nextest the gate prints a notice and falls back to cargo test (#1331)"; else bad "nextest fallback: $(cat "$work/out")"; fi
if run_gate "nextest run"; then bad "the gate passed although nextest failed (#1331)"; else
  if grep -q "FAILED at step" "$work/out" && grep -q "workspace tests" "$work/out" && ! grep -q "GATE PASSED" "$work/out"; then ok "a failing nextest run fails the gate at the workspace tests step (#1331)"; else bad "nextest failure: $(cat "$work/out")"; fi
fi

echo fails=$fails
