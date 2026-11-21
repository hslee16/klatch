#!/usr/bin/env bash
# shellcheck disable=SC2016
# (the single-quoted strings hold literal backticks, markdown cells; nothing is meant to expand)
# Tests for the union merge attributes in the root .gitattributes (#1065) and for the audit fragments (#1068). Real
# temp git repos (nothing is mocked): two branches that each add a line at the same place of a file with
# `merge=union` merge cleanly and keep both lines; a file without the attribute still conflicts. The audit is not
# union-merged any more: one file per row under proto/audit/, assembled by proto/scripts/authz-audit. Nothing here touches the network, this repo's history or a
# real home. The union driver is built in to git, so no config is set in the temp repos.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-union-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

# rows unique per section by the first cell, the same rule as authz_audit.rs: prints each duplicated "section|cell"
dup_rows() {
  awk '
    /^## / { section = substr($0, 4); next }
    /^\|/ {
      line = $0; sub(/^\|+/, "", line); n = index(line, "|"); cell = (n ? substr(line, 1, n - 1) : line)
      gsub(/^ +| +$/, "", cell)
      if (substr(cell, 1, 1) == "`") { key = section "|" cell; if (++seen[key] == 2) print key }
    }' "$1"
}

# newrepo NAME: a repo with the real .gitattributes and a base commit on main; the files are added by the caller
newrepo() {
  r="$work/$1"; rm -rf "$r"; mkdir -p "$r"
  git -C "$r" init -q -b main
  cp "$repo/.gitattributes" "$r/.gitattributes"
  mkdir -p "$r/proto/klatch/src/domain" "$r/proto/klatch-bench/src/scenarios"
}
# add_after FILE LINE NEW: insert NEW after the line that equals LINE (awk, so it is the same on macOS and Linux)
add_after() { awk -v at="$2" -v new="$3" '{ print } $0 == at { print new }' "$1" > "$1.tmp" && mv "$1.tmp" "$1"; }
commit_all() { git -C "$r" add -A && git -C "$r" commit -q -m "$1"; }

# The audit is fragment files (#1068), not a union-merged table: one file per row under proto/audit/<section>/, so
# no attribute is needed and two edits of one row are a real conflict a human resolves.
fragments() {
  mkdir -p "$r/proto/audit/01-wire-methods" "$r/proto/scripts"
  printf '# Audit\n\n## Wire methods\n\n| Wire method | Proof |\n|---|---|\n' > "$r/proto/audit/01-wire-methods/_head.md"
  printf '| `post` | test_post |\n' > "$r/proto/audit/01-wire-methods/post.md"
  printf '| `claim` | test_claim |\n' > "$r/proto/audit/01-wire-methods/claim.md"
  cp "$here/authz-audit" "$r/proto/scripts/authz-audit"
  printf 'proto/AUTHZ-AUDIT.md\n' > "$r/.gitignore"
}
assembled() { AUTHZ_AUDIT_FRAGMENTS="$r/proto/audit" AUTHZ_AUDIT_TARGET="$r/proto/AUTHZ-AUDIT.md" bash "$r/proto/scripts/authz-audit" "$@"; }

# 1. two branches add a different row: different files, so the merge is clean and the assembled document has both once
newrepo add
fragments; assembled; commit_all base
git -C "$r" checkout -q -b a; printf '| `alpha` | test_alpha |\n' > "$r/proto/audit/01-wire-methods/alpha.md"; assembled; commit_all a
git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; printf '| `beta` | test_beta |\n' > "$r/proto/audit/01-wire-methods/beta.md"; commit_all b
if git -C "$r" merge -q --no-edit a > "$work/out" 2>&1 && assembled \
  && [ "$(grep -c '^| `alpha`' "$r/proto/AUTHZ-AUDIT.md")" -eq 1 ] && [ "$(grep -c '^| `beta`' "$r/proto/AUTHZ-AUDIT.md")" -eq 1 ] \
  && [ -z "$(dup_rows "$r/proto/AUTHZ-AUDIT.md")" ] && assembled --check > /dev/null; then
  ok "audit fragments: two branches each adding a row merge cleanly and the assembled document has each row once"
else bad "audit add/add: $(cat "$work/out")"; fi

# 2. two branches edit the SAME row file: a real conflict, never a silent duplicate
newrepo edit
fragments; assembled; commit_all base
git -C "$r" checkout -q -b a; printf '| `claim` | test_claim_a |\n' > "$r/proto/audit/01-wire-methods/claim.md"; commit_all a
git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; printf '| `claim` | test_claim_b |\n' > "$r/proto/audit/01-wire-methods/claim.md"; commit_all b
if ! git -C "$r" merge -q --no-edit a > "$work/out" 2>&1 && grep -q '^<<<<<<<' "$r/proto/audit/01-wire-methods/claim.md"; then
  ok "audit fragments: two edits of one row conflict instead of leaving the row twice"
else bad "audit edit/edit did not conflict: $(cat "$work/out")"; fi

# the generated file is checked (it is not committed, so two PRs never both change it): a hand edit of it is found
newrepo drift
fragments; assembled; printf 'hand edit\n' >> "$r/proto/AUTHZ-AUDIT.md"
if ! assembled --check > /dev/null 2>&1; then ok "authz-audit --check fails when the generated file was edited by hand"; else bad "drift not found"; fi

# 3. the registries: `mod x;` lines and the scenario list, two branches adding at the same place
for f in proto/klatch/src/domain/mod.rs proto/klatch-bench/src/scenarios/mod.rs; do
  newrepo "reg-$(basename "$(dirname "$f")")"
  printf 'mod one;\nmod two;\n' > "$r/$f"; commit_all base
  git -C "$r" checkout -q -b a; printf 'mod one;\nmod two;\nmod alpha;\n' > "$r/$f"; commit_all a
  git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; printf 'mod one;\nmod two;\nmod beta;\n' > "$r/$f"; commit_all b
  if git -C "$r" merge -q --no-edit a > "$work/out" 2>&1 && grep -qx 'mod alpha;' "$r/$f" && grep -qx 'mod beta;' "$r/$f" \
    && [ "$(grep -c '^mod ' "$r/$f")" -eq 4 ]; then ok "$f: two added mod lines at the same place merge cleanly and keep both"
  else bad "registry $f: $(cat "$work/out")"; fi
done

# 4. a file without the attribute still conflicts (the attribute is the only difference)
newrepo ctl
printf 'mod one;\n' > "$r/proto/klatch/src/other.rs"; commit_all base
git -C "$r" checkout -q -b a; printf 'mod one;\nmod alpha;\n' > "$r/proto/klatch/src/other.rs"; commit_all a
git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; printf 'mod one;\nmod beta;\n' > "$r/proto/klatch/src/other.rs"; commit_all b
if ! git -C "$r" merge -q --no-edit a > /dev/null 2>&1 && grep -q '^<<<<<<<' "$r/proto/klatch/src/other.rs"; then
  ok "a file without the attribute still conflicts"
else bad "control did not conflict"; fi

# 5. the real attribute file: every listed path exists in this repo and git resolves merge=union for it
listed="$(awk '!/^#/ && $2 == "merge=union" { print $1 }' "$repo/.gitattributes")"
missing=""; unset_attr=""
for p in $listed; do
  [ -f "$repo/$p" ] || missing="$missing $p"
  [ "$(git -C "$repo" check-attr merge -- "$p" | sed 's/.*: //')" = union ] || unset_attr="$unset_attr $p"
done
if [ -n "$listed" ] && [ -z "$missing" ] && [ -z "$unset_attr" ]; then ok ".gitattributes: every merge=union path exists and git applies the driver ($(echo "$listed" | wc -w | tr -d ' ') files)"
else bad ".gitattributes listed=[$listed] missing=[$missing] unset=[$unset_attr]"; fi

# 6. the real fragments assemble
if AUTHZ_AUDIT_TARGET="$work/real-audit.md" "$here/authz-audit" && [ -z "$(dup_rows "$work/real-audit.md")" ] && [ "$(wc -l < "$work/real-audit.md")" -gt 200 ]; then ok "the real fragments assemble to a document with no duplicate row"; else bad "real fragments do not assemble cleanly"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all union merge tests passed"; else echo "$fails union merge test(s) failed"; exit 1; fi
