#!/usr/bin/env bash
# Tests for proto/scripts/fmt-after-merge (#1093). Real temp git repos and a fake `cargo` whose `fmt --all` sorts the
# lines of proto/klatch/src/gateway/mod.rs (what rustfmt does to `mod x;` lines) and whose `check` can be made to fail.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tool="$here/fmt-after-merge"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-fmtmerge-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

shim="$work/shim"; mkdir -p "$shim"
cat > "$shim/cargo" <<'SH'
#!/bin/sh
echo "$*" >> "$FAKE_LOG"
case "$1" in
  fmt) f="$(pwd)/klatch/src/gateway/mod.rs"
       [ -f "$f" ] && sort "$f" > "$f.tmp" && mv "$f.tmp" "$f"; exit 0 ;;
  check) [ -n "$FAKE_CHECK_FAIL" ] && { echo "fake compile error"; exit 1; }; exit 0 ;;
esac
exit 0
SH
chmod +x "$shim/cargo"

# newrepo NAME: proto/Cargo.toml and a sorted registry, committed
newrepo() {
  r="$work/$1"; rm -rf "$r"; mkdir -p "$r/proto/klatch/src/gateway"
  git -C "$r" init -q -b main
  : > "$r/proto/Cargo.toml"
  printf 'mod shared;\nmod ssh;\nmod ssh_key;\n' > "$r/proto/klatch/src/gateway/mod.rs"
  git -C "$r" add -A && git -C "$r" commit -q -m base
}
run() { (cd "$r" && env PATH="$shim:$PATH" FAKE_LOG="$work/log" FAKE_CHECK_FAIL="${CHECK_FAIL:-}" "$tool" > "$work/out" 2>&1); }

# 1. an unsorted registry (what a union merge leaves) is formatted and committed; the tree is clean after
newrepo unsorted; : > "$work/log"
printf 'mod ssh_key;\nmod shared;\nmod ssh;\n' > "$r/proto/klatch/src/gateway/mod.rs"
git -C "$r" commit -q -am merged
if run && [ -z "$(git -C "$r" status --porcelain)" ] && [ "$(git -C "$r" log --oneline | wc -l)" -eq 3 ] \
  && [ "$(head -1 "$r/proto/klatch/src/gateway/mod.rs")" = "mod shared;" ]; then ok "unsorted mod lines are formatted and committed"; else bad "unsorted case"; cat "$work/out"; fi
if git -C "$r" log -1 --format=%B | grep "Co-Authored-By: Claude Sonnet 5.5" > /dev/null; then ok "the commit carries the trailer"; else bad "trailer"; fi

# 2. already formatted: no commit
newrepo sorted; : > "$work/log"
if run && [ "$(git -C "$r" log --oneline | wc -l)" -eq 1 ]; then ok "a formatted tree gets no commit"; else bad "sorted case"; fi

# 3. the compile check runs and its failure is reported (exit 1)
newrepo broken; : > "$work/log"
if CHECK_FAIL=1 run; then bad "a compile failure must fail"; else ok "a compile failure exits non-zero"; fi
if grep -q '^check ' "$work/log"; then ok "cargo check ran"; else bad "cargo check did not run"; fi

# 4. no proto/Cargo.toml: nothing to do, success, cargo untouched
r="$work/none"; rm -rf "$r"; mkdir -p "$r"; git -C "$r" init -q -b main; : > "$work/log"
if run && [ ! -s "$work/log" ]; then ok "a repo with no proto workspace is skipped"; else bad "no-proto case"; fi

if [ "$fails" -eq 0 ]; then echo "test-fmt-after-merge: all passed"; else echo "test-fmt-after-merge: $fails failed"; exit 1; fi
