#!/bin/sh
# Every job in the release workflows must have a timeout-minutes (#572): a hung job otherwise runs for the 6 hour
# default. Jobs that call a reusable workflow have no runner and are skipped.
set -eu
cd "$(dirname "$0")/.."
bad=0
for f in .github/workflows/release.yml .github/workflows/build-release.yml .github/workflows/release-dry-run.yml .github/workflows/platform-tests.yml; do
  awk -v f="$f" '
    function end_job() { if (job != "" && runs && !timeout) { printf "%s: job %s has no timeout-minutes\n", f, job; bad = 1 } }
    /^  [a-z][a-z0-9-]*:[ ]*$/ && injobs { end_job(); job = $1; runs = 0; timeout = 0; next }
    /^jobs:/ { injobs = 1; next }
    injobs && /^    runs-on:/ { runs = 1 }
    injobs && /^    timeout-minutes:/ { timeout = 1 }
    END { end_job(); exit bad }
  ' "$f" || bad=1
done
[ "$bad" = 0 ] && echo "every release job has a timeout"
exit "$bad"
