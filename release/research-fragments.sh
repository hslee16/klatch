#!/usr/bin/env bash
# RESEARCH-NOTES section fragments (#859): one file per new research note in research/notes/, named <ticket>-<slug>.md,
# so parallel pull requests never fight over the next free section number in RESEARCH-NOTES.md. The number is given at release.
#   release/research-fragments.sh check [root]      fail on a badly named or badly formed fragment
#   release/research-fragments.sh collect [root]    print the sections as they would be appended, numbered; change nothing
#   release/research-fragments.sh assemble [root]   append the sections to RESEARCH-NOTES.md in ticket order, then delete the fragments
# Fragment: line 1 is the section title (the text after "## N. ", no heading marker); the rest is the body, written as
# markdown. A "### Title" heading in the body becomes "### N.k Title" (k counts from 1) at assembly. No "## " headings in the body.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mode="${1:-check}"
root="${2:-$(cd "$here/.." && pwd)}"
dir="$root/research/notes"
notes="$root/RESEARCH-NOTES.md"

fragments() { find "$dir" -maxdepth 1 -type f -name '*.md' ! -name README.md 2> /dev/null | awk -F/ '{n=$NF; sub(/-.*/,"",n); print n "\t" $0}' | sort -n -k1,1 -k2 | cut -f2-; }

check() {
  local fail=0 f base first
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    base="$(basename "$f")"
    if ! [[ "$base" =~ ^[0-9]+-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]]; then
      echo "BAD NAME  $f: expected <ticket>-<slug>.md (digits, then a lower-case slug of letters, digits and hyphens)" >&2; fail=1
    fi
    if [ ! -s "$f" ]; then echo "EMPTY     $f" >&2; fail=1; continue; fi
    first="$(head -n1 "$f")"
    if [ -z "${first//[[:space:]]/}" ] || [[ "$first" == '#'* ]]; then
      echo "BAD TITLE $f: line 1 must be the section title text (not blank, not a heading)" >&2; fail=1
    fi
    if [ -z "$(tail -n +2 "$f" | tr -d '[:space:]')" ]; then echo "NO BODY   $f: the section needs text after its title line" >&2; fail=1; fi
    if tail -n +2 "$f" | grep -q '^## '; then echo "H2 IN BODY $f: use ### headings inside a note; the title line is the ## heading" >&2; fail=1; fi
    if grep -q $'\xe2\x80\x94' "$f"; then echo "EM DASH   $f" >&2; fail=1; fi
  done < <(fragments)
  return "$fail"
}

# the highest top-level section number in RESEARCH-NOTES.md ("## 53. ..."); sections cited by number never change
last_number() {
  awk '/^## [0-9]+\. /{v=$2; sub(/\..*/,"",v); if (v+0>n) n=v+0} END{print n+0}' "$notes"
}

render() { # prints the numbered sections; $1 is the first number to use
  local n="$1" f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    awk -v n="$n" '
      NR==1 { printf "## %d. %s\n", n, $0; next }
      /^### / { k++; sub(/^### /, ""); printf "### %d.%d %s\n", n, k, $0; next }
      { print }' "$f"
    echo
    n=$((n + 1))
  done < <(fragments)
}

summary() { # ticket to section number, so "see the research note for #N" can be rewritten by hand
  local n="$1" f base
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    base="$(basename "$f")"; echo "section $n <- #${base%%-*} ($base)" >&2
    n=$((n + 1))
  done < <(fragments)
}

case "$mode" in
  check) check ;;
  collect) check && render "$(($(last_number) + 1))" ;;
  assemble)
    check
    if [ -z "$(fragments)" ]; then exit 0; fi
    first=$(($(last_number) + 1))
    summary "$first"
    body="$(render "$first")"
    { cat "$notes"; echo; printf '%s\n' "$body"; } | awk '/^$/{b++; next} {while(b>0){print ""; b--} print}' > "$notes.new"
    mv "$notes.new" "$notes"
    while IFS= read -r f; do rm -f "$f"; done < <(fragments)
    ;;
  *) echo "usage: $0 check|collect|assemble [root]" >&2; exit 2 ;;
esac
