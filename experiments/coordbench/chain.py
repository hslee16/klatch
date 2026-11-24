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
import streamshape

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


def _load_event(line):
    try:
        e = json.loads(line)
    except ValueError:
        return None
    return e if isinstance(e, dict) else None


def _add_block(b, turn, tools):
    if not isinstance(b, dict):
        return
    if b.get("type") == "tool_use" and streamshape.ident(b.get("id")) not in tools:
        tu = streamshape.tool_use(b)
        t = {"name": tu["name"], "input": tu["input"], "result": ""}
        tools[tu["id"]] = t
        turn["tools"].append(t)
    elif b.get("type") == "text" and isinstance(b.get("text"), str):
        turn["text"] = b["text"]


def _add_assistant(e, turns, by_id, tools):
    msg = streamshape.message(e)
    mid = streamshape.ident(msg.get("id")) or "anon-%d" % len(turns)
    if mid not in by_id:
        by_id[mid] = {"i": len(turns) + 1, "text": "", "tools": []}
        turns.append(by_id[mid])
    for b in streamshape.content(e):
        _add_block(b, by_id[mid], tools)


def _is_known_result(b, tools):
    return isinstance(b, dict) and b.get("type") == "tool_result" and streamshape.ident(b.get("tool_use_id")) in tools


def _add_results(e, tools):
    for b in streamshape.content(e):
        if _is_known_result(b, tools):
            tools[streamshape.ident(b["tool_use_id"])]["result"] = _text_of(b.get("content"))


def parse_stream(lines):
    """Turns of one agent: [{"i": 1.., "text", "tools": [{"name", "input", "result"}]}], in the order the messages came."""
    turns, by_id, tools = [], {}, {}
    for line in lines:
        e = _load_event(line)
        if e is None:
            continue
        if e.get("type") == "assistant":
            _add_assistant(e, turns, by_id, tools)
        elif e.get("type") == "user":
            _add_results(e, tools)
    return turns


def _command(tool):
    inp = tool["input"] if isinstance(tool["input"], dict) else {}
    return inp.get("command", "") if tool["name"] == "Bash" else ""


def _is_klatch_read(tool):
    return bool(re.search(r"(?:^|[\s;&|])(?:\S*/)?klatch\s", _command(tool))) or tool["name"] in ("SendMessage", "ListAgents")


def _poll_pipeline(cmd):
    """A poll, then only filters of its output."""
    first, *rest = [p for p in re.split(r"(?<!\|)\|(?!\|)", cmd)]
    return bool(POLL.match(first)) and all(FILTER.match(p) for p in rest)


def is_poll(tool):
    """True when one tool call only waits or looks. A Bash command is a poll when every simple command in it is (a
    `sleep 3; python3 -m unittest` is not); a Read of the shared task board and a ListAgents call are polls."""
    if tool["name"] == "ListAgents":
        return True
    if tool["name"] in ("Read", "Glob", "Grep"):
        return "TASKS.md" in json.dumps(tool["input"])
    if tool["name"] != "Bash":
        return False
    cmds = [c for c in SEPARATORS.split(_command(tool)) if c.strip()]
    return bool(cmds) and all(_poll_pipeline(c) for c in cmds)


def _shows(tool, dependency):
    base = dependency.rsplit("/", 1)[-1]
    res = tool["result"] or ""
    if _is_klatch_read(tool) or base not in res:
        return False
    return not NEGATIVE.search(res)


FILE_TOOLS = ("Write", "Edit", "NotebookEdit", "MultiEdit")


def _writes_one(tool, inp, blob, g):
    name = tool["name"]
    if name in FILE_TOOLS:
        return str(inp.get("file_path", "")).endswith(g)
    if name == "Bash":
        return g in _command(tool) and bool(WRITES.search(_command(tool)))
    return g in blob and "content" in inp


def _writes_guarded(tool, guarded):
    inp = tool["input"] if isinstance(tool["input"], dict) else {}
    blob = json.dumps(inp)
    return any(_writes_one(tool, inp, blob, g) for g in guarded)


def _first_seen_turn(turns, dependency):
    for t in turns:
        if any(_shows(tool, dependency) for tool in t["tools"]):
            return t["i"]
    return None


def _turn_classes(blocked, guarded):
    """(guess-build turns, idle turns) among the blocked turns."""
    guess = [t for t in blocked if any(_writes_guarded(x, guarded) for x in t["tools"])]
    idle = [t for t in blocked if t not in guess and (not t["tools"] or all(is_poll(x) for x in t["tools"]))]
    return guess, idle


def blocked_report(turns, dependency, guarded):
    """Idle and wasted turns of one agent that needs `dependency` (a path) before it can do its step.

    blocked_turns: turns before the dependency is seen (all turns when it never is). idle_turns: blocked turns that only
    wait or look (every tool call a poll, or no tool call at all). guess_build_turns: blocked turns that write a file in
    `guarded` (the dependent step, built before its input exists). wasted_turns: idle plus guess builds. A blocked turn that
    does anything else (reads the README, runs the tests, works on another part) is none of these."""
    seen = _first_seen_turn(turns, dependency)
    blocked = [t for t in turns if seen is None or t["i"] < seen]
    guess, idle = _turn_classes(blocked, guarded)
    return {"turns": len(turns), "dependency": dependency, "dependency_seen": seen is not None,
            "dependency_seen_turn": seen, "blocked_turns": len(blocked), "idle_turns": len(idle),
            "guess_build_turns": len(guess), "wasted_turns": len(idle) + len(guess)}


def _tool_events(tool, pat, turn):
    refused = bool(re.search(r"refus|held by|denied|conflict", tool["result"] or "", re.I))
    return [{"verb": m.group(1), "args": _args(m.group(2)), "refused": refused, "turn": turn}
            for line in _command(tool).split("\n") for m in pat.finditer(line)]


def _klatch_events(turns, klatch_bin):
    """[{"verb", "args", "refused", "turn"}] for the klatch claim, release, post, dm, ask, answer commands an agent ran, in order."""
    binre = re.escape(klatch_bin) if klatch_bin else r"(?:\S*/)?klatch"
    pat = re.compile(binre + r"\s+(?:-\S+\s+)*(" + "|".join(CLAIM_VERBS) + r")\b(.*)", re.S)
    return [e for t in turns for tool in t["tools"] for e in _tool_events(tool, pat, t["i"])]


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


def _claim_path(e):
    """claim ROOM PATH: the path is the argument after the room."""
    return e["args"][1] if len(e["args"]) >= 2 else (e["args"][0] if e["args"] else None)


def _collect(events, verb, refused=None):
    paths = (_claim_path(e) for e in events if e["verb"] == verb and refused in (None, e["refused"]))
    out = []
    for p in paths:
        if p and p not in out:
            out.append(p)
    return out


def _is_message_with(e, word):
    return e["verb"] in ("post", "dm", "handoff") and bool(e["args"]) and (
        e["verb"] == "handoff" or e["args"][-1].strip().lower().startswith(word))


def _first_message(events, word):
    for k, e in enumerate(events):
        if _is_message_with(e, word):
            return k
    return None


def handoff_claims(streams, arm, giver, taker, klatch_bin):
    """What the streams show about claims at the handoff (#539), or None for an arm without claims (B0, B1).

    Recorded, not graded. The streams say what the agents did with claims and which handoff words they posted; whether
    klatch itself moved or freed a claim needs the store, which run.py does not export yet, so transferred_by_klatch is null."""
    if not str(arm).startswith("K"):
        return None
    ga = _klatch_events(streams.get(giver, []), klatch_bin)
    tc = _klatch_events(streams.get(taker, []), klatch_bin)
    hand_k = _first_message(ga, "handoff")
    before = []
    if hand_k is not None:
        before = [_claim_path(e) for e in ga[:hand_k] if e["verb"] == "release" and _claim_path(e)]
    released = _collect(ga, "release")
    claimed = _collect(ga, "claim", False)
    return {"a_claimed": claimed, "a_released": released,
            "a_released_before_handoff": [p for p in before if p in claimed],
            "a_still_holding": [p for p in claimed if p not in released],
            "c_claimed": _collect(tc, "claim", False), "c_claim_refused": _collect(tc, "claim", True),
            "handoff_message": hand_k is not None, "checkpoint_message": _first_message(ga, "checkpoint") is not None,
            "transferred_by_klatch": None,
            "note": "from the streams only; whether klatch moved or freed the claim needs the store (not exported by run.py yet)"}


def _read_streams(log_dir, names):
    streams = {}
    for name in names:
        try:
            with open(os.path.join(log_dir, name + ".stream.jsonl"), errors="replace") as f:
                streams[name] = parse_stream(f)
        except OSError:
            continue
    return streams


def compute(log_dir, scn, arm, klatch_bin):
    """{"blocked": {agent: report or None}, "handoff_claims": ...} for one run, from <log_dir>/<agent>.stream.jsonl."""
    streams = _read_streams(log_dir, scn["agents"])
    blocked = {}
    for name, spec in scn["blocked"].items():
        blocked[name] = blocked_report(streams[name], spec["dependency"], spec["guarded"]) if name in streams else None
    h = scn.get("handoff") or {}
    return {"blocked": blocked, "handoff_claims": handoff_claims(streams, arm, h.get("from"), h.get("to"), klatch_bin)}
