#!/usr/bin/env bash
# The one SemVer to PEP 440 mapping (release/pep440.sh, #1234) for every kind of release: the PyPI version, the tag, the
# archive name, the round trip, the refusals, the ordering rules, and that install/install.sh carries an identical copy.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
# shellcheck source=release/pep440.sh
. "$here/pep440.sh"
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
eq() { # description, expected, actual
  if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: expected '$2', got '$3'"; fi
}

# semver, pep440, tag
table="0.2.0-alpha.1 0.2.0a1 v0.2.0a1
0.2.0-alpha.12 0.2.0a12 v0.2.0a12
0.2.0-beta.3 0.2.0b3 v0.2.0b3
0.2.0-rc.2 0.2.0rc2 v0.2.0rc2
0.1.0-dev.2 0.1.0.dev2 v0.1.0.dev2
0.1.0-dev.0 0.1.0.dev0 v0.1.0.dev0
0.2.0 0.2.0 v0.2.0
1.10.3 1.10.3 v1.10.3"
while read -r semver pep tag; do
  eq "$semver to PyPI" "$pep" "$(semver_to_pep440 "$semver")"
  eq "$semver to the tag" "$tag" "$(release_tag "$semver")"
  eq "$pep back to SemVer" "$semver" "$(pep440_to_semver "$pep")"
  eq "tag $tag back to SemVer" "$semver" "$(semver_of_tag "$tag")"
  eq "archive name for $semver" "klatch-$semver-x86_64-apple-darwin.tar.gz" "$(archive_name "$semver" x86_64-apple-darwin)"
  if tag_matches "$tag" "$semver"; then ok "tag $tag matches $semver"; else bad "tag $tag does not match $semver"; fi
done <<EOF2
$table
EOF2
if tag_matches "v0.2.0-alpha.1" "0.2.0-alpha.1"; then ok "the SemVer spelling of a tag is accepted too"; else bad "SemVer tag spelling refused"; fi
if tag_matches "v0.2.0a2" "0.2.0-alpha.1"; then bad "a tag for another number matched"; else ok "a tag for another number does not match"; fi
if tag_matches "0.2.0a1" "0.2.0-alpha.1"; then bad "a tag without v matched"; else ok "a tag needs its v"; fi

for v in 0.2.0-alpha.1 0.2.0-beta.1 0.2.0-rc.1 0.1.0-dev.2; do
  if is_prerelease "$v"; then ok "$v is a pre-release"; else bad "$v should be a pre-release"; fi
done
if is_prerelease 0.2.0; then bad "0.2.0 is not a pre-release"; else ok "0.2.0 is not a pre-release"; fi

# a label that is not mapped is refused, never passed through (a pass-through is how the five copies drifted)
for v in 0.2.0-gamma.1 0.2.0-alpha 0.2.0-alpha.x 0.2 v0.2.0 0.2.0-alpha.1.2 0.2.0+build 0.2.0-ALPHA.1 ""; do
  if semver_to_pep440 "$v" >/dev/null 2>&1; then bad "accepted '$v'"; else ok "refuses '$v'"; fi
done
for v in 0.2.0-alpha.1 0.2.0.a1 0.2.0c1 0.2.0.post1 0.2.0a; do
  if pep440_to_semver "$v" >/dev/null 2>&1; then bad "accepted PEP 440 '$v'"; else ok "refuses PEP 440 '$v'"; fi
done

# Ordering. Sort keys: X.Y.Z padded, then the pre-release rank and number; a stable release sorts above its pre-releases.
# shellcheck disable=SC2329  # called through below()
semver_key() { # SemVer compares labels as ASCII, so alpha < beta < dev < rc
  local core pre label n a b c
  core="${1%%-*}"; pre=""; [ "$core" = "$1" ] || pre="${1#*-}"
  IFS=. read -r a b c <<<"$core"
  if [ -z "$pre" ]; then printf '%05d%05d%05d~' "$a" "$b" "$c"; return; fi
  label="${pre%%.*}"; n="${pre#*.}"
  printf '%05d%05d%05d.%s.%09d' "$a" "$b" "$c" "$label" "$n"
}
# shellcheck disable=SC2329  # called through below()
pep_key() { # PEP 440 ranks dev < a < b < rc < final
  local s="$1" core rank n a b c
  case "$s" in
    *.dev*) core="${s%%.dev*}"; rank=0; n="${s##*.dev}" ;;
    *rc*) core="${s%%rc*}"; rank=3; n="${s##*rc}" ;;
    *a[0-9]*) core="${s%%a*}"; rank=1; n="${s##*a}" ;;
    *b[0-9]*) core="${s%%b*}"; rank=2; n="${s##*b}" ;;
    *) core="$s"; rank=4; n=0 ;;
  esac
  IFS=. read -r a b c <<<"$core"
  printf '%05d%05d%05d%d%09d' "$a" "$b" "$c" "$rank" "$n"
}
below() { # description, key function, lower, higher
  local lo hi
  lo="$($2 "$3")"; hi="$($2 "$4")"
  if [[ "$lo" < "$hi" ]]; then ok "$1"; else bad "$1 ($3 should sort below $4)"; fi
}
below "SemVer: 0.2.0-alpha.1 sorts above the published 0.1.0-dev.2" semver_key 0.1.0-dev.2 0.2.0-alpha.1
below "PEP 440: 0.2.0a1 sorts above the published 0.1.0.dev2" pep_key 0.1.0.dev2 0.2.0a1
below "SemVer: the trap, 0.1.0-alpha.1 sorts BELOW 0.1.0-dev.2 (never use it)" semver_key 0.1.0-alpha.1 0.1.0-dev.2
below "SemVer: alpha.1 < alpha.2" semver_key 0.2.0-alpha.1 0.2.0-alpha.2
below "SemVer: alpha.2 < alpha.10 (numeric)" semver_key 0.2.0-alpha.2 0.2.0-alpha.10
below "SemVer: alpha < beta" semver_key 0.2.0-alpha.2 0.2.0-beta.1
below "SemVer: beta < rc" semver_key 0.2.0-beta.2 0.2.0-rc.1
below "SemVer: rc < final" semver_key 0.2.0-rc.1 0.2.0
below "SemVer: 0.2.0 < 0.2.1-alpha.1" semver_key 0.2.0 0.2.1-alpha.1
below "PEP 440: a < b" pep_key 0.2.0a2 0.2.0b1
below "PEP 440: b < rc" pep_key 0.2.0b2 0.2.0rc1
below "PEP 440: rc < final" pep_key 0.2.0rc1 0.2.0
below "PEP 440: a2 < a10" pep_key 0.2.0a2 0.2.0a10
# the same X.Y.Z with a dev and an alpha is ordered differently by the two schemes, which is why a line uses one kind
below "SemVer: -dev.1 sorts above -alpha.1 of the same X.Y.Z (so do not mix them)" semver_key 0.2.0-alpha.1 0.2.0-dev.1
below "PEP 440: .dev1 sorts below a1 of the same X.Y.Z (the schemes disagree)" pep_key 0.2.0.dev1 0.2.0a1

# install/install.sh must carry the very same functions: it is downloaded alone and cannot source this file
extract() { sed -n '/^# BEGIN pep440$/,/^# END pep440$/p' "$1"; }
canonical="$(extract "$here/pep440.sh")"
copy="$(extract "$ROOT/install/install.sh")"
if [ -n "$canonical" ] && [ "$canonical" = "$copy" ]; then ok "install/install.sh carries an identical copy of the mapping"; else bad "install/install.sh's pep440 block differs from release/pep440.sh (copy the block between the BEGIN and END lines)"; fi

# the mapping is spelled nowhere else: the five sites of #1234 call it
stray="$(cd "$ROOT" && grep -rIlE "sed .*(-dev\\\\\.|\\\\\.dev)" .github release install proto/scripts 2>/dev/null | grep -v -e '^release/pep440.sh$' -e '^install/install.sh$' -e '^release/test-' || true)"
if [ -z "$stray" ]; then ok "no other script spells the dev mapping"; else bad "the mapping is spelled again in: $stray"; fi
exit "$fails"
