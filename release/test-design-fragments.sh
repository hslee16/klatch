#!/usr/bin/env bash
# release/design-fragments.sh must accept good fragments, reject bad ones, and assemble them into DESIGN.md (#807).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-dfrag.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
t="$work/t"
fresh() {
  rm -rf "$t"; mkdir -p "$t/design/items"; echo '# readme' > "$t/design/items/README.md"
  printf '# Design\n\n## 32. Decisions\n\nIntro.\n\n1. **First.** Text one.\n\n2. **Second.** Text two.\n\n103. **Old.** Text.\n\n104. **Older.** Last on main.\n' > "$t/DESIGN.md"
}
write() { printf '%b' "$2" > "$t/design/items/$1"; }
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
expect_ok() { if "$here/design-fragments.sh" check "$t" > /dev/null 2>&1; then ok "accepts: $1"; else bad "rejected: $1"; fi; }
expect_fail() { if "$here/design-fragments.sh" check "$t" > /dev/null 2>&1; then bad "did not notice: $1"; else ok "notices: $1"; fi; }
good='A thing (#9; no schema change).\nBody text of the item.\n'

fresh; expect_ok "no fragments"
fresh; write 9-a-thing.md "$good"; write 12-other.md "$good"; expect_ok "two good fragments"
fresh; write a-thing.md "$good"; expect_fail "a name with no ticket number"
fresh; write 9.md "$good"; expect_fail "a name with no slug"
fresh; write 9-Bad_Slug.md "$good"; expect_fail "a slug with capitals or underscores"
fresh; write 9-a.md ''; expect_fail "an empty file"
fresh; write 9-a.md 'Only a title.\n'; expect_fail "a title with no body"
fresh; write 9-a.md '\nBody only after a blank title line.\n'; expect_fail "a blank first line"
fresh; write 9-a.md "Title \xe2\x80\x94 x.\nBody.\n"; expect_fail "an em dash"
fresh; write 9-a.md '# Heading\nBody.\n'; expect_fail "a first line that is a heading"

fresh; write 20-late.md 'Late item (#20).\nLate body.\n'; write 9-early.md 'Early item (#9).\nEarly body.\n\nSecond paragraph.\n'
out="$("$here/design-fragments.sh" collect "$t" 2>/dev/null || true)"
if [ "$(grep -c '^[0-9]*\. \*\*' <<< "$out")" = 2 ] && grep -q '^105\. \*\*Early item (#9).\*\* Early body\.' <<< "$out" \
  && grep -q '^106\. \*\*Late item (#20).\*\* Late body\.' <<< "$out" && grep -q '^   Second paragraph\.' <<< "$out"; then
  ok "collect numbers by ticket order after the highest item"
else bad "collect output wrong: $out"; fi
if [ -f "$t/design/items/9-early.md" ] && ! grep -q '^105\.' "$t/DESIGN.md"; then ok "collect changes nothing"; else bad "collect modified the tree"; fi

"$here/design-fragments.sh" assemble "$t" > /dev/null 2>&1 || bad "assemble failed"
if grep -q '^105\. \*\*Early item' "$t/DESIGN.md" && grep -q '^106\. \*\*Late item' "$t/DESIGN.md" \
  && grep -q '^104\. \*\*Older' "$t/DESIGN.md" && grep -q '^1\. \*\*First' "$t/DESIGN.md" && grep -q '^2\. \*\*Second' "$t/DESIGN.md"; then
  ok "assemble appends the numbered items and keeps the old ones"
else bad "assemble result wrong: $(cat "$t/DESIGN.md")"; fi
if [ ! -e "$t/design/items/9-early.md" ] && [ ! -e "$t/design/items/20-late.md" ] && [ -f "$t/design/items/README.md" ]; then ok "assemble deletes the fragments and keeps the README"; else bad "fragments not deleted"; fi
if [ -n "$(tail -n1 "$t/DESIGN.md")" ]; then ok "DESIGN.md ends in a single newline"; else bad "bad file ending"; fi

fresh; write 9-a.md 'No body.\n'; if "$here/design-fragments.sh" assemble "$t" > /dev/null 2>&1; then bad "assemble accepted a bad fragment"; else ok "assemble refuses a bad fragment"; fi
if ! grep -q '^105\.' "$t/DESIGN.md"; then ok "a refused assemble leaves DESIGN.md alone"; else bad "assemble wrote anyway"; fi

fresh; if "$here/design-fragments.sh" assemble "$t" > /dev/null 2>&1; then ok "assemble with no fragments is a no-op"; else bad "assemble with no fragments failed"; fi
exit "$fails"
