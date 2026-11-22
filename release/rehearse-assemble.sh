#!/usr/bin/env bash
# Rehearse the release assembly (#1239) on a throwaway copy of the tree, so the real tree is never touched.
#
#   release/rehearse-assemble.sh [root]      (root defaults to the repository)
#
# In the copy it runs release/changelog-fragments.sh collect, release/design-fragments.sh assemble,
# release/research-fragments.sh assemble and release/check-versions.sh, then prints the diff stat, the count of changelog
# entries, and the "design item for #NNN" and "research note for #NNN" references that still need a number. The copy is
# removed on exit, also on failure. Exit status is non-zero when any step failed (the stat is still printed).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
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

changelog_out="$copy/.rehearse-changelog.txt"
# shellcheck disable=SC2016
step "changelog-fragments.sh collect" bash -c '"$1/release/changelog-fragments.sh" collect "$1" > "$2"' _ "$copy" "$changelog_out"
step "design-fragments.sh assemble" "$copy/release/design-fragments.sh" assemble "$copy"
step "research-fragments.sh assemble" "$copy/release/research-fragments.sh" assemble "$copy"
step "check-versions.sh" "$copy/release/check-versions.sh" "$copy"

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
