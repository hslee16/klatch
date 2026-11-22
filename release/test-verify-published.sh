#!/usr/bin/env bash
# release/verify-published.sh must check each registry, the GitHub release, the tap formula and a clean install from the public
# side, report PASS or FAIL for each, and exit non-zero when any fails (#1249). No network, no credential: curl, gh, npm and
# python3 (venv and pip only) are fakes that read local files standing in for the registries.
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-verify.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
REAL_PY="$(command -v python3)"
web="$work/web"; bin="$work/bin"; mkdir -p "$bin" "$work/tmp"
key() { printf '%s' "$*" | tr -c 'A-Za-z0-9\n' '_'; }

# curl: the last argument is the URL; a file named for it stands in for the answer, no file is a 404
cat > "$bin/curl" <<'SH'
#!/bin/sh
for a; do url="$a"; done
echo "curl $url" >> "$FAKE_LOG"
f="$FAKE_WEB/curl_$(printf '%s' "$url" | tr -c 'A-Za-z0-9\n' '_')"
[ -f "$f" ] || exit 22
cat "$f"
SH
# gh: only "gh api <path>" is allowed
cat > "$bin/gh" <<'SH'
#!/bin/sh
echo "gh $*" >> "$FAKE_LOG"
[ "$1" = api ] || exit 98
f="$FAKE_WEB/gh_$(printf '%s' "$2" | tr -c 'A-Za-z0-9\n' '_')"
[ -f "$f" ] || exit 1
cat "$f"
SH
cat > "$bin/npm" <<'SH'
#!/bin/sh
echo "npm $*" >> "$FAKE_LOG"
[ "$1" = view ] || exit 98
shift
f="$FAKE_WEB/npm_$(printf '%s' "$*" | tr -c 'A-Za-z0-9\n' '_')"
[ -f "$f" ] || exit 1
cat "$f"
SH
# python3: venv creation and pip are faked; everything else runs the real interpreter
cat > "$bin/python3" <<'SH'
#!/bin/sh
if [ "$1" = -m ] && [ "$2" = venv ]; then
  d="$3"; mkdir -p "$d/bin"
  cat > "$d/bin/pip" <<'PIP'
#!/bin/sh
echo "pip $*" >> "$FAKE_LOG"
case "$1" in
  install)
    d="$(cd "$(dirname "$0")/.." && pwd)"
    printf '#!/bin/sh\necho "klatch %s"\n' "$(cat "$FAKE_WEB/installed_version")" > "$d/bin/klatch"
    if [ -f "$FAKE_WEB/has_server" ]; then printf '#!/bin/sh\necho "usage: klatch-server http --listen 127.0.0.1:PORT"\n' > "$d/bin/klatch-server"; fi
    chmod +x "$d/bin/"* ;;
esac
PIP
  chmod +x "$d/bin/pip"
  exit 0
fi
if [ "$1" = -m ] && [ "$2" = pip ]; then
  echo "pip $*" >> "$FAKE_LOG"
  [ "$3" = index ] || exit 98
  f="$FAKE_WEB/pip_index"; [ -f "$f" ] || exit 1
  cat "$f"; exit 0
fi
exec "$REAL_PY" "$@"
SH
chmod +x "$bin"/*

v=0.2.0-alpha.1; tag=v0.2.0a1; pv=0.2.0a1; base=https://github.com/hslee16/klatch/releases/download/$tag
fresh() {
  rm -rf "$web" "$work/tmp" "$work/log"; mkdir -p "$web" "$work/tmp"; : > "$work/log"
  printf 'klatch (%s)\nAvailable versions: %s, 0.1.0.dev2\n' "$pv" "$pv" > "$web/pip_index"
  printf '%s\n' "$v" > "$web/npm_$(key "@klatch/client@$v" version)"
  printf '%s\n' "$v" > "$web/npm_$(key "@klatch/client" dist-tags.latest)"
  printf 'https://slsa.dev/provenance/v1\n' > "$web/npm_$(key "@klatch/client@$v" dist.attestations.provenance.predicateType)"
  printf '{"version":{"num":"%s","yanked":false}}\n' "$v" > "$web/curl_$(key "https://crates.io/api/v1/crates/klatch/$v")"
  assets='"install.sh","SHA256SUMS","SHA256SUMS.sigstore.json"'
  for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    assets="$assets,\"klatch-$v-$t.tar.gz\",\"klatch-$v-$t.cdx.json\""
  done
  python3 - "$assets" "$tag" > "$web/gh_$(key "repos/hslee16/klatch/releases/tags/$tag")" <<'PY'
import json, sys
names = [n.strip('"') for n in sys.argv[1].split(',')]
print(json.dumps({"tag_name": sys.argv[2], "draft": False, "prerelease": True, "assets": [{"name": n} for n in names]}))
PY
  {
    echo 'class Klatch < Formula'
    echo "  version \"$v\""
    for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
      echo "  url \"$base/klatch-$v-$t.tar.gz\""
    done
    echo 'end'
  } > "$web/curl_$(key "https://raw.githubusercontent.com/hslee16/homebrew-klatch/main/Formula/klatch.rb")"
  printf '%s\n' "$v" > "$web/installed_version"
  : > "$web/has_server"
}
run() { env PATH="$bin:$PATH" FAKE_WEB="$web" FAKE_LOG="$work/log" REAL_PY="$REAL_PY" TMPDIR="$work/tmp" "$here/verify-published.sh" "$@" > "$work/out" 2> "$work/err"; }
status() { grep -E "^(PASS|FAIL|SKIP) +$1" "$work/out" | head -1 | cut -d' ' -f1; }

fresh
if run $v; then ok "exits 0 when everything is in place"; else bad "good case failed: $(cat "$work/out" "$work/err")"; fi
for label in pypi npm-version npm-latest npm-provenance crates github-release github-assets tap-formula install-klatch install-klatch-server; do
  [ "$(status "$label")" = PASS ] && ok "PASS $label" || bad "$label is '$(status "$label")'"
done
[ -z "$(ls -A "$work/tmp")" ] && ok "the temporary install prefix is removed" || bad "left behind: $(ls "$work/tmp")"
grep -q '^gh api' "$work/log" && ! grep -q '^gh [^a]' "$work/log" && ok "gh was used for api calls only (REST)" || bad "gh calls: $(grep '^gh' "$work/log")"
grep -q 'graphql' "$work/log" && bad "GraphQL was used" || ok "no GraphQL"
grep -q 'pip install' "$work/log" && grep -q -- '--pre' "$work/log" && grep -q "klatch==$pv" "$work/log" && ok "pip installs klatch==$pv with --pre" || bad "pip log: $(grep pip "$work/log")"
grep -q 'sleep' "$here/verify-published.sh" && bad "the script sleeps" || ok "no sleep in the script (#1101)"

# the report file
fresh
if run --report "$work/report.md" $v && grep -q 'PASS' "$work/report.md" && grep -q "$v" "$work/report.md"; then ok "--report writes a comment for the epic"; else bad "report: $(cat "$work/report.md" 2>&1)"; fi

# failures: each one is named, others still run, exit non-zero
failcase() { # description label then a mutation in the caller
  if run $v; then bad "$1: exit 0"; else ok "$1: exits non-zero"; fi
  [ "$(status "$2")" = FAIL ] && ok "$1: $2 is FAIL" || bad "$1: $2 is '$(status "$2")'"
  [ "$(status pypi)" != "" ] && [ "$(status tap-formula)" != "" ] && ok "$1: the other checks still ran" || bad "$1: checks stopped early"
}
fresh; printf 'klatch (0.1.0.dev2)\nAvailable versions: 0.1.0.dev2\n' > "$web/pip_index"; failcase "PyPI lacks the version" pypi
fresh; rm "$web/npm_$(key "@klatch/client@$v" version)"; failcase "npm lacks the version" npm-version
fresh; printf '0.1.0-dev.2\n' > "$web/npm_$(key "@klatch/client" dist-tags.latest)"; failcase "npm latest is another version" npm-latest
fresh; rm "$web/npm_$(key "@klatch/client@$v" dist.attestations.provenance.predicateType)"; failcase "npm has no provenance" npm-provenance
fresh; printf '{"version":{"num":"%s","yanked":true}}\n' "$v" > "$web/curl_$(key "https://crates.io/api/v1/crates/klatch/$v")"; failcase "the crate is yanked" crates
fresh; rm "$web/curl_$(key "https://crates.io/api/v1/crates/klatch/$v")"; failcase "the crate is not on crates.io" crates
fresh; rm "$web/gh_$(key "repos/hslee16/klatch/releases/tags/$tag")"; failcase "the GitHub release is missing" github-release
fresh; python3 - "$web/gh_$(key "repos/hslee16/klatch/releases/tags/$tag")" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p)); d["assets"] = [a for a in d["assets"] if not a["name"].endswith("x86_64-unknown-linux-musl.cdx.json")]
json.dump(d, open(p, "w"))
PY
failcase "an SBOM is missing" github-assets
grep -q 'x86_64-unknown-linux-musl.cdx.json' "$work/out" && ok "the missing asset is named" || bad "missing asset not named"
fresh; sed -i.bak 's/^  version ".*"$/  version "0.1.0-dev.2"/' "$web/curl_$(key "https://raw.githubusercontent.com/hslee16/homebrew-klatch/main/Formula/klatch.rb")"; rm -f "$web"/*.bak; failcase "the tap formula is not bumped" tap-formula
fresh; printf '0.1.0-dev.2\n' > "$web/installed_version"; failcase "the installed klatch prints another version" install-klatch
fresh; rm "$web/has_server"; failcase "the install has no klatch-server" install-klatch-server

# --skip-install, and bad input
fresh
if run --skip-install $v && [ "$(status install-klatch)" = SKIP ] && ! grep -q 'pip install' "$work/log"; then ok "--skip-install skips the install and exits 0"; else bad "skip-install: $(cat "$work/out")"; fi
if run 0.2 > /dev/null 2>&1; then bad "a bad version passed"; else ok "a version that is not SemVer fails"; fi
if run > /dev/null 2>&1; then bad "no version passed"; else ok "no version fails"; fi
exit "$fails"
