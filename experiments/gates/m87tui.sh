#!/bin/bash
# interactive TUI: a busy receiver (does the message wait for the tool to finish?) and the held-message dialog (#87)
V=${V:-$HOME/.local/share/claude/versions/2.1.325}
D="$(dirname "$0")"
run() { python3 "$D/tui.py" --bin $V "$@" 2>&1 | tail -1; }
run --mode default --busy --wait 70
run --mode bypassPermissions --approve deliver --wait 20
run --mode bypassPermissions --approve deny --wait 20
