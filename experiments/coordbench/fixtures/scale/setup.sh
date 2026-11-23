#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED. Writes the initial tree of the generated repository into the empty directory DEST:
# the package with one stub module per unit, a visible test per module, README.md, the registry (scalekit/__init__.py) and
# CHANGELOG.md. The size and the seed of the generator come from COORDBENCH_GEN_MODULES and COORDBENCH_GEN_SEED, which the
# harness sets from the fixture's "generated" key; the run SEED (the second argument) is unused, so every run of a size gets
# the same repository. The harness does the git init, the first commit and the push to the per-run bare origin; this script
# makes no git calls and touches nothing outside DEST. Only the template is written: the reference solution (gen.py
# reference) stays out of every checkout.
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
modules="${COORDBENCH_GEN_MODULES:?COORDBENCH_GEN_MODULES is not set}"
seed="${COORDBENCH_GEN_SEED:?COORDBENCH_GEN_SEED is not set}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$dest"
python3 -I "$here/gen.py" write "$dest" --modules "$modules" --seed "$seed"
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +
