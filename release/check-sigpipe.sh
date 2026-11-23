#!/usr/bin/env bash
# Guard against the SIGPIPE flake class (#1357). Under `set -o pipefail`, `producer | grep -q X` (also `grep -qF`,
# `grep -m1`, `head -1`, `head -n 1`, `sed -q`) can fail although the consumer matched: the consumer exits at its first
# match, the producer is still writing, dies of SIGPIPE (141) and pipefail fails the whole pipeline. It only shows under
# CPU load, so it is a flake, not a failure you can reproduce on demand.
#   release/check-sigpipe.sh [file...]    default: every tracked shell script in the repository
# Fix a hit by feeding the consumer without a pipe (`grep -qF want <<< "$text"`), or by letting the consumer read all
# its input (`cmd | grep want > /dev/null`, `cmd | sed -n 1p`). A reviewed exception carries `# sigpipe-ok: <reason>` on
# the same line or on the line right above it (a producer that writes one short line, say).
# Only scripts that set pipefail are checked (tracked files and new untracked ones). A pipeline split over lines with a
# trailing `|` is read as one line. Comments and the text of single-quoted strings are not code and are not matched
# (a pipe inside double quotes, as in "$(cmd | head -1)", is code and is matched).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"

scripts() {
  if [ "$#" -gt 0 ]; then printf '%s\n' "$@"; return; fi
  local f
  git -C "$root" ls-files --cached --others --exclude-standard | while IFS= read -r f; do
    [ -f "$root/$f" ] && [ ! -L "$root/$f" ] || continue
    case "$f" in
      *.sh) echo "$root/$f" ;;
      *) if head -c 80 "$root/$f" | sed -n 1p | grep -qE '^#!.*(bash|/sh( |$)|env sh)'; then echo "$root/$f"; fi ;; # sigpipe-ok: reads a few bytes of one line
    esac
  done
}

fail=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  grep -qE '^[[:space:]]*set[[:space:]].*-[A-Za-z]*o[[:space:]]+pipefail' "$f" || continue
  out="$(awk -v file="$f" '
    # code(s): s without comments and without string text (single-quoted, ANSI-C quoted and double-quoted text); what is
    # run inside $(...) stays, also inside double quotes. The quote stack stk carries over lines.
    function top() { return substr(stk, length(stk), 1) }
    function push(k) { stk = stk k }
    function pop() { stk = substr(stk, 1, length(stk) - 1) }
    function code(s,   i, c, d, t, out, n) {
      out = ""; n = length(s)
      for (i = 1; i <= n; i++) {
        c = substr(s, i, 1); d = substr(s, i + 1, 1); t = top()
        if (t == "s") { if (c == SQ) pop(); continue }
        if (t == "S") { if (c == "\\") i++; else if (c == SQ) pop(); continue }
        if (c == "\\") { if (t != "d") out = out c d; i++; continue }
        if (t == "d") {
          if (c == DQ) pop(); else if (c == "$" && d == "(") { push("c"); out = out "(" ; i++ }
          continue
        }
        if (c == SQ) { push("s"); continue }
        if (c == "$" && d == SQ) { push("S"); i++; continue }
        if (c == DQ) { push("d"); continue }
        if (c == "$" && d == "(") { push("c"); out = out "$("; i++; continue }
        if (c == ")" && t == "c") { pop(); out = out c; continue }
        if (c == "#" && (i == 1 || substr(s, i - 1, 1) ~ /[ \t;|&(]/)) break
        out = out c
      }
      return out
    }
    function flush(   cd) {
      cd = acc_code
      if (acc != "" && cd ~ risky && acc !~ /sigpipe-ok:[ \t]*[^ \t]/ && prev !~ /sigpipe-ok:[ \t]*[^ \t]/) print file ":" start ": " acc
      prev = acc; acc = ""; acc_code = ""
    }
    BEGIN {
      SQ = sprintf("%c", 39); DQ = sprintf("%c", 34); stk = ""
      risky = "(^|[^|])\\|[ \t]*(grep[ \t]+((-[A-Za-z0-9-]+|[0-9]+)[ \t]+)*-([A-Za-z]*[qm][A-Za-z0-9]*|-quiet|-max-count)([ \t=]|$)|head[ \t]+(-n[ \t]*1|-1)([^0-9]|$)|sed[ \t]+-[A-Za-z]*q)"
    }
    {
      if (acc == "") start = NR
      line = $0
      sub(/\\$/, "", line)
      acc = acc (acc == "" ? "" : " ") line
      acc_code = acc_code (acc_code == "" ? "" : " ") code(line)
      if ($0 ~ /\|[ \t]*$/ && $0 !~ /\|\|[ \t]*$/) next
      if ($0 ~ /\\$/) next
      flush()
    }
    END { flush() }
  ' "$f")"
  if [ -n "$out" ]; then
    printf '%s\n' "$out" >&2
    fail=1
  fi
done < <(scripts "$@")

if [ "$fail" -ne 0 ]; then
  echo "check-sigpipe: the lines above pipe into an early-exit consumer in a script that sets pipefail (#1357)." >&2
  echo "  Use grep -q ... <<< \"\$text\", or let the consumer read everything (grep ... > /dev/null, sed -n 1p)," >&2
  echo "  or add '# sigpipe-ok: <reason>' for a reviewed exception." >&2
  exit 1
fi
echo "check-sigpipe: no early-exit consumer after a pipe in a pipefail script"
