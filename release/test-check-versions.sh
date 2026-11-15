#!/usr/bin/env bash
# release/check-versions.sh must pass when every place agrees and fail when any one of them drifts.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-versions.XXXXXX")"
trap 'rm -rf "$work"' EXIT
files="proto/Cargo.toml clients/python/pyproject.toml clients/python/package/wheel_init.py clients/typescript/package.json CHANGELOG.md README.md"

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
expect_fail "the changelog lost the entry"       CHANGELOG.md                              's/^## \[0\.1\.0-dev\.[0-9]*\]/## [0.0.0]/'
expect_fail "the README names another version"   README.md                                 's/cargo install klatch --version [^ ]*/cargo install klatch --version 9.9.9/'
exit "$fails"
