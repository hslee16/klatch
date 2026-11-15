#!/usr/bin/env bash
# Build the Linux key custody test images and run the key tests inside them (epic #387, #381).
#
# Usage: proto/linux-keys/run.sh [bus|locked|nobus|bare|all] [--bench]
#   bus    a session bus and an unlocked gnome-keyring login collection (the Secret Service backend, for real)
#   locked the same bus, but the keyring is locked and there is no display to ask on
#   nobus  secret-tool installed, no session bus (a headless box: the key file fallback)
#   bare   no secret-tool at all
#   all    the four, in that order (default)
#   --bench  also measure key fetch and hook latency (#384) and print the numbers
#   clean    remove the images and cache volumes this made
#
# Docker only. The repository is mounted read-only; the build cache lives in two named volumes so a rerun is quick.
# No home directory and no environment variable of yours enters a container.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
variants=()
bench=0
for arg in "$@"; do
  case "$arg" in
    bus | locked | nobus | bare) variants+=("$arg") ;;
    all) variants=(bus locked nobus bare) ;;
    --bench) bench=1 ;;
    clean)
      docker image rm -f klatch-linux-keys:bus klatch-linux-keys:nobus klatch-linux-keys:bare klatch-linux-keys:base 2> /dev/null || true
      docker volume rm -f klatch-linux-keys-target klatch-linux-keys-registry > /dev/null
      exit 0
      ;;
    -h | --help)
      sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done
[ "${#variants[@]}" -gt 0 ] || variants=(bus locked nobus bare)

command -v docker > /dev/null || { echo "docker is required" >&2; exit 1; }

failed=0
for v in "${variants[@]}"; do
  image="$v"; [ "$v" = locked ] && image=bus
  echo "=== building image klatch-linux-keys:$image"
  docker build -q --target "$image" -t "klatch-linux-keys:$image" -f "$here/Dockerfile" "$here" > /dev/null
  echo "=== running the key tests: $v"
  # the container user owns the volumes' mount points, so a first run can write the caches
  if ! docker run --rm \
    -v "$root:/src:ro" \
    -v klatch-linux-keys-target:/home/tester/target \
    -v klatch-linux-keys-registry:/home/tester/.cargo/registry \
    "klatch-linux-keys:$image" bash /src/proto/linux-keys/in-container.sh "$v" "$bench"; then
    echo "=== FAILED: $v" >&2
    failed=1
  fi
done
if [ "$failed" -eq 0 ]; then echo "=== all variants passed"; else exit 1; fi
