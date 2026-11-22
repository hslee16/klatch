#!/usr/bin/env bash
# release/rollback-rehearsal.sh must print the full command sequence in dry mode (compared with a fixture), refuse --execute
# without what it needs, and with fakes run only the reversible steps (#1242). No network, no credential: gh, cargo and npm
# are fakes that record their arguments; the tokens in the environment are dummies that must never be printed.
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-rollback.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
bin="$work/bin"; mkdir -p "$bin" "$work/tmp"
for tool in gh cargo npm; do
  printf '#!/bin/sh\necho "%s $*" >> "%s/calls.log"\necho 4242\nexit 0\n' "$tool" "$work" > "$bin/$tool"
done
printf '#!/bin/sh\necho "git $*" >> "%s/calls.log"\nexit 99\n' "$work" > "$bin/git"
chmod +x "$bin"/*
secret_gh="ghp_DUMMYSECRETGH"; secret_cargo="cio_DUMMYSECRETCARGO"; secret_npm="npm_DUMMYSECRETNPM"  # npm is set only to prove it is never used
clean_env() { env -u GH_TOKEN -u GITHUB_TOKEN -u CARGO_REGISTRY_TOKEN -u NPM_TOKEN PATH="$bin:$PATH" TMPDIR="$work/tmp" "$@"; }
full_env() { env GH_TOKEN="$secret_gh" CARGO_REGISTRY_TOKEN="$secret_cargo" NPM_TOKEN="$secret_npm" PATH="$bin:$PATH" TMPDIR="$work/tmp" "$@"; }

# dry mode: the default, compared with the fixture; nothing is called even with tokens present
rm -f "$work/calls.log"
if full_env "$here/rollback-rehearsal.sh" 0.2.0-alpha.1 > "$work/out" 2> "$work/err"; then ok "dry mode exits 0"; else bad "dry failed: $(cat "$work/err")"; fi
if diff "$here/testdata/rollback-dry-0.2.0-alpha.1.txt" "$work/out" > "$work/diff"; then ok "dry output equals the fixture"; else bad "dry output differs: $(head -20 "$work/diff")"; fi
[ ! -e "$work/calls.log" ] && ok "dry mode ran nothing" || bad "dry mode called: $(cat "$work/calls.log")"
grep -q "$secret_gh\|$secret_cargo\|$secret_npm" "$work/out" "$work/err" && bad "a token was printed" || ok "no token is printed"
if clean_env "$here/rollback-rehearsal.sh" --dry-run 0.2.0-alpha.1 > "$work/out2" 2>&1 && cmp -s "$work/out" "$work/out2"; then ok "--dry-run is the same as the default"; else bad "--dry-run differs"; fi
if clean_env "$here/rollback-rehearsal.sh" 0.1.0.dev2 > /dev/null 2>&1; then bad "a PEP 440 spelling passed"; else ok "a version that is not SemVer fails"; fi
if clean_env "$here/rollback-rehearsal.sh" > /dev/null 2>&1; then bad "no version passed"; else ok "no version fails"; fi
# read the whole output first: grep -q closing the pipe early can SIGPIPE the script and fail the pipeline under pipefail
clean_env "$here/rollback-rehearsal.sh" 0.3.0-dev.7 > "$work/out3" 2> /dev/null || true
if grep -q 'cargo yank --version 0.3.0-dev.7 klatch' "$work/out3"; then ok "the commands follow the version"; else bad "version not substituted"; fi

# --execute refuses without its environment, and says which names are missing (never a value)
rm -f "$work/calls.log"
if clean_env "$here/rollback-rehearsal.sh" --execute 0.2.0-alpha.1 > "$work/out" 2> "$work/err"; then bad "--execute ran with no tokens"; else ok "--execute refuses with no tokens"; fi
grep -q 'GH_TOKEN' "$work/err" && grep -q 'CARGO_REGISTRY_TOKEN' "$work/err" && ok "the refusal names every missing variable" || bad "refusal text: $(cat "$work/err")"
grep -q 'NPM_TOKEN' "$work/err" && bad "the refusal asks for NPM_TOKEN, which npm deprecate cannot use unattended (#1295)" || ok "NPM_TOKEN is not asked for"
if env GH_TOKEN="$secret_gh" PATH="$bin:$PATH" TMPDIR="$work/tmp" "$here/rollback-rehearsal.sh" --execute 0.2.0-alpha.1 > /dev/null 2> "$work/err"; then bad "--execute ran with CARGO_REGISTRY_TOKEN missing"; else grep -q 'CARGO_REGISTRY_TOKEN' "$work/err" && ok "one missing token refuses" || bad "text: $(cat "$work/err")"; fi
if full_env "$here/rollback-rehearsal.sh" --execute 0.2.0 > /dev/null 2> "$work/err"; then bad "--execute ran on a release version"; else grep -qi 'pre-release' "$work/err" && ok "--execute refuses a version that is not a pre-release" || bad "text: $(cat "$work/err")"; fi
[ ! -e "$work/calls.log" ] && ok "refusals ran nothing" || bad "ran: $(cat "$work/calls.log")"

# --execute with all tokens (fakes): only the reversible steps run, in order, and no token is printed
rm -f "$work/calls.log"
if full_env "$here/rollback-rehearsal.sh" --execute 0.2.0-alpha.1 > "$work/out" 2> "$work/err"; then ok "--execute exits 0 with fakes"; else bad "execute failed: $(cat "$work/err")"; fi
want="gh api repos/hslee16/klatch/releases -f tag_name=v0.2.0a1 -f name=klatch 0.2.0-alpha.1 (rollback rehearsal) -F draft=true -F prerelease=true --jq .id
gh api repos/hslee16/klatch/releases/4242 --jq .draft
gh api -X DELETE repos/hslee16/klatch/releases/4242
cargo yank --version 0.2.0-alpha.1 klatch
cargo yank --undo --version 0.2.0-alpha.1 klatch"
got="$(grep -v '^npm' "$work/calls.log")"
[ "$got" = "$want" ] && ok "gh and cargo ran exactly the reversible steps, in order" || bad "calls: $got"
# npm deprecate needs a one-time password (EOTP), so it never runs unattended: the two commands are printed for the owner (#1295)
grep -q '^npm' "$work/calls.log" && bad "npm ran: $(grep '^npm' "$work/calls.log")" || ok "npm was not run (deprecate needs a one-time password)"
grep -q 'npm deprecate "@klatch/client@0.2.0-alpha.1" "rollback rehearsal: ignore this version"' "$work/err" && grep -q 'npm deprecate "@klatch/client@0.2.0-alpha.1" ""' "$work/err" && ok "the two npm commands are printed for the owner" || bad "npm commands not printed: $(cat "$work/err")"
grep -qi 'one-time password' "$work/err" && ok "the output says why npm is by hand" || bad "no explanation"
grep -q -- '--otp' "$work/out" "$work/err" "$work/calls.log" && bad "--otp appears on a command line" || ok "no one-time password on any command line"
grep -q 'NPM_TOKEN' "$here/rollback-rehearsal.sh" "$here/testdata/rollback-dry-0.2.0-alpha.1.txt" && bad "NPM_TOKEN is still referenced" || ok "NPM_TOKEN is gone"
grep -q 'yank-version' "$work/out" && ok "the dry run notes the yank-version scope" || true
grep -q "$secret_gh\|$secret_cargo\|$secret_npm" "$work/out" "$work/err" "$work/calls.log" && bad "a token leaked into output or arguments" || ok "no token in output or in any command line"
grep -q '^git' "$work/calls.log" && bad "a manual step was run" || ok "PyPI and the tap revert were not run"
[ -z "$(ls -A "$work/tmp")" ] && ok "no temporary file is left behind" || bad "left behind: $(ls "$work/tmp")"
exit "$fails"
