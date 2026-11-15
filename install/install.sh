#!/bin/sh
# klatch installer (#429): downloads the release archive for this machine, checks its SHA-256 against the release's
# SHA256SUMS, and installs the `klatch` binary into a directory you own. It never uses sudo and never changes an
# existing install unless every check has passed.
#
#   curl -fsSLO https://github.com/hslee16/klatch/releases/download/<tag>/install.sh   # download it,
#   less install.sh                                                                    # read it,
#   sh install.sh                                                                      # then run it
#
# Do not pipe it into a shell: read what you run. (A release's install.sh is checksummed and attested with the archives;
# `gh attestation verify install.sh --repo hslee16/klatch` checks that it is the file the release workflow built.)
#
# Settings (environment):
#   KLATCH_VERSION      release tag, for example v0.1.0.dev1 (default: the newest release, pre-releases included)
#   KLATCH_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#
# If the GitHub CLI is installed and logged in, the download's build provenance is verified too
# (`gh attestation verify`); otherwise only the checksum is. See SECURITY.md for verifying by hand.
# For tests: KLATCH_INSTALL_BASE_URL (a directory laid out like the releases), KLATCH_OS, KLATCH_ARCH.
set -eu

REPO="hslee16/klatch"
BASE="${KLATCH_INSTALL_BASE_URL:-https://github.com/$REPO/releases/download}"
DEST="${KLATCH_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'klatch install: %s\n' "$*" >&2; exit 1; }

# --- fetch -------------------------------------------------------------------------------------------------------
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
  fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q "$1" -O "$2"; }
  fetch_stdout() { wget -q "$1" -O -; }
else
  die "needs curl or wget"
fi

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else die "needs sha256sum or shasum"
  fi
}

# --- which archive -----------------------------------------------------------------------------------------------
# On Linux there are two builds: a dynamic one that needs glibc 2.34 or later (gnu), and a fully static one that needs
# no system libraries (musl). The static one is chosen when glibc is older, when the system is not glibc (Alpine) and when it
# cannot be told; KLATCH_LIBC=gnu or musl chooses by hand.
linux_libc() {
  case "${KLATCH_LIBC:-}" in
    gnu | musl) printf '%s' "$KLATCH_LIBC"; return ;;
  esac
  info="${KLATCH_LDD_OUTPUT-$(ldd --version 2>&1 || true)}"
  case "$info" in
    *musl*) printf musl ;;
    *GLIBC* | *glibc* | *"GNU libc"*)
      # the last number of the first line is the glibc version, for example 2.35
      ver=$(printf '%s\n' "$info" | head -n 1 | grep -Eo '[0-9]+\.[0-9]+' | tail -n 1)
      major=${ver%%.*}
      minor=${ver#*.}
      if [ -n "$ver" ] && { [ "$major" -gt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -ge 34 ]; }; }; then
        printf gnu
      else
        printf musl
      fi
      ;;
    *) printf musl ;;
  esac
}

os="${KLATCH_OS:-$(uname -s)}"
arch="${KLATCH_ARCH:-$(uname -m)}"
case "${KLATCH_LIBC:-}" in
  "" | gnu | musl) ;;
  *) die "KLATCH_LIBC must be gnu or musl, not ${KLATCH_LIBC}" ;;
esac
case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux) os_part="unknown-linux-$(linux_libc)" ;;
  *) die "unsupported platform: $os $arch (supported: macOS and Linux, on arm64 and x86_64)" ;;
esac
case "$arch" in
  arm64 | aarch64) arch_part="aarch64" ;;
  x86_64 | amd64) arch_part="x86_64" ;;
  *) die "unsupported platform: $os $arch (supported: macOS and Linux, on arm64 and x86_64)" ;;
esac
target="$arch_part-$os_part"

tag="${KLATCH_VERSION:-}"
if [ -z "$tag" ]; then
  [ -z "${KLATCH_INSTALL_BASE_URL:-}" ] || die "set KLATCH_VERSION when KLATCH_INSTALL_BASE_URL is set"
  # the newest release, pre-releases included (GitHub's "latest" skips them)
  tag=$(fetch_stdout "https://api.github.com/repos/$REPO/releases?per_page=1" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$tag" ] || die "could not find the newest release; set KLATCH_VERSION"
fi
# a tag names the version in the files: v0.1.0.dev1 is 0.1.0-dev.1, v0.1.0 is 0.1.0
version=$(printf '%s' "${tag#v}" | sed 's/\.dev\([0-9]\)/-dev.\1/')
name="klatch-$version-$target"
archive="$name.tar.gz"

say "klatch $version for $target"
case "$target" in
  *-linux-musl) say "(the static build, which needs no system libraries; the dynamic build needs glibc 2.34 or later)" ;;
esac

tmp=$(mktemp -d "${TMPDIR:-/tmp}/klatch-install.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch "$BASE/$tag/$archive" "$tmp/$archive" || die "could not download $BASE/$tag/$archive"
fetch "$BASE/$tag/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download SHA256SUMS"

# --- verify ------------------------------------------------------------------------------------------------------
expected=$(awk -v f="$archive" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "$archive is not listed in SHA256SUMS; nothing installed"
actual=$(sha256_of "$tmp/$archive")
[ "$actual" = "$expected" ] || die "checksum mismatch for $archive (expected $expected, got $actual); nothing installed"
say "checksum verified"

if [ -z "${KLATCH_INSTALL_BASE_URL:-}" ]; then
  if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    gh attestation verify "$tmp/$archive" --repo "$REPO" >/dev/null 2>&1 \
      || die "build provenance check failed for $archive; nothing installed"
    say "build provenance verified (gh attestation verify)"
  else
    say "build provenance not checked (install and log in to the GitHub CLI to check it; see SECURITY.md)"
  fi
fi

# --- install -----------------------------------------------------------------------------------------------------
tar -xzf "$tmp/$archive" -C "$tmp" || die "could not unpack $archive"
[ -f "$tmp/$name/klatch" ] || die "$archive does not contain klatch"

mkdir -p "$DEST" 2>/dev/null || die "cannot create $DEST (this installer never uses sudo; set KLATCH_INSTALL_DIR to a directory you own)"
[ -w "$DEST" ] || die "cannot write to $DEST (this installer never uses sudo; set KLATCH_INSTALL_DIR to a directory you own)"
# write beside the target and rename, so an existing install is replaced whole or not at all
cp "$tmp/$name/klatch" "$DEST/.klatch.new.$$" || die "cannot write to $DEST"
chmod 755 "$DEST/.klatch.new.$$"
mv -f "$DEST/.klatch.new.$$" "$DEST/klatch"

say "installed $("$DEST/klatch" --version) to $DEST/klatch"
case "$version" in
  *-*) say "This is an early pre-release with no outside security review: not for sensitive data." ;;
esac
case ":$PATH:" in
  *":$DEST:"*) ;;
  *) say "note: $DEST is not on your PATH; add it, for example: export PATH=\"$DEST:\$PATH\"" ;;
esac
