#!/usr/bin/env bash
# Rehearse the release assembly (#1239) on a throwaway copy of the tree, so the real tree is never touched.
#
#   release/rehearse-assemble.sh [--check-tests] [root]      (root defaults to the repository)
#
# In the copy it runs release/design-fragments.sh assemble, release/research-fragments.sh assemble, release/renumber-refs.sh,
# release/changelog-fragments.sh collect (after the renumbering, so the entries carry the numbers) and release/check-versions.sh, then prints the diff stat, the count of changelog
# entries, and the "design item for #NNN" and "research note for #NNN" references that still need a number (none, after
# release/renumber-refs.sh has run on the copy, #1244; a reference with no item makes it fail). The copy is
# With --check-tests (#1286) it also runs, on the assembled copy, the cheap checks that assembling breaks: the workspace version
# in proto/THIRD-PARTY-LICENSES.txt (the cheap part of proto/check-notices.sh, which needs cargo-about), the
# release_scope_docs and negotiate cargo tests (one forbids phrases in the assembled DESIGN.md, one reads an item by name) and the
# release/test-*.sh shell tests (not this script's own test). REHEARSE_CARGO overrides cargo; REHEARSE_TARGET_DIR the cargo target
# directory (default: a fresh directory per run, removed on exit, so a test binary built for an earlier copy is never reused,
# #1292; the cost is a full build each run). The copy is
# removed on exit, also on failure. Exit status is non-zero when any step failed (the stat is still printed).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
check_tests=0
if [ "${1:-}" = "--check-tests" ]; then check_tests=1; shift; fi
root="${1:-$(cd "$here/.." && pwd)}"
[ -d "$root" ] || { echo "rehearse-assemble: no such directory: $root" >&2; exit 2; }
root="$(cd "$root" && pwd)"
command -v git > /dev/null 2>&1 || { echo "rehearse-assemble: git is needed for the diff stat" >&2; exit 2; }

copy="$(mktemp -d "${TMPDIR:-/tmp}/klatch-rehearse-copy.XXXXXX")"
trap 'rm -rf "$copy"' EXIT
# build output and VCS state are not part of what the assembly touches
(cd "$root" && tar -cf - --exclude=./.git --exclude=./proto/target --exclude=./.claude --exclude='node_modules' .) | (cd "$copy" && tar -xf -)

g() { git -C "$copy" -c user.name=rehearse -c user.email=rehearse@invalid -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"; }
g init -q
g add -A -f
g commit -q -m baseline --no-verify

fail=0
step() { # label command...
  local label="$1"; shift
  echo "== $label"
  if "$@"; then :; else echo "STEP FAILED  $label" >&2; fail=1; fi
}

map="$copy/.rehearse-map.txt"
: > "$map"
# the assemble summaries (on stderr) are the ticket to number map that renumber-refs.sh reads
# shellcheck disable=SC2016
assemble() { # script root map
  local rc=0
  "$1" assemble "$2" 2> "$3.err" || rc=$?
  cat "$3.err" >&2; cat "$3.err" >> "$3"; rm -f "$3.err"
  return "$rc"
}
step "design-fragments.sh assemble" assemble "$copy/release/design-fragments.sh" "$copy" "$map"
step "research-fragments.sh assemble" assemble "$copy/release/research-fragments.sh" "$copy" "$map"
step "renumber-refs.sh" "$copy/release/renumber-refs.sh" "$map" "$copy"
changelog_out="$copy/.rehearse-changelog.txt"
# shellcheck disable=SC2016
step "changelog-fragments.sh collect" bash -c '"$1/release/changelog-fragments.sh" collect "$1" > "$2"' _ "$copy" "$changelog_out"
rm -f "$map"
step "check-versions.sh" "$copy/release/check-versions.sh" "$copy"

notices_version() {
  local v
  v="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$copy/proto/Cargo.toml" | sed -n 1p)"
  [ -n "$v" ] || { echo "no workspace version in proto/Cargo.toml" >&2; return 1; }
  grep -qE "^- klatch $v( |\$)" "$copy/proto/THIRD-PARTY-LICENSES.txt" 2> /dev/null || {
    echo "proto/THIRD-PARTY-LICENSES.txt does not carry klatch $v (run: proto/check-notices.sh --write)" >&2; return 1; }
}
shell_tests() {
  local t rc=0
  for t in "$copy"/release/test-*.sh; do
    [ -e "$t" ] || continue
    [ "$(basename "$t")" != "test-rehearse-assemble.sh" ] || continue
    echo "-- $(basename "$t")"
    (cd "$copy" && bash "$t") || { echo "failed: $(basename "$t")" >&2; rc=1; }
  done
  return "$rc"
}
if [ "$check_tests" -eq 1 ]; then
  cargo_bin="${REHEARSE_CARGO:-cargo}"
  # a fresh target directory per run (#1292): cargo bakes env!("CARGO_MANIFEST_DIR") into test binaries, so a shared target
  # directory would run the binary built for the previous (deleted) copy. REHEARSE_TARGET_DIR overrides it and is kept.
  if [ -n "${REHEARSE_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR="$REHEARSE_TARGET_DIR"
  else
    fresh_target="$(mktemp -d "${TMPDIR:-/tmp}/klatch-rehearse-target.XXXXXX")"
    trap 'rm -rf "$copy" "$fresh_target"' EXIT
    export CARGO_TARGET_DIR="$fresh_target"
  fi
  step "notices carry the workspace version" notices_version
  # shellcheck disable=SC2016
  step "cargo test release_scope_docs" bash -c 'cd "$1/proto" && "$2" test -p klatch --test release_scope_docs' _ "$copy" "$cargo_bin"
  # shellcheck disable=SC2016
  step "cargo test negotiate" bash -c 'cd "$1/proto" && "$2" test -p klatch --lib negotiate' _ "$copy" "$cargo_bin"
  step "release shell tests" shell_tests
fi

entries=0
if [ -f "$changelog_out" ]; then entries="$(grep -c '^- ' "$changelog_out" || true)"; fi
rm -f "$changelog_out"

echo "== diff stat (what the assembly changes)"
g add -A -f
g diff --cached --stat

echo "== summary"
echo "changelog entries: $entries"
refs() { # phrase
  # a grep with no match is not an error here
  { cd "$copy" && grep -rEoh --include='*.md' "$1 #[0-9]+" . 2> /dev/null || true; }
}
for phrase in 'design item for' 'research note for'; do
  list="$(refs "$phrase")"
  count=0; [ -z "$list" ] || count="$(printf '%s\n' "$list" | grep -c .)"
  echo "$phrase #NNN references: $count"
  [ -z "$list" ] || printf '%s\n' "$list" | sed 's/.* \(#[0-9]*\)$/\1/' | sort -u | sed "s/^/  needs a number: $phrase /"
done

[ "$fail" -eq 0 ] || { echo "rehearse-assemble: a step failed; read the messages above" >&2; exit 1; }
echo "rehearse-assemble: ok (the real tree was not touched)"
