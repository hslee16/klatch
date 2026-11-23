#!/usr/bin/env bash
# Hidden checker for fixture F2 onefile (#1367). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main, full history). It lives outside the fixture directory so no agent
# checkout contains it. Prints one JSON object on its last stdout line: {"checks": [...], "units_completed", "units_total",
# "merge_state", "correct", "lost_edits", "lost_table_entries", "per_agent"}. Exits 0 whenever it could run.
# check.sh --file PATH prints {"A": bool, "B": bool, "C": bool}: which tickets' behaviour one version of core.py has (the
# harness runs it on every commit it finds, to count edits that were committed and later lost).
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "${1:-}" = "--file" ]; then
  exec python3 -I "$here/check.py" --file "${2:?usage: check.sh --file PATH}"
fi
dir="${1:?usage: check.sh CHECKOUT}"
exec python3 -I "$here/check.py" "$dir"
