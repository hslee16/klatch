#!/usr/bin/env bash
# Print the GitHub release notes for a version (#1240): release/RELEASE-NOTES.template.md with the version, the PyPI
# spelling, the tag and the date filled in, the known limits taken from SECURITY.md ("What is not protected") and from the
# open rows of proto/audit/07-gaps-found-and-ticketed, and the rest (scope sentence, platforms, out of scope, the macOS
# xattr note, verification and install lines) from the template.
#
#   release/release-notes.sh <semver> [--date YYYY-MM-DD] [--root DIR] > notes.md
#   gh release create v0.2.0a1 --prerelease --notes-file notes.md ...      (RELEASING.md section 2)
#
# It fails when SECURITY.md has no "What is not protected" section, when the audit directory has no open row, when a
# placeholder is left, or when the text contains an em dash. It reads files only.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
version=""; date_arg=""; root="$(cd "$here/.." && pwd)"
while [ $# -gt 0 ]; do
  case "$1" in
    --date) date_arg="${2:?--date needs a value}"; shift 2 ;;
    --root) root="${2:?--root needs a value}"; shift 2 ;;
    -*) echo "release-notes: unknown option $1" >&2; exit 2 ;;
    *) [ -z "$version" ] || { echo "release-notes: one version only" >&2; exit 2; }; version="$1"; shift ;;
  esac
done
[ -n "$version" ] || { echo "usage: release/release-notes.sh <semver> [--date YYYY-MM-DD] [--root DIR]" >&2; exit 2; }
[ -n "$date_arg" ] || date_arg="$(date +%Y-%m-%d)"

python3 -I - "$version" "$date_arg" "$root" "$here/RELEASE-NOTES.template.md" <<'PY'
import os, re, sys

version, date, root, template_path = sys.argv[1:5]
EM = "—"
# gap rows that name a ticket but record no limit (descriptive rows)
SKIP_TICKETS = {"181"}


def die(msg):
    sys.stderr.write("release-notes: " + msg + "\n")
    sys.exit(1)


m = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc|dev)\.(\d+))?", version)
if not m:
    die("version %r is not SemVer like 0.2.0-alpha.1" % version)
core, kind, n = m.groups()
if kind is None:
    pypi = core
elif kind == "dev":
    pypi = "%s.dev%s" % (core, n)
else:
    pypi = "%s%s%s" % (core, {"alpha": "a", "beta": "b", "rc": "rc"}[kind], n)
tag = "v" + pypi
if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
    die("date %r is not YYYY-MM-DD" % date)


def read(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError as e:
        die("cannot read %s: %s" % (path, e.strerror))


def first_sentence(text, limit=240):
    depth = 0
    for i, ch in enumerate(text):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(0, depth - 1)
        elif ch == "." and depth == 0 and (i + 1 == len(text) or text[i + 1] == " "):
            return text[: i + 1]
    if len(text) > limit:
        text = text[:limit].rsplit(" ", 1)[0] + " ..."
    return text


# SECURITY.md: every bullet of "What is not protected", wrapped lines joined
security = read(os.path.join(root, "SECURITY.md"))
section = None
for line in security.splitlines():
    if line.startswith("## "):
        if line.startswith("## What is not protected"):
            section = []
        elif section is not None:
            break
    elif section is not None:
        section.append(line)
if not section:
    die('SECURITY.md has no "What is not protected" section (or it is empty)')
bullets, cur = [], None
for line in section:
    if line.startswith("- "):
        cur = [line[2:].strip()]
        bullets.append(cur)
    elif line.strip() and cur is not None and line.startswith(" "):
        cur.append(line.strip())
    elif not line.strip():
        cur = None
sec_lines = ["- " + " ".join(b) for b in bullets]
if not sec_lines:
    die('SECURITY.md "What is not protected" has no bullets')

# audit gap rows that are still open: the ticket cell does not say "fixed"
gap_dir = os.path.join(root, "proto", "audit", "07-gaps-found-and-ticketed")
if not os.path.isdir(gap_dir):
    die("no audit gap directory: %s" % gap_dir)
gap_lines = []
for name in sorted(os.listdir(gap_dir)):
    if name.startswith("_") or not name.endswith(".md"):
        continue
    for line in read(os.path.join(gap_dir, name)).splitlines():
        line = line.strip()
        if not (line.startswith("|") and line.endswith("|")):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) < 2:
            continue
        gap, ticket = cells[0], cells[-1]
        if re.search(r"fixed", gap + " " + ticket, re.I) or set(re.findall(r"#(\d+)", ticket)) & SKIP_TICKETS:
            continue
        gap_lines.append("- %s (%s)" % (first_sentence(gap).rstrip(".") + ".", ticket))
if not gap_lines:
    die("no open audit gap row found in %s (the extraction is broken)" % gap_dir)

out = read(template_path)
for key, val in (
    ("{{VERSION}}", version),
    ("{{PYPI_VERSION}}", pypi),
    ("{{TAG}}", tag),
    ("{{DATE}}", date),
    ("{{KNOWN_LIMITS_SECURITY}}", "\n".join(sec_lines)),
    ("{{KNOWN_LIMITS_GAPS}}", "\n".join(gap_lines)),
):
    out = out.replace(key, val)
if "{{" in out or "}}" in out:
    die("a placeholder is left (or a source contains braces): " + ", ".join(sorted(set(re.findall(r"\{\{[^}]*\}\}", out)))))
if EM in out:
    die("the notes contain an em dash; fix the source text")
sys.stdout.write(out)
PY
