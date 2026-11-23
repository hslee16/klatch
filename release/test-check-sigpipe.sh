#!/usr/bin/env bash
# release/check-sigpipe.sh must flag a script under `set -o pipefail` that pipes into an early-exit consumer, accept
# the safe forms and the reviewed exceptions, and the repository's own scripts must be clean (#1357).
# shellcheck disable=SC2016
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-sigpipe.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
# fixture NAME BODY: a script with a pipefail header (or none, when the name starts with plain-)
fixture() {
  local f="$work/$1.sh"
  if [[ "$1" == plain-* ]]; then printf '#!/usr/bin/env bash\nset -eu\n%s\n' "$2" > "$f"; else printf '#!/usr/bin/env bash\nset -euo pipefail\n%s\n' "$2" > "$f"; fi
  echo "$f"
}
expect_flag() { # description, body
  local f; n=$((n + 1)); f="$(fixture "r$n" "$2")"
  if "$here/check-sigpipe.sh" "$f" > /dev/null 2>&1; then bad "did not flag: $1"; else ok "flags: $1"; fi
}
expect_pass() { # description, body
  local f; n=$((n + 1)); f="$(fixture "${3:-p}$n" "$2")"
  if "$here/check-sigpipe.sh" "$f" > /dev/null 2>&1; then ok "accepts: $1"; else bad "flagged: $1"; fi
}
n=0

expect_flag "printf into grep -q" 'printf "%s" "$x" | grep -q foo'
expect_flag "echo into grep -qF" 'echo "$x" | grep -qF foo'
expect_flag "a command into grep -q" 'git log | grep -q foo'
expect_flag "grep -q after other flags" 'cat f | grep -E -q foo'
expect_flag "grep with the q among combined flags" 'cat f | grep -xqi foo'
expect_flag "grep --quiet" 'cat f | grep --quiet foo'
expect_flag "grep -m1" 'cat f | grep -m1 foo'
expect_flag "grep --max-count" 'cat f | grep --max-count=1 foo'
expect_flag "head -1" 'x="$(cmd | head -1)"'
expect_flag "head -n 1" 'x="$(cmd | head -n 1)"'
expect_flag "head -n1" 'x="$(cmd | head -n1)"'
expect_flag "sed -q" 'cmd | sed -q "1q"'
expect_flag "the pipe at the end of the line" $'cmd |\n  grep -q foo'
expect_flag "a pipe inside if and &&" 'if [ -n "$a" ] && cmd | grep -q foo; then :; fi'
expect_flag "a negated pipeline" '! cmd | grep -q foo'

expect_pass "a here-string" 'grep -qF foo <<< "$x"'
expect_pass "grep without -q, output discarded" 'cmd | grep foo > /dev/null'
expect_pass "grep -c (reads all input)" 'x="$(cmd | grep -c foo)"'
expect_pass "head -n 5 (not an early exit on the first line)" 'cmd | head -n 5 > /dev/null'
expect_pass "sed -n 1p (reads all input)" 'x="$(cmd | sed -n 1p)"'
expect_pass "grep -q on a file, no pipe" 'grep -q foo file || true'
expect_pass "the second grep of || is not a pipe" 'a || grep -q foo file'
expect_pass "a quoted pattern that holds -q after a pipe" 'cmd | grep -vE "x (a|b) -q y" > /dev/null'
expect_pass "a comment that shows the pattern" '# do not write cmd | grep -q foo here'
expect_pass "a reviewed exception on the same line" 'cmd | grep -q foo # sigpipe-ok: cmd prints one short line'
expect_pass "a reviewed exception on the line before" $'# sigpipe-ok: cmd prints one short line\ncmd | grep -q foo'
expect_pass "a script without pipefail" 'cmd | grep -q foo' plain-

# the marker needs a reason, and covers one line only
expect_flag "a marker with no reason" 'cmd | grep -q foo # sigpipe-ok:'
expect_flag "a marker two lines up" $'# sigpipe-ok: reviewed\n:\ncmd | grep -q foo'

# the report names the file and the line
f="$(fixture rep $'true\ncmd | grep -q foo')"
out="$("$here/check-sigpipe.sh" "$f" 2>&1 || true)"
if grep -q "rep.sh:4:" <<< "$out"; then ok "the report names the file and the line"; else bad "report: $out"; fi

# precise rule: text in single quotes and comments is not code; a real pipe in code is, whatever else is on the line
expect_pass "the pattern inside a single-quoted string" "x='cmd | grep -q foo'"
expect_pass "the pattern in a multi-line single-quoted string" $'x=\'first line\ncmd | grep -q foo\nlast\'\ntrue'
expect_pass "the pattern in a trailing comment" 'true # cmd | grep -q foo'
expect_pass "an apostrophe in a comment does not open a string" $'# it\'s fine\ncmd | grep foo > /dev/null'
expect_flag "a real pipe after a single-quoted argument" "printf '%s' \"\$x\" | grep -q y"
expect_flag "a real pipe after a double-quoted argument" 'echo "a b" | grep -q y'
expect_flag "a pipe inside a command substitution in double quotes" 'x="$(cmd | head -1)"'
expect_flag "a real pipe on the line after a quoted pattern" $'x=\'cmd | grep -q foo\'\nprintf %s "$x" | grep -q y'
expect_flag "a real pipe after an apostrophe in a double-quoted string" $'echo "it\'s" | grep -q y'
expect_flag "a real pipe after an escaped quote" 'echo \"x | grep -q y'

# untracked files are scanned too: a script that is new in a branch is checked before it is committed
r="$work/repo"
mkdir -p "$r/release"; cp "$here/check-sigpipe.sh" "$r/release/"
git -C "$r" init -q
printf '#!/usr/bin/env bash\nset -euo pipefail\ncmd | grep -q foo\n' > "$r/new.sh"
if "$r/release/check-sigpipe.sh" > /dev/null 2>&1; then bad "an untracked risky script was not scanned"; else ok "an untracked risky script is flagged"; fi
git -C "$r" -c user.name=t -c user.email=t@t add new.sh
if "$r/release/check-sigpipe.sh" > /dev/null 2>&1; then bad "a staged risky script was not scanned"; else ok "a staged risky script is flagged"; fi

# the repository's own scripts are clean, from the entry points the gate and a person use (gate: repo root, no arguments)
root="$(cd "$here/.." && pwd)"
for dir in "$root" "$here" "$root/proto"; do
  if out="$(cd "$dir" && "$root/release/check-sigpipe.sh" 2>&1)"; then ok "no script in the repository pipes into an early-exit consumer under pipefail (run from ${dir#"$root"}/)"; else bad "repository scripts (from $dir): $out"; fi
done

[ "$fails" = 0 ] && echo "check-sigpipe: all tests passed"
exit "$fails"
