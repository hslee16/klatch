#!/usr/bin/env bash
# Assert what a built package holds (#1243). Callable on CI output and on downloaded artifacts.
#
#   release/check-package-contents.sh wheel path/to/klatch-<version>-<tag>.whl
#   release/check-package-contents.sh npm path/to/klatch-<version>.tgz
#
# wheel: the klatch and klatch-server commands (both executable) and LICENSE and THIRD-PARTY-LICENSES.txt.
# npm:   dist/, README.md, LICENSE and package.json only; no binary and nothing else.
# (A release archive is checked by release/check-archive.sh.)
set -euo pipefail
kind="${1:?usage: check-package-contents.sh wheel|npm FILE}"
file="${2:?usage: check-package-contents.sh wheel|npm FILE}"
[ -f "$file" ] || { echo "check-package-contents: $file is not a file" >&2; exit 1; }

case "$kind" in
  wheel)
    python3 -I - "$file" <<'PY'
import re, sys, zipfile
path = sys.argv[1]
base = path.split("/")[-1]
m = re.match(r"klatch-([^-]+)-", base)
if not m:
    sys.exit("check-package-contents: " + base + " is not named like a klatch wheel")
version = m.group(1)
zf = zipfile.ZipFile(path)
names = zf.namelist()
data = "klatch-" + version + ".data/"
for want in ("scripts/klatch", "scripts/klatch-server"):
    if data + want not in names:
        sys.exit("check-package-contents: " + base + " does not contain " + want)
    if not zf.getinfo(data + want).external_attr >> 16 & 0o111:
        sys.exit("check-package-contents: " + base + ": " + want + " is not executable")
for want in ("LICENSE", "THIRD-PARTY-LICENSES.txt"):
    if not any(n.endswith("/licenses/" + want) and n.startswith("klatch-" + version + ".dist-info/") for n in names):
        sys.exit("check-package-contents: " + base + " does not contain licenses/" + want)
print("wheel contents ok: " + base)
PY
    ;;
  npm)
    listing=$(tar -tzf "$file" | sed 's|/$||' | sort)
    bad=$(printf '%s\n' "$listing" | grep -Ev '^package$|^package/(dist|dist/.+|README\.md|LICENSE|package\.json)$' || true)
    if [ -n "$bad" ]; then
      echo "check-package-contents: $file holds files outside dist, README.md, LICENSE and package.json:" >&2
      printf '%s\n' "$bad" >&2
      exit 1
    fi
    for want in package/README.md package/LICENSE package/package.json; do
      grep -qx "$want" <<< "$listing" || { echo "check-package-contents: $file does not contain $want" >&2; exit 1; }
    done
    grep -q '^package/dist/.' <<< "$listing" || { echo "check-package-contents: $file has an empty or missing dist/" >&2; exit 1; }
    echo "npm contents ok: $(basename "$file")"
    ;;
  *)
    echo "check-package-contents: unknown kind $kind (wheel or npm)" >&2
    exit 2
    ;;
esac
