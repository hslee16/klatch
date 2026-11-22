#!/usr/bin/env bash
# The tag checks in .github/workflows/build-release.yml (#1260): every job that checks the release tag must accept the tag
# of a release in either spelling through release/pep440.sh, and refuse every other tag. The steps are read out of the real
# workflow file and run against fixture version files, so what is tested is what CI runs. No network.
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
wf="$ROOT/.github/workflows/build-release.yml"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-tagchk.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }
# shellcheck source=release/pep440.sh
. "$here/pep440.sh"

# each step whose name starts "The tag must name": its job, its working directory and its script
python3 -I - "$wf" "$work/steps" <<'PY'
import os, re, sys
wf, out = sys.argv[1:3]
os.makedirs(out)
lines = open(wf, encoding="utf-8").read().split("\n")
job, wd, n, i = None, ".", 0, 0
while i < len(lines):
    line = lines[i]
    m = re.match(r"^  ([a-z][a-z-]*):\s*$", line)
    if m:
        job, wd = m.group(1), "."
    mw = re.match(r"^        working-directory: (\S+)\s*$", line)
    if mw:
        wd = mw.group(1)
    if re.match(r"^      - name: The tag must name", line):
        j = i + 1
        while not re.match(r"^        run: \|\s*$", lines[j]):
            j += 1
        body = []
        j += 1
        while j < len(lines) and (lines[j].startswith("          ") or lines[j].strip() == ""):
            body.append(lines[j][10:])
            j += 1
        n += 1
        base = os.path.join(out, "%d-%s" % (n, job))
        open(base + ".sh", "w").write("\n".join(body) + "\n")
        open(base + ".wd", "w").write(wd)
        i = j
        continue
    i += 1
if n < 5:
    sys.exit("expected at least five tag check steps, found %d" % n)
PY

# a workspace holding one release's version files: $1 SemVer
sandbox() {
  local semver="$1" pep
  pep="$(semver_to_pep440 "$semver")"
  rm -rf "$work/ws"; mkdir -p "$work/ws/clients/python" "$work/ws/clients/typescript" "$work/ws/proto" "$work/ws/release"
  cp "$here/pep440.sh" "$work/ws/release/pep440.sh"
  printf '[project]\nname = "klatch"\nversion = "%s"\n' "$pep" > "$work/ws/clients/python/pyproject.toml"
  printf '[package]\nname = "klatch"\nversion = "%s"\n' "$semver" > "$work/ws/proto/Cargo.toml"
  cp "$work/ws/proto/Cargo.toml" "$work/ws/Cargo.toml"
  printf '{"name":"@klatch/client","version":"%s"}\n' "$semver" > "$work/ws/clients/typescript/package.json"
}
# run one step file against the workspace with tag $2; exit status of the step
runstep() { # file, tag
  local base wd
  base="${1%.sh}"; wd="$(cat "$base.wd")"
  : > "$work/ws/env"
  (cd "$work/ws/$wd" && TAG="$2" GITHUB_WORKSPACE="$work/ws" GITHUB_ENV="$work/ws/env" bash "$1") > "$work/last.out" 2>&1
}
expect() { # description, tag, want (pass|fail), semver
  local f got
  sandbox "$4"
  for f in "$work"/steps/*.sh; do
    if runstep "$f" "$2"; then got=pass; else got=fail; fi
    [ "$got" = "$3" ] && ok "$1 ($(basename "${f%.sh}"))" || bad "$1: $(basename "${f%.sh}") gave $got, wanted $3"
  done
}

n="$(find "$work/steps" -name '*.sh' | wc -l | tr -d ' ')"
[ "$n" -ge 5 ] && ok "found $n tag check steps in the workflow" || bad "only $n tag check steps"

expect "the rehearsal tag v0.2.0a1 passes every job" v0.2.0a1 pass 0.2.0-alpha.1
expect "a beta tag passes" v0.3.0b2 pass 0.3.0-beta.2
expect "a release candidate tag passes" v1.0.0rc1 pass 1.0.0-rc.1
expect "a dev tag passes" v0.1.0.dev2 pass 0.1.0-dev.2
expect "a stable tag passes" v1.2.3 pass 1.2.3
expect "the SemVer spelling of the tag passes everywhere alike" v0.2.0-alpha.1 pass 0.2.0-alpha.1
expect "another pre-release number fails" v0.2.0a2 fail 0.2.0-alpha.1
expect "a stable tag on a pre-release version fails" v0.2.0 fail 0.2.0-alpha.1
expect "another version fails" v0.3.0 fail 0.2.0
expect "a tag without its v fails" 0.2.0a1 fail 0.2.0-alpha.1
expect "an empty tag fails" "" fail 0.2.0-alpha.1

# the wheel job also refuses a Python version that is a different release than the crate (the archives are named by the crate)
sandbox 0.2.0-alpha.1
printf '[project]\nname = "klatch"\nversion = "0.2.0a2"\n' > "$work/ws/clients/python/pyproject.toml"
for f in "$work"/steps/*python-wheels.sh; do
  if runstep "$f" v0.2.0a2; then bad "the wheel job accepted a Python version 0.2.0a2 beside a crate at 0.2.0-alpha.1"; else ok "the wheel job refuses a Python version that is not the crate's release"; fi
done

# the wheel job keeps handing the two versions on to its later steps
sandbox 0.2.0-alpha.1
for f in "$work"/steps/*python-wheels.sh; do
  runstep "$f" v0.2.0a1 || true
  grep -qx 'PYPI_VERSION=0.2.0a1' "$work/ws/env" && grep -qx 'CARGO_VERSION=0.2.0-alpha.1' "$work/ws/env" && ok "PYPI_VERSION and CARGO_VERSION are exported" || bad "env: $(cat "$work/ws/env")"
done

# no step compares a tag by plain equality any more
if grep -n 'TAG" != "v\|TAG" = "v' "$wf" > "$work/eq.out"; then bad "a tag check still compares by plain equality: $(cat "$work/eq.out")"; else ok "no tag check compares by plain equality"; fi
exit "$fails"
