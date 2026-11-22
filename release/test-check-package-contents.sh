#!/usr/bin/env bash
# Tests for release/check-package-contents.sh (#1243) on fixture wheels and npm tarballs: a good one passes, one missing
# a file or holding a stray one fails.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-pkgcontents-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok  %s\n' "$1"; }
bad() { printf 'FAIL  %s\n' "$1" >&2; fails=1; }
V=0.1.0.dev2
check() { "$here/check-package-contents.sh" "$@" >/dev/null 2>&1; }

# wheel fixtures, built with python: SKIP names an entry to leave out, NOEXEC drops the server's executable bit
mkwheel() { # out-file [skip-entry] [noexec]
  python3 -I - "$1" "${2:-}" "${3:-}" "$V" <<'PY'
import sys, zipfile
out, skip, noexec, v = sys.argv[1:5]
d = "klatch-%s.data/scripts/" % v
entries = [d + "klatch", d + "klatch-server", "klatch-%s.dist-info/licenses/LICENSE" % v,
           "klatch-%s.dist-info/licenses/THIRD-PARTY-LICENSES.txt" % v, "klatch/__init__.py"]
with zipfile.ZipFile(out, "w") as zf:
    for e in entries:
        if e == skip:
            continue
        zi = zipfile.ZipInfo(e)
        mode = 0o755 if e.startswith(d) and not (noexec and e.endswith("-server")) else 0o644
        zi.external_attr = mode << 16
        zf.writestr(zi, "x")
PY
}
mkwheel "$work/klatch-$V-py3-none-any.whl"
if check wheel "$work/klatch-$V-py3-none-any.whl"; then ok "a good wheel passes"; else bad "a good wheel failed"; fi
mkdir "$work/a" "$work/b" "$work/c"
mkwheel "$work/a/klatch-$V-py3-none-any.whl" "klatch-$V.data/scripts/klatch-server"
if check wheel "$work/a/klatch-$V-py3-none-any.whl"; then bad "a wheel without klatch-server passed"; else ok "a wheel without klatch-server fails"; fi
mkwheel "$work/b/klatch-$V-py3-none-any.whl" "klatch-$V.dist-info/licenses/THIRD-PARTY-LICENSES.txt"
if check wheel "$work/b/klatch-$V-py3-none-any.whl"; then bad "a wheel without the notices passed"; else ok "a wheel without THIRD-PARTY-LICENSES.txt fails"; fi
mkwheel "$work/c/klatch-$V-py3-none-any.whl" "" noexec
if check wheel "$work/c/klatch-$V-py3-none-any.whl"; then bad "a non-executable klatch-server passed"; else ok "a non-executable klatch-server fails"; fi

# npm fixtures: extra arguments add a file under package/, a leading "-" removes one
mknpm() { # out-file extra-file...
  local out="$1"; shift
  local s; s=$(mktemp -d "$work/npm.XXXXXX")
  mkdir -p "$s/package/dist"
  echo x > "$s/package/dist/klatch.js"; echo x > "$s/package/README.md"; echo x > "$s/package/LICENSE"; echo '{}' > "$s/package/package.json"
  for f in "$@"; do
    case "$f" in
      -*) rm -f "$s/package/${f#-}" ;;
      *) echo x > "$s/package/$f" ;;
    esac
  done
  tar -czf "$out" -C "$s" package
}
mknpm "$work/good.tgz"
if check npm "$work/good.tgz"; then ok "a good npm tarball passes"; else bad "a good npm tarball failed"; fi
mknpm "$work/nolicense.tgz" -LICENSE
if check npm "$work/nolicense.tgz"; then bad "an npm tarball without LICENSE passed"; else ok "an npm tarball without LICENSE fails"; fi
mknpm "$work/nodist.tgz" -dist/klatch.js
if check npm "$work/nodist.tgz"; then bad "an npm tarball with an empty dist passed"; else ok "an npm tarball with an empty dist fails"; fi
mknpm "$work/binary.tgz" klatch
if check npm "$work/binary.tgz"; then bad "an npm tarball holding a binary passed"; else ok "an npm tarball holding a binary fails"; fi
mknpm "$work/stray.tgz" notes.txt
if check npm "$work/stray.tgz"; then bad "an npm tarball with a stray file passed"; else ok "an npm tarball with a stray file fails"; fi
if check bogus "$work/good.tgz"; then bad "an unknown kind passed"; else ok "an unknown kind is refused"; fi

exit "$fails"
