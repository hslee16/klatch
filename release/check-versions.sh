#!/usr/bin/env bash
# One release is one number (VERSIONING.md): the crate, the PyPI package, the npm package and the Python wheel's
# __version__ must agree, CHANGELOG.md must have an entry for it, and README.md must name it in the cargo install line.
# The Homebrew formula (the staged copy release/homebrew/klatch.rb, or the path given) must carry the version, the four
# archive URLs and, when it has them, SHA-256 values (checked against a SHA256SUMS file when one is given). The SemVer to
# PEP 440 spelling (0.2.0-alpha.1 and 0.2.0a1) comes from release/pep440.sh, the one place it is written.
# The release workflow repeats the check against the tag; this runs on every pull request so a mismatch is found early.
#
#   release/check-versions.sh [root [formula [SHA256SUMS]]]   (root defaults to the repository, formula to
#                                                             release/homebrew/klatch.rb under root)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-$(cd "$here/.." && pwd)}"
FORMULA="${2:-$ROOT/release/homebrew/klatch.rb}"
SUMS="${3:-}"
# shellcheck source=release/pep440.sh
. "$here/pep440.sh"

first() { sed -n "$2" "$ROOT/$1" | head -n 1; }
cargo=$(first proto/Cargo.toml 's/^version = "\(.*\)"$/\1/p')
pypi=$(first clients/python/pyproject.toml 's/^version = "\(.*\)"$/\1/p')
wheel=$(first clients/python/package/wheel_init.py 's/^__version__ = "\(.*\)"$/\1/p')
npm=$(first clients/typescript/package.json 's/^ *"version": *"\(.*\)",$/\1/p')

fail=0
check() { # label actual expected
  if [ "$2" != "$3" ]; then echo "MISMATCH  $1 is '$2', expected '$3'" >&2; fail=1; fi
}
[ -n "$cargo" ] || { echo "no version in proto/Cargo.toml" >&2; exit 1; }
pep=$(semver_to_pep440 "$cargo") || {
  echo "MISMATCH  the crate version '$cargo' is not X.Y.Z or X.Y.Z-(alpha|beta|rc|dev).N (release/pep440.sh)" >&2
  exit 1
}
check "clients/python/pyproject.toml" "$pypi" "$pep"
check "clients/python/package/wheel_init.py" "$wheel" "$pep"
check "clients/typescript/package.json" "$npm" "$cargo"

grep -qF "## [$cargo]" "$ROOT/CHANGELOG.md" || { echo "MISMATCH  CHANGELOG.md has no '## [$cargo]' entry" >&2; fail=1; }
grep -qF "cargo install klatch --version $cargo" "$ROOT/README.md" || { echo "MISMATCH  README.md does not say 'cargo install klatch --version $cargo'" >&2; fail=1; }

# the Homebrew formula: the version, one URL per platform archive, and the SHA-256 lines when present
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu"
tag=$(release_tag "$cargo")
fcheck() { echo "MISMATCH  formula $FORMULA: $1" >&2; fail=1; }
if [ ! -f "$FORMULA" ]; then
  fcheck "no such file"
else
  fver=$(sed -n 's/^ *version "\(.*\)" *$/\1/p' "$FORMULA" | head -n 1)
  [ "$fver" = "$cargo" ] || fcheck "version is '$fver', expected '$cargo'"
  seen=""
  pairs=""
  last=""
  while IFS= read -r line; do
    case "$line" in
      *'url "'*)
        url=${line#*url \"}
        url=${url%%\"*}
        last=""
        for target in $targets; do
          if [ "$url" = "https://github.com/hslee16/klatch/releases/download/$tag/$(archive_name "$cargo" "$target")" ]; then
            seen="$seen $target"
            last="$target"
          fi
        done
        [ -n "$last" ] || fcheck "url '$url' is not one of the four archives of $cargo (expected .../download/$tag/klatch-$cargo-<target>.tar.gz)"
        ;;
      *'sha256 "'*)
        sum=${line#*sha256 \"}
        sum=${sum%%\"*}
        if ! printf '%s\n' "$sum" | grep -Eq '^[0-9a-f]{64}$'; then
          fcheck "sha256 '$sum' is not 64 hex digits (url for ${last:-no recognised url})"
        elif [ -z "$last" ]; then
          fcheck "sha256 '$sum' follows no recognised url"
        else
          pairs="$pairs $last:$sum"
        fi
        last=""
        ;;
    esac
  done < "$FORMULA"
  for target in $targets; do
    n=0
    for t in $seen; do [ "$t" != "$target" ] || n=$((n + 1)); done
    [ "$n" -eq 1 ] || fcheck "url for $target appears $n times, expected once"
  done
  if [ -n "$SUMS" ]; then
    if [ ! -f "$SUMS" ]; then
      fcheck "SHA256SUMS $SUMS does not exist"
    else
      for target in $targets; do
        archive=$(archive_name "$cargo" "$target")
        expected=$(awk -v f="$archive" '$2 == f { print $1 }' "$SUMS")
        have=""
        for p in $pairs; do [ "${p%%:*}" != "$target" ] || have=${p#*:}; done
        if [ -z "$expected" ]; then fcheck "SHA256SUMS has no line for $archive"
        elif [ -z "$have" ]; then fcheck "sha256 for $target ($archive) is missing, SHA256SUMS has $expected"
        elif [ "$have" != "$expected" ]; then fcheck "sha256 for $target ($archive) is $have, SHA256SUMS says $expected"; fi
      done
    fi
  fi
fi

[ "$fail" -eq 0 ] || exit 1
echo "versions agree: $cargo (PyPI $pypi, tag $tag, formula $(basename "$FORMULA"))"
