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

# sigpipe-ok: the fixture texts below are the risky patterns on purpose
expect_flag "printf into grep -q" 'printf "%s" "$x" | grep -q foo' # sigpipe-ok: fixture
expect_flag "echo into grep -qF" 'echo "$x" | grep -qF foo' # sigpipe-ok: fixture
expect_flag "a command into grep -q" 'git log | grep -q foo' # sigpipe-ok: fixture
expect_flag "grep -q after other flags" 'cat f | grep -E -q foo' # sigpipe-ok: fixture
expect_flag "grep with the q among combined flags" 'cat f | grep -xqi foo' # sigpipe-ok: fixture
expect_flag "grep --quiet" 'cat f | grep --quiet foo' # sigpipe-ok: fixture
expect_flag "grep -m1" 'cat f | grep -m1 foo' # sigpipe-ok: fixture
expect_flag "grep --max-count" 'cat f | grep --max-count=1 foo' # sigpipe-ok: fixture
expect_flag "head -1" 'x="$(cmd | head -1)"' # sigpipe-ok: fixture
expect_flag "head -n 1" 'x="$(cmd | head -n 1)"' # sigpipe-ok: fixture
expect_flag "head -n1" 'x="$(cmd | head -n1)"' # sigpipe-ok: fixture
expect_flag "sed -q" 'cmd | sed -q "1q"' # sigpipe-ok: fixture
expect_flag "the pipe at the end of the line" $'cmd |\n  grep -q foo' # sigpipe-ok: fixture
expect_flag "a pipe inside if and &&" 'if [ -n "$a" ] && cmd | grep -q foo; then :; fi' # sigpipe-ok: fixture
expect_flag "a negated pipeline" '! cmd | grep -q foo' # sigpipe-ok: fixture

expect_pass "a here-string" 'grep -qF foo <<< "$x"'
expect_pass "grep without -q, output discarded" 'cmd | grep foo > /dev/null'
expect_pass "grep -c (reads all input)" 'x="$(cmd | grep -c foo)"'
expect_pass "head -n 5 (not an early exit on the first line)" 'cmd | head -n 5 > /dev/null'
expect_pass "sed -n 1p (reads all input)" 'x="$(cmd | sed -n 1p)"'
expect_pass "grep -q on a file, no pipe" 'grep -q foo file || true'
expect_pass "the second grep of || is not a pipe" 'a || grep -q foo file'
expect_pass "a quoted pattern that holds -q after a pipe" 'cmd | grep -vE "x (a|b) -q y" > /dev/null'
expect_pass "a comment that shows the pattern" '# do not write cmd | grep -q foo here'
expect_pass "a reviewed exception on the same line" 'cmd | grep -q foo # sigpipe-ok: cmd prints one short line' # sigpipe-ok: fixture
expect_pass "a reviewed exception on the line before" $'# sigpipe-ok: cmd prints one short line\ncmd | grep -q foo' # sigpipe-ok: fixture
expect_pass "a script without pipefail" 'cmd | grep -q foo' plain- # sigpipe-ok: fixture

# the marker needs a reason, and covers one line only
expect_flag "a marker with no reason" 'cmd | grep -q foo # sigpipe-ok:' # sigpipe-ok: fixture
expect_flag "a marker two lines up" $'# sigpipe-ok: reviewed\n:\ncmd | grep -q foo' # sigpipe-ok: fixture

# the report names the file and the line
f="$(fixture rep $'true\ncmd | grep -q foo')" # sigpipe-ok: fixture
out="$("$here/check-sigpipe.sh" "$f" 2>&1 || true)"
if grep -q "rep.sh:4:" <<< "$out"; then ok "the report names the file and the line"; else bad "report: $out"; fi

# the repository's own scripts are clean
if out="$("$here/check-sigpipe.sh" 2>&1)"; then ok "no script in the repository pipes into an early-exit consumer under pipefail"; else bad "repository scripts: $out"; fi

[ "$fails" = 0 ] && echo "check-sigpipe: all tests passed"
exit "$fails"
