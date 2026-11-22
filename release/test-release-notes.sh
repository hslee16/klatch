#!/usr/bin/env bash
# release/release-notes.sh must fill the template from SECURITY.md and the audit gap rows, leave no placeholder and no em dash,
# and fail when a source section is missing (#1240).
# shellcheck disable=SC2015
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/klatch-notes.XXXXXX")"
trap 'rm -rf "$work"' EXIT
fails=0
t="$work/t"
ok() { echo "  ok  $1"; }
bad() { echo "FAIL  $1" >&2; fails=1; }

fresh() {
  rm -rf "$t"; mkdir -p "$t/release" "$t/proto/audit/07-gaps-found-and-ticketed"
  cp "$here/RELEASE-NOTES.template.md" "$t/release/"
  cat > "$t/SECURITY.md" <<'MD'
# Security

## What is protected

- **At rest.** Encrypted.

## What is not protected (read this)

- **A hostile process running as you.** It can read the key and attach to a
  running klatch. Root can do anything.
- **Claim text is not sealed.** A reason is meant to be a path.
- **The first release of a store** on a machine with a running older klatch: see #373.

## Reporting a problem

Use the Security tab.
MD
  printf '## Gaps found and ticketed\n\n| Gap | Ticket |\n|---|---|\n' > "$t/proto/audit/07-gaps-found-and-ticketed/_head.md"
  printf '\nTail text.\n' > "$t/proto/audit/07-gaps-found-and-ticketed/_tail.md"
  printf '| A key bundle proves a member and not its harness, so a worker is the generic floor | #880 |\n' > "$t/proto/audit/07-gaps-found-and-ticketed/a-key.md"
  printf '| A thing that was broken | fixed in #1196 |\n' > "$t/proto/audit/07-gaps-found-and-ticketed/b-fixed.md"
  printf '| Descriptive row, not a limit | #181 |\n' > "$t/proto/audit/07-gaps-found-and-ticketed/c-desc.md"
}
run() { "$here/release-notes.sh" "$@" > "$work/out" 2> "$work/err"; }

fresh
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then ok "exits 0 on good sources"; else bad "failed: $(cat "$work/err")"; fi
grep -q '0.2.0-alpha.1' "$work/out" && grep -q 'v0.2.0a1' "$work/out" && grep -q '2026-12-01' "$work/out" && ok "version, tag and date are filled" || bad "version, tag or date missing"
grep -q 'hostile process running as you' "$work/out" && grep -q 'Claim text is not sealed' "$work/out" && grep -q 'see #373' "$work/out" && ok "every SECURITY.md limit is listed" || bad "a SECURITY.md limit is missing"
grep -q 'Root can do anything' "$work/out" && ok "a wrapped SECURITY.md bullet is joined" || bad "wrapped bullet cut"
grep -q 'key bundle proves a member' "$work/out" && grep -q '#880' "$work/out" && ok "an open gap row is listed with its ticket" || bad "open gap missing"
grep -q 'A thing that was broken' "$work/out" && bad "a fixed gap row is listed" || ok "a fixed gap row is left out"
grep -q 'Descriptive row' "$work/out" && bad "a skipped ticket's row is listed" || ok "a descriptive row (#181) is left out"
if grep -q '{{' "$work/out"; then bad "a placeholder is left"; else ok "no placeholder is left"; fi
if grep -q $'\xe2\x80\x94' "$work/out"; then bad "an em dash is in the notes"; else ok "no em dash"; fi
for want in 'xattr -d com.apple.quarantine' 'cosign verify-blob' 'shasum -a 256 -c SHA256SUMS' 'gh attestation verify' 'brew install hslee16/klatch/klatch' 'pipx install klatch==0.2.0a1' 'cargo install klatch --version 0.2.0-alpha.1' 'sensitive data' 'Out of scope' 'klatch-server' 'macOS arm64' 'Platform tests workflow' 'TLS and mutual TLS' 'Codex adapter' 'Windows'; do
  grep -qF "$want" "$work/out" && ok "contains: $want" || bad "missing: $want"
done

fresh; sed -i.bak '/^## What is not protected/,/^## Reporting/{/^## Reporting/!d;}' "$t/SECURITY.md"; rm -f "$t/SECURITY.md.bak"
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then bad "a missing SECURITY.md section passed"; else ok "a missing SECURITY.md section fails"; fi
grep -q 'What is not protected' "$work/err" && ok "the failure names the section" || bad "failure text: $(cat "$work/err")"

fresh; rm -rf "$t/proto/audit/07-gaps-found-and-ticketed"
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then bad "a missing gap directory passed"; else ok "a missing gap directory fails"; fi

fresh; rm -f "$t/proto/audit/07-gaps-found-and-ticketed/a-key.md" "$t/proto/audit/07-gaps-found-and-ticketed/c-desc.md"
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then bad "no open gap rows passed"; else ok "no open gap rows fails (the extraction is broken)"; fi

fresh; sed -i.bak 's/^Root can do anything\./Root can do anything {{X}}/' "$t/SECURITY.md"; sed -i.bak 's/running klatch. Root/running klatch {{X}}. Root/' "$t/SECURITY.md"; rm -f "$t/SECURITY.md.bak"
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then bad "a placeholder in a source passed"; else ok "a placeholder in a source fails"; fi

fresh; printf -- '- **Dash.** A \xe2\x80\x94 B.\n' >> "$t/SECURITY.md"; sed -i.bak 's/^## Reporting a problem/- **Dash.** A \xe2\x80\x94 B.\n\n## Reporting a problem/' "$t/SECURITY.md"; rm -f "$t/SECURITY.md.bak"
if run 0.2.0-alpha.1 --date 2026-12-01 --root "$t"; then bad "an em dash in a source passed"; else ok "an em dash in a source fails"; fi

if run 0.2 --root "$t"; then bad "a bad version passed"; else ok "a version that is not SemVer fails"; fi
if run --root "$t"; then bad "no version passed"; else ok "no version fails"; fi

# the real tree: notes for 0.2.0-alpha.1 carry the epic's lists
if "$here/release-notes.sh" 0.2.0-alpha.1 --date 2026-12-01 --root "$ROOT" > "$work/real" 2> "$work/err"; then ok "the real tree produces notes"; else bad "real tree: $(cat "$work/err")"; fi
for want in 'No outside security review' 'Linux key custody' 'Loopback HTTP and MCP' '#1146' 'Remote transports' 'Windows' 'acp'; do
  grep -qiF "$want" "$work/real" && ok "real notes contain: $want" || bad "real notes miss: $want"
done
exit "$fails"
