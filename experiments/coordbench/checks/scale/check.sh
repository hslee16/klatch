#!/usr/bin/env bash
# Hidden checker for fixture F7 scale (#1372). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main, full history). It lives outside the fixture directory so no
# agent checkout contains it. The size and the seed of the generated fixture come from the environment the harness sets
# from the fixture's "generated" key: COORDBENCH_GEN_MODULES and COORDBENCH_GEN_SEED. Prints one JSON object on its last
# stdout line: {"checks": [...], "units_completed", "units_total", "tickets_completed", "tickets_total", "merge_state",
# "correct", "per_agent"}. Exits 0 whenever it could run, 2 when the size or seed is missing.
set -uo pipefail
dir="${1:?usage: check.sh CHECKOUT}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -I "$here/check.py" "$dir"
