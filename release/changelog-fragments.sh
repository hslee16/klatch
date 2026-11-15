#!/usr/bin/env bash
# Changelog fragments (#502): one file per change in changelog.d/, named <topic>.<category>.md, so parallel pull
# requests do not collide on the same lines of CHANGELOG.md.
#   release/changelog-fragments.sh check [root]     fail on a badly named or badly formed fragment
#   release/changelog-fragments.sh collect [root]   print the entries grouped by category, ready to paste
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mode="${1:-check}"
root="${2:-$(cd "$here/.." && pwd)}"
dir="$root/changelog.d"
categories="added changed deprecated removed fixed security"

fragments() { find "$dir" -maxdepth 1 -type f -name '*.md' ! -name README.md | sort; }

check() {
  local fail=0 f base cat
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    base="$(basename "$f" .md)"
    cat="${base##*.}"
    if [ "$cat" = "$base" ] || ! grep -qw -- "$cat" <<< "$categories"; then
      echo "BAD NAME  $f: expected <topic>.<category>.md with a category of: $categories" >&2; fail=1
    fi
    if [ ! -s "$f" ]; then echo "EMPTY     $f" >&2; fail=1; fi
    if grep -qv '^- ' "$f"; then echo "BAD LINE  $f: every line must start with '- '" >&2; fail=1; fi
    if grep -q $'\xe2\x80\x94' "$f"; then echo "EM DASH   $f" >&2; fail=1; fi
  done < <(fragments)
  return "$fail"
}

collect() {
  local cat f files
  for cat in $categories; do
    files="$(fragments | grep "\\.$cat\\.md\$" || true)"
    [ -n "$files" ] || continue
    printf '### %s\n\n' "$(tr '[:lower:]' '[:upper:]' <<< "${cat:0:1}")${cat:1}"
    while IFS= read -r f; do cat "$f"; done <<< "$files"
    echo
  done
}

case "$mode" in
  check) check ;;
  collect) check && collect ;;
  *) echo "usage: $0 check|collect [root]" >&2; exit 2 ;;
esac
