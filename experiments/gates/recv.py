#!/usr/bin/env python3
"""Inbound gate probe (tickets #85 #86 #87). Starts ONE headless Claude Code receiver in a throwaway directory,
posts to its inbox socket from this process (not a child of the receiver), and prints what the receiver did.

  recv.py --bin PATH --mode default|bypassPermissions|... [--inbound accept|hold|refuse] [--auth none|wrong|right]
          [--busy] [--from NAME] [--msgs N] [--wait SECONDS]

It never touches a real klatch store. It uses the user's Claude login (an isolated CLAUDE_CONFIG_DIR logs out) with
a throwaway --settings file and --setting-sources local. Only the receiver's own cwd is a scratch directory.
Output: one JSON object on stdout (events seen: peer_message_hold frames, whether the marker was acted on)."""
import argparse, json, os, socket, subprocess, sys, tempfile, threading, time, uuid

p = argparse.ArgumentParser()
p.add_argument("--bin", default=os.path.expanduser("~/.local/share/claude/versions/2.1.325"))
p.add_argument("--mode", default="default")
p.add_argument("--inbound", default=None)
p.add_argument("--auth", default="none", choices=["none", "wrong", "right"])
p.add_argument("--busy", action="store_true", help="post while the receiver is mid-turn (a 25 s sleep)")
p.add_argument("--from", dest="frm", default=None)
p.add_argument("--msgs", type=int, default=1)
p.add_argument("--wait", type=float, default=40)
p.add_argument("--model", default="haiku")
p.add_argument("--expiry-ms", default=None, help="shorten the hold approval deadline (env CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS)")
p.add_argument("--sender-mode", default=None, help="send with a real second Claude session (SendMessage) in this permission mode, instead of the raw socket")
p.add_argument("--extra", default="", help="extra raw line to send before the message (JSON)")
a = p.parse_args()

work = tempfile.mkdtemp(prefix="gates-")
proj = os.path.join(work, "proj"); os.makedirs(proj)
settings = {"permissions": {"allow": ["Bash(sleep:*)", "Bash(python3:*)"]}}
if a.inbound:
    settings["crossSessionInbound"] = a.inbound
sf = os.path.join(work, "settings.json"); json.dump(settings, open(sf, "w"))
cmd = [a.bin, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
       "--model", a.model, "--permission-mode", a.mode, "--settings", sf, "--setting-sources", "local"]
env = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE_CODE_") and k != "CLAUDECODE"}
if a.expiry_ms: env["CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS"] = a.expiry_ms
proc = subprocess.Popen(cmd, cwd=proj, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                        text=True, env=env)
events = []
def reader():
    for line in proc.stdout:
        try:
            events.append((time.time(), json.loads(line)))
        except Exception:
            pass
threading.Thread(target=reader, daemon=True).start()

def user(text):
    proc.stdin.write(json.dumps({"type": "user", "message": {"role": "user", "content": text}}) + "\n")
    proc.stdin.flush()

def results():
    return [e for _, e in events if e.get("type") == "result"]

def wait_for(pred, timeout):
    t = time.time()
    while time.time() - t < timeout:
        if pred():
            return True
        time.sleep(0.3)
    return False

if a.busy:
    user("Run this exact shell command in the foreground, never in the background: python3 -c 'import time; time.sleep(30)' ; then reply with the word done.")
else:
    user("Reply with the single word ready.")
# the registry record appears when the session starts
reg = os.path.expanduser(f"~/.claude/sessions/{proc.pid}.json")
if not wait_for(lambda: os.path.exists(reg), 20):
    print(json.dumps({"error": "no registry record", "stderr": proc.stderr.read()[:300] if proc.poll() else ""})); proc.kill(); sys.exit(1)
rec = json.load(open(reg))
sock_path = rec["messagingSocketPath"]
if a.busy:
    wait_for(lambda: any(e.get("type") == "assistant" and "tool_use" in json.dumps(e) for _, e in events), 30)
    time.sleep(2)
else:
    wait_for(lambda: len(results()) >= 1, 40)
tokfile = [f for f in os.listdir(os.path.dirname(reg)) if f.startswith(f"{proc.pid}.") and f.endswith(".key")]
token = None
if tokfile:
    try: token = json.load(open(os.path.join(os.path.dirname(reg), tokfile[0]))).get("peerToken")
    except Exception: pass

markers = []
try: status_at_post = json.load(open(reg)).get("status")
except Exception: status_at_post = None
t_post = time.time()
sender_out = None
if a.sender_mode:
    mk = "MK" + uuid.uuid4().hex[:6].upper(); markers.append(mk)
    sproj = os.path.join(work, "sender"); os.makedirs(sproj)
    sset = os.path.join(work, "sender.json"); json.dump({}, open(sset, "w"))
    prompt = (f"Use the ListAgents tool, find the other Claude session named {rec.get('name')}, and use the SendMessage tool to send it "
              f"exactly this text: Reply with exactly the word {mk} and nothing else. Then say sent, or say what went wrong in one line.")
    sp = subprocess.run([a.bin, "-p", prompt, "--output-format", "json", "--model", a.model, "--permission-mode", a.sender_mode,
                         "--settings", sset, "--setting-sources", "local", "--allowedTools", "ListAgents", "SendMessage"],
                        cwd=sproj, capture_output=True, text=True, env=env, timeout=120)
    try: sender_out = json.loads(sp.stdout).get("result", "")[:300]
    except Exception: sender_out = (sp.stdout + sp.stderr)[:300]
for i in range(0 if a.sender_mode else a.msgs):
    mk = "MK" + uuid.uuid4().hex[:6].upper()
    markers.append(mk)
    s = socket.socket(socket.AF_UNIX); s.connect(sock_path)
    if a.auth != "none":
        s.sendall((json.dumps({"type": "auth", "token": token if a.auth == "right" else "00" * 16}) + "\n").encode())
    if a.extra:
        s.sendall((a.extra + "\n").encode())
    line = {"type": "user", "message": {"role": "user", "content": f"Reply with exactly the word {mk} and nothing else."},
            "priority": "now", "uuid": str(uuid.uuid4())}
    if a.frm: line["from"] = a.frm
    s.sendall((json.dumps(line) + "\n").encode()); s.close()
    time.sleep(0.5)

def acted():
    txt = json.dumps([e for _, e in events if e.get("type") == "assistant"])
    return [m for m in markers if txt.count(m) >= 1 and any(m in json.dumps(e) for _, e in events if e.get("type") == "assistant")]
wait_for(lambda: len(acted()) == len(markers), a.wait)
holds = [{"dt": round(t - t_post, 1), **{k: e.get(k) for k in ("state", "cause", "outcome", "lane", "from")}}
         for t, e in events if e.get("subtype") == "peer_message_hold"]
out = {"bin": os.path.basename(a.bin), "mode": a.mode, "inbound": a.inbound, "auth": a.auth, "busy": a.busy, "status_at_post": status_at_post,
       "from": a.frm, "expiry_ms": a.expiry_ms, "version": rec.get("version"), "posted": len(markers), "acted_on": len(acted()),
       "sender_mode": a.sender_mode, "sender_said": sender_out, "receiver_name": rec.get("name"), "holds": holds, "results": len(results())}
print(json.dumps(out))
proc.kill()
subprocess.run(["rm", "-rf", work])
