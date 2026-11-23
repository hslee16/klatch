#!/usr/bin/env bash
# Hidden checker for fixture F4 specialists (#1369). Interface: check.sh CHECKOUT, run by the harness after the agents exit,
# against a fresh clone of the run's origin (branch main). It lives outside the fixture directory so no agent checkout
# contains it. Prints one JSON object on its last stdout line: {"checks": [...], "units_completed", "units_total", "slots",
# "wrong_guesses", "stale", "unanswered", "plausible_default_guesses", "merge_state", "correct"}. Exits 0 whenever it could run.
set -uo pipefail
dir="${1:?usage: check.sh CHECKOUT}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -I "$here/check.py" "$dir"
