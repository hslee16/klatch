#!/usr/bin/env bash
# Run the session discovery tests against a real Linux process table in a Debian container (#171).
#
# Usage: proto/linux-discovery/run.sh [clean]
#
# Reuses the image of proto/linux-keys (Debian 12, Rust, procps). The repository is mounted read-only, the build
# cache lives in two named volumes, and no home directory, credential or environment variable of yours enters the
# container. The tests use a fixture registry in a temp directory; the real ~/.claude/sessions is never read.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
img=klatch-171-discovery:base
vt=klatch-171-discovery-target
vr=klatch-171-discovery-registry
if [ "${1:-}" = clean ]; then
  docker image rm -f "$img" 2> /dev/null || true
  docker volume rm -f "$vt" "$vr" > /dev/null
  exit 0
fi
command -v docker > /dev/null || { echo "docker is required" >&2; exit 1; }

docker build -q --target base -t "$img" -f "$here/../linux-keys/Dockerfile" "$here/../linux-keys" > /dev/null
docker run --rm \
  -v "$root:/src:ro" \
  -v "$vt:/home/tester/target" \
  -v "$vr:/home/tester/.cargo/registry" \
  "$img" cargo test --locked -p klatch --lib harness::
