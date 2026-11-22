#!/usr/bin/env bash
# Compare two Homebrew formulas, the staged copy (release/homebrew/klatch.rb) and the tap's (#1260). They may differ in the
# version line (present or absent), the download URLs and the SHA-256 values, and in nothing else: any other difference is drift, printed as a diff
# on stderr with a non-zero status.
#
#   release/compare-formula.sh [--allow-klatch-server] <staged formula> <tap formula>
#
# --allow-klatch-server (#1287): the one named exception, used before a release (release/bump-formula.sh). The staged copy is the
# release candidate and the tap holds the previous release, which may not install klatch-server yet. With the flag the staged
# copy may be ahead of the tap by exactly two lines: `bin.install "klatch", "klatch-server"` in place of `bin.install "klatch"`,
# and the test line `assert_match "usage: klatch-server", shell_output("#{bin}/klatch-server --help")`. Nothing else is
# allowed, and the tap being ahead of the staged copy is still drift. Without the flag the comparison is strict, which is what
# the post-release check (release/verify-published.sh) uses: after a release the two must match.
#
# It reads two local files and nothing else (callers fetch the tap file with a REST read).
set -euo pipefail
allow=0
if [ "${1:-}" = "--allow-klatch-server" ]; then allow=1; shift; fi
[ $# -eq 2 ] || { echo "usage: release/compare-formula.sh [--allow-klatch-server] <staged formula> <tap formula>" >&2; exit 2; }
for f in "$1" "$2"; do [ -f "$f" ] || { echo "compare-formula: no file $f" >&2; exit 2; }; done

work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-cmpf.XXXXXX")"
trap 'rm -rf "$work"' EXIT
normalize() {
  # a version line is dropped, not masked: the formula has none since #1297 (brew audit --strict rejects it as redundant with
  # the URL), and a line on one side only is not drift when the URLs agree
  sed -E -e '/^[[:space:]]*version "[^"]*"[[:space:]]*$/d' \
    -e 's/^([[:space:]]*)url "[^"]*"[[:space:]]*$/\1url "URL"/' \
    -e 's/^([[:space:]]*)sha256 "[^"]*"[[:space:]]*$/\1sha256 "SHA256"/' "$1"
}
normalize "$1" > "$work/staged"
normalize "$2" > "$work/tap"
if diff -u --label "staged:$1" --label "tap:$2" "$work/staged" "$work/tap" > "$work/diff"; then
  exit 0
fi
if [ "$allow" -eq 1 ]; then
  # undo the two lines of the allowance in the staged copy, each at most once and only as exact lines, then compare again
  awk '
    !i && /^[[:space:]]*bin\.install "klatch", "klatch-server"[[:space:]]*$/ { sub(/, "klatch-server"/, ""); i = 1 }
    !t && /^[[:space:]]*assert_match "usage: klatch-server", shell_output\("#\{bin\}\/klatch-server --help"\)[[:space:]]*$/ { t = 1; next }
    { print }
  ' "$work/staged" > "$work/staged.base"
  if diff -u --label "staged:$1" --label "tap:$2" "$work/staged.base" "$work/tap" > "$work/diff2"; then
    exit 0
  fi
  cp "$work/diff2" "$work/diff"
fi
{
  echo "compare-formula: the staged formula and the tap formula differ beyond the version, URLs and hashes:"
  cat "$work/diff"
} >&2
exit 1
