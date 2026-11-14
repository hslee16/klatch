#!/usr/bin/env bash
# The third-party licence notices (#428). Every binary, wheel and crate we ship carries THIRD-PARTY-LICENSES.txt, and
# SQLCipher's BSD licence requires its notice in a binary distribution. The file is the cargo-about output (the Rust
# dependencies) plus the licences of the C sources that sqlcipher-src and openssl-src compile in, which cargo-about
# does not read (they sit in those crates' source trees). This regenerates it from the dependency tree and fails if the
# committed copy is stale, if a dependency has no accepted licence, or if the vendored notices are missing.
# `--write` rewrites the committed file.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out="$here/THIRD-PARTY-LICENSES.txt"
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

crate_dir() {
  (cd "$here" && cargo metadata --format-version 1 --locked) | python3 -I -c '
import json, sys
name = sys.argv[1]
m = json.load(sys.stdin)
hits = [p["manifest_path"].rsplit("/", 1)[0] for p in m["packages"] if p["name"] == name]
if not hits:
    sys.exit("package " + name + " is not in the dependency tree")
print(hits[0])
' "$1"
}

sqlcipher_dir=$(crate_dir sqlcipher-src)
openssl_dir=$(crate_dir openssl-src)
vendored=(
  "SQLCipher (Zetetic LLC), compiled in by sqlcipher-src|$sqlcipher_dir/sqlcipher/LICENSE-sqlcipher"
  "libtomcrypt, carried by SQLCipher|$sqlcipher_dir/sqlcipher/LICENSE-libtomcrypt"
  "OpenSSL, compiled in by openssl-src|$openssl_dir/openssl/LICENSE.txt"
)
for entry in "${vendored[@]}"; do
  [ -s "${entry#*|}" ] || { echo "vendored licence file missing: ${entry#*|}" >&2; exit 1; }
done

{
  (cd "$here" && cargo about generate --locked --fail -c about.toml about.hbs)
  echo
  echo "VENDORED C SOURCES"
  echo
  echo "The licences of the C code that sqlcipher-src and openssl-src build into the binary. SQLite itself is in the public"
  echo "domain (the SQLite Blessing above)."
  for entry in "${vendored[@]}"; do
    echo
    echo "### ${entry%%|*}"
    echo
    cat "${entry#*|}"
  done
} > "$tmp"

if [ "${1:-}" = "--write" ]; then
  cp "$tmp" "$out"
  echo "wrote $out"
  exit 0
fi
[ -f "$out" ] || { echo "missing $out (run: proto/check-notices.sh --write)" >&2; exit 1; }
if ! diff -q "$tmp" "$out" >/dev/null; then
  echo "$out is stale (run: proto/check-notices.sh --write)" >&2
  exit 1
fi
# The actual text of each vendored licence must be in the file, not a paraphrase.
for entry in "${vendored[@]}"; do
  f="${entry#*|}"
  first=$(grep -m1 -v '^[[:space:]]*$' "$f")
  grep -qF -- "$first" "$out" || { echo "text of ${entry%%|*} is missing from $out" >&2; exit 1; }
done
grep -qi "zetetic" "$sqlcipher_dir/sqlcipher/LICENSE-sqlcipher" || { echo "unexpected SQLCipher licence text" >&2; exit 1; }
if grep -q '&quot;\|&amp;\|&lt;' "$out"; then
  echo "$out has HTML-escaped text" >&2
  exit 1
fi
echo "notices are current ($(grep -c '^### ' "$out") sections)"
