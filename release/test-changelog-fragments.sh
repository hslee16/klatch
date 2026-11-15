#!/usr/bin/env bash
# release/changelog-fragments.sh must accept good fragments, reject bad ones, and group them by category.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-fragments.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
fresh() { rm -rf "$work/t"; mkdir -p "$work/t/changelog.d"; echo '# readme' > "$work/t/changelog.d/README.md"; }
write() { printf '%b' "$2" > "$work/t/changelog.d/$1"; }
expect_ok() { if "$here/changelog-fragments.sh" check "$work/t" > /dev/null 2>&1; then echo "  ok  accepts: $1"; else echo "FAIL  rejected: $1" >&2; fails=1; fi; }
expect_fail() { if "$here/changelog-fragments.sh" check "$work/t" > /dev/null 2>&1; then echo "FAIL  did not notice: $1" >&2; fails=1; else echo "  ok  notices: $1"; fi; }

fresh; expect_ok "no fragments"
fresh; write 495-a.added.md '- One thing.\n'; write 500-b.fixed.md '- Another.\n- And more.\n'; expect_ok "two good fragments"
fresh; write bad.md '- x\n'; expect_fail "a name with no category"
fresh; write bad.invented.md '- x\n'; expect_fail "an unknown category"
fresh; write bad.added.md ''; expect_fail "an empty file"
fresh; write bad.added.md 'no bullet\n'; expect_fail "a line without a bullet"
fresh; write bad.added.md "- a \xe2\x80\x94 b\n"; expect_fail "an em dash"
fresh; write 1-a.added.md '- First.\n'; write 2-b.fixed.md '- Second.\n'; write 3-c.added.md '- Third.\n'
out="$("$here/changelog-fragments.sh" collect "$work/t")"
if [ "$(grep -c '^### ' <<< "$out")" = 2 ] && grep -q '^### Added' <<< "$out" && grep -q '^### Fixed' <<< "$out" \
  && [ "$(grep -n 'First\|Third\|Second' <<< "$out" | cut -d: -f1 | tr '\n' ' ')" = "3 4 8 " ]; then
  echo "  ok  collect groups by category"
else echo "FAIL  collect output wrong:" >&2; echo "$out" >&2; fails=1; fi
exit "$fails"
