#!/usr/bin/env bash
# release/compare-formula.sh must pass when two Homebrew formulas differ only in the version, the URLs and the hashes, and
# fail with a readable diff on any other difference (#1260). No network.
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-cmpf.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
a="$work/a.rb"; b="$work/b.rb"
cp "$here/homebrew/klatch.rb" "$a"
cmp_() { "$here/compare-formula.sh" "$a" "$b" > "$work/out" 2> "$work/err"; }

cp "$a" "$b"
if cmp_; then ok "identical formulas match"; else bad "identical formulas differ: $(cat "$work/err")"; fi

# a bump: version, URLs and hashes change, nothing else
sed -e 's/0\.1\.0-dev\.2/0.2.0-alpha.1/g' -e 's/v0\.1\.0\.dev2/v0.2.0a1/g' -e 's/sha256 "[0-9a-f]*"/sha256 "0000000000000000000000000000000000000000000000000000000000000000"/' "$a" > "$b"
if cmp_; then ok "a different version, URLs and hashes still match"; else bad "a bump was reported as drift: $(cat "$work/err")"; fi

cp "$a" "$b"; sed -i.bak 's/^  license "MIT"$/  license "Apache-2.0"/' "$b"; rm -f "$b.bak"
if cmp_; then bad "a changed license passed"; else ok "a changed license fails"; fi
grep -q 'license' "$work/err" && ok "the diff names the changed line" || bad "no diff on stderr: $(cat "$work/err")"

cp "$a" "$b"; sed -i.bak '/bin.install "klatch"/d' "$b"; rm -f "$b.bak"
if cmp_; then bad "a removed install line passed"; else ok "a removed install line fails"; fi

cp "$a" "$b"; sed -i.bak 's/ENV\["KLATCH_NAME"\] = "brew-test"/ENV["KLATCH_NAME"] = "other"/' "$b"; rm -f "$b.bak"
if cmp_; then bad "a changed test block passed"; else ok "a changed test block fails"; fi

cp "$a" "$b"; printf '  # a comment\n' >> "$b"
if cmp_; then bad "an added line passed"; else ok "an added line fails"; fi

cp "$a" "$b"; sed -i.bak 's/on_intel do/on_intel_x do/' "$b"; rm -f "$b.bak"
if cmp_; then bad "a changed block name passed"; else ok "a changed block name fails"; fi

# the klatch-server allowance (#1287): the staged formula (the release candidate) may be ahead of the tap (the previous
# release) by exactly the klatch-server install and test lines, and only with --allow-klatch-server
ahead() { # a staged copy with the klatch-server lines, in $1
  sed -e 's/^    bin.install "klatch"$/    bin.install "klatch", "klatch-server"/' \
    -e '/shell_output("#{bin}\/klatch --version")$/a\
    assert_match "usage: klatch-server", shell_output("#{bin}/klatch-server --help")' "$a" > "$1"
}
cmp_allow() { "$here/compare-formula.sh" --allow-klatch-server "$a" "$b" > "$work/out" 2> "$work/err"; }
ahead "$work/s.rb"; grep -q 'klatch-server --help' "$work/s.rb" && grep -q 'bin.install "klatch", "klatch-server"' "$work/s.rb" && ok "the fixture has both klatch-server lines" || bad "fixture not built"
cp "$a" "$b"; cp "$work/s.rb" "$a"
if cmp_allow; then ok "staged ahead by exactly the klatch-server lines passes with the allowance"; else bad "the allowance did not apply: $(cat "$work/err")"; fi
if cmp_; then bad "staged ahead passed without the allowance"; else ok "staged ahead fails without the allowance (strict)"; fi
cp "$work/s.rb" "$b"
if cmp_allow; then ok "equal formulas with the klatch-server lines pass with the allowance"; else bad "equal ahead formulas failed: $(cat "$work/err")"; fi
if cmp_; then ok "strict compare of equal formulas with the klatch-server lines passes"; else bad "strict equal failed: $(cat "$work/err")"; fi
cp "$work/s.rb" "$a"
cp "$work/s.rb" "$b"; sed -i.bak 's/^  license "MIT"$/  license "Apache-2.0"/' "$b"; rm -f "$b.bak"
if cmp_allow; then bad "the allowance hid a license change"; else ok "the allowance does not hide another difference"; fi
# a different extra line next to the klatch-server lines fails
cp "$work/s.rb" "$a"; printf '  # a comment\n' >> "$a"
cp "$here/homebrew/klatch.rb" "$b"
if cmp_allow; then bad "an extra comment passed with the allowance"; else ok "any other extra line fails with the allowance"; fi
cp "$work/s.rb" "$a"; sed -i.bak 's#klatch-server --help#klatch-server --version#' "$a"; rm -f "$a.bak"
if cmp_allow; then bad "a different klatch-server test line passed"; else ok "a different klatch-server test line fails with the allowance"; fi
cp "$work/s.rb" "$a"; sed -i.bak 's#bin.install "klatch", "klatch-server"#bin.install "klatch", "klatch-server", "other"#' "$a"; rm -f "$a.bak"
if cmp_allow; then bad "a different install line passed"; else ok "a different install line fails with the allowance"; fi
cp "$work/s.rb" "$a"; sed -i.bak 's#^\(    assert_match "usage: klatch-server".*\)$#\1\n\1#' "$a"; rm -f "$a.bak"
if cmp_allow; then bad "a duplicated klatch-server test line passed"; else ok "a duplicated klatch-server test line fails with the allowance"; fi
# the tap ahead of the staged copy is not allowed (the allowance is one way)
cp "$here/homebrew/klatch.rb" "$a"; cp "$work/s.rb" "$b"
if cmp_allow; then bad "a tap ahead of the staged copy passed"; else ok "a tap ahead of the staged copy fails"; fi
cp "$here/homebrew/klatch.rb" "$a"; cp "$here/homebrew/klatch.rb" "$b"

# usage and missing files
if "$here/compare-formula.sh" "$a" > /dev/null 2>&1; then bad "one argument passed"; else ok "one argument is refused"; fi
if "$here/compare-formula.sh" "$a" "$work/none.rb" > /dev/null 2>&1; then bad "a missing file passed"; else ok "a missing file is refused"; fi

# the staged copy in the repository is itself a formula the check accepts against itself
"$here/compare-formula.sh" "$here/homebrew/klatch.rb" "$here/homebrew/klatch.rb" > /dev/null 2>&1 && ok "the staged formula matches itself" || bad "the staged formula does not match itself"
exit "$fails"
