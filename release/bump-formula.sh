#!/usr/bin/env bash
# Bump the Homebrew formula for a release (#1241): version, the four download URLs and their SHA-256 values from the
# release's SHA256SUMS, plus the klatch-server install and test lines (RELEASING.md section 4).
#
#   release/bump-formula.sh [--patch FILE] [--pr-body FILE] [--rehearse ARCHIVES_DIR] <semver> <SHA256SUMS> <tap-checkout>
#
# It rewrites <tap-checkout>/Formula/klatch.rb, checks the result (one version, four URLs with the tag, four SHA-256 values
# equal to the SHA256SUMS lines, klatch-server installed and tested), prints the patch, and prints the REST commands that
# open the tap pull request. It never pushes, never calls gh or git, and never touches the network: the human runs the
# printed commands. A failure leaves the formula as it was.
#
# --rehearse DIR: after the bump, a local rehearsal with Homebrew against the archives in DIR through file:// URLs
# (brew audit --strict --new, brew install --build-from-source, brew test), in a temporary local tap that is removed at the
# end. It is skipped with a message when brew is missing. It changes the machine's Homebrew while it runs: use a machine you
# can spare, not a production one.
set -euo pipefail

patch_file=""; pr_body=""; rehearse_dir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --patch) patch_file="${2:?--patch needs a file}"; shift 2 ;;
    --pr-body) pr_body="${2:?--pr-body needs a file}"; shift 2 ;;
    --rehearse) rehearse_dir="${2:?--rehearse needs a directory}"; shift 2 ;;
    -*) echo "bump-formula: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
[ $# -eq 3 ] || { echo "usage: release/bump-formula.sh [--patch FILE] [--pr-body FILE] [--rehearse ARCHIVES_DIR] <semver> <SHA256SUMS> <tap-checkout>" >&2; exit 2; }
version="$1"; sums="$2"; tap="$3"
formula="$tap/Formula/klatch.rb"
[ -f "$sums" ] || { echo "bump-formula: no SHA256SUMS at $sums" >&2; exit 1; }
[ -f "$formula" ] || { echo "bump-formula: no formula at $formula" >&2; exit 1; }

work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-bump.XXXXXX")"
tapname=""
cleanup() {
  if [ -n "$tapname" ] && command -v brew > /dev/null 2>&1; then
    brew uninstall --force "$tapname/klatch" > /dev/null 2>&1 || true
    brew untap "$tapname" > /dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# the rewrite and its verification are one step: nothing is written unless the result checks out
python3 -I - "$version" "$sums" "$formula" "$work/klatch.rb" "$work/tag" <<'PY'
import re, sys

version, sums_path, formula_path, out_path, tag_path = sys.argv[1:6]
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"]
BASE = "https://github.com/hslee16/klatch/releases/download"


def die(msg):
    sys.stderr.write("bump-formula: " + msg + "\n")
    sys.exit(1)


m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc|dev)\.(\d+))?", version)
if not m:
    die("version %r is not SemVer like 0.2.0-alpha.1" % version)
core, kind, n = m.groups()
pypi = core if kind is None else ("%s.dev%s" % (core, n) if kind == "dev" else "%s%s%s" % (core, {"alpha": "a", "beta": "b", "rc": "rc"}[kind], n))
tag = "v" + pypi

sums = {}
with open(sums_path, encoding="utf-8") as f:
    for line in f:
        parts = line.split(None, 1)
        if len(parts) == 2:
            sums[parts[1].strip().lstrip("*")] = parts[0].strip()
want = {}
for t in TARGETS:
    name = "klatch-%s-%s.tar.gz" % (version, t)
    digest = sums.get(name)
    if digest is None:
        die("SHA256SUMS has no line for %s" % name)
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        die("the checksum for %s is not 64 hex digits: %r" % (name, digest))
    want[t] = (name, digest)

with open(formula_path, encoding="utf-8") as f:
    lines = f.read().split("\n")

out, pending, seen = [], None, []
vcount = 0
for line in lines:
    mv = re.match(r'^(\s*)version "[^"]*"\s*$', line)
    if mv:
        line = '%sversion "%s"' % (mv.group(1), version)
        vcount += 1
    mu = re.match(r'^(\s*)url "[^"]*-(%s)\.tar\.gz"\s*$' % "|".join(re.escape(t) for t in TARGETS), line)
    if mu:
        t = mu.group(2)
        line = '%surl "%s/%s/%s"' % (mu.group(1), BASE, tag, want[t][0])
        if pending is not None:
            die("a url line has no sha256 line before the next url")
        pending = t
        seen.append(t)
    ms = re.match(r'^(\s*)sha256 "[^"]*"\s*$', line)
    if ms:
        if pending is None:
            die("a sha256 line has no url line before it")
        line = '%ssha256 "%s"' % (ms.group(1), want[pending][1])
        pending = None
    if re.match(r'^\s*bin\.install "klatch"\s*$', line):
        line = re.sub(r'bin\.install "klatch"', 'bin.install "klatch", "klatch-server"', line)
    out.append(line)
    if re.search(r'shell_output\("#\{bin\}/klatch --version"\)', line) and not any("klatch-server --help" in l for l in lines):
        indent = re.match(r"^(\s*)", line).group(1)
        out.append('%sassert_match "usage: klatch-server", shell_output("#{bin}/klatch-server --help")' % indent)

if pending is not None:
    die("the last url line has no sha256 line")
if vcount != 1:
    die("expected one version line in the formula, found %d" % vcount)
if sorted(seen) != sorted(TARGETS):
    die("the formula must have one url and sha256 pair for each of %s; found %s" % (", ".join(TARGETS), ", ".join(seen) or "none"))
text = "\n".join(out)

# verify what will be written
chk = {
    "one version line": len(re.findall(r'^\s*version "%s"$' % re.escape(version), text, re.M)) == 1,
    "four urls with the tag": len(re.findall(r'url "%s/%s/klatch-%s-[a-z0-9_-]+\.tar\.gz"' % (re.escape(BASE), re.escape(tag), re.escape(version)), text)) == 4,
    "four sha256 values": all(('sha256 "%s"' % want[t][1]) in text for t in TARGETS) and len(re.findall(r'^\s*sha256 "', text, re.M)) == 4,
    "klatch-server installed": 'bin.install "klatch", "klatch-server"' in text,
    "klatch-server tested": "klatch-server --help" in text,
}
bad = [k for k, v in chk.items() if not v]
if bad:
    die("the rewritten formula failed its own check: " + ", ".join(bad))
with open(out_path, "w", encoding="utf-8") as f:
    f.write(text)
with open(tag_path, "w", encoding="utf-8") as f:
    f.write(tag)
PY
tag="$(cat "$work/tag")"

diff -u --label a/Formula/klatch.rb --label b/Formula/klatch.rb "$formula" "$work/klatch.rb" > "$work/change.patch" || true
cp "$work/klatch.rb" "$formula"
if [ -s "$work/change.patch" ]; then cat "$work/change.patch"; else echo "bump-formula: the formula already matches $version (no change)"; fi
[ -z "$patch_file" ] || cp "$work/change.patch" "$patch_file"

{
  echo "Bump klatch to $version"
  echo
  echo "Release: https://github.com/hslee16/klatch/releases/tag/$tag"
  echo
  echo "The version, the four archive URLs and their SHA-256 values come from the release's SHA256SUMS (generated by"
  echo "release/bump-formula.sh in hslee16/klatch). klatch-server is installed and tested too. Before merging, check that the"
  echo "tap CI (audit, install, test on macOS and Linux) is green."
} > "$work/pr-body.md"
[ -z "$pr_body" ] || cp "$work/pr-body.md" "$pr_body"

branch="bump-$tag"
echo
echo "# Nothing has been pushed. To open the tap pull request, run these yourself (REST only):"
echo "git -C $tap checkout -b $branch"
echo "git -C $tap add Formula/klatch.rb"
echo "git -C $tap commit -m 'Bump klatch to $version'"
echo "git -C $tap push -u origin $branch"
echo "gh api repos/hslee16/homebrew-klatch/pulls -f title='Bump klatch to $version' -f head=$branch -f base=main -F body=@${pr_body:-pr-body.md}"
[ -n "$pr_body" ] || echo "# (the PR body above is the text below; save it as pr-body.md or pass --pr-body FILE next time)"
[ -n "$pr_body" ] || sed 's/^/#   /' "$work/pr-body.md"

if [ -n "$rehearse_dir" ]; then
  echo
  echo "# Local tap rehearsal"
  if ! command -v brew > /dev/null 2>&1; then
    echo "brew is not installed: the rehearsal is skipped. Install Homebrew, or rely on the tap's own CI on the pull request."
    exit 0
  fi
  [ -d "$rehearse_dir" ] || { echo "bump-formula: no archives directory $rehearse_dir" >&2; exit 1; }
  rehearse_dir="$(cd "$rehearse_dir" && pwd)"
  tapname="klatchrehearse/local"
  brew tap-new --no-git "$tapname" > /dev/null
  tapdir="$(brew --repository "$tapname" 2> /dev/null || true)"
  [ -n "$tapdir" ] || tapdir="$work/tap"
  mkdir -p "$tapdir/Formula"
  sed "s#url \"$(printf '%s' 'https://github.com/hslee16/klatch/releases/download')/$tag/#url \"file://$rehearse_dir/#" "$formula" > "$tapdir/Formula/klatch.rb"
  grep -q 'file://' "$tapdir/Formula/klatch.rb" || { echo "bump-formula: could not point the rehearsal formula at $rehearse_dir" >&2; exit 1; }
  echo "archives: file://$rehearse_dir/"
  brew audit --strict --new "$tapname/klatch"
  brew install --build-from-source "$tapname/klatch"
  brew test "$tapname/klatch"
  echo "rehearsal passed"
fi
