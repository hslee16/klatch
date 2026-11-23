#!/usr/bin/env bash
# Hidden checker for fixture F1 tallykit (#1366). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main, full history). It lives outside the fixture directory so no
# agent checkout contains it. Prints one JSON object on its last stdout line: {"checks": [...], "units_completed",
# "units_total", "merge_state", "correct", "per_agent"}. Exits 0 whenever it could run.
set -uo pipefail
dir="${1:?usage: check.sh CHECKOUT}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -I "$here/check.py" "$dir"
