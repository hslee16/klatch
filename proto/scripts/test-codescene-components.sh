#!/usr/bin/env bash
# Runs the CodeScene components tests (#1457) and the exact-partition check on the real tree, so a new top-level
# directory or source file must be assigned to a component in .codescene/components.json. Python only; the apply tests
# talk to a fake server on 127.0.0.1 and nothing leaves this machine.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$(python3 -I "$here/test_codescene_components.py" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ]; then
  echo "  ok    codescene-components tests ($(printf '%s\n' "$out" | sed -n 's/^Ran /Ran /p'))"
else
  printf '%s\n' "$out" | tail -40
  echo "  FAIL  codescene-components tests"
fi
if chk="$("$here/codescene-components" check 2>&1)"; then
  echo "  ok    $chk"
else
  printf '%s\n' "$chk" | tail -40
  echo "  FAIL  codescene-components check"
  rc=1
fi
exit "$rc"
