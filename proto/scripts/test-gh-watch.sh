#!/usr/bin/env bash
# Runs the gh-watch fixture tests (#1329). Python only, no network, no gh. The launcher is shellchecked by the gate.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$(python3 -I "$here/test_gh_watch.py" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ]; then echo "  ok    gh-watch tests ($(printf '%s\n' "$out" | grep '^Ran '))"; else printf '%s\n' "$out" | tail -40; echo "  FAIL  gh-watch tests"; fi
exit "$rc"
