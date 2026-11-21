#!/usr/bin/env bash
# shellcheck disable=SC2016
# (the single-quoted strings hold literal backticks, markdown cells; nothing is meant to expand)
# Tests for the union merge attributes in the root .gitattributes (#1065). Real temp git repos (nothing is mocked):
# two branches that each add a line at the same place of a file with `merge=union` merge cleanly and keep both
# lines; two branches that edit the SAME existing row are merged by the driver into two copies of it, and the
# duplicate-row check (the awk below mirrors proto/klatch/tests/authz_audit.rs `duplicate_rows`) fails on that
# result. A file without the attribute still conflicts. Nothing here touches the network, this repo's history or a
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

audit_base='# Audit

## Wire methods

| Wire method | Gateway call | Proof |
|---|---|---|
| `post` | `post` | test_post |
| `claim` | `claim` | test_claim |
| `who` | `who` | test_who |

## Other

| `post` | same key, another section | t |
'

# 1. two branches add a different row at the same place (the end of the table): clean, both rows kept
newrepo add
printf '%s' "$audit_base" > "$r/proto/AUTHZ-AUDIT.md"; commit_all base
git -C "$r" checkout -q -b a; add_after "$r/proto/AUTHZ-AUDIT.md" '| `who` | `who` | test_who |' '| `alpha` | `alpha` | test_alpha |'; commit_all a
git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; add_after "$r/proto/AUTHZ-AUDIT.md" '| `who` | `who` | test_who |' '| `beta` | `beta` | test_beta |'; commit_all b
if git -C "$r" merge -q --no-edit a > "$work/out" 2>&1 \
  && grep -q '^| `alpha`' "$r/proto/AUTHZ-AUDIT.md" && grep -q '^| `beta`' "$r/proto/AUTHZ-AUDIT.md" \
  && ! grep -q '^<<<<<<<' "$r/proto/AUTHZ-AUDIT.md" && [ -z "$(dup_rows "$r/proto/AUTHZ-AUDIT.md")" ]; then
  ok "AUTHZ-AUDIT.md: two branches each adding a row at the same place merge cleanly and keep both rows"
else bad "audit add/add: $(cat "$work/out")"; fi

# 2. two branches edit the SAME existing row: the union driver keeps both versions, and the duplicate check fails
newrepo edit
printf '%s' "$audit_base" > "$r/proto/AUTHZ-AUDIT.md"; commit_all base
git -C "$r" checkout -q -b a; sed -i.bak 's/test_claim/test_claim_a/' "$r/proto/AUTHZ-AUDIT.md"; rm -f "$r/proto/AUTHZ-AUDIT.md.bak"; commit_all a
git -C "$r" checkout -q main; git -C "$r" checkout -q -b b; sed -i.bak 's/test_claim/test_claim_b/' "$r/proto/AUTHZ-AUDIT.md"; rm -f "$r/proto/AUTHZ-AUDIT.md.bak"; commit_all b
git -C "$r" merge -q --no-edit a > "$work/out" 2>&1; rc=$?
dups="$(dup_rows "$r/proto/AUTHZ-AUDIT.md")"
if [ "$rc" -eq 0 ] && grep -q 'test_claim_a' "$r/proto/AUTHZ-AUDIT.md" && grep -q 'test_claim_b' "$r/proto/AUTHZ-AUDIT.md" \
  && [ "$dups" = 'Wire methods|`claim`' ]; then
  ok "AUTHZ-AUDIT.md: two edits of one row are not merged silently into a pass: the duplicate-row check finds the row twice"
else bad "audit edit/edit rc=$rc dups=[$dups] $(cat "$work/out")"; fi
# the same key in another section is not a duplicate (the base has one)
if [ -z "$(dup_rows "$work/add/proto/AUTHZ-AUDIT.md")" ]; then ok "the duplicate-row check allows the same key in two sections"; else bad "section keys"; fi

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

# 6. the real audit has no duplicate row now
if [ -z "$(dup_rows "$repo/proto/AUTHZ-AUDIT.md")" ]; then ok "the real AUTHZ-AUDIT.md has no duplicate row"; else bad "real audit dups: $(dup_rows "$repo/proto/AUTHZ-AUDIT.md")"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all union merge tests passed"; else echo "$fails union merge test(s) failed"; exit 1; fi
