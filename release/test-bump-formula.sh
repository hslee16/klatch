#!/usr/bin/env bash
# release/bump-formula.sh must rewrite the formula from a SHA256SUMS, verify the result, print the tap PR commands, refuse bad
# input, and never push or publish anything itself (#1241). No network: gh, git and brew are fakes that record their use.
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-bump.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }

# a fake bin directory: gh and git must never be called; brew records its arguments
bin="$work/bin"; mkdir -p "$bin"
for tool in gh git curl; do printf '#!/bin/sh\necho "%s $*" >> "%s/forbidden.log"\nexit 99\n' "$tool" "$work" > "$bin/$tool"; done
printf '#!/bin/sh\necho "brew $*" >> "%s/brew.log"\nexit 0\n' "$work" > "$bin/brew"
chmod +x "$bin"/*

h() { printf '%064d' "$1"; }   # a fake 64-digit checksum
tap="$work/tap"
fresh() {
  rm -rf "$tap" "$work/forbidden.log" "$work/brew.log"; mkdir -p "$tap/Formula"
  cat > "$tap/Formula/klatch.rb" <<'RB'
class Klatch < Formula
  desc "Substrate for agent rooms"
  homepage "https://github.com/hslee16/klatch"
  version "0.1.0-dev.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-aarch64-apple-darwin.tar.gz"
      sha256 "b0fc61917d5fe074c10950b922824368a758a1d3ad2a791d554ba1c1d764d8a9"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-x86_64-apple-darwin.tar.gz"
      sha256 "26482262ea32854057d8d2e217accdb1981ee1624705a2c37a624c342fc605c0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "4f37c00b6b484244f62ccd431cea3ce047081b7b5ad6172b9a195200a5dd9bd8"
    end
    on_intel do
      url "https://github.com/hslee16/klatch/releases/download/v0.1.0.dev2/klatch-0.1.0-dev.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "551946c2f28a85f452e0cbf2c84a0ca5952f531f302527b73d44c5574f88c087"
    end
  end

  def install
    bin.install "klatch"
    doc.install "LICENSE", "THIRD-PARTY-LICENSES.txt"
  end

  test do
    assert_match "klatch #{version}", shell_output("#{bin}/klatch --version")
    ENV["KLATCH_HOME"] = testpath/"home"
  end
end
RB
  cp "$tap/Formula/klatch.rb" "$work/staged.rb"   # the staged copy in the repository, in step with the tap
  v=0.2.0-alpha.1
  {
    echo "$(h 1)  klatch-$v-aarch64-apple-darwin.tar.gz"
    echo "$(h 2)  klatch-$v-x86_64-apple-darwin.tar.gz"
    echo "$(h 3)  klatch-$v-aarch64-unknown-linux-gnu.tar.gz"
    echo "$(h 4)  klatch-$v-x86_64-unknown-linux-gnu.tar.gz"
    echo "$(h 5)  klatch-$v-aarch64-unknown-linux-musl.tar.gz"
    echo "$(h 6)  klatch-$v-x86_64-unknown-linux-musl.tar.gz"
    echo "$(h 7)  klatch-$v-aarch64-apple-darwin.cdx.json"
    echo "$(h 8)  install.sh"
  } > "$work/SHA256SUMS"
}
run() { PATH="$bin:$PATH" "$here/bump-formula.sh" --staged "$work/staged.rb" "$@" > "$work/out" 2> "$work/err"; }
f="$tap/Formula/klatch.rb"

fresh
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then ok "exits 0 on good input"; else bad "failed: $(cat "$work/err")"; fi
grep -q '^  version "0.2.0-alpha.1"$' "$f" && ok "the version is bumped" || bad "version not bumped"
[ "$(grep -c 'v0.2.0a1/klatch-0.2.0-alpha.1-' "$f")" = 4 ] && ok "all four URLs use the tag and the version" || bad "urls: $(grep url "$f")"
grep -q 'dev' "$f" && bad "an old dev string is left" || ok "no old version string is left"
for i in 1 2 3 4; do grep -q "sha256 \"$(h $i)\"" "$f" && ok "sha256 $i is in place" || bad "sha256 $i missing"; done
grep -q "$(h 5)\|$(h 7)\|$(h 8)" "$f" && bad "a checksum of a file the formula does not use leaked in" || ok "only the four formula archives are used"
grep -q 'bin.install "klatch", "klatch-server"' "$f" && ok "install lines include klatch-server" || bad "install line"
grep -q 'klatch-server --help' "$f" && ok "the test block runs klatch-server --help" || bad "test line"
[ "$(grep -c 'klatch-server --help' "$f")" = 1 ] && ok "one test line only" || bad "test line duplicated"
grep -q '^--- a/Formula/klatch.rb' "$work/out" && grep -q '^+  version "0.2.0-alpha.1"' "$work/out" && ok "prints the patch" || bad "no patch in output"
grep -q 'gh api' "$work/out" && grep -q 'homebrew-klatch/pulls' "$work/out" && ok "prints the REST commands for the tap PR" || bad "no PR commands"
grep -q 'graphql\|gh pr edit\|gh issue view' "$work/out" && bad "prints a GraphQL-backed gh command" || ok "no GraphQL-backed command is printed"
[ ! -e "$work/forbidden.log" ] && ok "gh, git and curl were never called" || bad "called: $(cat "$work/forbidden.log")"

# idempotent: a second run changes nothing
cp "$f" "$work/after1"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap" && cmp -s "$f" "$work/after1"; then ok "a second run changes nothing"; else bad "not idempotent"; fi

# --patch and --pr-body write files
fresh
if run --patch "$work/p.patch" --pr-body "$work/pr.md" 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap" && [ -s "$work/p.patch" ] && [ -s "$work/pr.md" ]; then ok "--patch and --pr-body write files"; else bad "files not written: $(cat "$work/err")"; fi
grep -q 'v0.2.0a1' "$work/pr.md" && ok "the PR body names the tag" || bad "PR body"
cp "$tap/Formula/klatch.rb" "$work/bumped.rb"
fresh
if (cd "$tap" && patch -s -p1 < "$work/p.patch") && cmp -s "$f" "$work/bumped.rb"; then ok "the patch applies to the old formula and gives the bumped one"; else bad "patch does not reproduce"; fi

# refusals
fresh; sed -i.bak '/aarch64-unknown-linux-gnu.tar.gz/d' "$work/SHA256SUMS"; rm -f "$work/SHA256SUMS.bak"; cp "$f" "$work/before"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "a missing archive line passed"; else ok "a SHA256SUMS missing an archive fails"; fi
cmp -s "$f" "$work/before" && ok "and the formula is left as it was" || bad "formula changed on failure"
fresh; sed -i.bak "s/^$(h 1)/zz/" "$work/SHA256SUMS"; rm -f "$work/SHA256SUMS.bak"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "a bad checksum passed"; else ok "a checksum that is not 64 hex digits fails"; fi
fresh; if run 0.2.0 "$work/SHA256SUMS" "$tap"; then bad "a version with no matching archives passed"; else ok "a version the SHA256SUMS does not name fails"; fi
fresh; if run 0.2 "$work/SHA256SUMS" "$tap"; then bad "a bad version passed"; else ok "a version that is not SemVer fails"; fi
fresh; rm "$f"; if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "no formula passed"; else ok "a tap with no Formula/klatch.rb fails"; fi
fresh; if run 0.2.0-alpha.1 "$work/nonexistent" "$tap"; then bad "no SHA256SUMS passed"; else ok "a missing SHA256SUMS fails"; fi
fresh; sed -i.bak '/on_intel do/,/end/{/sha256/d;}' "$f"; rm -f "$f.bak"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "a formula with a missing sha256 line passed"; else ok "a formula that does not have four url and sha256 pairs fails"; fi

# drift between the staged copy and the tap formula (#1260): refused, and nothing is written
fresh; sed -i.bak 's/^  license "MIT"$/  license "Apache-2.0"/' "$work/staged.rb"; rm -f "$work/staged.rb.bak"; cp "$f" "$work/before"; cp "$work/staged.rb" "$work/staged.before"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "a staged copy that drifted from the tap passed"; else ok "a staged copy that differs from the tap fails"; fi
grep -q 'license' "$work/err" && ok "the drift is shown as a diff" || bad "no diff: $(cat "$work/err")"
cmp -s "$f" "$work/before" && cmp -s "$work/staged.rb" "$work/staged.before" && ok "and neither formula is changed" || bad "a formula changed on drift"
fresh; rm "$work/staged.rb"
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then bad "a missing staged copy passed"; else ok "a missing staged copy fails"; fi
# after a good bump the staged copy is the bumped formula, so the post-publish comparison with the tap holds
fresh
if run 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap" && cmp -s "$f" "$work/staged.rb"; then ok "the staged copy is updated to the bumped formula"; else bad "staged copy not updated"; fi
# the default staged copy is the one in this repository (the fake tap here is not like it)
fresh
if PATH="$bin:$PATH" "$here/bump-formula.sh" 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap" > "$work/out" 2> "$work/err"; then bad "the default staged copy was not compared"; else grep -q 'staged' "$work/err" && ok "without --staged the repository copy is compared" || bad "unexpected error: $(cat "$work/err")"; fi

# rehearsal: brew is called in order against file:// archives; without brew it skips with a message
fresh; mkdir -p "$work/archives"
if run --rehearse "$work/archives" 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap"; then ok "--rehearse exits 0 with a fake brew"; else bad "rehearse failed: $(cat "$work/err")"; fi
grep -q 'audit --strict --new' "$work/brew.log" && grep -q 'install --build-from-source' "$work/brew.log" && grep -q '^brew test' "$work/brew.log" && ok "brew audit, install and test ran" || bad "brew log: $(cat "$work/brew.log" 2>&1)"
grep -q 'file://' "$work/out" && ok "the rehearsal uses file:// archives" || bad "no file:// in output"
grep -q '^brew untap\|^brew uninstall' "$work/brew.log" && ok "the rehearsal cleans up after itself" || bad "no cleanup in brew log"
[ ! -e "$work/forbidden.log" ] && ok "rehearsal never called gh, git or curl" || bad "called: $(cat "$work/forbidden.log")"
fresh; mkdir -p "$work/archives"; rm "$bin/brew"
if PATH="$bin:/usr/bin:/bin" "$here/bump-formula.sh" --staged "$work/staged.rb" --rehearse "$work/archives" 0.2.0-alpha.1 "$work/SHA256SUMS" "$tap" > "$work/out" 2> "$work/err"; then grep -qi 'brew is not installed' "$work/out" && ok "without brew the rehearsal says so and skips" || bad "no skip message: $(cat "$work/out")"; else bad "no brew failed the script: $(cat "$work/err")"; fi
exit "$fails"
