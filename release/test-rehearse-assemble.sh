#!/usr/bin/env bash
# release/rehearse-assemble.sh must assemble on a throwaway copy, print the stat and the renumbering candidates, fail on a bad
# fragment, and never touch the tree it was given (#1239).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-rehearse.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
t="$work/t"
mkdir -p "$work/tmp"
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
vfiles="proto/Cargo.toml clients/python/pyproject.toml clients/python/package/wheel_init.py clients/typescript/package.json CHANGELOG.md README.md"

fresh() {
  rm -rf "$t"; mkdir -p "$t/release" "$t/changelog.d" "$t/design/items" "$t/research/notes"
  cp "$here"/*-fragments.sh "$here/check-versions.sh" "$t/release/"
  for f in $vfiles; do mkdir -p "$t/$(dirname "$f")"; cp "$ROOT/$f" "$t/$f"; done
  echo '# readme' > "$t/changelog.d/README.md"; echo '# readme' > "$t/design/items/README.md"; echo '# readme' > "$t/research/notes/README.md"
  printf '# Design\n\n## 32. Decisions\n\n104. **Older.** Last on main.\n' > "$t/DESIGN.md"
  printf '# Research notes\n\n## 53. Last on main\n\nLast body.\n' > "$t/RESEARCH-NOTES.md"
  printf -- '- One thing.\n' > "$t/changelog.d/9-a.added.md"
  printf 'A thing (#9).\nBody that mentions the design item for #12 and the research note for #13.\n' > "$t/design/items/9-a-thing.md"
  printf 'A study (#9)\nIntro.\n\n### Built\n\nDetail, see the design item for #9.\n' > "$t/research/notes/9-a-study.md"
}
snapshot() { (cd "$t" && find . -type f -exec cksum {} + | sort -k3); }
run() { TMPDIR="$work/tmp" "$here/rehearse-assemble.sh" "$t" > "$work/out" 2> "$work/err"; }

fresh; before="$(snapshot)"
if run; then ok "exits 0 on a good tree"; else bad "good tree failed: $(cat "$work/err")"; fi
if [ "$(snapshot)" = "$before" ]; then ok "the real tree is byte for byte unchanged"; else bad "the tree changed"; fi
if [ -z "$(ls -A "$work/tmp")" ]; then ok "the copy is removed"; else bad "left behind: $(ls "$work/tmp")"; fi
if grep -q 'DESIGN.md' "$work/out" && grep -q 'RESEARCH-NOTES.md' "$work/out"; then ok "prints the diff stat"; else bad "no stat: $(cat "$work/out")"; fi
if grep -q 'changelog entries: 1' "$work/out" && grep -q 'versions agree' "$work/out"; then ok "reports the changelog count and the version check"; else bad "summary: $(cat "$work/out")"; fi
if grep -q 'design item for #NNN references: 2' "$work/out" && grep -q 'research note for #NNN references: 1' "$work/out"; then ok "counts the renumbering candidates"; else bad "candidates: $(cat "$work/out")"; fi
if grep -q '#12' "$work/out" && grep -q '#13' "$work/out"; then ok "names the tickets to renumber"; else bad "tickets not named"; fi

fresh; printf "no bullet\n" > "$t/changelog.d/9-a.added.md"; before="$(snapshot)"
if run; then bad "a bad changelog fragment passed"; else ok "a bad changelog fragment exits non-zero"; fi
if [ "$(snapshot)" = "$before" ] && [ -z "$(ls -A "$work/tmp")" ]; then ok "a failure leaves the tree alone and removes the copy"; else bad "failure left traces"; fi

fresh; printf 'Title only.\n' > "$t/design/items/9-a-thing.md"
if run; then bad "a bodiless design fragment passed"; else ok "a bad design fragment exits non-zero"; fi

fresh; sed -i.bak 's/^version = "\(.*\)"$/version = "9.9.9"/' "$t/proto/Cargo.toml"; rm -f "$t/proto/Cargo.toml.bak"
if run; then bad "a version mismatch passed"; else ok "a version mismatch exits non-zero"; fi

if TMPDIR="$work/tmp" "$here/rehearse-assemble.sh" "$work/nonexistent" > /dev/null 2>&1; then bad "a missing root passed"; else ok "a missing root exits non-zero"; fi
exit "$fails"
