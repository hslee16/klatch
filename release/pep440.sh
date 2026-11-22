#!/bin/sh
# The one SemVer to PEP 440 mapping (#1234). A release has one number, spelled two ways: SemVer in the crate, the npm
# package, the archive names and `klatch --version`; PEP 440 in the PyPI package, the wheel and the git tag.
#
#   SemVer          PEP 440 and tag
#   0.2.0-alpha.1   0.2.0a1      v0.2.0a1
#   0.2.0-beta.1    0.2.0b1      v0.2.0b1
#   0.2.0-rc.1      0.2.0rc1     v0.2.0rc1
#   0.1.0-dev.2     0.1.0.dev2   v0.1.0.dev2
#   0.2.0           0.2.0        v0.2.0
#
# Source it (`. release/pep440.sh`), do not run it. It is POSIX sh, so check-versions.sh, the release workflow and the
# tests all use the same functions. install/install.sh is downloaded alone and cannot source this file, so it carries a
# copy of the block below; release/test-pep440.sh fails when the two differ. Change the block here, then copy it.
#
# A version or label that is not in the table is refused (non-zero status, nothing printed), never passed through.

# BEGIN pep440
# SemVer 0.2.0-alpha.1 to PEP 440 0.2.0a1 (a stable X.Y.Z is unchanged)
semver_to_pep440() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc|dev)\.[0-9]+)?$' || return 1
  printf '%s\n' "$1" | sed -e 's/-alpha\.\([0-9][0-9]*\)$/a\1/' -e 's/-beta\.\([0-9][0-9]*\)$/b\1/' \
    -e 's/-rc\.\([0-9][0-9]*\)$/rc\1/' -e 's/-dev\.\([0-9][0-9]*\)$/.dev\1/'
}
# PEP 440 0.2.0a1 back to SemVer 0.2.0-alpha.1
pep440_to_semver() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+((a|b|rc)[0-9]+|\.dev[0-9]+)?$' || return 1
  printf '%s\n' "$1" | sed -e 's/\.dev\([0-9][0-9]*\)$/-dev.\1/' -e 's/\([0-9]\)a\([0-9][0-9]*\)$/\1-alpha.\2/' \
    -e 's/\([0-9]\)b\([0-9][0-9]*\)$/\1-beta.\2/' -e 's/\([0-9]\)rc\([0-9][0-9]*\)$/\1-rc.\2/'
}
# the git tag of a SemVer release: v0.2.0a1
release_tag() {
  _p=$(semver_to_pep440 "$1") || return 1
  printf 'v%s\n' "$_p"
}
# the SemVer version a tag names: v0.2.0a1 gives 0.2.0-alpha.1 (a tag needs its v)
semver_of_tag() {
  case "$1" in v?*) pep440_to_semver "${1#v}" ;; *) return 1 ;; esac
}
# tag_matches TAG SEMVER: the tag is v plus the PEP 440 spelling, or v plus the SemVer spelling
tag_matches() {
  _p=$(semver_to_pep440 "$2") || return 1
  [ "$1" = "v$_p" ] || [ "$1" = "v$2" ]
}
# the release archive of a SemVer version for a target triple: klatch-0.2.0-alpha.1-aarch64-apple-darwin.tar.gz
archive_name() {
  semver_to_pep440 "$1" >/dev/null || return 1
  printf 'klatch-%s-%s.tar.gz\n' "$1" "$2"
}
# a SemVer version with a pre-release part
is_prerelease() {
  case "$1" in *-*) return 0 ;; *) return 1 ;; esac
}
# END pep440
