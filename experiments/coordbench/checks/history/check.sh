#!/usr/bin/env bash
# Hidden checker for fixture F9 history (#1374). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main, full history). It lives outside the fixture directory so no agent
# checkout contains it. Prints one JSON object on its last stdout line: {"checks": [...], "units_completed", "units_total",
# "merge_state", "correct", "facts", "errors", "duplicated_helpers", "rewritten_modules"}. Exits 0 whenever it could run.
# check.sh --tree DIR prints {"task", "facts", "correct"} for any directory holding one version of the tree (no git, no
# visible tests): the harness runs it on every commit the late joiner made, to find the first correct contribution.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "${1:-}" = "--tree" ]; then
  exec python3 -I "$here/check.py" --tree "${2:?usage: check.sh --tree DIR}"
fi
dir="${1:?usage: check.sh CHECKOUT}"
exec python3 -I "$here/check.py" "$dir"
