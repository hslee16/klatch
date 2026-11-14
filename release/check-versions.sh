#!/usr/bin/env bash
# One release is one number (VERSIONING.md): the crate, the PyPI package, the npm package and the Python wheel's
# __version__ must agree, CHANGELOG.md must have an entry for it, and README.md must name it in the cargo install line.
# The release workflow repeats the check against the tag; this runs on every pull request so a mismatch is found early.
#
#   release/check-versions.sh [root]      (root defaults to the repository)
set -euo pipefail
ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"

first() { sed -n "$2" "$ROOT/$1" | head -n 1; }
cargo=$(first proto/Cargo.toml 's/^version = "\(.*\)"$/\1/p')
pypi=$(first clients/python/pyproject.toml 's/^version = "\(.*\)"$/\1/p')
wheel=$(first clients/python/package/wheel_init.py 's/^__version__ = "\(.*\)"$/\1/p')
npm=$(first clients/typescript/package.json 's/^ *"version": *"\(.*\)",$/\1/p')
# PEP 440 spells the same pre-release 0.1.0.dev2 where SemVer says 0.1.0-dev.2
semver_of_pypi() { printf '%s' "$1" | sed 's/\.dev\([0-9]\)/-dev.\1/'; }

fail=0
check() { # label actual
  if [ "$2" != "$cargo" ]; then echo "MISMATCH  $1 is '$2', the crate is '$cargo'" >&2; fail=1; fi
}
[ -n "$cargo" ] || { echo "no version in proto/Cargo.toml" >&2; exit 1; }
check "clients/python/pyproject.toml" "$(semver_of_pypi "$pypi")"
check "clients/python/package/wheel_init.py" "$(semver_of_pypi "$wheel")"
check "clients/typescript/package.json" "$npm"

grep -qF "## [$cargo]" "$ROOT/CHANGELOG.md" || { echo "MISMATCH  CHANGELOG.md has no '## [$cargo]' entry" >&2; fail=1; }
grep -qF "cargo install klatch --version $cargo" "$ROOT/README.md" || { echo "MISMATCH  README.md does not say 'cargo install klatch --version $cargo'" >&2; fail=1; }

[ "$fail" -eq 0 ] || exit 1
echo "versions agree: $cargo (PyPI $pypi)"
