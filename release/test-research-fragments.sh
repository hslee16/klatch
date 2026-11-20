#!/usr/bin/env bash
# release/research-fragments.sh must accept good fragments, reject bad ones, and assemble them into RESEARCH-NOTES.md (#859).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-rfrag.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
t="$work/t"
fresh() {
  rm -rf "$t"; mkdir -p "$t/research/notes"; echo '# readme' > "$t/research/notes/README.md"
  printf '# Research notes\n\n## 1. First\n\nText.\n\n### 1.1 Sub\n\nSub text.\n\n## 52. Older\n\n### 52.8d Update\n\nx\n\n## 53. Last on main (2026-11-15)\n\nLast body.\n' > "$t/RESEARCH-NOTES.md"
}
write() { printf '%b' "$2" > "$t/research/notes/$1"; }
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
expect_ok() { if "$here/research-fragments.sh" check "$t" > /dev/null 2>&1; then ok "accepts: $1"; else bad "rejected: $1"; fi; }
expect_fail() { if "$here/research-fragments.sh" check "$t" > /dev/null 2>&1; then bad "did not notice: $1"; else ok "notices: $1"; fi; }
good='A study (2026-11-20, ticket #9)\nIntro paragraph.\n\n### What was built\n\nDetail.\n\n### Results\n\nMore.\n'

fresh; expect_ok "no fragments"
fresh; write 9-a-study.md "$good"; write 12-other.md "$good"; expect_ok "two good fragments"
fresh; write a-study.md "$good"; expect_fail "a name with no ticket number"
fresh; write 9.md "$good"; expect_fail "a name with no slug"
fresh; write 9-Bad_Slug.md "$good"; expect_fail "a slug with capitals or underscores"
fresh; write 9-a.md ''; expect_fail "an empty file"
fresh; write 9-a.md 'Only a title\n'; expect_fail "a title with no body"
fresh; write 9-a.md '\nBody only.\n'; expect_fail "a blank first line"
fresh; write 9-a.md "Title \xe2\x80\x94 x\nBody.\n"; expect_fail "an em dash"
fresh; write 9-a.md '## 54. Heading\nBody.\n'; expect_fail "a first line that is a heading"
fresh; write 9-a.md 'Title\nBody.\n\n## Stray section\n\nx\n'; expect_fail "a level 2 heading inside the body"

fresh; write 20-late.md 'Late study (#20)\nLate intro.\n\n### Only part\n\nLate body.\n'; write 9-early.md "$good"
out="$("$here/research-fragments.sh" collect "$t" 2>/dev/null || true)"
if [ "$(grep -c '^## [0-9]*\. ' <<< "$out")" = 2 ] && grep -q '^## 54\. A study (2026-11-20, ticket #9)$' <<< "$out" \
  && grep -q '^## 55\. Late study (#20)$' <<< "$out" && grep -q '^### 54\.1 What was built$' <<< "$out" \
  && grep -q '^### 54\.2 Results$' <<< "$out" && grep -q '^### 55\.1 Only part$' <<< "$out"; then
  ok "collect numbers sections and subsections by ticket order after the highest section"
else bad "collect output wrong: $out"; fi
if [ -f "$t/research/notes/9-early.md" ] && ! grep -q '^## 54\.' "$t/RESEARCH-NOTES.md"; then ok "collect changes nothing"; else bad "collect modified the tree"; fi

"$here/research-fragments.sh" assemble "$t" > /dev/null 2>&1 || bad "assemble failed"
if grep -q '^## 54\. A study' "$t/RESEARCH-NOTES.md" && grep -q '^## 55\. Late study' "$t/RESEARCH-NOTES.md" \
  && grep -q '^## 53\. Last on main' "$t/RESEARCH-NOTES.md" && grep -q '^### 52\.8d Update' "$t/RESEARCH-NOTES.md" && grep -q '^## 1\. First' "$t/RESEARCH-NOTES.md"; then
  ok "assemble appends the numbered sections and keeps the old ones"
else bad "assemble result wrong: $(cat "$t/RESEARCH-NOTES.md")"; fi
if [ ! -e "$t/research/notes/9-early.md" ] && [ ! -e "$t/research/notes/20-late.md" ] && [ -f "$t/research/notes/README.md" ]; then ok "assemble deletes the fragments and keeps the README"; else bad "fragments not deleted"; fi
if [ -n "$(tail -n1 "$t/RESEARCH-NOTES.md")" ]; then ok "RESEARCH-NOTES.md ends in a single newline"; else bad "bad file ending"; fi

fresh; write 9-a.md 'No body\n'; if "$here/research-fragments.sh" assemble "$t" > /dev/null 2>&1; then bad "assemble accepted a bad fragment"; else ok "assemble refuses a bad fragment"; fi
if ! grep -q '^## 54\.' "$t/RESEARCH-NOTES.md"; then ok "a refused assemble leaves RESEARCH-NOTES.md alone"; else bad "assemble wrote anyway"; fi

fresh; if "$here/research-fragments.sh" assemble "$t" > /dev/null 2>&1; then ok "assemble with no fragments is a no-op"; else bad "assemble with no fragments failed"; fi
exit "$fails"
