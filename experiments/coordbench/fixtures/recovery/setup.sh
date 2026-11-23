#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED. Writes the initial tree of the repository into the empty directory DEST: the
# package with four stub modules, a visible test file per module, README.md and docs/FORMATS.md. The harness does the git
# init, the first commit and the push to the per-run bare origin; this script makes no git calls and touches nothing
# outside DEST. Only template/ is copied: reference/ (the solution the checker is validated against) stays out of every
# checkout. SEED is unused: the fixture is the same in every run.
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$dest"
cp -R "$here/template/." "$dest/"
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +
