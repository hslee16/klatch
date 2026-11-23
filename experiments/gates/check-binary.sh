#!/usr/bin/env bash
# Check a Claude Code binary against what RESEARCH-NOTES 47 recorded (#554). Manual: run it when the Claude Code
# version changes, never in CI (it needs a real binary). It runs `strings` on the binary and reports
#   1. the expression that decides whether the inbound socket token is enforced (47.1),
#   2. the causes a held cross-session message can carry (47.2),
#   3. the default of the dialog deadline (47.2).
# Exit 0: the authRequired expression is the recorded one. Exit 1: it differs (the token may now be enforced, or
# never; re-run experiments/gates/recv.py). Exit 2: usage, an unreadable binary, or the expression not found.
# The causes and the dialog default are reported with "differs" when they changed but do not change the exit code.
# It only reads the binary; it starts nothing and sends nothing.
set -uo pipefail

RECORDED_EXPR='requireAuth ?? (platform === "windows")'
RECORDED_CAUSES='explicit-setting managed-setting repo-setting invalid-setting bypass-default mode-unknown mode-mismatch no-mode-asserted'
RECORDED_DIALOG='5m'

if [ "$#" -ne 1 ] || [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
  echo "Usage: $0 <path to a Claude Code binary, for example ~/.local/share/claude/versions/2.1.325>" >&2
  exit 2
fi
bin="$1"
[ -r "$bin" ] && [ -f "$bin" ] || { echo "cannot read $bin" >&2; exit 2; }

work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-check-binary.XXXXXX")"
trap 'rm -rf "$work"' EXIT
strings -a "$bin" > "$work/s"

echo "binary: $bin"

# 1. authRequired. A minifier renames the variable and the function, so the expression is read through the
# function it calls and written back in plain words.
expr_line="$(grep -oE 'authRequired=[A-Za-z0-9_$]+\.requireAuth\?\?[^,;]+' "$work/s" | sed -n 1p)"
if [ -z "$expr_line" ]; then
  echo "authRequired: not found (the socket server may have changed shape; read the binary by hand)"
  exit 2
fi
fallback="${expr_line##*\?\?}"
actual=""
case "$fallback" in
  *'()')
    fn="${fallback%()}"
    esc="$(printf '%s' "$fn" | sed 's/[$]/[$]/g')"
    body="$(grep -oE "function ${esc}\(\)\{[^}]*\}" "$work/s" | sed -n 1p)"
    # `return <call>()==="windows"` is the platform test the notes recorded
    if grep -qE '^function [A-Za-z0-9_$]+\(\)\{return [A-Za-z0-9_$.]+\(\)==="windows"\}$' <<< "$body"; then
      actual='requireAuth ?? (platform === "windows")'
    else
      actual="requireAuth ?? ${body:-$fallback}"
    fi
    ;;
  *) actual="requireAuth ?? $fallback" ;;
esac
echo "authRequired: $actual"
echo "recorded:     $RECORDED_EXPR"

# 2. the causes of a held message
cause_text="$(grep -oE 'cause:[A-Za-z0-9_$]+\(\[[^]]*"explicit-setting"[^]]*\]\)' "$work/s" | sed -n 1p)"
causes="$(printf '%s' "$cause_text" | grep -oE '"[a-z-]+"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')"
echo "crossSessionInbound causes: ${causes:-not found}"
[ "$causes" = "$RECORDED_CAUSES" ] || echo "note: causes differ from the recorded list ($RECORDED_CAUSES)"

# 3. the dialog deadline default, from the text that describes the dialogExpiry setting
dialog="$(grep -oE 'Defaults to [0-9]+[a-z]+ to match' "$work/s" | sed -n 1p | awk '{print $3}')"
choices="$(grep -oE 'dialogExpiry:\(\)=>[A-Za-z0-9_$]+\(\[[^]]*\]\)' "$work/s" | sed -n 1p | grep -oE '"[0-9a-z]+"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')"
env_override="no"
grep -q 'CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS' "$work/s" && env_override="yes (CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS)"
echo "dialog default: ${dialog:-not found} (dialogExpiry choices: ${choices:-not found}; environment override: $env_override)"
[ "$dialog" = "$RECORDED_DIALOG" ] || echo "note: dialog default differs from the recorded $RECORDED_DIALOG"

if [ "$actual" = "$RECORDED_EXPR" ]; then
  echo "RESULT: the authRequired expression is the recorded one (RESEARCH-NOTES 47.1)"
  exit 0
fi
echo "RESULT: authRequired DIFFERS from the recorded expression; the inbound token may now be enforced or never enforced. Re-run experiments/gates/recv.py and update RESEARCH-NOTES 47.1."
exit 1
