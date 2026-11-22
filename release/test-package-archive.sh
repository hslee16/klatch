#!/usr/bin/env bash
# The release archive layout (#1236): every archive holds `klatch` and `klatch-server`, LICENSE and
# THIRD-PARTY-LICENSES.txt, and the one SBOM covers both binaries. These tests run release/package-archive.sh,
# release/check-archive.sh, release/check-packaged-server.sh and release/merge-sbom.py on stand-in binaries, so no build
# and no network is needed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-archive-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { printf '  ok  %s\n' "$1"; }
bad() { printf 'FAIL  %s\n' "$1" >&2; fails=1; }

VERSION=0.1.0-dev.2
TARGET=x86_64-unknown-linux-musl
NAME="klatch-$VERSION-$TARGET"

# stand-ins: klatch prints its version line; the server is a python program that behaves like the real one where the
# release checks look (version, help, a refused non-loopback bind, a daemon that makes its socket and removes it on TERM)
make_bins() { # dir [bad-bind]
  mkdir -p "$1"
  printf '#!/bin/sh\necho "klatch %s (protocol 1, store schema 36)"\n' "$VERSION" > "$1/klatch"
  cat > "$1/klatch-server" <<PY
#!/usr/bin/env python3
import os, signal, sys
args = sys.argv[1:]
if args in (["--version"], ["-V"]):
    print("klatch-server $VERSION")
elif args in (["--help"], ["-h"]):
    print("usage: klatch-server --version | http --listen 127.0.0.1:PORT | daemon --socket PATH | channel")
elif args[:1] == ["http"] and "--listen" in args:
    addr = args[args.index("--listen") + 1]
    if not addr.startswith("127.") and not ${2:-0}:
        sys.stderr.write("klatch-server: refusing to listen on " + addr + ": only 127.0.0.1 and ::1 are allowed\n")
        sys.exit(1)
    print("listening on " + addr)
elif args[:1] == ["daemon"] and "--socket" in args:
    sock = args[args.index("--socket") + 1]
    def stop(*_):
        os.unlink(sock)
        sys.exit(0)
    signal.signal(signal.SIGTERM, stop)
    open(sock, "w").close()
    while True:
        signal.pause()
else:
    sys.exit(2)
PY
  chmod 755 "$1/klatch" "$1/klatch-server"
  echo MIT > "$work/LICENSE"; echo notices > "$work/THIRD-PARTY-LICENSES.txt"
}

package() { # bin-dir out-dir
  "$here/package-archive.sh" --version "$VERSION" --target "$TARGET" --bin-dir "$1" --out-dir "$2" \
    --license "$work/LICENSE" --notices "$work/THIRD-PARTY-LICENSES.txt"
}

# 1. the archive lists both binaries and the licences, under one directory
make_bins "$work/bin"
package "$work/bin" "$work/out" >/dev/null
listing=$(tar -tzf "$work/out/$NAME.tar.gz" | sort)
want=$(printf '%s\n' "$NAME/" "$NAME/LICENSE" "$NAME/THIRD-PARTY-LICENSES.txt" "$NAME/klatch" "$NAME/klatch-server" | sort)
if [ "$listing" = "$want" ] || [ "$(printf '%s\n' "$listing" | grep -v '/$')" = "$(printf '%s\n' "$want" | grep -v '/$')" ]; then
  ok "the archive holds klatch, klatch-server, LICENSE and THIRD-PARTY-LICENSES.txt"
else
  bad "archive listing: $listing"
fi

# 2. a checksum file is written beside it and is right
if (cd "$work/out" && { sha256sum -c "$NAME.tar.gz.sha256" >/dev/null 2>&1 || shasum -a 256 -c "$NAME.tar.gz.sha256" >/dev/null 2>&1; }); then
  ok "the archive has a checksum file that verifies"
else
  bad "checksum file does not verify"
fi

# 3. the layout check passes on it, and says what it checked
if out=$("$here/check-archive.sh" "$work/out/$NAME.tar.gz" 2>&1) && grep -q "klatch-server" <<<"$out"; then
  ok "check-archive.sh accepts it and names klatch-server"
else
  bad "check-archive.sh: $out"
fi

# 4. an archive without klatch-server is refused (the state before #1236)
rm -rf "$work/old"; mkdir -p "$work/old/$NAME"
cp "$work/bin/klatch" "$work/old/$NAME/"
cp "$work/LICENSE" "$work/THIRD-PARTY-LICENSES.txt" "$work/old/$NAME/"
tar -czf "$work/old/$NAME.tar.gz" -C "$work/old" "$NAME"
if "$here/check-archive.sh" "$work/old/$NAME.tar.gz" >/dev/null 2>&1; then bad "an archive with no klatch-server passed"; else ok "an archive without klatch-server is refused"; fi

# 5. the packaging script refuses to build such an archive
rm -rf "$work/half"; mkdir -p "$work/half"; cp "$work/bin/klatch" "$work/half/"
if package "$work/half" "$work/halfout" >/dev/null 2>&1; then bad "packaged without klatch-server"; else ok "package-archive.sh refuses a missing klatch-server"; fi

# 6. a non-executable binary in an archive is refused
make_bins "$work/noexec-bin"; chmod 644 "$work/noexec-bin/klatch-server"
rm -rf "$work/noexec-stage"; mkdir -p "$work/noexec-stage/$NAME"
cp "$work/noexec-bin/klatch" "$work/noexec-bin/klatch-server" "$work/LICENSE" "$work/THIRD-PARTY-LICENSES.txt" "$work/noexec-stage/$NAME/"
tar -czf "$work/noexec-stage/$NAME.tar.gz" -C "$work/noexec-stage" "$NAME"
if "$here/check-archive.sh" "$work/noexec-stage/$NAME.tar.gz" >/dev/null 2>&1; then bad "a non-executable klatch-server passed"; else ok "a non-executable klatch-server is refused"; fi

# 7. --smoke unpacks it and runs the packaged-server check on the binary inside
if "$here/check-archive.sh" "$work/out/$NAME.tar.gz" --smoke >"$work/smoke.out" 2>&1; then ok "--smoke runs the packaged server check on the unpacked binary"; else bad "--smoke: $(cat "$work/smoke.out")"; fi

# 8. the packaged-server check: a server that accepts a non-loopback bind fails it
make_bins "$work/badbin" 1
if "$here/check-packaged-server.sh" "$work/badbin/klatch-server" >"$work/bad.out" 2>&1; then bad "a server that binds 0.0.0.0 passed"; elif grep -q "0.0.0.0" "$work/bad.out"; then ok "a server that accepts a non-loopback bind fails the check"; else bad "wrong failure: $(cat "$work/bad.out")"; fi

# 9. the packaged-server check: a binary with no --version fails it
printf '#!/bin/sh\nexit 2\n' > "$work/noversion"; chmod 755 "$work/noversion"
if "$here/check-packaged-server.sh" "$work/noversion" >/dev/null 2>&1; then bad "a server with no --version passed"; else ok "a server with no --version fails the check"; fi

# 10. the SBOM merge: components and dependencies of both binaries, once each
cat > "$work/klatch.cdx.json" <<'J'
{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,
 "metadata":{"component":{"type":"application","bom-ref":"klatch-bin","name":"klatch","version":"0.1.0-dev.2"}},
 "components":[{"type":"library","bom-ref":"serde","name":"serde","version":"1.0.0"},
               {"type":"library","bom-ref":"rusqlite","name":"rusqlite","version":"0.1.0"}],
 "dependencies":[{"ref":"klatch-bin","dependsOn":["serde","rusqlite"]},{"ref":"serde","dependsOn":[]},{"ref":"rusqlite","dependsOn":[]}]}
J
cat > "$work/server.cdx.json" <<'J'
{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,
 "metadata":{"component":{"type":"application","bom-ref":"server-bin","name":"klatch-server","version":"0.1.0-dev.2"}},
 "components":[{"type":"library","bom-ref":"serde","name":"serde","version":"1.0.0"},
               {"type":"library","bom-ref":"subtle","name":"subtle","version":"2.6.1"}],
 "dependencies":[{"ref":"server-bin","dependsOn":["serde","subtle"]},{"ref":"serde","dependsOn":[]},{"ref":"subtle","dependsOn":[]}]}
J
python3 -I "$here/merge-sbom.py" "$work/klatch.cdx.json" "$work/server.cdx.json" "$work/merged.cdx.json"
if python3 -I - "$work/merged.cdx.json" <<'PY'
import json, sys
b = json.load(open(sys.argv[1]))
names = sorted(c["name"] for c in b["components"])
assert names == ["klatch-server", "rusqlite", "serde", "subtle"], names
assert b["metadata"]["component"]["name"] == "klatch", b["metadata"]
refs = [d["ref"] for d in b["dependencies"]]
assert len(refs) == len(set(refs)), refs
assert {"klatch-bin", "server-bin", "serde", "subtle", "rusqlite"} <= set(refs), refs
assert b["specVersion"] == "1.5"
PY
then ok "the merged SBOM lists both binaries and every dependency once"; else bad "merged SBOM"; fi

# 11. a merge of two SBOMs that are not CycloneDX 1.5 is refused
echo '{"bomFormat":"other"}' > "$work/junk.json"
if python3 -I "$here/merge-sbom.py" "$work/klatch.cdx.json" "$work/junk.json" "$work/x.json" >/dev/null 2>&1; then bad "merged junk"; else ok "merging something that is not a CycloneDX 1.5 SBOM fails"; fi

exit "$fails"
