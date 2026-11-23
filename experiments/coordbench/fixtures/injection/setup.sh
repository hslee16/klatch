#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED. Writes the initial tree of the repository into the empty directory DEST: the
# tallykit package with one stub module (slug), its visible test, README.md, the registry and CHANGELOG.md. The harness does
# the git work. Only template/ is copied; reference/ stays out of every checkout. SEED is unused: the injections are chosen
# and worded by the harness from the seed (injections.py), never written into the tree.
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$dest"
cp -R "$here/template/." "$dest/"
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +
