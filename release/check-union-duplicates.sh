#!/usr/bin/env bash
# Duplicate entries in files that are merged with `merge=union` (#1064, #1065). A union merge keeps both sides' lines,
# so two branches that each add the same entry leave it twice, with no conflict to warn anyone.
#   release/check-union-duplicates.sh [root]    default root: the repository this script is in
# Every tracked file whose `merge` attribute is `union` (git check-attr, so globs in .gitattributes count) is read and
# an entry that appears twice is reported with the file, the line number and the line. The rule is per file type and
# conservative, only lines that are list entries count:
#   *.rs   one-line registry entries: `mod x;`, `pub mod x;`, `pub(crate) mod x;`, `use ...;` / `pub use ...;` with no
#          braces, and a bare call entry such as `x::all(),`. Anything else (code, comments, multi-line `{ }`) is skipped.
#   *.md   list items (`- `, `* `, `1. `) and table rows that are not a separator row. Headings, fences, rules and prose
#          are skipped.
#   other  every line that has a letter or digit and does not start with `#` or `//` (comments).
# Blank lines never count. A known legitimate duplicate goes in release/union-duplicates.allow (override with
# UNION_DUP_ALLOW), one line each: `<path> @@ <exact line, trimmed> @@ <reason>`; `#` lines are comments and an entry
# without a reason is an error. Fix a hit by deleting one copy. No file with `merge=union` and no .gitattributes pass.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "${1:-$here/..}" && pwd)"
allow="${UNION_DUP_ALLOW:-$root/release/union-duplicates.allow}"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/klatch-uniondup.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

git -C "$root" ls-files -z > "$tmp/files"
git -C "$root" check-attr -z --stdin merge < "$tmp/files" > "$tmp/attrs"
# output is NUL separated path, attribute, value triples
unions="$(tr '\0' '\n' < "$tmp/attrs" | awk 'NR % 3 == 1 { p = $0 } NR % 3 == 0 && $0 == "union" { print p }')"

if [ -f "$allow" ]; then
  bad="$(awk -F ' @@ ' '/^[[:space:]]*(#|$)/ { next } NF < 3 || $1 == "" || $2 == "" || $3 == "" { printf "%s:%d: %s\n", FILENAME, NR, $0 }' "$allow")"
  if [ -n "$bad" ]; then
    # shellcheck disable=SC2016
    printf 'union-duplicates: bad allowlist entries (need `<path> @@ <line> @@ <reason>`):\n%s\n' "$bad" >&2
    exit 1
  fi
fi

fail=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  [ -f "$root/$f" ] || continue
  out="$(awk -v file="$f" -v allow="$allow" '
    BEGIN {
      kind = (file ~ /\.rs$/) ? "rs" : (file ~ /\.md$/) ? "md" : "other"
      while (allow != "" && (getline a < allow) > 0) {
        if (a ~ /^[[:space:]]*(#|$)/) continue
        n = split(a, p, " @@ ")
        if (n >= 3 && p[1] == file) ok[p[2]] = 1
      }
    }
    function counts(s) {
      if (s == "") return 0
      if (kind == "rs") {
        if (s ~ /^(pub(\([a-z]+\))? )?(mod|use) [^{}]*;$/) return 1
        if (s ~ /^[A-Za-z_][A-Za-z0-9_:]*\([^()]*\),$/) return 1
        return 0
      }
      if (kind == "md") {
        if (s ~ /^\|[ :|-]*$/) return 0
        return (s ~ /^([-*] |[0-9]+\. |\|)/)
      }
      if (s ~ /^(#|\/\/)/) return 0
      return (s ~ /[A-Za-z0-9]/)
    }
    {
      line = $0; sub(/\r$/, "", line); gsub(/^[ \t]+|[ \t]+$/, "", line)
      if (!counts(line) || (line in ok)) next
      if (line in first) printf "%s:%d: %s (first at line %d)\n", file, NR, line, first[line]
      else first[line] = NR
    }' "$root/$f")"
  if [ -n "$out" ]; then printf '%s\n' "$out" >&2; fail=1; fi
done <<< "$unions"

if [ "$fail" -ne 0 ]; then
  echo "union-duplicates: a file merged with merge=union has a line twice (a union merge keeps both sides' lines)." >&2
  echo "Fix: delete one copy. A legitimate repeat goes in release/union-duplicates.allow with a reason." >&2
  exit 1
fi
echo "union-duplicates: ok"
