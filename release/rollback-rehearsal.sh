#!/usr/bin/env bash
# The rollback rehearsal (#1242, and #576 for the human): for a version, the exact commands that hide, yank, deprecate or revert
# it on every registry, in the order of RELEASING.md "Rolling back". Dry by default: it prints the commands and runs nothing.
#
#   release/rollback-rehearsal.sh [--dry-run] <semver>       print the sequence (the default)
#   release/rollback-rehearsal.sh --execute <semver>         run the reversible steps against a THROWAWAY pre-release
#
# --execute needs the human's tokens in the environment at run time (never stored, never printed, never put on a command
# line): GH_TOKEN (a GitHub token that can write releases of hslee16/klatch), CARGO_REGISTRY_TOKEN (crates.io, scope
# yank-version) and NPM_TOKEN (npm, written to a private temporary npmrc that is removed at the end). It refuses a version
# that is not a pre-release. It runs only the lines that are not comments: a draft pre-release is created and deleted,
# the crate is yanked and un-yanked, the npm version is deprecated and the deprecation removed. The PyPI yank and the tap
# revert are by hand (printed as comments). Every step is reversible; the throwaway version number itself is not:
# crates.io, npm and PyPI never give a published number back, so the rehearsal consumes the version it is run for.
set -euo pipefail

mode="dry"; version=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) mode="dry"; shift ;;
    --execute) mode="execute"; shift ;;
    -*) echo "rollback-rehearsal: unknown option $1" >&2; exit 2 ;;
    *) [ -z "$version" ] || { echo "rollback-rehearsal: one version only" >&2; exit 2; }; version="$1"; shift ;;
  esac
done
[ -n "$version" ] || { echo "usage: release/rollback-rehearsal.sh [--dry-run|--execute] <semver, like 0.2.0-alpha.1>" >&2; exit 2; }

re='^([0-9]+\.[0-9]+\.[0-9]+)(-(alpha|beta|rc|dev)\.([0-9]+))?$'
[[ "$version" =~ $re ]] || { echo "rollback-rehearsal: '$version' is not SemVer like 0.2.0-alpha.1 (the PEP 440 spelling is derived)" >&2; exit 2; }
core="${BASH_REMATCH[1]}"; kind="${BASH_REMATCH[3]}"; num="${BASH_REMATCH[4]}"
case "$kind" in
  "") pypi="$core" ;;
  dev) pypi="$core.dev$num" ;;
  alpha) pypi="${core}a$num" ;;
  beta) pypi="${core}b$num" ;;
  rc) pypi="${core}rc$num" ;;
esac
tag="v$pypi"

sequence() {
  cat <<'EOF'
# klatch rollback rehearsal for @VERSION@ (tag @TAG@, PyPI @PYPI@)
# DRY RUN: these commands are printed, not run. Lines that start with "# " are notes, or commands for a real incident;
# --execute never runs them. See RELEASING.md, "Rolling back".

# 1. GitHub draft pre-release: create, read back, delete. Reversible; consumes no version.
RELEASE_ID=$(gh api repos/hslee16/klatch/releases -f tag_name=@TAG@ -f name='klatch @VERSION@ (rollback rehearsal)' -F draft=true -F prerelease=true --jq .id)
gh api repos/hslee16/klatch/releases/"$RELEASE_ID" --jq .draft
gh api -X DELETE repos/hslee16/klatch/releases/"$RELEASE_ID"
# Real incident, GitHub release: hide it (the installer then no longer finds it) or remove one bad file.
# Do not delete the tag while a registry version points at it.
#   gh api -X PATCH repos/hslee16/klatch/releases/"$(gh api repos/hslee16/klatch/releases/tags/@TAG@ --jq .id)" -F draft=true
#   gh api -X DELETE repos/hslee16/klatch/releases/assets/ASSET_ID

# 2. crates.io: yank, then un-yank. Reversible. Needs CARGO_REGISTRY_TOKEN with the yank-version scope.
cargo yank --version @VERSION@ klatch
cargo yank --undo --version @VERSION@ klatch
# PERMANENT: the version number. A published crate version is never deleted or reused; the next number replaces it.

# 3. npm: deprecate, then undo. Reversible. Needs NPM_TOKEN (written to a private temporary npmrc, never printed).
npm deprecate "@klatch/client@@VERSION@" "rollback rehearsal: ignore this version" --userconfig "$NPMRC"
npm deprecate "@klatch/client@@VERSION@" "" --userconfig "$NPMRC"
# PERMANENT: the version number. An unpublished npm version can never be published again under that number.
# A staged version that is wrong is rejected on npmjs.com (Staged Packages) instead of approved.

# 4. PyPI: there is no command; do it by hand on https://pypi.org/manage/project/klatch/releases/
# Yank: open @PYPI@, choose "Yank release", give a reason. pip skips a yanked version unless it is pinned exactly.
# Un-yank: the same page, "Unyank release". Deleting a release cannot be undone.
# PERMANENT: the file names of @PYPI@ are never reusable, even after a delete.

# 5. Homebrew tap: revert the bump commit and open a pull request (printed for a real incident, never run here).
#   git clone https://github.com/hslee16/homebrew-klatch tap && cd tap
#   git checkout -b revert-@TAG@
#   git revert --no-edit BUMP_COMMIT
#   git push -u origin revert-@TAG@
#   gh api repos/hslee16/homebrew-klatch/pulls -f title='Revert klatch @VERSION@' -f head=revert-@TAG@ -f base=main
EOF
}

text="$(sequence)"
text="${text//@VERSION@/$version}"
text="${text//@TAG@/$tag}"
text="${text//@PYPI@/$pypi}"

if [ "$mode" = "dry" ]; then
  printf '%s\n' "$text"
  exit 0
fi

# --execute: refuse unless everything it needs is there; the messages name variables, never values
missing=""
[ -n "${GH_TOKEN:-}" ] || missing="$missing GH_TOKEN"
[ -n "${CARGO_REGISTRY_TOKEN:-}" ] || missing="$missing CARGO_REGISTRY_TOKEN"
[ -n "${NPM_TOKEN:-}" ] || missing="$missing NPM_TOKEN"
if [ -n "$missing" ]; then
  echo "rollback-rehearsal: --execute refuses: set these in the environment first (typed by you at run time, never stored):$missing" >&2
  exit 1
fi
[ -n "$kind" ] || { echo "rollback-rehearsal: --execute refuses a version that is not a pre-release ($version); rehearse on a throwaway pre-release such as 0.2.0-alpha.9" >&2; exit 1; }

npmrc="$(mktemp "${TMPDIR:-/tmp}/klatch-npmrc.XXXXXX")"
trap 'rm -f "$npmrc"' EXIT
chmod 600 "$npmrc"
printf '//registry.npmjs.org/:_authToken=%s\n' "$NPM_TOKEN" > "$npmrc"
export NPMRC="$npmrc"
unset NPM_TOKEN
echo "rollback-rehearsal: executing the reversible steps for $version (a draft release, yank and un-yank, deprecate and undo)" >&2
# the comment lines are not run; -x shows each command (the tokens are in the environment and the npmrc, not in the commands)
printf '%s\n' "$text" | bash -e -x /dev/stdin
echo "rollback-rehearsal: done. PyPI and the tap revert are by hand (see the dry run). $version is now used on every registry it was published to."
