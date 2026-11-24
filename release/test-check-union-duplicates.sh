#!/usr/bin/env bash
# release/check-union-duplicates.sh finds a line twice in a file merged with merge=union, per file type (#1064).
# Real temp git repos, nothing mocked.
# shellcheck disable=SC2016
# (single-quoted strings hold literal backticks; nothing is meant to expand)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-uniondup-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
r="$work/r"
fresh() { rm -rf "$r"; mkdir -p "$r/release"; git -C "$r" init -q -b main; printf '%s\n' "$@" > "$r/.gitattributes"; }
put() { mkdir -p "$(dirname "$r/$1")"; printf '%b' "$2" > "$r/$1"; }
run() { git -C "$r" add -A; UNION_DUP_ALLOW="${ALLOW:-$r/release/union-duplicates.allow}" "$here/check-union-duplicates.sh" "$r" > "$work/out" 2>&1; }
expect_ok() { if run; then ok "passes: $1"; else bad "failed: $1: $(cat "$work/out")"; fi; }
expect_fail() { if run; then bad "did not notice: $1"; else ok "notices: $1"; fi; }
out_has() { if grep -qF -- "$1" "$work/out"; then ok "message has: $1"; else bad "message lacks '$1': $(cat "$work/out")"; fi; }

fresh 'a/mod.rs merge=union'
put a/mod.rs 'mod x;\nmod y;\n'; expect_ok "a clean rust registry"
put a/mod.rs 'mod x;\nmod y;\nmod x;\n'; expect_fail "a duplicate mod line"
out_has "a/mod.rs:3: mod x;"; out_has "first at line 1"; out_has "delete one copy"
put a/mod.rs 'pub use x::A;\npub use x::A;\n'; expect_fail "a duplicate pub use"
put a/mod.rs 'x::all(),\ny::all(),\nx::all(),\n'; expect_fail "a duplicate call entry"
put a/mod.rs 'mod x;\n\n\nmod y;\n// same\n// same\n    let a = 1;\n    let a = 1;\n'; expect_ok "blank lines, comments and code lines are ignored in rust"
put a/mod.rs 'pub use x::{\n    A,\n};\npub use y::{\n    A,\n};\n'; expect_ok "multi-line brace lists are skipped"

fresh 'notes.md merge=union'
put notes.md '# T\n\n## S\n\n- one\n- two\n\n## S\n\n---\n---\n```\n```\n\nprose\nprose\n'; expect_ok "headings, rules, fences, prose and blank lines repeat in markdown"
put notes.md '- one\n- two\n- one\n'; expect_fail "a duplicate list item"
put notes.md '| a | b |\n|---|---|\n| x | 1 |\n|---|---|\n| x | 1 |\n'; expect_fail "a duplicate table row"
put notes.md '| a | b |\n|---|---|\n| x | 1 |\n## H\n|---|---|\n| y | 2 |\n'; expect_ok "table separator rows repeat"

fresh 'list.txt merge=union'
put list.txt '# c\n# c\nalpha\nbeta\n\n\n---\n---\n'; expect_ok "comments, blanks and punctuation repeat in other files"
put list.txt 'alpha\nbeta\nalpha\n'; expect_fail "a duplicate line in another file type"

fresh '*.lst merge=union'
put x/y.lst 'a\na\n'; expect_fail "a glob pattern in .gitattributes"

fresh 'a/mod.rs merge=union'
put a/mod.rs 'mod x;\nmod x;\n'; put b/mod.rs 'mod x;\nmod x;\n'; put a/other.txt 'x\nx\n'
put release/union-duplicates.allow '# reasons\na/mod.rs @@ mod x; @@ two cfg blocks\n'
expect_ok "an allowlisted line (and a file not marked union is not checked)"
put release/union-duplicates.allow 'b/mod.rs @@ mod x; @@ other file\n'; expect_fail "an allowlist entry for another file does not apply"
put release/union-duplicates.allow 'a/mod.rs @@ mod x;\n'; expect_fail "an allowlist entry without a reason is an error"
out_has "bad allowlist"

fresh 'a/mod.rs merge=binary'
put a/mod.rs 'mod x;\nmod x;\n'; expect_ok "a file not marked union"
fresh '# nothing'
put a/mod.rs 'mod x;\nmod x;\n'; expect_ok "a .gitattributes with no union"
fresh; rm -f "$r/.gitattributes"; put a/mod.rs 'mod x;\nmod x;\n'; expect_ok "a missing .gitattributes"

[ "$fails" = 0 ] && echo "all union duplicate checks ok"
exit "$fails"
