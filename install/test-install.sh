#!/bin/sh
# Tests for install.sh (#429). It builds a fake release on disk (an archive holding a stand-in `klatch`, and a
# SHA256SUMS) and points the installer at it with KLATCH_INSTALL_BASE_URL, so nothing touches the network.
#   install/test-install.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
installer="$here/install.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/klatch-install-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
fails=0
pass() { printf '  ok    %s\n' "$1"; }
fail() { printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); }

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# a fake release for tag v0.1.0.dev1 (files are named for version 0.1.0-dev.1) and one target
make_release() { # dir target
  rel="$1/v0.1.0.dev1"; target="$2"; name="klatch-0.1.0-dev.1-$target"
  mkdir -p "$rel" "$1/stage/$name"
  printf '#!/bin/sh\necho "klatch 0.1.0-dev.1"\n' > "$1/stage/$name/klatch"
  chmod 755 "$1/stage/$name/klatch"
  echo MIT > "$1/stage/$name/LICENSE"; echo notices > "$1/stage/$name/THIRD-PARTY-LICENSES.txt"
  tar -czf "$rel/$name.tar.gz" -C "$1/stage" "$name"
  printf '%s  %s\n' "$(sha256 "$rel/$name.tar.gz")" "$name.tar.gz" > "$rel/SHA256SUMS"
}

run_install() { # dir [env assignments...]; installs into $dir/bin
  d="$1"; shift
  env "$@" KLATCH_INSTALL_BASE_URL="file://$d" KLATCH_VERSION=v0.1.0.dev1 KLATCH_INSTALL_DIR="$d/bin" \
    KLATCH_OS=Linux KLATCH_ARCH=x86_64 sh "$installer" >"$d/out" 2>&1
}

# 1. the happy path
d="$work/t1"; mkdir -p "$d"; make_release "$d" x86_64-unknown-linux-gnu
if run_install "$d" && [ -x "$d/bin/klatch" ] && [ "$("$d/bin/klatch")" = "klatch 0.1.0-dev.1" ]; then pass "installs the verified binary and it runs"; else fail "happy path: $(cat "$d/out")"; fi
if grep -q "checksum" "$d/out"; then pass "says it checked the checksum"; else fail "no checksum message"; fi

# 2. a tampered archive is refused and nothing is installed
d="$work/t2"; mkdir -p "$d"; make_release "$d" x86_64-unknown-linux-gnu
echo tamper >> "$d/v0.1.0.dev1/klatch-0.1.0-dev.1-x86_64-unknown-linux-gnu.tar.gz"
if run_install "$d"; then fail "installed a tampered archive"; elif [ ! -e "$d/bin/klatch" ] && grep -qi "mismatch" "$d/out"; then pass "refuses a tampered archive and installs nothing"; else fail "tamper: $(cat "$d/out")"; fi

# 3. an archive that SHA256SUMS does not list is refused
d="$work/t3"; mkdir -p "$d"; make_release "$d" x86_64-unknown-linux-gnu
: > "$d/v0.1.0.dev1/SHA256SUMS"
if run_install "$d"; then fail "installed an unlisted archive"; elif [ ! -e "$d/bin/klatch" ]; then pass "refuses an archive with no checksum entry"; else fail "unlisted left a binary"; fi

# 4. an unsupported platform is refused with a message
d="$work/t4"; mkdir -p "$d"; make_release "$d" x86_64-unknown-linux-gnu
if env KLATCH_INSTALL_BASE_URL="file://$d" KLATCH_VERSION=v0.1.0.dev1 KLATCH_INSTALL_DIR="$d/bin" KLATCH_OS=Plan9 KLATCH_ARCH=mips sh "$installer" >"$d/out" 2>&1; then fail "accepted Plan9"; elif grep -qi "unsupported" "$d/out"; then pass "refuses an unsupported platform"; else fail "platform message: $(cat "$d/out")"; fi

# 5. a failed install leaves an existing klatch exactly as it was
d="$work/t5"; mkdir -p "$d/bin"; make_release "$d" x86_64-unknown-linux-gnu
printf '#!/bin/sh\necho old\n' > "$d/bin/klatch"; chmod 755 "$d/bin/klatch"
echo tamper >> "$d/v0.1.0.dev1/klatch-0.1.0-dev.1-x86_64-unknown-linux-gnu.tar.gz"
run_install "$d" >/dev/null 2>&1
if [ "$("$d/bin/klatch")" = "old" ]; then pass "a refused upgrade keeps the old binary"; else fail "old binary was replaced"; fi

# 6. an upgrade replaces the binary
d="$work/t6"; mkdir -p "$d/bin"; make_release "$d" x86_64-unknown-linux-gnu
printf '#!/bin/sh\necho old\n' > "$d/bin/klatch"; chmod 755 "$d/bin/klatch"
if run_install "$d" && [ "$("$d/bin/klatch")" = "klatch 0.1.0-dev.1" ]; then pass "an upgrade replaces the binary"; else fail "upgrade: $(cat "$d/out")"; fi

# 7. it never asks for sudo, even if the directory cannot be written
d="$work/t7"; mkdir -p "$d/bin" "$d/fakebin"; make_release "$d" x86_64-unknown-linux-gnu
printf '#!/bin/sh\necho called > "%s/sudo-called"\nexit 1\n' "$d" > "$d/fakebin/sudo"; chmod 755 "$d/fakebin/sudo"
chmod 555 "$d/bin"
env PATH="$d/fakebin:$PATH" KLATCH_INSTALL_BASE_URL="file://$d" KLATCH_VERSION=v0.1.0.dev1 KLATCH_INSTALL_DIR="$d/bin" \
  KLATCH_OS=Linux KLATCH_ARCH=x86_64 sh "$installer" >"$d/out" 2>&1; rc=$?
chmod 755 "$d/bin"
if [ "$rc" -ne 0 ] && [ ! -e "$d/sudo-called" ]; then pass "never calls sudo (an unwritable directory is an error)"; else fail "sudo or success: rc=$rc $(cat "$d/out")"; fi

# 8. it warns when the install directory is not on PATH
d="$work/t8"; mkdir -p "$d"; make_release "$d" x86_64-unknown-linux-gnu
run_install "$d" PATH=/usr/bin:/bin
if grep -q "not on your PATH" "$d/out"; then pass "warns when the directory is not on PATH"; else fail "no PATH warning: $(cat "$d/out")"; fi

# 9. the arm and macOS names map to the release targets
for pair in "Darwin:arm64:aarch64-apple-darwin" "Darwin:x86_64:x86_64-apple-darwin" "Linux:aarch64:aarch64-unknown-linux-gnu" "Linux:amd64:x86_64-unknown-linux-gnu"; do
  os=${pair%%:*}; rest=${pair#*:}; arch=${rest%%:*}; target=${rest#*:}
  d="$work/t9-$os-$arch"; mkdir -p "$d"; make_release "$d" "$target"
  if env KLATCH_INSTALL_BASE_URL="file://$d" KLATCH_VERSION=v0.1.0.dev1 KLATCH_INSTALL_DIR="$d/bin" KLATCH_OS="$os" KLATCH_ARCH="$arch" sh "$installer" >"$d/out" 2>&1 && [ -x "$d/bin/klatch" ]; then pass "$os $arch installs the $target archive"; else fail "$os $arch: $(cat "$d/out")"; fi
done

echo
if [ "$fails" -eq 0 ]; then echo "all install tests passed"; else echo "$fails install test(s) failed"; exit 1; fi
