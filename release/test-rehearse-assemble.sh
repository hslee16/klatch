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
vfiles="release/homebrew/klatch.rb proto/Cargo.toml clients/python/pyproject.toml clients/python/package/wheel_init.py clients/typescript/package.json CHANGELOG.md README.md"

fresh() {
  rm -rf "$t"; mkdir -p "$t/release" "$t/changelog.d" "$t/design/items" "$t/research/notes"
  cp "$here"/*-fragments.sh "$here/renumber-refs.sh" "$here/check-versions.sh" "$here/pep440.sh" "$t/release/"
  for f in $vfiles; do mkdir -p "$t/$(dirname "$f")"; cp "$ROOT/$f" "$t/$f"; done
  echo '# readme' > "$t/changelog.d/README.md"; echo '# readme' > "$t/design/items/README.md"; echo '# readme' > "$t/research/notes/README.md"
  printf '# Design\n\n## 32. Decisions\n\n104. **Older.** Last on main.\n' > "$t/DESIGN.md"
  printf '# Research notes\n\n## 53. Last on main\n\nLast body.\n' > "$t/RESEARCH-NOTES.md"
  printf -- '- One thing.\n' > "$t/changelog.d/9-a.added.md"
  printf 'A thing (#9).\nBody that mentions the design item for #9 and the research note for #9.\n' > "$t/design/items/9-a-thing.md"
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
if grep -q 'design item for #NNN references: 0' "$work/out" && grep -q 'research note for #NNN references: 0' "$work/out"; then ok "leaves no placeholder after renumbering"; else bad "candidates: $(cat "$work/out")"; fi
if grep -q 'design item 105' "$work/out" && grep -q 'research note section 54' "$work/out"; then ok "prints the renumbered references"; else bad "renumbering not shown: $(cat "$work/out")"; fi

fresh; printf 'A thing (#9).\nSee the design item for #12.\n' > "$t/design/items/9-a-thing.md"
if run; then bad "a reference with no item passed"; else ok "a reference with no item exits non-zero"; fi
if grep -q '#12' "$work/err"; then ok "names the ticket with no item"; else bad "err: $(cat "$work/err")"; fi

fresh; printf "no bullet\n" > "$t/changelog.d/9-a.added.md"; before="$(snapshot)"
if run; then bad "a bad changelog fragment passed"; else ok "a bad changelog fragment exits non-zero"; fi
if [ "$(snapshot)" = "$before" ] && [ -z "$(ls -A "$work/tmp")" ]; then ok "a failure leaves the tree alone and removes the copy"; else bad "failure left traces"; fi

fresh; printf 'Title only.\n' > "$t/design/items/9-a-thing.md"
if run; then bad "a bodiless design fragment passed"; else ok "a bad design fragment exits non-zero"; fi

fresh; sed -i.bak 's/^version = "\(.*\)"$/version = "9.9.9"/' "$t/proto/Cargo.toml"; rm -f "$t/proto/Cargo.toml.bak"
if run; then bad "a version mismatch passed"; else ok "a version mismatch exits non-zero"; fi

# --check-tests (#1286): the checks assembling can break, run on the copy. Fake cargo records its calls.
fakecargo="$work/fakecargo"
cat > "$fakecargo" <<'FAKE'
#!/usr/bin/env bash
echo "cargo $* (in $(pwd))" >> "$FAKE_LOG"
[ -z "${FAKE_CARGO_FAIL:-}" ] || { echo "fake cargo: failing"; exit 101; }
FAKE
chmod +x "$fakecargo"
export FAKE_LOG="$work/fake.log"
ct() { : > "$FAKE_LOG"; TMPDIR="$work/tmp" REHEARSE_CARGO="$fakecargo" "$here/rehearse-assemble.sh" --check-tests "$t" > "$work/out" 2> "$work/err"; }
notices() { printf 'x\n- klatch %s (https://example.invalid)\n- klatch-bench %s\n' "$1" "$1" > "$t/proto/THIRD-PARTY-LICENSES.txt"; }
curver() { sed -n 's/^version = "\(.*\)"$/\1/p' "$t/proto/Cargo.toml" | head -1; }
fresh; notices "$(curver)"; printf '#!/usr/bin/env bash\necho shell-test-ran >> "$FAKE_LOG"\n' > "$t/release/test-fake.sh"; chmod +x "$t/release/test-fake.sh"
printf '#!/usr/bin/env bash\necho recursion >> "$FAKE_LOG"\n' > "$t/release/test-rehearse-assemble.sh"; chmod +x "$t/release/test-rehearse-assemble.sh"
before="$(snapshot)"
if ct; then ok "--check-tests passes on a good tree"; else bad "--check-tests failed: $(cat "$work/err")"; fi
if grep -q 'test -p klatch --test release_scope_docs' "$FAKE_LOG" && grep -q 'test -p klatch --lib negotiate' "$FAKE_LOG"; then ok "runs the doc pinning cargo tests"; else bad "cargo calls: $(cat "$FAKE_LOG")"; fi
if grep -q 'in .*klatch-rehearse-copy.*/proto)' "$FAKE_LOG"; then ok "cargo runs in the throwaway copy"; else bad "cwd: $(cat "$FAKE_LOG")"; fi
if grep -q shell-test-ran "$FAKE_LOG" && ! grep -q recursion "$FAKE_LOG"; then ok "runs the release shell tests but not itself"; else bad "shell tests: $(cat "$FAKE_LOG")"; fi
if [ "$(snapshot)" = "$before" ]; then ok "--check-tests leaves the tree unchanged"; else bad "tree changed"; fi

fresh; notices 0.0.1-stale; if ct; then bad "a stale notices version passed"; else ok "a stale notices version exits non-zero"; fi
if grep -q 'THIRD-PARTY-LICENSES' "$work/err"; then ok "names the notices file"; else bad "err: $(cat "$work/err")"; fi
fresh; notices "$(curver)"; FAKE_CARGO_FAIL=1 ct && bad "a failing cargo test passed" || ok "a failing cargo test exits non-zero"
fresh; notices "$(curver)"; printf '#!/usr/bin/env bash\nexit 1\n' > "$t/release/test-bad.sh"; chmod +x "$t/release/test-bad.sh"
if ct; then bad "a failing shell test passed"; else ok "a failing release shell test exits non-zero"; fi
fresh; notices 0.0.1-stale; if run; then ok "default rehearsal skips the extra checks"; else bad "default ran extra checks: $(cat "$work/err")"; fi

if TMPDIR="$work/tmp" "$here/rehearse-assemble.sh" "$work/nonexistent" > /dev/null 2>&1; then bad "a missing root passed"; else ok "a missing root exits non-zero"; fi
exit "$fails"
