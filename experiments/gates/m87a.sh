#!/bin/bash
# matrix: receiver permission mode x crossSessionInbound, raw socket sender (cannot attest a mode)
V=${V:-$HOME/.local/share/claude/versions/2.1.325}
for mode in default acceptEdits plan bypassPermissions; do
  for inb in "" accept hold refuse; do
    python3 "$(dirname "$0")/recv.py" --bin $V --mode $mode ${inb:+--inbound $inb} --expiry-ms 8000 --wait 20 2>&1 | tail -1
  done
done
