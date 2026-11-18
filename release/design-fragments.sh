#!/usr/bin/env bash
# DESIGN item fragments (#807): one file per new design item in design/items/, named <ticket>-<slug>.md, so parallel
# pull requests never fight over the next free item number in DESIGN.md. The number is given at release.
#   release/design-fragments.sh check [root]      fail on a badly named or badly formed fragment
#   release/design-fragments.sh collect [root]    print the items as they would be appended, numbered; change nothing
#   release/design-fragments.sh assemble [root]   append the items to DESIGN.md in ticket order, then delete the fragments
# Fragment: line 1 is the item title (the text inside the bold, no ** and no heading marker); the rest is the body.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mode="${1:-check}"
root="${2:-$(cd "$here/.." && pwd)}"
dir="$root/design/items"
design="$root/DESIGN.md"

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
    if [ -z "${first//[[:space:]]/}" ] || [[ "$first" == '#'* ]] || [[ "$first" == '**'* ]]; then
      echo "BAD TITLE $f: line 1 must be the item title text (not blank, not a heading, no **)" >&2; fail=1
    fi
    if [ -z "$(tail -n +2 "$f" | tr -d '[:space:]')" ]; then echo "NO BODY   $f: the item needs text after its title line" >&2; fail=1; fi
    if grep -q $'\xe2\x80\x94' "$f"; then echo "EM DASH   $f" >&2; fail=1; fi
  done < <(fragments)
  return "$fail"
}

# the highest item number in the last section of DESIGN.md (the numbered decisions list that grows)
last_number() {
  awk '/^## /{n=0} /^[0-9]+\. \*\*/{v=$0; sub(/\..*/,"",v); if (v+0>n) n=v+0} END{print n+0}' "$design"
}

render() { # prints the numbered items; $1 is the first number to use
  local n="$1" f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    awk -v n="$n" '
      NR==1 { title=$0; next }
      { sub(/[ \t]+$/,"") }
      /^$/ { gap=1; next }
      { if (np==0 || gap) { p[++np]=$0 } else { p[np]=p[np] " " $0 }
        gap=0 }
      END {
        printf "%d. **%s** %s\n", n, title, p[1]
        for (i=2;i<=np;i++) printf "\n   %s\n", p[i]
      }' "$f"
    echo
    n=$((n + 1))
  done < <(fragments)
}

summary() { # ticket to item number, so "see design item for #N" can be rewritten by hand
  local n="$1" f base
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    base="$(basename "$f")"; echo "item $n <- #${base%%-*} ($base)" >&2
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
    { cat "$design"; echo; printf '%s\n' "$body"; } | awk '/^$/{b++; next} {while(b>0){print ""; b--} print}' > "$design.new"
    mv "$design.new" "$design"
    while IFS= read -r f; do rm -f "$f"; done < <(fragments)
    ;;
  *) echo "usage: $0 check|collect|assemble [root]" >&2; exit 2 ;;
esac
