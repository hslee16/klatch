#!/usr/bin/env bash
# Hidden checker for the trivial fixture. Interface: check.sh CHECKOUT, run by the harness after the agents exit, against
# a fresh clone of the run's origin (branch main). It lives outside the fixture directory so no agent checkout contains
# it. A check with "unit": false is structural (correctness), not a unit of work. It prints one JSON object {"checks": [{"id": ..., "pass": true|false}, ...]} on stdout and exits 0 whenever it
# could run; a nonzero exit means the checker itself broke (the harness records that, it does not guess).
set -uo pipefail
dir="${1:?usage: check.sh CHECKOUT}"
cd "$dir" || exit 2
run() { python3 -c "$1" > /dev/null 2>&1 && echo true || echo false; }
a=$(run "import alpha; assert alpha.alpha() == 'alpha-done'")
b=$(run "import beta; assert beta.beta() == 'beta-done'")
if grep -rqE '^(<<<<<<<|>>>>>>>) ' --include='*.py' . 2> /dev/null; then clean=false; else clean=true; fi
python3 -m unittest discover -s tests -t . > /dev/null 2>&1 && tests=true || tests=false
printf '{"checks": [{"id": "alpha", "pass": %s}, {"id": "beta", "pass": %s}, {"id": "no-conflict-markers", "unit": false, "pass": %s}, {"id": "visible-tests", "unit": false, "pass": %s}]}\n' \
  "$a" "$b" "$clean" "$tests"
