#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED, with the arm in COORDBENCH_ARM (default B0). Writes the repository of the job at its
# middle into the empty directory DEST: a git repository with a PREBUILT history of 25 commits by three earlier agents (ada, bo
# and cy; fixed authors, dates and messages, so the commit ids are the same in every run), built from history.json by
# build_history.py. fixture.json says "prebuilt_history": true, so the harness does not make its own first commit: it pushes
# DEST's main to the per-run bare origin as it is. In arm B1 the commits also carry TASKS.md and DECISIONS.md; in every other
# arm they do not exist, and the decisions are in the commit messages only. This script touches nothing outside DEST. reference/,
# variants/ and history.json's source gen_facts.py stay out of every checkout. SEED is unused: the facts come from the fixed
# seed in gen_facts.py and are the same in every run.
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$dest"
exec python3 -I "$here/build_history.py" "$dest" "${COORDBENCH_ARM:-B0}"
