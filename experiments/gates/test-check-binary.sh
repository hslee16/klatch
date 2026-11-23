#!/usr/bin/env bash
# the fake binaries are literal JavaScript, full of `$` and backticks that must not expand
# shellcheck disable=SC2016
# Tests for check-binary.sh (#554), run against small fake "binaries" (text files with the strings the real one
# carries), so no Claude Code install is needed. Cheap enough for the gate; the check itself is manual.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
check="$here/check-binary.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-check-binary.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

causes='cause:B(["explicit-setting","managed-setting","repo-setting","invalid-setting","bypass-default","mode-unknown","mode-mismatch","no-mode-asserted"])'
dialog='dialogExpiry:()=>B(["60s","5m","10m","never"]).optional().describe(`Max time. Defaults to 5m to match the long-standing remote-dialog deadline; "never" disables it. The CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS env var, when set, overrides this.`)'
fake() { # name, auth line, function line, causes, dialog
  printf 'noise before\n%s\n%s\n%s\n%s\nnoise after\n' "$2" "$3" "$4" "$5" > "$work/$1"
}
auth='this.x=1;a().authRequired=s.requireAuth??z3r(),a().firstLineDeadlineMs=s.firstLineDeadlineMs??Uoo;'
fn='function z3r(){return R()==="windows"}'

fake same "$auth" "$fn" "$causes" "$dialog"
out="$("$check" "$work/same" 2>&1)"; code=$?
if [ "$code" -eq 0 ]; then ok "the recorded expression exits 0"; else bad "same expression exited $code: $out"; fi
case "$out" in *'requireAuth ?? (platform === "windows")'*) ok "it prints the expression in readable form";; *) bad "no readable expression: $out";; esac
case "$out" in *explicit-setting*no-mode-asserted*) ok "it lists the crossSessionInbound causes";; *) bad "no cause list: $out";; esac
case "$out" in *"dialog default: 5m"*) ok "it reports the dialog default";; *) bad "no dialog default: $out";; esac

# a minifier renames the function and the variable; the meaning is the same, so the answer is the same
fake renamed 'q().authRequired=zz.requireAuth??Ab$1()' 'function Ab$1(){return Qx()==="windows"}' "$causes" "$dialog"
if "$check" "$work/renamed" > "$work/o" 2>&1; then ok "renamed minified identifiers are not a change"; else bad "renamed: $(cat "$work/o")"; fi

fake flipped "$auth" 'function z3r(){return R()!=="windows"}' "$causes" "$dialog"
if "$check" "$work/flipped" > "$work/o" 2>&1; then bad "a flipped platform test exited 0"; elif [ "$?" -eq 1 ] && grep -q "DIFFERS" "$work/o"; then ok "a changed expression exits 1 and says DIFFERS"; else bad "flipped: $(cat "$work/o")"; fi

fake always 'a().authRequired=s.requireAuth??!0' "$fn" "$causes" "$dialog"
if "$check" "$work/always" > "$work/o" 2>&1; then bad "an always-on token exited 0"; else ok "a different shape of expression also exits non-zero"; fi

fake gone 'nothing here' "$fn" "$causes" "$dialog"
if "$check" "$work/gone" > "$work/o" 2>&1; then bad "a binary with no expression exited 0"; elif [ "$?" -eq 2 ] && grep -q "not found" "$work/o"; then ok "no expression found exits 2 (cannot confirm)"; else bad "gone: $(cat "$work/o")"; fi

fake morecauses "$auth" "$fn" 'cause:B(["explicit-setting","new-cause","no-mode-asserted"])' "$dialog"
out="$("$check" "$work/morecauses" 2>&1)"; code=$?
if [ "$code" -eq 0 ] && grep -q "causes differ" <<< "$out"; then ok "changed causes are reported but do not fail the check"; else bad "causes: $code $out"; fi

fake newdialog "$auth" "$fn" "$causes" 'dialogExpiry:()=>B(["60s"]).describe(`Defaults to 10m to match`)'
out="$("$check" "$work/newdialog" 2>&1)"; code=$?
if [ "$code" -eq 0 ] && grep -q "dialog default differs" <<< "$out"; then ok "a changed dialog default is reported but does not fail"; else bad "dialog: $code $out"; fi

if "$check" > "$work/o" 2>&1; then bad "no argument exited 0"; elif [ "$?" -eq 2 ] && grep -qi "usage" "$work/o"; then ok "no argument prints usage and exits 2"; else bad "usage: $(cat "$work/o")"; fi
if "$check" "$work/missing-file" > "$work/o" 2>&1; then bad "a missing file exited 0"; else ok "a missing file exits non-zero"; fi

echo
if [ "$fails" -eq 0 ]; then echo "all check-binary tests passed"; else echo "$fails check-binary test(s) failed"; exit 1; fi
