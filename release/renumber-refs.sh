#!/usr/bin/env bash
# Renumber the "design item for #T" and "research note for #T" references after assembly (#1244).
#
#   release/renumber-refs.sh MAPFILE [root]      (root defaults to the repository)
#
# MAPFILE holds the summary lines that `release/design-fragments.sh assemble` and `release/research-fragments.sh assemble`
# print on stderr ("item N <- #T (file)" and "section N <- #T (file)"); other lines are ignored. Every
# "design item for #T" becomes "design item N" and every "research note for #T" becomes "research note section N", in the
# text files under root, except .git, target, node_modules, release/ (its tests carry fixtures) and *.jsonl (data).
# Each change is printed as file:line. If a referenced ticket has no number in the map, nothing is changed, the tickets are
# named on stderr and the exit status is 1. A reference to the literal text "#NNN" is not a reference.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
map="${1:-}"
root="${2:-$(cd "$here/.." && pwd)}"
[ -n "$map" ] && [ -f "$map" ] || { echo "renumber-refs: usage: $0 MAPFILE [root]; no such map file: ${map:-}" >&2; exit 2; }
[ -d "$root" ] || { echo "renumber-refs: no such directory: $root" >&2; exit 2; }
root="$(cd "$root" && pwd)"

# kind (item or section) and ticket to number
design_map() { sed -nE 's/^item ([0-9]+) <- #([0-9]+) .*/\2 \1/p' "$map"; }
research_map() { sed -nE 's/^section ([0-9]+) <- #([0-9]+) .*/\2 \1/p' "$map"; }

scan() { # prints "file<TAB>kind<TAB>ticket" for every reference
  cd "$root"
  grep -rIEo --exclude-dir=.git --exclude-dir=target --exclude-dir=node_modules --exclude-dir=release --exclude='*.jsonl' \
    '(design item|research note) for #[0-9]+' . 2> /dev/null \
    | sed -E 's/^([^:]*):(design item|research note) for #([0-9]+)$/\1\t\2\t\3/' || true
}

refs="$(scan)"
[ -n "$refs" ] || { echo "renumber-refs: no references to renumber"; exit 0; }

dmap="$(design_map)"; rmap="$(research_map)"
missing=""
while IFS=$'\t' read -r _ kind ticket; do
  if [ "$kind" = "design item" ]; then tbl="$dmap"; else tbl="$rmap"; fi
  printf '%s\n' "$tbl" | awk -v t="$ticket" '$1==t{f=1} END{exit !f}' || missing="$missing
$kind for #$ticket"
done <<< "$refs"
if [ -n "$missing" ]; then
  echo "renumber-refs: these references have no assembled item or section (nothing was changed):" >&2
  printf '%s\n' "$missing" | sed '/^$/d' | sort -u | sed 's/^/  /' >&2
  exit 1
fi


# one perl pass per file with the two tables; the log (file:line: old -> new) goes to stdout
export DMAP="$dmap" RMAP="$rmap" ROOT="$root"
while IFS= read -r f; do
  perl -e '
    for (split /\n/, $ENV{DMAP}) { my ($t, $n) = split " "; $d{$t} = $n }
    for (split /\n/, $ENV{RMAP}) { my ($t, $n) = split " "; $r{$t} = $n }
    my $f = $ARGV[0];
    open(my $in, "<", "$ENV{ROOT}/$f") or die "$f: $!";
    my @out;
    while (my $line = <$in>) {
      my $new = $line;
      $new =~ s/design item for #(\d+)/exists $d{$1} ? "design item $d{$1}" : $&/ge;
      $new =~ s/research note for #(\d+)/exists $r{$1} ? "research note section $r{$1}" : $&/ge;
      if ($new ne $line) {
        my @a = $line =~ /((?:design item|research note) for #\d+)/g;
        my @b = $new =~ /(design item \d+|research note section \d+)/g;
        my $name = $f; $name =~ s{^\./}{};
        print "$name:$.: " . join(", ", @a) . " -> " . join(", ", @b) . "\n";
      }
      push @out, $new;
    }
    close $in;
    open(my $o, ">", "$ENV{ROOT}/$f") or die "$f: $!";
    print $o @out;
    close $o;
  ' "$f"
done < <(printf '%s\n' "$refs" | cut -f1 | sort -u)
