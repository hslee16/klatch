#!/usr/bin/env bash
# Check the layout of a release archive (#1236): one top directory named like the archive, holding `klatch` and
# `klatch-server` (both executable), LICENSE and THIRD-PARTY-LICENSES.txt, and nothing else. With --smoke it also
# unpacks the archive and runs release/check-packaged-server.sh on the klatch-server inside it.
#
#   release/check-archive.sh klatch-<version>-<target>.tar.gz [--smoke]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
archive="${1:?usage: check-archive.sh ARCHIVE [--smoke]}"
smoke="${2:-}"
name=$(basename "$archive" .tar.gz)

listing=$(tar -tzf "$archive" | sed 's|/$||' | sort)
want=$(printf '%s\n' "$name" "$name/LICENSE" "$name/THIRD-PARTY-LICENSES.txt" "$name/klatch" "$name/klatch-server" | sort)
if [ "$listing" != "$want" ]; then
  echo "check-archive: $archive does not hold exactly klatch, klatch-server, LICENSE and THIRD-PARTY-LICENSES.txt under $name/" >&2
  echo "found:" >&2
  printf '%s\n' "$listing" >&2
  exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/klatch-check-archive.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$archive" -C "$tmp"
for b in klatch klatch-server; do
  [ -x "$tmp/$name/$b" ] || { echo "check-archive: $b in $archive is not executable" >&2; exit 1; }
done
echo "archive layout ok: $name holds klatch, klatch-server, LICENSE, THIRD-PARTY-LICENSES.txt"

if [ "$smoke" = "--smoke" ]; then
  "$here/check-packaged-server.sh" "$tmp/$name/klatch-server"
elif [ -n "$smoke" ]; then
  echo "check-archive: unknown argument $smoke" >&2
  exit 2
fi
