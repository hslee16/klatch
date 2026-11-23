#!/usr/bin/env bash
# Hidden checker for fixture F3 chain (#1368). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main, full history). It lives outside the fixture directory so no agent
# checkout contains it. Prints one JSON object on its last stdout line: {"checks": [...], "units_completed", "units_total",
# "steps_correct", "quirk_honoured", "lost_at_handoff", "order", "merge_state", "correct"}. Exits 0 whenever it could run.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dir="${1:?usage: check.sh CHECKOUT}"
exec python3 -I "$here/check.py" "$dir"
