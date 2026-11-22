#!/usr/bin/env bash
# release/check-versions.sh must pass when every place agrees and fail when any one of them drifts.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-versions.XXXXXX")"
trap 'rm -rf "$work"' EXIT
files="proto/Cargo.toml clients/python/pyproject.toml clients/python/package/wheel_init.py clients/typescript/package.json CHANGELOG.md README.md release/homebrew/klatch.rb"
# shellcheck source=release/pep440.sh
. "$here/pep440.sh"

fresh() {
  rm -rf "$work/t"; mkdir -p "$work/t"
  for f in $files; do mkdir -p "$work/t/$(dirname "$f")"; cp "$ROOT/$f" "$work/t/$f"; done
}
fails=0
expect_fail() { # description, file, sed expression
  fresh
  sed -i.bak "$3" "$work/t/$2"; rm -f "$work/t/$2.bak"
  if "$here/check-versions.sh" "$work/t" >/dev/null 2>&1; then echo "FAIL  did not notice: $1" >&2; fails=1; else echo "  ok  notices: $1"; fi
}

fresh
if "$here/check-versions.sh" "$work/t" >/dev/null; then echo "  ok  passes when every place agrees"; else echo "FAIL  the real tree does not pass" >&2; fails=1; fi
expect_fail "the crate version moved"            proto/Cargo.toml                          's/^version = "\(.*\)"$/version = "9.9.9"/'
expect_fail "the PyPI version moved"             clients/python/pyproject.toml             's/^version = "\(.*\)"$/version = "9.9.9"/'
expect_fail "the wheel __version__ moved"        clients/python/package/wheel_init.py      's/^__version__ = ".*"$/__version__ = "9.9.9"/'
expect_fail "the npm version moved"              clients/typescript/package.json           's/^\( *\)"version": *".*",$/\1"version": "9.9.9",/'
expect_fail "the changelog lost the entry"       CHANGELOG.md                              's/^## \[[0-9][^]]*\]/## [0.0.0]/'
expect_fail "the README names another version"   README.md                                 's/cargo install klatch --version [^ ]*/cargo install klatch --version 9.9.9/'

# --- every kind of release number (#1234): the whole tree moves to one version and must still agree ----------------------
# retarget TREE SEMVER: rewrite every place and the formula to SEMVER, with the PEP 440 spelling where PyPI and the tag want it
retarget() {
  local t="$1" sv="$2" pep old_sv old_pep
  pep="$(semver_to_pep440 "$sv")"
  old_sv="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$t/proto/Cargo.toml" | head -n 1)"
  old_pep="$(semver_to_pep440 "$old_sv")"
  sed -i.bak "s/^version = \".*\"$/version = \"$sv\"/" "$t/proto/Cargo.toml"
  sed -i.bak "s/^version = \".*\"$/version = \"$pep\"/" "$t/clients/python/pyproject.toml"
  sed -i.bak "s/^__version__ = \".*\"$/__version__ = \"$pep\"/" "$t/clients/python/package/wheel_init.py"
  sed -i.bak "s/^\( *\)\"version\": *\".*\",$/\1\"version\": \"$sv\",/" "$t/clients/typescript/package.json"
  sed -i.bak "s/^## \[$old_sv\]/## [$sv]/" "$t/CHANGELOG.md"
  sed -i.bak "s/cargo install klatch --version [^ ]*/cargo install klatch --version $sv/" "$t/README.md"
  sed -i.bak -e "s/$old_sv/$sv/g" -e "s#/v$old_pep/#/v$pep/#g" "$t/release/homebrew/klatch.rb"
  find "$t" -name '*.bak' -exec rm -f {} +
}
for sv in 0.2.0-alpha.1 0.2.0-beta.2 0.2.0-rc.1 0.3.0-dev.4 0.2.0 1.0.0; do
  fresh; retarget "$work/t" "$sv"
  if "$here/check-versions.sh" "$work/t" >"$work/out" 2>&1; then echo "  ok  a whole tree at $sv agrees"; else echo "FAIL  a whole tree at $sv: $(cat "$work/out")" >&2; fails=1; fi
done
fresh; retarget "$work/t" 0.2.0-alpha.1
if "$here/check-versions.sh" "$work/t" 2>&1 | grep -q "PyPI 0.2.0a1"; then echo "  ok  the success line names the PyPI spelling"; else echo "FAIL  success line lacks PyPI 0.2.0a1" >&2; fails=1; fi
fresh; retarget "$work/t" 0.2.0-alpha.1
sed -i.bak 's/^version = ".*"$/version = "0.2.0-alpha.1"/' "$work/t/clients/python/pyproject.toml"; rm -f "$work/t/clients/python/pyproject.toml.bak"
if "$here/check-versions.sh" "$work/t" >/dev/null 2>&1; then echo "FAIL  PyPI in the SemVer spelling accepted" >&2; fails=1; else echo "  ok  notices: PyPI in the SemVer spelling (0.2.0-alpha.1 for 0.2.0a1)"; fi
fresh; retarget "$work/t" 0.2.0-alpha.1
sed -i.bak 's/^version = ".*"$/version = "0.2.0b1"/' "$work/t/clients/python/pyproject.toml"; rm -f "$work/t/clients/python/pyproject.toml.bak"
if "$here/check-versions.sh" "$work/t" >/dev/null 2>&1; then echo "FAIL  PyPI beta accepted for an alpha release" >&2; fails=1; else echo "  ok  notices: PyPI says beta while the crate says alpha"; fi
fresh; sed -i.bak 's/^version = ".*"$/version = "0.2.0-gamma.1"/' "$work/t/proto/Cargo.toml"; rm -f "$work/t/proto/Cargo.toml.bak"
if "$here/check-versions.sh" "$work/t" >/dev/null 2>&1; then echo "FAIL  an unmapped label was accepted" >&2; fails=1; else echo "  ok  an unmapped label (-gamma.1) is refused"; fi

# --- the Homebrew formula (#1235): version, four URLs, and the SHA-256 lines when present -----------------------------------
formula_fail() { # description, expected message fragment, sed expression (on the formula), [SHA256SUMS file]
  fresh; sed -i.bak "$3" "$work/t/release/homebrew/klatch.rb"; rm -f "$work/t/release/homebrew/klatch.rb.bak"
  if "$here/check-versions.sh" "$work/t" "$work/t/release/homebrew/klatch.rb" ${4:+"$4"} >"$work/out" 2>&1; then echo "FAIL  formula: did not notice $1" >&2; fails=1
  elif grep -q "$2" "$work/out"; then echo "  ok  formula: notices $1"; else echo "FAIL  formula: $1 failed without naming '$2': $(cat "$work/out")" >&2; fails=1; fi
}
formula_fail "a stale version line"          'formula.*version'   '/^ *version "/d;/^  homepage /a\
  version "0.0.1"'
formula_fail "a URL for another release"     'formula.*url'       '/aarch64-apple-darwin.tar.gz"$/s#download/v[^/]*/#download/v9.9.9/#'
formula_fail "a URL with the wrong archive"  'formula.*url'       's/klatch-[0-9][0-9a-z.-]*-aarch64-apple-darwin/klatch-aarch64-apple-darwin/'
formula_fail "a URL for the wrong platform"  'formula.*aarch64-unknown-linux-gnu' 's/aarch64-unknown-linux-gnu/aarch64-unknown-linux-musl/'
formula_fail "a missing URL"                 'formula.*url'       '/^ *url ".*x86_64-apple-darwin.tar.gz"$/,/^ *sha256 /d'
formula_fail "a malformed SHA-256"           'formula.*sha256'    's/sha256 "[0-9a-f]*"/sha256 "abc123"/'
cur0="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/proto/Cargo.toml" | head -n 1)"
sums0="$work/SHA256SUMS0"; : > "$sums0"
for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  printf '%s  klatch-%s-%s.tar.gz\n' "$(awk -v t="$t" '$0 ~ ("url \".*-" t "\\.tar\\.gz\"") { getline; gsub(/[^0-9a-f]/, "", $2); print $2 }' "$ROOT/release/homebrew/klatch.rb" | head -n 1)" "$cur0" "$t" >> "$sums0"
done
# --- a formula with and without the version line (#1297): the version then comes from the four URLs ------------------------
# fixtures are built from the real staged copy: one with the line added, one with it removed
with_line() { sed -e '/^ *version "/d' -e '/^  homepage /a\
  version "'"$1"'"' "$work/t/release/homebrew/klatch.rb"; }
without_line() { sed -e '/^ *version "/d' "$work/t/release/homebrew/klatch.rb"; }
fresh
with_line "$cur0" > "$work/with.rb"; without_line > "$work/without.rb"
if grep -q '^  version "' "$work/with.rb" && ! grep -q '^ *version "' "$work/without.rb"; then echo "  ok  fixtures: one formula with the version line, one without"; else echo "FAIL  version-line fixtures not built" >&2; fails=1; fi
if "$here/check-versions.sh" "$work/t" "$work/with.rb" >"$work/out" 2>&1; then echo "  ok  a formula with the version line passes"; else echo "FAIL  with line: $(cat "$work/out")" >&2; fails=1; fi
if "$here/check-versions.sh" "$work/t" "$work/without.rb" >"$work/out" 2>&1; then echo "  ok  a formula with no version line passes (version from the URLs)"; else echo "FAIL  without line: $(cat "$work/out")" >&2; fails=1; fi
if "$here/check-versions.sh" "$work/t" "$work/without.rb" "$sums0" >"$work/out" 2>&1; then echo "  ok  a formula with no version line passes against SHA256SUMS"; else echo "FAIL  without line, sums: $(cat "$work/out")" >&2; fails=1; fi
with_line "9.9.9" > "$work/stale.rb"
if "$here/check-versions.sh" "$work/t" "$work/stale.rb" >"$work/out" 2>&1; then echo "FAIL  a stale version line passed" >&2; fails=1; elif grep -q 'formula.*version' "$work/out"; then echo "  ok  notices: a version line that disagrees"; else echo "FAIL  stale line message: $(cat "$work/out")" >&2; fails=1; fi
sed 's#/download/v[^/]*/#/download/v9.9.9/#' "$work/without.rb" > "$work/other.rb"
if "$here/check-versions.sh" "$work/t" "$work/other.rb" >"$work/out" 2>&1; then echo "FAIL  no version line and URLs of another release passed" >&2; fails=1; elif grep -q 'formula.*url' "$work/out"; then echo "  ok  notices: no version line and the URLs name another release"; else echo "FAIL  other-release message: $(cat "$work/out")" >&2; fails=1; fi
sed -e 's#klatch-[0-9][0-9a-z.-]*-\(aarch64-apple-darwin\)#klatch-9.9.9-\1#' "$work/without.rb" > "$work/other2.rb"
if "$here/check-versions.sh" "$work/t" "$work/other2.rb" >"$work/out" 2>&1; then echo "FAIL  no version line and one archive of another version passed" >&2; fails=1; else echo "  ok  notices: no version line and one archive carries another version"; fi
fresh
if "$here/check-versions.sh" "$work/t" "$work/nope.rb" >/dev/null 2>&1; then echo "FAIL  a missing formula file passed" >&2; fails=1; else echo "  ok  a formula path that does not exist fails"; fi
# a SHA256SUMS file: every sha256 line must match its archive's line
sums="$work/SHA256SUMS"
cur="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/proto/Cargo.toml" | head -n 1)"
linux_sum="$(awk '/url ".*-x86_64-unknown-linux-gnu\.tar\.gz"/ { getline; gsub(/[^0-9a-f]/, "", $2); print $2 }' "$ROOT/release/homebrew/klatch.rb" | head -n 1)"
: > "$sums"
for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  sum="$(awk -v t="$t" '$0 ~ ("url \".*-" t "\\.tar\\.gz\"") { getline; gsub(/[^0-9a-f]/, "", $2); print $2 }' "$work/t/release/homebrew/klatch.rb" | head -n 1)"
  printf '%s  klatch-%s-%s.tar.gz\n' "$sum" "$cur" "$t" >> "$sums"
done
if "$here/check-versions.sh" "$work/t" "$work/t/release/homebrew/klatch.rb" "$sums" >"$work/out" 2>&1; then echo "  ok  the formula's SHA-256 values match SHA256SUMS"; else echo "FAIL  good SHA256SUMS: $(cat "$work/out")" >&2; fails=1; fi
formula_fail "a wrong SHA-256 against SHA256SUMS" 'formula.*sha256.*x86_64-unknown-linux-gnu' "s/$linux_sum/0000000000000000000000000000000000000000000000000000000000000000/" "$sums"
fresh
grep -v '^ *sha256 ' "$work/t/release/homebrew/klatch.rb" > "$work/nosha.rb"
if "$here/check-versions.sh" "$work/t" "$work/nosha.rb" >/dev/null 2>&1; then echo "  ok  no sha256 lines is fine without a SHA256SUMS file"; else echo "FAIL  a formula without sha256 lines was refused" >&2; fails=1; fi
if "$here/check-versions.sh" "$work/t" "$work/nosha.rb" "$sums" >"$work/out" 2>&1; then echo "FAIL  SHA256SUMS given but the formula has no sha256 lines" >&2; fails=1; elif grep -q "sha256" "$work/out"; then echo "  ok  a SHA256SUMS file needs the sha256 lines"; else echo "FAIL  message lacks sha256: $(cat "$work/out")" >&2; fails=1; fi

if "$here/test-pep440.sh" >"$work/out" 2>&1; then echo "  ok  the mapping table test passes (release/test-pep440.sh)"; else echo "FAIL  release/test-pep440.sh: $(grep FAIL "$work/out")" >&2; fails=1; fi
exit "$fails"
