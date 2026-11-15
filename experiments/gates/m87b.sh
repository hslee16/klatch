#!/bin/bash
# matrix: real sender session (SendMessage) mode x receiver mode (x crossSessionInbound)
V=${V:-$HOME/.local/share/claude/versions/2.1.325}
D="$(dirname "$0")"
run() { python3 "$D/recv.py" --bin $V --expiry-ms 8000 --wait 30 "$@" 2>&1 | tail -1; }
for s in default bypassPermissions; do for r in default bypassPermissions; do run --mode $r --sender-mode $s; done; done
run --mode default --sender-mode acceptEdits
run --mode bypassPermissions --sender-mode default --inbound accept
run --mode default --sender-mode bypassPermissions --inbound accept
run --mode default --sender-mode default --inbound refuse
