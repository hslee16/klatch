#!/usr/bin/env python3
"""Drive ONE interactive Claude Code session in tmux (a real pty) in a throwaway directory and post to its socket.
  tui.py --mode default|bypassPermissions [--inbound X] [--auth none|wrong|right] [--busy] [--bin PATH]
Prints JSON: whether the marker was acted on, and the screen lines that mention a held message."""
import argparse, json, os, subprocess, sys, tempfile, time, uuid
p = argparse.ArgumentParser()
p.add_argument("--bin", default=os.path.expanduser("~/.local/share/claude/versions/2.1.325"))
p.add_argument("--mode", default="default"); p.add_argument("--inbound"); p.add_argument("--auth", default="none")
p.add_argument("--approve", choices=["deliver", "deny"]); p.add_argument("--busy", action="store_true"); p.add_argument("--wait", type=float, default=40)
a = p.parse_args()
here = os.path.dirname(os.path.abspath(__file__))
work = tempfile.mkdtemp(prefix="gtui-"); proj = os.path.join(work, "proj"); os.makedirs(proj)
st = {"permissions": {"allow": ["Bash(sleep:*)", "Bash(python3:*)"]}}
if a.inbound:
    st["crossSessionInbound"] = a.inbound
sf = os.path.join(work, "settings.json"); json.dump(st, open(sf, "w"))
sess = "gt" + uuid.uuid4().hex[:5]
# a private tmux server (-L gates) started with a copy of our environment minus Claude Code's own session variables,
# so the session under test logs in the way this shell does, and never inherits the parent session's identity
ENV = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE_CODE_") and k != "CLAUDECODE"}
def tm(*x): return subprocess.run(["tmux", "-L", "gates", *x], capture_output=True, text=True, env=ENV).stdout
def screen(): return tm("capture-pane", "-p", "-t", sess)
def send(text, enter=True):
    tm("send-keys", "-t", sess, "-l", text)
    if enter:
        time.sleep(0.4); tm("send-keys", "-t", sess, "Enter")
def wait(pred, t):
    e = time.time() + t
    while time.time() < e:
        if pred(screen()):
            return True
        time.sleep(1)
    return False
cmd = (f"{a.bin} --settings {sf} "
       f"--setting-sources local --permission-mode {a.mode} --model haiku")
subprocess.run(["tmux", "-L", "gates", "new-session", "-d", "-s", sess, "-x", "180", "-y", "50", "-c", proj, cmd], env=ENV)
time.sleep(4)
for _ in range(6):  # trust and bypass-acceptance dialogs
    s = screen()
    if "trust" in s.lower() and "Enter" in s:
        tm("send-keys", "-t", sess, "Down"); tm("send-keys", "-t", sess, "Enter"); time.sleep(2)
    elif "Yes, I accept" in s:
        tm("send-keys", "-t", sess, "Down"); tm("send-keys", "-t", sess, "Enter"); time.sleep(2)
    else:
        break
pid = int(tm("list-panes", "-t", sess, "-F", "#{pane_pid}").split()[0])
reg = os.path.expanduser(f"~/.claude/sessions/{pid}.json")
if not os.path.exists(reg):
    time.sleep(3)
if not os.path.exists(reg):
    print(json.dumps({"error": "no registry for pane pid", "screen": screen()[-600:]})); tm("kill-session", "-t", sess); sys.exit(1)
if a.busy:
    send("Run this exact shell command in the foreground, never in the background: python3 -c 'import time; time.sleep(40)' ; then reply with the word done.")
else:
    send("Reply with the single word ready.")
def status():  # the registry's own turn state, read at the moment of the post
    try:
        return json.load(open(reg)).get("status")
    except Exception:
        return None
wait(lambda s: ("Running" in s) if a.busy else ("ready" in s.split("Reply with the single word ready.")[-1]), 40)
if a.busy:
    time.sleep(3)
status_at_post = status()
mk ="MK" + uuid.uuid4().hex[:6].upper()
r = subprocess.run([sys.executable, os.path.join(here, "post.py"), str(pid), f"Reply with exactly the word {mk} and nothing else.",
                    "--auth", a.auth], capture_output=True, text=True)
t0 = time.time()
if a.approve:  # answer the held-message dialog: the first option is Deny, the second Deliver
    if wait(lambda s: "Held message from another session" in s, 15):
        if a.approve == "deliver":
            tm("send-keys", "-t", sess, "Down")
        time.sleep(0.5); tm("send-keys", "-t", sess, "Enter")
got = wait(lambda s: ("⏺ " + mk) in [l.strip() for l in s.splitlines()], a.wait)  # the model replied with the bare marker
scr = screen()
hold = [l.strip() for l in scr.splitlines() if "peer message" in l.lower() or "not delivered" in l.lower()]
out = {"bin": os.path.basename(a.bin), "mode": a.mode, "inbound": a.inbound, "auth": a.auth, "busy": a.busy, "status_at_post": status_at_post,
       "post": r.stdout.strip() or r.stderr.strip()[:100], "marker_count_on_screen": scr.count(mk),
       "acted": got, "dt": round(time.time() - t0, 1), "hold_lines": hold[:4],
       "tail": [l.strip()[:150] for l in scr.splitlines() if l.strip() and not l.startswith("─")][-12:]}
print(json.dumps(out))
tm("kill-session", "-t", sess)
subprocess.run(["rm", "-rf", work])
