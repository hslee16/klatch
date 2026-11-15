#!/usr/bin/env python3
"""Post one line to a running Claude Code session's inbox socket, as a process that is not its child.
  post.py PID TEXT [--auth none|wrong|right] [--frm NAME]
Reads only ~/.claude/sessions/PID.json and (for --auth right) the PID key file. Never prints the token."""
import argparse, json, os, socket, uuid
p = argparse.ArgumentParser(); p.add_argument("pid"); p.add_argument("text")
p.add_argument("--auth", default="none"); p.add_argument("--frm", default=None)
a = p.parse_args()
d = os.path.expanduser("~/.claude/sessions")
rec = json.load(open(f"{d}/{a.pid}.json"))
s = socket.socket(socket.AF_UNIX); s.connect(rec["messagingSocketPath"])
if a.auth != "none":
    tok = "00" * 16
    if a.auth == "right":
        k = [f for f in os.listdir(d) if f.startswith(f"{a.pid}.") and f.endswith(".key")][0]
        tok = json.load(open(f"{d}/{k}"))["peerToken"]
    s.sendall((json.dumps({"type": "auth", "token": tok}) + "\n").encode())
line = {"type": "user", "message": {"role": "user", "content": a.text}, "priority": "now", "uuid": str(uuid.uuid4())}
if a.frm:
    line["from"] = a.frm
s.sendall((json.dumps(line) + "\n").encode()); s.close(); print("sent")
