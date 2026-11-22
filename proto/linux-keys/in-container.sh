#!/usr/bin/env bash
# Runs inside a test container (see run.sh). $1 is the variant, $2 is 1 to measure latency too.
set -euo pipefail
variant="$1"
bench="${2:-0}"

if { [ "$variant" = bus ] || [ "$variant" = locked ]; } && [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  # A private session bus for this run only; the login collection is made and unlocked with a throwaway password,
  # which is what a desktop login does with the user's password.
  exec dbus-run-session -- bash "$0" "$@"
fi
if [ "$variant" = bus ]; then
  echo -n 'throwaway-login-password' | gnome-keyring-daemon --unlock --components=secrets > /dev/null
fi

if [ "$variant" = locked ]; then
  # the keyring daemon is running but its login collection was never unlocked and there is no display to ask on
  gnome-keyring-daemon --start --components=secrets > /dev/null 2>&1
  cd /src/proto
  cargo test -q --locked -p klatch --test linux_locked -- --ignored a_locked_keyring_fails a_real_keyring_with_no_login
  # the login collection exists and is then locked again (a desktop session that ended), on a bus of its own
  dbus-run-session -- bash -c '
    echo -n throwaway-login-password | gnome-keyring-daemon --unlock --components=secrets > /dev/null 2>&1
    echo -n 0000 | secret-tool store --label seed service klatch-seed name seed
    busctl --user call org.freedesktop.secrets /org/freedesktop/secrets org.freedesktop.Secret.Service Lock \
      ao 1 /org/freedesktop/secrets/collection/login > /dev/null
    cd /src/proto
    DISPLAY= cargo test -q --locked -p klatch --test linux_locked -- --ignored a_real_locked_login 2>&1
  '
  exit 0
fi

cd /src/proto
cargo test -q --locked -p klatch --lib keys::
cargo test -q --locked -p klatch --test encrypted --test linux_headless
if [ "$variant" = bus ]; then
  cargo test -q --locked -p klatch --test linux_keyring -- --ignored
fi
if [ "$bench" = 1 ]; then
  bash /src/proto/linux-keys/measure.sh "$variant"
fi
