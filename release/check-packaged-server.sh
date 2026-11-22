#!/usr/bin/env bash
# The checks a release runs on the klatch-server binary it ships (#1236). The claim being kept is the one in SECURITY.md:
# klatch-server opens loopback listeners only, starts no program and makes no outbound connection. This checks the
# packaged binary's side of it: it names its version, it prints help, it REFUSES a non-loopback bind, and a daemon starts
# on a Unix socket in a throwaway home and stops on SIGTERM, removing the socket. It never touches a real store or key.
#
#   release/check-packaged-server.sh path/to/klatch-server [expected-version]
set -euo pipefail
bin="${1:?usage: check-packaged-server.sh BINARY [VERSION]}"
want_version="${2:-}"
fail() { echo "check-packaged-server: $*" >&2; exit 1; }
[ -x "$bin" ] || fail "$bin is not an executable file"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/ks.XXXXXX")
pid=""
cleanup() {
  [ -z "$pid" ] || kill -9 "$pid" 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/home" "$tmp/sock"
chmod 700 "$tmp" "$tmp/home" "$tmp/sock"
server() { env -i PATH="/usr/bin:/bin" HOME="$tmp/home" KLATCH_HOME="$tmp/home" KLATCH_KEY_DIR="$tmp/home/keys" "$bin" "$@"; }

version=$(server --version) || fail "--version failed"
case "$version" in
  "klatch-server "*) ;;
  *) fail "--version prints '$version', wanted 'klatch-server <version>'" ;;
esac
if [ -n "$want_version" ] && [ "$version" != "klatch-server $want_version" ]; then
  fail "--version prints '$version', wanted 'klatch-server $want_version'"
fi
echo "  ok  $version"

help=$(server --help) || fail "--help failed"
case "$help" in *daemon*http* | *http*daemon*) ;; *) fail "--help does not name the daemon and http subcommands" ;; esac
echo "  ok  --help"

# a non-loopback bind must be refused, on every wildcard form
for addr in 0.0.0.0:0 "[::]:0"; do
  if out=$(server http --listen "$addr" 2>&1); then
    fail "http --listen $addr was accepted (a non-loopback bind must be refused): $out"
  fi
  case "$out" in *refus*) ;; *) fail "http --listen $addr failed, but not with a refusal: $out" ;; esac
done
echo "  ok  http --listen 0.0.0.0:0 and [::]:0 are refused"

# a daemon starts and stops
sock="$tmp/sock/d.sock"
# not through the `server` function: its subshell would hide the binary's own pid from kill
env -i PATH="/usr/bin:/bin" HOME="$tmp/home" KLATCH_HOME="$tmp/home" KLATCH_KEY_DIR="$tmp/home/keys" \
  "$bin" daemon --socket "$sock" >"$tmp/daemon.log" 2>&1 &
pid=$!
tries=0
while [ ! -e "$sock" ]; do
  kill -0 "$pid" 2>/dev/null || fail "the daemon exited at start: $(cat "$tmp/daemon.log")"
  tries=$((tries + 1))
  [ "$tries" -lt 300 ] || fail "the daemon made no socket in 30 seconds: $(cat "$tmp/daemon.log")"
  sleep 0.1
done
kill -TERM "$pid"
tries=0
while kill -0 "$pid" 2>/dev/null; do
  tries=$((tries + 1))
  [ "$tries" -lt 300 ] || fail "the daemon did not stop on SIGTERM within 30 seconds"
  sleep 0.1
done
wait "$pid" 2>/dev/null || true
pid=""
[ ! -e "$sock" ] || fail "the daemon left its socket behind"
echo "  ok  daemon --socket started and stopped on SIGTERM, socket removed"
echo "packaged klatch-server ok"
