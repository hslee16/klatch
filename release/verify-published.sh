#!/usr/bin/env bash
# Post-publish verification from the public side (#1249, RELEASING.md section 3): given a version, check PyPI, npm,
# crates.io, the GitHub release, the Homebrew tap formula, then install into a temporary prefix and run the binaries.
#
#   release/verify-published.sh [--skip-install] [--report FILE] <semver>      (for example 0.2.0-alpha.1)
#
# Each check prints PASS, FAIL or SKIP and a detail, and a failing check does not stop the others. The exit status is
# non-zero when any check failed. --report writes the results as markdown, ready to paste as a comment on the release epic.
# It reads public data only: no credential is needed and none is printed (gh uses your login for REST reads only). Not
# checked here: the cosign signature and build provenance of the archives; run the commands in SECURITY.md ("Verifying a
# download") on a downloaded archive. The install runs in a throwaway directory with its own KLATCH_HOME and key directory.
set -euo pipefail

skip_install=0; report=""; version=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-install) skip_install=1; shift ;;
    --report) report="${2:?--report needs a file}"; shift 2 ;;
    -*) echo "verify-published: unknown option $1" >&2; exit 2 ;;
    *) [ -z "$version" ] || { echo "verify-published: one version only" >&2; exit 2; }; version="$1"; shift ;;
  esac
done
[ -n "$version" ] || { echo "usage: release/verify-published.sh [--skip-install] [--report FILE] <semver, like 0.2.0-alpha.1>" >&2; exit 2; }
re='^([0-9]+\.[0-9]+\.[0-9]+)(-(alpha|beta|rc|dev)\.([0-9]+))?$'
[[ "$version" =~ $re ]] || { echo "verify-published: '$version' is not SemVer like 0.2.0-alpha.1" >&2; exit 2; }
core="${BASH_REMATCH[1]}"; kind="${BASH_REMATCH[3]}"; num="${BASH_REMATCH[4]}"
case "$kind" in
  "") pypi="$core" ;;
  dev) pypi="$core.dev$num" ;;
  alpha) pypi="${core}a$num" ;;
  beta) pypi="${core}b$num" ;;
  rc) pypi="${core}rc$num" ;;
esac
tag="v$pypi"
repo="hslee16/klatch"
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu aarch64-unknown-linux-musl x86_64-unknown-linux-musl"
ua="klatch-verify-published"

results="$(mktemp "${TMPDIR:-/tmp}/klatch-verify-results.XXXXXX")"
installdir=""
cleanup() { rm -f "$results"; [ -z "$installdir" ] || rm -rf "$installdir"; }
trap cleanup EXIT
failed=0

record() { # status label detail
  printf '%-4s  %-22s %s\n' "$1" "$2" "$3"
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$results"
  [ "$1" != FAIL ] || failed=1
}
pass() { record PASS "$1" "$2"; }
fail() { record FAIL "$1" "$2"; }
fetch() { curl -fsSL -A "$ua" --max-time 30 "$1"; }

echo "verify-published: klatch $version (tag $tag, PyPI $pypi)"

# PyPI: the version is in the index (pre-releases need --pre)
if out="$(python3 -m pip index versions klatch --pre 2> /dev/null)"; then
  if printf '%s\n' "$out" | sed -n 's/^Available versions: *//p' | tr ',' '\n' | tr -d ' ' | grep -qx "$pypi"; then pass pypi "klatch $pypi is on PyPI"; else fail pypi "klatch $pypi is not in 'pip index versions klatch'"; fi
else fail pypi "pip index versions klatch failed (pip too old, or no network)"; fi

# npm: the version exists, is the latest, and carries provenance
got="$(npm view "@klatch/client@$version" version 2> /dev/null || true)"
if [ "$got" = "$version" ]; then pass npm-version "@klatch/client@$version exists"; else fail npm-version "npm view @klatch/client@$version gave '${got:-nothing}'"; fi
latest="$(npm view "@klatch/client" dist-tags.latest 2> /dev/null || true)"
if [ "$latest" = "$version" ]; then pass npm-latest "dist-tag latest is $version"; else fail npm-latest "dist-tag latest is '${latest:-nothing}' (a staged version is not public until approved with the key)"; fi
prov="$(npm view "@klatch/client@$version" dist.attestations.provenance.predicateType 2> /dev/null || true)"
if [ -n "$prov" ]; then pass npm-provenance "provenance attached ($prov)"; else fail npm-provenance "no provenance attestation on @klatch/client@$version"; fi

# crates.io: the version exists and is not yanked
if out="$(fetch "https://crates.io/api/v1/crates/klatch/$version" 2> /dev/null)"; then
  verdict="$(printf '%s' "$out" | python3 -I -c 'import json,sys
d = json.load(sys.stdin)["version"]
print("yanked" if d.get("yanked") else d.get("num", ""))' 2> /dev/null || true)"
  if [ "$verdict" = "$version" ]; then pass crates "klatch $version is on crates.io and not yanked"; else fail crates "crates.io says '${verdict:-unreadable}' for klatch $version"; fi
else fail crates "crates.io has no klatch $version"; fi

# GitHub release: published (not a draft), a pre-release when the version is one, and every file is there
want_assets="install.sh SHA256SUMS SHA256SUMS.sigstore.json"
for t in $targets; do want_assets="$want_assets klatch-$version-$t.tar.gz klatch-$version-$t.cdx.json"; done
if out="$(gh api "repos/$repo/releases/tags/$tag" 2> /dev/null)"; then
  verdict="$(printf '%s' "$out" | python3 -I -c 'import json,sys
d = json.load(sys.stdin)
print("draft" if d.get("draft") else "published", "prerelease" if d.get("prerelease") else "release")' 2> /dev/null || true)"
  want_kind=release; [ -z "$kind" ] || want_kind=prerelease
  if [ "$verdict" = "published $want_kind" ]; then pass github-release "$tag is published as a $want_kind"; else fail github-release "$tag is '${verdict:-unreadable}', expected 'published $want_kind'"; fi
  have="$(printf '%s' "$out" | python3 -I -c 'import json,sys
print("\n".join(a["name"] for a in json.load(sys.stdin).get("assets", [])))' 2> /dev/null || true)"
  missing=""
  for a in $want_assets; do printf '%s\n' "$have" | grep -qxF "$a" || missing="$missing $a"; done
  if [ -z "$missing" ]; then pass github-assets "all 6 archives, 6 SBOMs, install.sh, SHA256SUMS and its signature are attached"; else fail github-assets "missing:$missing"; fi
else
  fail github-release "no release for tag $tag (gh api repos/$repo/releases/tags/$tag failed)"
  fail github-assets "no release to read the files from"
fi

# Homebrew tap: the formula on main names this version and this tag's four archives
if out="$(fetch "https://raw.githubusercontent.com/hslee16/homebrew-klatch/main/Formula/klatch.rb" 2> /dev/null)"; then
  urls="$(printf '%s\n' "$out" | grep -cF "/$tag/klatch-$version-" || true)"
  if printf '%s\n' "$out" | grep -qF "version \"$version\"" && [ "$urls" -eq 4 ]; then pass tap-formula "the tap formula is at $version with four $tag URLs (its CI result is on the tap pull request)"; else fail tap-formula "the tap formula does not name version $version with four $tag URLs (found $urls)"; fi
else fail tap-formula "could not fetch Formula/klatch.rb from hslee16/homebrew-klatch main"; fi

# a clean install from PyPI into a throwaway virtual environment, then run both binaries
if [ "$skip_install" -eq 1 ]; then
  record SKIP install-klatch "--skip-install"
  record SKIP install-klatch-server "--skip-install"
else
  installdir="$(mktemp -d "${TMPDIR:-/tmp}/klatch-verify.XXXXXX")"
  if python3 -m venv "$installdir/venv" > /dev/null 2>&1 && "$installdir/venv/bin/pip" install --pre --quiet "klatch==$pypi" > /dev/null 2>&1; then
    out="$(env KLATCH_HOME="$installdir/home" KLATCH_KEY_DIR="$installdir/keys" "$installdir/venv/bin/klatch" --version 2> /dev/null || true)"
    if printf '%s\n' "$out" | grep -qF "klatch $version"; then pass install-klatch "klatch --version prints '$out'"; else fail install-klatch "klatch --version printed '${out:-nothing}', expected 'klatch $version'"; fi
    if [ -x "$installdir/venv/bin/klatch-server" ] && out="$("$installdir/venv/bin/klatch-server" --help 2> /dev/null)" && printf '%s\n' "$out" | grep -qF "usage: klatch-server"; then pass install-klatch-server "klatch-server --help prints its usage"; else fail install-klatch-server "klatch-server is missing or its --help failed"; fi
  else
    fail install-klatch "could not create a virtual environment or pip install klatch==$pypi"
    fail install-klatch-server "no install to run it from"
  fi
fi

if [ -n "$report" ]; then
  {
    echo "Post-publish verification of klatch $version (tag $tag) on $(date -u +%Y-%m-%d), run with release/verify-published.sh:"
    echo
    while IFS="$(printf '\t')" read -r st label detail; do echo "- $st $label: $detail"; done < "$results"
    echo
    echo "Not covered by the script: the cosign signature and build provenance of a downloaded archive (SECURITY.md, Verifying a download), the tap pull request CI, and a fresh-machine run of the README."
  } > "$report"
fi

if [ "$failed" -ne 0 ]; then echo "verify-published: FAILED, read the FAIL lines above" >&2; exit 1; fi
echo "verify-published: every check passed"
