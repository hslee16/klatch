#!/usr/bin/env python3
"""asyncRewake in the interactive TUI under a delay (ticket #85). One real Claude Code TUI in tmux, throwaway dir.
A SessionStart hook (async + asyncRewake, timeout T) sleeps DELAY seconds, writes the epoch to a file, prints a marker
on stderr and exits 2. The session answers one prompt first, so it is idle when the hook ends.
  wake.py --delay 30 [--timeout 900] [--busy-at-end] [--bin PATH] [--headless]
No klatch involved: this isolates Claude Code's behaviour. Prints JSON."""
import argparse, json, os, subprocess, sys, tempfile, time, uuid
p = argparse.ArgumentParser()
p.add_argument("--bin", default=os.path.expanduser("~/.local/share/claude/versions/2.1.325"))
p.add_argument("--delay", type=int, default=30); p.add_argument("--timeout", type=int, default=900)
p.add_argument("--busy-at-end", action="store_true", help="make the session run a foreground tool when the hook ends")
p.add_argument("--hold", type=int, default=0, help="extra seconds to wait beyond the delay")
a = p.parse_args()
work = tempfile.mkdtemp(prefix="gwake-"); proj = os.path.join(work, "proj"); os.makedirs(proj)
mk = "WK" + uuid.uuid4().hex[:6].upper(); stamp = os.path.join(work, "stamp")
cmd = f"sleep {a.delay}; date +%s > {stamp}; echo 'hook says: reply with exactly the word {mk} and nothing else' >&2; exit 2"
st = {"permissions": {"allow": ["Bash(sleep:*)", "Bash(python3:*)"]},
      "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": cmd, "async": True, "asyncRewake": True, "timeout": a.timeout}]}]}}
sf = os.path.join(work, "settings.json"); json.dump(st, open(sf, "w"))
sess = "gw" + uuid.uuid4().hex[:5]
# a private tmux server (-L gates) started with a copy of our environment minus Claude Code's own session variables,
# so the session under test logs in the way this shell does, and never inherits the parent session's identity
ENV = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE_CODE_") and k != "CLAUDECODE"}
def tm(*x): return subprocess.run(["tmux", "-L", "gates", *x], capture_output=True, text=True, env=ENV).stdout
def screen(): return tm("capture-pane", "-p", "-t", sess, "-S", "-200")
def send(text):
    tm("send-keys", "-t", sess, "-l", text); time.sleep(0.4); tm("send-keys", "-t", sess, "Enter")
env = ""
subprocess.run(["tmux", "-L", "gates", "new-session", "-d", "-s", sess, "-x", "180", "-y", "50", "-c", proj,
                f"{env} {a.bin} --settings {sf} --setting-sources local --permission-mode default --model haiku"], env=ENV)
t_start = time.time(); time.sleep(4)
if "trust" in screen().lower():
    tm("send-keys", "-t", sess, "Down"); tm("send-keys", "-t", sess, "Enter"); time.sleep(2); t_start = time.time()
send("Reply with the single word ready.")
e = time.time() + 30
while time.time() < e and "ready" not in screen().split("Reply with the single word ready.")[-1]:
    time.sleep(1)
if a.busy_at_end:  # start a foreground tool call that is still running when the hook ends
    left = a.delay - (time.time() - t_start)
    time.sleep(max(0, left - 10))
    send("Run this exact shell command in the foreground, never in the background: python3 -c 'import time; time.sleep(25)' ; then say finished.")
deadline = time.time() + a.delay + 60 + a.hold
woke = None
while time.time() < deadline:
    s = screen()
    if os.path.exists(stamp) and s.count(mk) >= 1:  # the model replies with the marker (the hook text itself is not shown)
        woke = time.time(); break
    time.sleep(2)
hook_end = int(open(stamp).read()) if os.path.exists(stamp) else None
s = screen()
out = {"bin": os.path.basename(a.bin), "delay": a.delay, "timeout": a.timeout, "busy_at_end": a.busy_at_end,
       "hook_finished": hook_end is not None, "marker_count_on_screen": s.count(mk),
       "woke": woke is not None, "wake_latency_s": round(woke - hook_end, 1) if woke and hook_end else None,
       "tail": [l.strip()[:150] for l in s.splitlines() if l.strip() and not l.startswith("─")][-8:]}
print(json.dumps(out))
tm("kill-session", "-t", sess)
subprocess.run(["rm", "-rf", work])
