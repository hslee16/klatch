#!/usr/bin/env bash
# Compare two Homebrew formulas, the staged copy (release/homebrew/klatch.rb) and the tap's (#1260). They may differ in the
# version, the download URLs and the SHA-256 values, and in nothing else: any other difference is drift, printed as a diff
# on stderr with a non-zero status.
#
#   release/compare-formula.sh <staged formula> <tap formula>
#
# It reads two local files and nothing else (callers fetch the tap file with a REST read).
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: release/compare-formula.sh <staged formula> <tap formula>" >&2; exit 2; }
for f in "$1" "$2"; do [ -f "$f" ] || { echo "compare-formula: no file $f" >&2; exit 2; }; done

work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-cmpf.XXXXXX")"
trap 'rm -rf "$work"' EXIT
normalize() {
  sed -E -e 's/^([[:space:]]*)version "[^"]*"[[:space:]]*$/\1version "VERSION"/' \
    -e 's/^([[:space:]]*)url "[^"]*"[[:space:]]*$/\1url "URL"/' \
    -e 's/^([[:space:]]*)sha256 "[^"]*"[[:space:]]*$/\1sha256 "SHA256"/' "$1"
}
normalize "$1" > "$work/staged"
normalize "$2" > "$work/tap"
if diff -u --label "staged:$1" --label "tap:$2" "$work/staged" "$work/tap" > "$work/diff"; then
  exit 0
fi
{
  echo "compare-formula: the staged formula and the tap formula differ beyond the version, URLs and hashes:"
  cat "$work/diff"
} >&2
exit 1
