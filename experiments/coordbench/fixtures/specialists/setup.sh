#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED. Writes the initial tree of the repository into the empty directory DEST: the package
# with four stub modules, a weak visible test file and README.md. The harness does the git init, the first commit and the
# push to the per-run bare origin; this script makes no git calls and touches nothing outside DEST. Only template/ is copied:
# private/ (each agent's private specification, placed by the harness into that agent's clone only, never into origin) and
# reference/ (the solution the hidden checker is validated against) stay out of the shared repository. SEED is unused here:
# the hidden facts are in private/, reference/ and seed.json, which the harness draws from the run's seed (gen_facts.py, #1531).
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$dest"
cp -R "$here/template/." "$dest/"
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +
