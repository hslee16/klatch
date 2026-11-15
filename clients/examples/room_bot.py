#!/usr/bin/env python3
"""The smallest klatch integration: read a room's history, post, resume. Standard library only.

    KLATCH_NAME=my-bot python3 room_bot.py ROOM        (or KLATCH_IDENTITY_FILE=/path/to/bundle for a verified member)
"""
import json
import os
import subprocess
import sys

KLATCH = os.environ.get("KLATCH_BIN", "klatch")


def frames(args):
    """Spawn klatch, yield each frame it prints (one JSON object per line), and wait for it to exit."""
    child = subprocess.Popen([KLATCH, *args], stdout=subprocess.PIPE, text=True)
    for line in child.stdout:
        if line.strip():
            frame = json.loads(line)
            if frame.get("v") != 1:
                raise SystemExit(f"unsupported protocol version: {frame.get('v')!r}")
            yield frame  # a frame type you do not know is yours to ignore
    if child.wait() != 0:
        raise SystemExit(f"klatch {args[0]} failed")


def history(room, from_seq=0):
    """Hello, then every message after from_seq, then caught_up. `klatch stream` needs no daemon."""
    who, messages = None, []
    for frame in frames(["stream", room, "--from-seq", str(from_seq)]):
        if frame["type"] == "hello":
            who = f"{frame['member']} ({frame['attribution']})"
        elif frame["type"] == "message":
            messages.append(frame)
        elif frame["type"] == "caught_up":
            break
    return who, messages


def post(room, body):
    """One request on a `serve --stdio` session; the response carrying our id is the answer."""
    session = subprocess.Popen(
        [KLATCH, "serve", "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
    )
    try:
        next(session.stdout)  # the hello frame comes first
        request = {"v": 1, "type": "request", "id": "1", "method": "post", "params": {"room": room, "body": body}}
        session.stdin.write(json.dumps(request) + "\n")
        for line in session.stdout:
            frame = json.loads(line)
            if frame["type"] == "response" and frame["id"] == "1":
                if not frame["ok"]:
                    raise SystemExit(f"refused: {frame['error']['code']}: {frame['error']['message']}")
                return frame["result"]["seq"]
    finally:
        session.stdin.close()  # closing the input ends the session
        session.wait()


room = sys.argv[1]
who, messages = history(room)
print("member:", who)
for m in messages:
    print(f"history: {m['sender']}: {m['body']}")
last_seq = messages[-1]["seq"] if messages else 0
me = who.split(" ")[0]
seq = post(room, f"hello from {me}")
print("posted: seq", seq)
_, later = history(room, from_seq=last_seq)  # resume: only what came after the last seq we processed
for m in later:
    print(f"resumed: {m['sender']}: {m['body']}")
