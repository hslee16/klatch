#!/usr/bin/env bash
# release/renumber-refs.sh must rewrite "design item for #T" and "research note for #T" to the assigned numbers from the
# assemble summaries, fail loudly (changing nothing) when a ticket has no item, and leave the excluded paths alone (#1244).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-renumber.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
t="$work/t"
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }

fresh() {
  rm -rf "$t"; mkdir -p "$t/docs" "$t/proto/src" "$t/release" "$t/proto/target"
  printf 'See the design item for #12 and the research note for #13.\nAgain the design item for #12.\n' > "$t/docs/a.md"
  printf '// per the design item for #14 (see also the research note for #13)\n' > "$t/proto/src/x.rs"
  printf 'the design item for #12 stays\n' > "$t/release/fixture.md"
  printf 'the design item for #12 stays\n' > "$t/proto/target/gen.md"
  printf '{"text": "the design item for #12 is data"}\n' > "$t/data.jsonl"
  printf 'Placeholder text "the design item for #NNN" is not a reference.\n' > "$t/docs/b.md"
  printf 'item 105 <- #12 (12-a.md)\nitem 106 <- #14 (14-b.md)\nsection 54 <- #13 (13-c.md)\nnoise line\n' > "$work/map"
}
snapshot() { (cd "$t" && find . -type f -exec cksum {} + | sort -k3); }

fresh
if "$here/renumber-refs.sh" "$work/map" "$t" > "$work/out" 2> "$work/err"; then ok "exits 0 when every ticket has a number"; else bad "failed: $(cat "$work/err")"; fi
if grep -q 'design item 105' "$t/docs/a.md" && grep -q 'research note section 54' "$t/docs/a.md" && ! grep -q '#12' "$t/docs/a.md"; then ok "rewrites every reference in a markdown file"; else bad "a.md: $(cat "$t/docs/a.md")"; fi
if grep -q 'design item 106' "$t/proto/src/x.rs" && grep -q 'research note section 54' "$t/proto/src/x.rs"; then ok "rewrites references in code comments"; else bad "x.rs: $(cat "$t/proto/src/x.rs")"; fi
if grep -q 'for #12' "$t/release/fixture.md" && grep -q 'for #12' "$t/proto/target/gen.md" && grep -q 'for #12' "$t/data.jsonl"; then ok "leaves release/, target/ and .jsonl alone"; else bad "excluded path changed"; fi
if grep -q 'for #NNN' "$t/docs/b.md"; then ok "leaves the #NNN placeholder text alone"; else bad "NNN changed"; fi
if grep -q 'docs/a.md' "$work/out" && grep -q '#12' "$work/out" && grep -q '105' "$work/out"; then ok "prints what changed"; else bad "no diff: $(cat "$work/out")"; fi
if "$here/renumber-refs.sh" "$work/map" "$t" > /dev/null 2>&1; then ok "a second run is a no-op that exits 0"; else bad "second run failed"; fi

fresh; printf 'item 105 <- #12 (12-a.md)\nsection 54 <- #13 (13-c.md)\n' > "$work/map"; before="$(snapshot)"
if "$here/renumber-refs.sh" "$work/map" "$t" > "$work/out" 2> "$work/err"; then bad "a reference with no item passed"; else ok "a reference with no item exits non-zero"; fi
if grep -q '#14' "$work/err"; then ok "names the ticket with no item"; else bad "err: $(cat "$work/err")"; fi
if [ "$(snapshot)" = "$before" ]; then ok "a failure changes nothing"; else bad "tree changed on failure"; fi

if "$here/renumber-refs.sh" "$work/nonexistent" "$t" > /dev/null 2>&1; then bad "a missing map passed"; else ok "a missing map file exits non-zero"; fi
exit "$fails"
