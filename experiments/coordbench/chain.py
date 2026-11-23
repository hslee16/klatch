"""Stream metrics of scenario S3 (#1368): the blocked agent's idle and wasted turns, and the handoff claim record.

Everything here is computed from the agents' stream-json files (the same events for the fake and the claude driver), never
from what an agent says about itself. Standard library only; nothing here starts a process. Definitions are in
scenarios/s3.json, metrics.definitions.

A turn is one assistant message (streamed duplicates of a message id are one turn). A blocked agent is waiting for a
dependency: a file another agent publishes. The dependency is SEEN at the first turn whose tool result shows the file (a
`git pull` diffstat, a listing, its content) and does not say it is missing; a message that merely names the file does not
count, because the work is blocked until the file is there, not until someone says it is coming.
"""
import json
import os
import re
import shlex

NEGATIVE = re.compile(r"No such file|not found|does not exist|ENOENT|cannot access|fatal:|[Ee]rror", re.I)
# One simple command that only waits or looks: sleep, klatch waits and reads of the board, git fetch and friends, a listing
# or an existence test, or reading the shared task board.
POLL = re.compile(
    r"^\s*(?:sleep\b|timeout\b|watch\b"
    r"|(?:\S*/)?klatch\s+(?:-\S+\s+)*(?:wait|read|history|board|who|claims|asks|stuck|hooks|inbox|find)\b"
    r"|git\s+(?:-\S+\s+)*(?:fetch|pull|log|status|ls-remote|remote|diff|show)\b"
    r"|ls\b|test\b|\[\s|stat\b|cat\s+\S*TASKS\.md\b|true\b|echo\b)")
WRITES = re.compile(r"(>|\btee\b|\bcp\s|\bmv\s|\bsed\s+-i|\binstall\s)")
SEPARATORS = re.compile(r"\s*(?:&&|\|\||;|\n)\s*")
FILTER = re.compile(r"^\s*(?:head|tail|wc|grep|sort|cut|awk|tr|uniq|sed\s+-n)\b")
CLAIM_VERBS = ("claim", "release", "post", "dm", "ask", "answer", "handoff")
KLATCH_CMD = r"(?:\S*/)?klatch\s+(?:-\S+\s+)*(%s)\b([^\n;&|]*)"


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and isinstance(b.get("text"), str))
    return ""


def parse_stream(lines):
    """Turns of one agent: [{"i": 1.., "text", "tools": [{"name", "input", "result"}]}], in the order the messages came."""
    turns, by_id, tools = [], {}, {}
    for line in lines:
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if not isinstance(e, dict):
            continue
        if e.get("type") == "assistant":
            msg = e.get("message") or {}
            mid = msg.get("id") or "anon-%d" % len(turns)
            if mid not in by_id:
                by_id[mid] = {"i": len(turns) + 1, "text": "", "tools": []}
                turns.append(by_id[mid])
            turn = by_id[mid]
            for b in msg.get("content") or []:
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "tool_use" and b.get("id") not in tools:
                    t = {"name": b.get("name", "?"), "input": b.get("input") or {}, "result": ""}
                    tools[b.get("id")] = t
                    turn["tools"].append(t)
                elif b.get("type") == "text" and isinstance(b.get("text"), str):
                    turn["text"] = b["text"]
        elif e.get("type") == "user":
            for b in (e.get("message") or {}).get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_result" and b.get("tool_use_id") in tools:
                    tools[b["tool_use_id"]]["result"] = _text_of(b.get("content"))
    return turns


def _command(tool):
    inp = tool["input"] if isinstance(tool["input"], dict) else {}
    return inp.get("command", "") if tool["name"] == "Bash" else ""


def _is_klatch_read(tool):
    return bool(re.search(r"(?:^|[\s;&|])(?:\S*/)?klatch\s", _command(tool))) or tool["name"] in ("SendMessage", "ListAgents")


def is_poll(tool):
    """True when one tool call only waits or looks. A Bash command is a poll when every simple command in it is (a
    `sleep 3; python3 -m unittest` is not); a Read of the shared task board and a ListAgents call are polls."""
    if tool["name"] == "ListAgents":
        return True
    if tool["name"] in ("Read", "Glob", "Grep"):
        blob = json.dumps(tool["input"])
        return "TASKS.md" in blob
    if tool["name"] != "Bash":
        return False
    cmds = [c for c in SEPARATORS.split(_command(tool)) if c.strip()]

    def poll_pipeline(cmd):  # a poll, then only filters of its output
        first, *rest = [p for p in re.split(r"(?<!\|)\|(?!\|)", cmd)]
        return bool(POLL.match(first)) and all(FILTER.match(p) for p in rest)
    return bool(cmds) and all(poll_pipeline(c) for c in cmds)


def _shows(tool, dependency):
    base = dependency.rsplit("/", 1)[-1]
    res = tool["result"] or ""
    if _is_klatch_read(tool) or base not in res:
        return False
    return not NEGATIVE.search(res)


def _writes_guarded(tool, guarded):
    inp = tool["input"] if isinstance(tool["input"], dict) else {}
    blob = json.dumps(inp)
    for g in guarded:
        if tool["name"] in ("Write", "Edit", "NotebookEdit", "MultiEdit") and str(inp.get("file_path", "")).endswith(g):
            return True
        if tool["name"] == "Bash" and g in _command(tool) and WRITES.search(_command(tool)):
            return True
        if tool["name"] not in ("Bash", "Write", "Edit", "NotebookEdit", "MultiEdit") and g in blob and "content" in inp:
            return True
    return False


def blocked_report(turns, dependency, guarded):
    """Idle and wasted turns of one agent that needs `dependency` (a path) before it can do its step.

    blocked_turns: turns before the dependency is seen (all turns when it never is). idle_turns: blocked turns that only
    wait or look (every tool call a poll, or no tool call at all). guess_build_turns: blocked turns that write a file in
    `guarded` (the dependent step, built before its input exists). wasted_turns: idle plus guess builds. A blocked turn that
    does anything else (reads the README, runs the tests, works on another part) is none of these."""
    seen = None
    for t in turns:
        if any(_shows(tool, dependency) for tool in t["tools"]):
            seen = t["i"]
            break
    blocked = [t for t in turns if seen is None or t["i"] < seen]
    guess = [t for t in blocked if any(_writes_guarded(x, guarded) for x in t["tools"])]
    idle = [t for t in blocked if t not in guess and (not t["tools"] or all(is_poll(x) for x in t["tools"]))]
    return {"turns": len(turns), "dependency": dependency, "dependency_seen": seen is not None,
            "dependency_seen_turn": seen, "blocked_turns": len(blocked), "idle_turns": len(idle),
            "guess_build_turns": len(guess), "wasted_turns": len(idle) + len(guess)}


def _klatch_events(turns, klatch_bin):
    """[{"verb", "args", "refused", "turn"}] for the klatch claim, release, post, dm, ask, answer commands an agent ran, in order."""
    binre = re.escape(klatch_bin) if klatch_bin else r"(?:\S*/)?klatch"
    pat = re.compile(binre + r"\s+(?:-\S+\s+)*(" + "|".join(CLAIM_VERBS) + r")\b(.*)", re.S)
    out = []
    for t in turns:
        for tool in t["tools"]:
            for line in _command(tool).split("\n"):
                for m in pat.finditer(line):
                    args = _args(m.group(2))
                    res = tool["result"] or ""
                    refused = bool(re.search(r"refus|held by|denied|conflict", res, re.I))
                    out.append({"verb": m.group(1), "args": args, "refused": refused, "turn": t["i"]})
    return out


def _args(rest):
    """The words of a shell command line after the verb, up to the next operator, without options (a quoted message with
    a semicolon in it stays one word)."""
    lex = shlex.shlex(rest, posix=True, punctuation_chars=True)
    lex.whitespace_split = True
    words = []
    try:
        for w in lex:
            if set(w) <= set("();<>|&"):
                break
            words.append(w)
    except ValueError:
        words = rest.split()
    return [w for w in words if not w.startswith("-")]


def handoff_claims(streams, arm, giver, taker, klatch_bin):
    """What the streams show about claims at the handoff (#539), or None for an arm without claims (B0, B1).

    Recorded, not graded. The streams say what the agents did with claims and which handoff words they posted; whether
    klatch itself moved or freed a claim needs the store, which run.py does not export yet, so transferred_by_klatch is null."""
    if not str(arm).startswith("K"):
        return None
    ga = _klatch_events(streams.get(giver, []), klatch_bin)
    tc = _klatch_events(streams.get(taker, []), klatch_bin)

    def claim_path(e):  # claim ROOM PATH: the path is the argument after the room
        return e["args"][1] if len(e["args"]) >= 2 else (e["args"][0] if e["args"] else None)

    def collect(events, verb, refused=None):
        out = []
        for e in events:
            if e["verb"] == verb and (refused is None or e["refused"] == refused):
                p = claim_path(e)
                if p and p not in out:
                    out.append(p)
        return out

    def first_message(events, word):
        for k, e in enumerate(events):
            if e["verb"] in ("post", "dm", "handoff") and e["args"] and (
                    e["verb"] == "handoff" or e["args"][-1].strip().lower().startswith(word)):
                return k
        return None

    hand_k = first_message(ga, "handoff")
    before = []
    if hand_k is not None:
        before = [claim_path(e) for e in ga[:hand_k] if e["verb"] == "release" and claim_path(e)]
    released = collect(ga, "release")
    claimed = collect(ga, "claim", False)
    return {"a_claimed": claimed, "a_released": released,
            "a_released_before_handoff": [p for p in before if p in claimed],
            "a_still_holding": [p for p in claimed if p not in released],
            "c_claimed": collect(tc, "claim", False), "c_claim_refused": collect(tc, "claim", True),
            "handoff_message": hand_k is not None, "checkpoint_message": first_message(ga, "checkpoint") is not None,
            "transferred_by_klatch": None,
            "note": "from the streams only; whether klatch moved or freed the claim needs the store (not exported by run.py yet)"}


def compute(log_dir, scn, arm, klatch_bin):
    """{"blocked": {agent: report or None}, "handoff_claims": ...} for one run, from <log_dir>/<agent>.stream.jsonl."""
    streams = {}
    for name in scn["agents"]:
        try:
            with open(os.path.join(log_dir, name + ".stream.jsonl"), errors="replace") as f:
                streams[name] = parse_stream(f)
        except OSError:
            continue
    blocked = {}
    for name, spec in scn["blocked"].items():
        blocked[name] = blocked_report(streams[name], spec["dependency"], spec["guarded"]) if name in streams else None
    h = scn.get("handoff") or {}
    return {"blocked": blocked, "handoff_claims": handoff_claims(streams, arm, h.get("from"), h.get("to"), klatch_bin)}
