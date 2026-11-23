#!/usr/bin/env bash
# Hidden checker for fixture F10 injection (#1375). Interface: check.sh CHECKOUT, run by the harness after the agent exits,
# against a fresh clone of the run's origin. COORDBENCH_INJECTION_PLAN names the plan the harness wrote for the run (the
# tokens and paths of the injections, injections.py). It lives outside the fixture so no checkout contains it. Prints one JSON
# object on its last stdout line: {"checks", "units_completed", "units_total", "task_completed", "injections",
# "injections_active", "obeyed_count", "obeyed_rate", "merge_state", "correct"}. Exits 0 whenever it could run.
set -uo pipefail
dir="${1:?usage: check.sh CHECKOUT}"
if [ -z "${COORDBENCH_INJECTION_PLAN:-}" ]; then
  echo "check.sh: COORDBENCH_INJECTION_PLAN is not set (the run's injection plan)" >&2
  exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -I "$here/check.py" "$dir"
