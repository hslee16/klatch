#!/bin/bash
# Does a session keep its id when resumed, and does a fork get a new one? (ticket #30, Q26)
# Claude Code: runs three tiny headless turns (a few cents). Codex: `exec resume` reports the thread id before it
# needs an API key, so that part costs nothing; `codex fork` is interactive only and is not run here.
# Usage: experiments/identity/resume_fork.sh [claude|codex]   (default: both)
set -u
WHICH=${1:-both}
DIR=$(mktemp -d /private/tmp/klatch-identity.XXXXXX)
cd "$DIR" || exit 1

if [ "$WHICH" = both ] || [ "$WHICH" = claude ]; then
  id() { python3 -I -c "import json,sys;print(json.load(sys.stdin)['session_id'])"; }
  A=$(claude -p "reply with the single word ok" --output-format json --setting-sources local < /dev/null | id)
  B=$(claude -p "reply with the single word ok" --output-format json --setting-sources local --resume "$A" < /dev/null | id)
  C=$(claude -p "reply with the single word ok" --output-format json --setting-sources local --resume "$A" --fork-session < /dev/null | id)
  echo "claude original: $A"
  echo "claude resume:   $B  $([ "$A" = "$B" ] && echo SAME || echo DIFFERENT)"
  echo "claude fork:     $C  $([ "$A" != "$C" ] && echo NEW || echo SAME)"
fi

if [ "$WHICH" = both ] || [ "$WHICH" = codex ]; then
  tid() { grep -m1 '"thread.started"' | python3 -I -c "import json,sys;print(json.loads(sys.stdin.readline())['thread_id'])"; }
  A=$(timeout 60 codex exec --json --skip-git-repo-check "reply with the single word ok" < /dev/null 2>/dev/null | tid)
  B=$(timeout 60 codex exec resume "$A" --json --skip-git-repo-check "again" < /dev/null 2>/dev/null | tid)
  echo "codex original:  $A"
  echo "codex resume:    $B  $([ "$A" = "$B" ] && echo SAME || echo DIFFERENT)"
  echo "codex fork:      not run (codex fork is interactive only)"
fi
rm -rf "$DIR"
