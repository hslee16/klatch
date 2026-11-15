#!/bin/bash
# interactive TUI (tmux pty), raw socket sender: receiver mode x busy x auth x crossSessionInbound (tickets #86, #87)
V=${V:-$HOME/.local/share/claude/versions/2.1.325}
D="$(dirname "$0")"
run() { python3 "$D/tui.py" --bin $V --wait 25 "$@" 2>&1 | tail -1; }
run --mode default --auth wrong
run --mode default --busy
run --mode bypassPermissions
run --mode bypassPermissions --busy
run --mode bypassPermissions --inbound accept --auth wrong
run --mode default --inbound hold
run --mode default --inbound refuse
