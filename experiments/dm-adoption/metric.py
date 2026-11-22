#!/usr/bin/env python3
"""Read a klatch store after a run and print one JSON line: did the exchange move to a dm? (#1194)

The count comes from what the store holds, never from what an agent says about itself. Input is the text of
`klatch history --last 200` read as one of the members (every room that member joined, each under a `[room]` header,
rows as `[seq ts sender] body`), either from a file (--history FILE, used by the tests) or by running the binary
(--klatch BIN --as NAME, with KLATCH_HOME and KLATCH_KEY_DIR from the environment; the caller points them at a
throwaway store).

An exchange is a pair of members that posted 3 or more messages between them in one room (a room with three or more
posters is read pair by pair, so the unit is meant for runs with two agents). It moved when the
pair has a `dm.` room or an `ask-N` working room holding at least --min-dm messages (default 3). adoption is moved
divided by exchanges (0.0 to 1.0), or null when there was no exchange (outcome "no-exchange", which repeat.py counts
as a failed run instead of a silent zero).

The line is in the form experiments/variance/repeat.py reads: {"agents": {"exchange": {...}, "alpha": {...}}}.
`--hook-events NAME=FILE` (stream-json recorded with --include-hook-events) adds per agent hook_events,
hook_post_tool_use and nudge_lines, and the sum of nudge_lines on the exchange entry (#1210).
`--result NAME=FILE` adds an agent from a `claude -p --output-format json` result (turns, tokens, cost_usd,
duration_s, outcome). Besides adoption the exchange entry has exchanges, moved, lobby_messages, dm_messages
(dm and ask-N rooms), reply_latency_s (mean gap between consecutive messages by different members in dm and ask-N
rooms) and lobby_reply_latency_s (the same in the lobby), both null when there is no such gap. Timestamps in the store
are epoch seconds (milliseconds are recognised by size).

This script spends nothing and starts no agent. It only reads.
"""
import argparse
import itertools
import json
import os
import re
import subprocess
import sys

ROOM = re.compile(r"^\[([^\s\]]+)\]\s*$")
ROW = re.compile(r"^\[(\d+) (\d+) ([^\]]*)\] ?(.*)$")
MORE = re.compile(r"klatch history (\S+) --since (\d+)\]")
DM_NAME = re.compile(r"^dm\.([^.]+)\.([^.]+)$")
ASK_NAME = re.compile(r"^ask-\d+$")
PAGE_LIMIT = 50


def parse(text):
    """{room: [(seq, ts, sender, body), ...]} from the all-rooms history text."""
    rooms = {}
    cur = None
    for line in text.splitlines():
        m = ROOM.match(line)
        if m:
            cur = m.group(1)
            rooms.setdefault(cur, [])
            continue
        m = ROW.match(line)
        if m and cur is not None:
            sender = m.group(3).split()[0] if m.group(3).split() else m.group(3)
            rooms[cur].append((int(m.group(1)), int(m.group(2)), sender, m.group(4)))
    return rooms


def is_working_room(name):
    return name.startswith("dm.") or bool(ASK_NAME.match(name))


def members_of(name, msgs):
    senders = {s for _, _, s, _ in msgs}
    m = DM_NAME.match(name)
    if m:
        senders |= {m.group(1), m.group(2)}
    return senders


def secs(ts):
    return ts / 1000.0 if ts > 10**11 else float(ts)


def gaps(msgs):
    """Seconds between each message and the previous one when the sender differs."""
    out = []
    ordered = sorted(msgs, key=lambda m: m[0])
    for prev, cur in zip(ordered, ordered[1:]):
        if prev[2] != cur[2]:
            out.append(max(0.0, secs(cur[1]) - secs(prev[1])))
    return out


def mean(xs):
    return sum(xs) / len(xs) if xs else None


def analyse(rooms, min_dm):
    lobby = rooms.get("lobby", [])
    working = {n: m for n, m in rooms.items() if is_working_room(n)}
    # an exchange: two members who together posted 3 or more messages in one room
    pairs = {}
    for name, msgs in rooms.items():
        who = sorted(members_of(name, msgs))
        for a, b in itertools.combinations(who, 2):
            theirs = [m for m in msgs if m[2] in (a, b)]
            pairs.setdefault((a, b), {})[name] = theirs
    exchanges = 0
    moved = 0
    for per_room in pairs.values():
        if not any(len(m) >= 3 for m in per_room.values()):
            continue
        exchanges += 1
        if any(is_working_room(n) and len(m) >= min_dm for n, m in per_room.items()):
            moved += 1
    dm_msgs = [m for msgs in working.values() for m in msgs]
    rec = {
        "exchanges": exchanges,
        "moved": moved,
        "adoption": (moved / exchanges) if exchanges else None,
        "lobby_messages": len(lobby),
        "dm_messages": len(dm_msgs),
        "reply_latency_s": mean([g for msgs in working.values() for g in gaps(msgs)]),
        "lobby_reply_latency_s": mean(gaps(lobby)),
        "outcome": "ok" if exchanges else "no-exchange",
    }
    return rec


def agent_from_result(path):
    try:
        with open(path) as f:
            obj = json.loads(f.read().strip().splitlines()[-1])
    except (OSError, ValueError, IndexError):
        return {"outcome": "no-result"}
    if not isinstance(obj, dict):
        return {"outcome": "no-result"}
    usage = obj.get("usage") or {}
    rec = {}
    if "num_turns" in obj:
        rec["turns"] = obj["num_turns"]
    if usage:
        rec["tokens"] = sum(
            usage.get(k, 0) or 0
            for k in ("input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")
        )
    if "total_cost_usd" in obj:
        rec["cost_usd"] = obj["total_cost_usd"]
    if "duration_ms" in obj:
        rec["duration_s"] = obj["duration_ms"] / 1000.0
    rec["outcome"] = "error" if obj.get("is_error") else "ok"
    return rec


# the nudge line of every wording so far starts "klatch: you and NAME exchanged N messages in ROOM"; counted at the
# start of a line, so a model quoting it mid-sentence or a message body (flattened to one row) does not count
NUDGE_LINE = re.compile(r"^[ \t]*klatch: you and \S+ exchanged \d+ messages in \S+", re.M)


def tool_result_texts(ev):
    """The text of every tool_result block in a stream-json `user` event (a string or a list of text blocks)."""
    msg = ev.get("message")
    content = msg.get("content") if isinstance(msg, dict) else None
    if not isinstance(content, list):
        return []
    out = []
    for block in content:
        if not isinstance(block, dict) or block.get("type") != "tool_result":
            continue
        c = block.get("content")
        if isinstance(c, str):
            out.append(c)
        elif isinstance(c, list):
            out.extend(b.get("text", "") for b in c if isinstance(b, dict) and isinstance(b.get("text"), str))
    return out


def hook_counts(path):
    """Count what the hooks said in a `claude -p --output-format stream-json --include-hook-events` stream (#1210).

    hook_events is the number of hook_response events, hook_post_tool_use those of the PostToolUse hook, hook_nudge_lines
    the nudge lines in hook output (one text counted once though output and stdout repeat it), tool_result_nudge_lines
    those in tool results (what `klatch wait` and `klatch read` print, #1222), and nudge_lines their sum. Only hook
    events and tool results are read: a model quoting the line does not count. A missing file counts zero.
    """
    rec = {"hook_events": 0, "hook_post_tool_use": 0, "hook_nudge_lines": 0, "tool_result_nudge_lines": 0, "nudge_lines": 0}
    try:
        f = open(path)
    except OSError:
        return rec
    with f:
        for line in f:
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if not isinstance(ev, dict):
                continue
            if ev.get("type") == "user":
                for text in tool_result_texts(ev):
                    rec["tool_result_nudge_lines"] += len(NUDGE_LINE.findall(text))
                continue
            if ev.get("type") != "system" or ev.get("subtype") != "hook_response":
                continue
            rec["hook_events"] += 1
            if ev.get("hook_event") == "PostToolUse":
                rec["hook_post_tool_use"] += 1
            text = ev.get("output") or ev.get("stdout") or ""
            if isinstance(text, str):
                rec["hook_nudge_lines"] += len(NUDGE_LINE.findall(text))
    rec["nudge_lines"] = rec["hook_nudge_lines"] + rec["tool_result_nudge_lines"]
    return rec


def run_history(binary, member, args):
    env = dict(os.environ)
    env["KLATCH_NAME"] = member
    p = subprocess.run([binary, "history"] + args, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env)
    return p.stdout


def read_store(binary, member):
    """All rooms the member joined, paged past the history byte budget."""
    text = run_history(binary, member, ["--last", "200"])
    rooms = parse(text)
    for room in list(rooms):
        section = section_of(text, room)
        for _ in range(PAGE_LIMIT):
            m = MORE.search(section)
            if not m or m.group(1) != room:
                break
            section = run_history(binary, member, [room, "--since", m.group(2)])
            rooms[room].extend(parse("[%s]\n%s" % (room, section)).get(room, []))
    return rooms


def section_of(text, room):
    lines = text.splitlines()
    out = []
    on = False
    for line in lines:
        m = ROOM.match(line)
        if m:
            on = m.group(1) == room
            continue
        if on:
            out.append(line)
    return "\n".join(out)


def main():
    p = argparse.ArgumentParser(description="dm adoption metric from a klatch store (#1194).")
    p.add_argument("--history", help="read this file (the text of `klatch history` in every room) instead of running klatch")
    p.add_argument("--klatch", help="the klatch binary to read the store with")
    p.add_argument("--as", dest="member", default="alpha", help="the member to read as (default alpha)")
    p.add_argument("--min-dm", type=int, default=3, help="messages a dm or ask-N room needs to count as a move (default 3)")
    p.add_argument("--arm", help="the arm label, copied into the result")
    p.add_argument("--hook-events", action="append", default=[], metavar="NAME=FILE", help="a stream-json file with hook events for one agent; adds hook_events, hook_post_tool_use and nudge_lines")
    p.add_argument("--binary", help="the klatch binary under test, copied into the result")
    p.add_argument("--binary-sha256", help="its checksum, copied into the result")
    p.add_argument("--result", action="append", default=[], metavar="NAME=FILE", help="a claude -p JSON result for one agent")
    args = p.parse_args()
    if bool(args.history) == bool(args.klatch):
        p.error("give exactly one of --history FILE or --klatch BIN")
    if args.history:
        with open(args.history) as f:
            rooms = parse(f.read())
    else:
        rooms = read_store(args.klatch, args.member)
    agents = {}
    for item in args.result:
        name, _, path = item.partition("=")
        if not name or not path:
            p.error("--result wants NAME=FILE")
        agents[name] = agent_from_result(path)
    agents["exchange"] = analyse(rooms, args.min_dm)
    if args.hook_events:
        total = 0
        for item in args.hook_events:
            name, _, path = item.partition("=")
            if not name or not path:
                p.error("--hook-events wants NAME=FILE")
            counts = hook_counts(path)
            agents.setdefault(name, {}).update(counts)
            total += counts["nudge_lines"]
        agents["exchange"]["nudge_lines"] = total
    out = {"agents": agents}
    if args.arm:
        out["arm"] = args.arm
    if args.binary:
        out["binary"] = args.binary
    if args.binary_sha256:
        out["binary_sha256"] = args.binary_sha256
    print(json.dumps(out, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
