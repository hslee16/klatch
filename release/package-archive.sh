#!/usr/bin/env bash
# Build one release archive (#1236): klatch-<version>-<target>.tar.gz holding `klatch`, `klatch-server`, LICENSE and
# THIRD-PARTY-LICENSES.txt under one directory, and its .sha256 file. Both binaries are required: an archive with only
# one of them is an error. build-release.yml calls this, so what CI packs is what release/test-package-archive.sh tests.
#
#   release/package-archive.sh --version V --target T --bin-dir DIR --out-dir OUT --license LICENSE --notices FILE
#
# DIR holds the two (already stripped) binaries. OUT receives the archive and the checksum file.
set -euo pipefail

version=""; target=""; bin_dir=""; out_dir=""; license=""; notices=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) version="$2"; shift 2 ;;
    --target) target="$2"; shift 2 ;;
    --bin-dir) bin_dir="$2"; shift 2 ;;
    --out-dir) out_dir="$2"; shift 2 ;;
    --license) license="$2"; shift 2 ;;
    --notices) notices="$2"; shift 2 ;;
    *) echo "package-archive: unknown argument $1" >&2; exit 2 ;;
  esac
done
for v in version target bin_dir out_dir license notices; do
  [ -n "${!v}" ] || { echo "package-archive: --${v//_/-} is required" >&2; exit 2; }
done
for f in "$bin_dir/klatch" "$bin_dir/klatch-server" "$license" "$notices"; do
  [ -f "$f" ] || { echo "package-archive: missing $f" >&2; exit 1; }
done

name="klatch-$version-$target"
stage=$(mktemp -d "${TMPDIR:-/tmp}/klatch-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name" "$out_dir"
install -m 0755 "$bin_dir/klatch" "$stage/$name/klatch"
install -m 0755 "$bin_dir/klatch-server" "$stage/$name/klatch-server"
install -m 0644 "$license" "$stage/$name/LICENSE"
install -m 0644 "$notices" "$stage/$name/THIRD-PARTY-LICENSES.txt"
tar -czf "$out_dir/$name.tar.gz" -C "$stage" "$name"

python3 -I - "$out_dir/$name.tar.gz" > "$out_dir/$name.tar.gz.sha256" <<'PY'
import hashlib, os, sys
p = sys.argv[1]
print(hashlib.sha256(open(p, "rb").read()).hexdigest() + "  " + os.path.basename(p))
PY
cat "$out_dir/$name.tar.gz.sha256"
