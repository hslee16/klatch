#!/bin/sh
# klatch installer (#429): downloads the release archive for this machine, checks its SHA-256 against the release's
# SHA256SUMS, and installs the `klatch` and `klatch-server` binaries (both are in the one archive) into a directory you own. It never uses sudo and never changes an
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
#   KLATCH_INSTALL_DIR  where to put the binaries (default: ~/.local/bin)
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

# The SemVer to PEP 440 mapping. This block is a copy of release/pep440.sh (the single source: change it there, then
# copy it here; release/test-pep440.sh fails when the two differ). A downloaded installer cannot source a file.
# BEGIN pep440
# SemVer 0.2.0-alpha.1 to PEP 440 0.2.0a1 (a stable X.Y.Z is unchanged)
semver_to_pep440() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc|dev)\.[0-9]+)?$' || return 1
  printf '%s\n' "$1" | sed -e 's/-alpha\.\([0-9][0-9]*\)$/a\1/' -e 's/-beta\.\([0-9][0-9]*\)$/b\1/' \
    -e 's/-rc\.\([0-9][0-9]*\)$/rc\1/' -e 's/-dev\.\([0-9][0-9]*\)$/.dev\1/'
}
# PEP 440 0.2.0a1 back to SemVer 0.2.0-alpha.1
pep440_to_semver() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+((a|b|rc)[0-9]+|\.dev[0-9]+)?$' || return 1
  printf '%s\n' "$1" | sed -e 's/\.dev\([0-9][0-9]*\)$/-dev.\1/' -e 's/\([0-9]\)a\([0-9][0-9]*\)$/\1-alpha.\2/' \
    -e 's/\([0-9]\)b\([0-9][0-9]*\)$/\1-beta.\2/' -e 's/\([0-9]\)rc\([0-9][0-9]*\)$/\1-rc.\2/'
}
# the git tag of a SemVer release: v0.2.0a1
release_tag() {
  _p=$(semver_to_pep440 "$1") || return 1
  printf 'v%s\n' "$_p"
}
# the SemVer version a tag names: v0.2.0a1 gives 0.2.0-alpha.1 (a tag needs its v)
semver_of_tag() {
  case "$1" in v?*) pep440_to_semver "${1#v}" ;; *) return 1 ;; esac
}
# tag_matches TAG SEMVER: the tag is v plus the PEP 440 spelling, or v plus the SemVer spelling
tag_matches() {
  _p=$(semver_to_pep440 "$2") || return 1
  [ "$1" = "v$_p" ] || [ "$1" = "v$2" ]
}
# the release archive of a SemVer version for a target triple: klatch-0.2.0-alpha.1-aarch64-apple-darwin.tar.gz
archive_name() {
  semver_to_pep440 "$1" >/dev/null || return 1
  printf 'klatch-%s-%s.tar.gz\n' "$1" "$2"
}
# a SemVer version with a pre-release part
is_prerelease() {
  case "$1" in *-*) return 0 ;; *) return 1 ;; esac
}
# END pep440

tag="${KLATCH_VERSION:-}"
if [ -z "$tag" ]; then
  [ -z "${KLATCH_INSTALL_BASE_URL:-}" ] || die "set KLATCH_VERSION when KLATCH_INSTALL_BASE_URL is set"
  # the newest release, pre-releases included (GitHub's "latest" skips them)
  tag=$(fetch_stdout "https://api.github.com/repos/$REPO/releases?per_page=1" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$tag" ] || die "could not find the newest release; set KLATCH_VERSION"
fi
# a tag names the version in the files: v0.2.0a1 is 0.2.0-alpha.1, v0.1.0.dev1 is 0.1.0-dev.1, v0.2.0 is 0.2.0
version=$(semver_of_tag "$tag") || die "cannot read a version from the tag '$tag' (expected v0.2.0, v0.2.0a1, v0.2.0b1, v0.2.0rc1 or v0.1.0.dev1)"
archive=$(archive_name "$version" "$target")
name="${archive%.tar.gz}"

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
for bin in klatch klatch-server; do
  [ -f "$tmp/$name/$bin" ] || die "$archive does not contain $bin; nothing installed"
done

mkdir -p "$DEST" 2>/dev/null || die "cannot create $DEST (this installer never uses sudo; set KLATCH_INSTALL_DIR to a directory you own)"
[ -w "$DEST" ] || die "cannot write to $DEST (this installer never uses sudo; set KLATCH_INSTALL_DIR to a directory you own)"
# Both binaries came out of the one archive whose checksum was verified above. Write both beside their targets first and
# rename only when both are written, so an existing install is replaced whole or not at all.
for bin in klatch klatch-server; do
  if ! cp "$tmp/$name/$bin" "$DEST/.$bin.new.$$"; then
    rm -f "$DEST/.klatch.new.$$" "$DEST/.klatch-server.new.$$"
    die "cannot write to $DEST"
  fi
  chmod 755 "$DEST/.$bin.new.$$"
done
for bin in klatch klatch-server; do
  mv -f "$DEST/.$bin.new.$$" "$DEST/$bin"
done

say "installed $("$DEST/klatch" --version) to $DEST/klatch"
say "installed $("$DEST/klatch-server" --version) to $DEST/klatch-server"
case "$version" in
  *-*) say "This is an early pre-release with no outside security review: not for sensitive data." ;;
esac
case ":$PATH:" in
  *":$DEST:"*) ;;
  *) say "note: $DEST is not on your PATH; add it, for example: export PATH=\"$DEST:\$PATH\"" ;;
esac
