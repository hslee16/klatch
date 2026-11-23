"""Scenario S9 support (#1374): the prebuilt room history, the stream analysis (what history the late joiner read, and when)
and the first correct contribution found by replaying the agent's commits against the hidden checker.

All of it works on the raw stream-json of the agents (so one reader serves the fake and the real driver) and on git
repositories; nothing here starts a model. Standard library only.

First correct contribution (operational definition): the agent's commits are replayed in the order it made them (every
non-merge commit under the agent's git author name, in its clone and in the origin). For each, the tree of that commit is
given to the hidden checker's `--tree` mode, which tests the ticket and the three decided facts exactly as the final check
does. The first commit whose tree has the ticket working and all three facts ok is the first correct contribution. It is
tied to the stream by the abbreviated commit id git prints in the tool result of the `git commit` call (or, when the stream
does not show it, by the position of the commit among the agent's `git commit` calls); the turns and tokens used up to and
including the turn of that call are the cost of getting to it. No correct commit: null.
"""
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

TOKEN_KEYS = (("input", "input_tokens"), ("output", "output_tokens"), ("cache_read", "cache_read_input_tokens"),
              ("cache_creation", "cache_creation_input_tokens"))
EDIT_TOOLS = {"Write", "Edit", "MultiEdit", "NotebookEdit"}
BOARD_FILES = ("TASKS.md", "DECISIONS.md")
ROOM_VERBS = ("history", "read", "board", "find", "asks", "tasks", "claims", "card")
BRIEFING = re.compile(r"\[briefing \S+")


# --- the prebuilt room history ----------------------------------------------------------------------------------------

def post_room(klatch_bin, room, steps, env, cwd):
    """Run the scripted klatch commands, each as its member (KLATCH_NAME), through the binary. {{ask:KEY}} in an argument is
    the number the ask called KEY got. Returns the number of messages posted (post, ask and answer steps). Raises RuntimeError
    on the first command that fails or that names an ask not made yet. The caller gives a temporary store (KLATCH_HOME and
    KLATCH_KEY_DIR in env); this function never chooses one."""
    asks = {}
    posted = 0
    for st in steps:
        argv = []
        for a in st["argv"]:
            for key in re.findall(r"\{\{ask:(\w+)\}\}", a):
                if key not in asks:
                    raise RuntimeError("the room script uses ask %r before it is made" % key)
                a = a.replace("{{ask:%s}}" % key, str(asks[key]))
            argv.append(a)
        if len(argv) > 1 and argv[1] == "lobby":
            argv[1] = room  # the scripts are written for the default room
        p = subprocess.run([klatch_bin] + argv, cwd=cwd, env=dict(env, KLATCH_NAME=st["as"]), stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, timeout=120)
        if p.returncode != 0:
            raise RuntimeError("room history step failed (%s as %s): %s" % (argv[0], st["as"], p.stdout.strip()[-200:]))
        if argv[0] in ("post", "ask", "answer"):
            posted += 1
        if argv[0] == "ask":
            m = re.search(r"asked #(\d+)", p.stdout)
            if not m or not st.get("key"):
                raise RuntimeError("could not read the number of the ask: %s" % p.stdout.strip()[-200:])
            asks[st["key"]] = int(m.group(1))
    return posted


# --- reading the stream ------------------------------------------------------------------------------------------------

def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and isinstance(b.get("text"), str))
    return ""


def classify(name, inp, klatch_bin=None):
    """The kind of history read a tool call is, or None. git_log: git log, show, blame, shortlog or reflog. board_file: TASKS.md
    or DECISIONS.md read with the Read tool or by a shell command. klatch_<verb>: the klatch commands that read the record
    (history, read, board, find, asks, tasks, claims, card)."""
    inp = inp if isinstance(inp, dict) else {}
    if name == "Bash":
        cmd = inp.get("command", "")
        if re.search(r"(?:^|[;&|(\s])git\s+(?:-\S+\s+(?:\S+\s+)?)*(?:log|show|blame|shortlog|reflog)\b", cmd):
            return "git_log"
        binre = re.escape(klatch_bin) if klatch_bin else r"klatch"
        m = re.search(binre + r"\s+(?:-\S+\s+(?:[^-\s]\S*\s+)?)*(%s)\b" % "|".join(ROOM_VERBS), cmd)
        if m:
            return "klatch_" + m.group(1)
        if any(b in cmd for b in BOARD_FILES) and re.search(r"\b(cat|head|tail|sed|less|more|grep|rg|awk|bat)\b", cmd):
            return "board_file"
        return None
    if name in ("Read", "Grep", "Glob"):
        blob = json.dumps(inp)
        if any(b in blob for b in BOARD_FILES):
            return "board_file"
    return None


def is_edit(name, inp):
    if name in EDIT_TOOLS:
        return True
    if name == "Bash":
        return bool(re.search(r"(?:^|[;&|(\s])git\s+(?:-\S+\s+(?:\S+\s+)?)*(?:add|commit|apply|merge|push)\b", (inp or {}).get("command", "")))
    return False


def is_commit(name, inp):
    return name == "Bash" and bool(re.search(r"(?:^|[;&|(\s])git\s+(?:-\S+\s+(?:\S+\s+)?)*commit\b", (inp or {}).get("command", "")))


class Timeline:
    """The turns of one agent across its sessions, in order. A turn is one assistant message (by id)."""

    def __init__(self, klatch_bin=None):
        self.klatch_bin = klatch_bin
        self.turns = []     # {"session", "usage", "calls": [call]}
        self.briefings = []  # {"session", "before_turn", "chars"}: SessionStart text that carried a briefing
        self.hook_chars = 0
        self._ids = {}
        self._calls = {}
        self.session = 0

    def start_session(self):
        self.session += 1

    def feed(self, line):
        line = line.strip()
        if not line:
            return
        try:
            e = json.loads(line)
        except ValueError:
            return
        if not isinstance(e, dict):
            return
        t = e.get("type")
        if t == "system" and e.get("subtype") == "hook_response":
            text = e.get("output") or e.get("stdout") or ""
            if isinstance(text, str):
                self.hook_chars += len(text)
                if BRIEFING.search(text):
                    self.briefings.append({"session": self.session, "before_turn": len(self.turns), "chars": len(text)})
        elif t == "assistant":
            msg = e.get("message") or {}
            mid = msg.get("id") or "anon-%d-%d" % (self.session, len(self.turns))
            key = (self.session, mid)
            if key not in self._ids:
                self._ids[key] = {"session": self.session, "usage": {}, "calls": []}
                self.turns.append(self._ids[key])
            turn = self._ids[key]
            if isinstance(msg.get("usage"), dict):
                turn["usage"] = msg["usage"]
            for b in msg.get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    name, inp = b.get("name", "?"), b.get("input") or {}
                    call = {"n": sum(len(x["calls"]) for x in self.turns), "id": b.get("id"), "name": name, "input": inp, "turn": len(self.turns) - 1,
                            "kind": classify(name, inp, self.klatch_bin), "edit": is_edit(name, inp),
                            "commit": is_commit(name, inp), "result": ""}
                    turn["calls"].append(call)
                    self._calls[(self.session, b.get("id"))] = call
        elif t == "user":
            for b in (e.get("message") or {}).get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    call = self._calls.get((self.session, b.get("tool_use_id")))
                    if call is not None:
                        call["result"] = _text_of(b.get("content"))

    def tokens_through(self, turn_index):
        """Cumulative tokens of the turns 0..turn_index inclusive: each kind and the total."""
        acc = {k: 0 for k, _ in TOKEN_KEYS}
        for t in self.turns[:turn_index + 1]:
            for k, src in TOKEN_KEYS:
                acc[k] += int((t["usage"] or {}).get(src, 0) or 0)
        acc["total"] = sum(acc[k] for k, _ in TOKEN_KEYS)
        return acc

    def calls(self):
        return [c for t in self.turns for c in t["calls"]]


def timeline_from_sessions(sessions, klatch_bin=None):
    """sessions: a list of lists of stream lines, one per session of the agent, in order."""
    tl = Timeline(klatch_bin)
    for lines in sessions:
        tl.start_session()
        for ln in lines:
            tl.feed(ln)
    return tl


# --- replaying the agent's commits against the checker -----------------------------------------------------------------

def _git(repo, args, env, binary=False):
    p = subprocess.run(["git", "-C", repo] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    return p.returncode, (p.stdout if binary else p.stdout.decode("utf-8", "replace"))


def agent_commits(repos, author, env):
    """[(sha, repo)] of the non-merge commits by `author` in the repositories, oldest first by the first repository's own
    order, then any commit only a later repository has. Merges are not the agent's own work."""
    seen, out = set(), []
    for repo in repos:
        rc, text = _git(repo, ["log", "--all", "--no-merges", "--reverse", "--topo-order", "--format=%H%x01%an"], env)
        if rc != 0:
            continue
        for ln in text.splitlines():
            sha, _, an = ln.partition("\x01")
            if an == author and sha not in seen:
                seen.add(sha)
                out.append((sha, repo))
    return out


def extract_tree(repo, sha, dest, env):
    rc, data = _git(repo, ["archive", "--format=tar", sha], env, binary=True)
    if rc != 0:
        return False
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:
        for m in tar.getmembers():
            target = os.path.realpath(os.path.join(dest, m.name))
            if not (target == os.path.realpath(dest) or target.startswith(os.path.realpath(dest) + os.sep)):
                continue
            if m.isfile() or m.isdir():
                tar.extract(m, dest)
    return True


def replay(repos, author, check_sh, env, timeout=120):
    """[{"sha", "task", "facts", "correct"}] for each commit of the agent, oldest first, by `check_sh --tree`. A commit whose
    tree cannot be extracted or checked is listed with correct false and an "error"."""
    out = []
    for sha, repo in agent_commits(repos, author, env):
        d = tempfile.mkdtemp(prefix="cb-replay-")
        try:
            if not extract_tree(repo, sha, d, env):
                out.append({"sha": sha, "correct": False, "error": "git archive failed"})
                continue
            try:
                p = subprocess.run(["bash", check_sh, "--tree", d], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env,
                                   text=True, timeout=timeout)
                obj = json.loads(p.stdout.strip().splitlines()[-1])
            except (subprocess.SubprocessError, ValueError, IndexError, OSError):
                out.append({"sha": sha, "correct": False, "error": "the checker did not run"})
                continue
            out.append({"sha": sha, "task": obj.get("task"), "facts": obj.get("facts"), "correct": bool(obj.get("correct"))})
        finally:
            shutil.rmtree(d, ignore_errors=True)
    return out


def locate_commit(tl, sha, ordinal):
    """The call of the stream that made the commit: the `git commit` call whose result shows the abbreviated id, else the
    ordinal-th `git commit` call (0-based). Returns (call, how) or (None, None)."""
    commits = [c for c in tl.calls() if c["commit"]]
    for c in commits:
        if re.search(r"\b%s" % re.escape(sha[:7]), c["result"] or ""):
            return c, "sha"
    if ordinal < len(commits):
        return commits[ordinal], "ordinal"
    return None, None


# --- the metrics -------------------------------------------------------------------------------------------------------

def analyze(sessions, klatch_bin, arm, replayed):
    """The S9 metrics of one run from the agent's sessions (lists of stream lines) and the replay of its commits."""
    tl = timeline_from_sessions(sessions, klatch_bin)
    calls = tl.calls()
    reads = [c for c in calls if c["kind"]]
    kinds = {}
    for c in reads:
        kinds[c["kind"]] = kinds.get(c["kind"], 0) + 1
    by_session = [sum(1 for c in reads if tl.turns[c["turn"]]["session"] == s) for s in range(1, tl.session + 1)]
    first_edit = next((c for c in calls if c["edit"]), None)
    reads_before_edit = [c for c in reads if first_edit is None or c["n"] < first_edit["n"]]
    briefing_before_edit = [b for b in tl.briefings if first_edit is None or b["before_turn"] <= first_edit["turn"]]
    room_reads = [c for c in reads if c["kind"].startswith("klatch_")]
    room_before_edit = [c for c in room_reads if first_edit is None or c["n"] < first_edit["n"]]
    is_k = arm.startswith("K")
    first_correct = None
    for i, r in enumerate(replayed):
        if r.get("correct"):
            call, how = locate_commit(tl, r["sha"], i)
            if call is not None:
                reads_before = sum(1 for c in reads if c["n"] <= call["n"])
                first_correct = {"commit": r["sha"], "commit_index": i + 1, "located_by": how, "turn": call["turn"] + 1,
                                 "session": tl.turns[call["turn"]]["session"], "tokens": tl.tokens_through(call["turn"]),
                                 "history_reads_before": reads_before}
            else:
                first_correct = {"commit": r["sha"], "commit_index": i + 1, "located_by": None, "turn": None,
                                 "session": None, "tokens": None, "history_reads_before": None}
            break
    return {
        "sessions": tl.session,
        "turns": len(tl.turns),
        "tokens": tl.tokens_through(len(tl.turns) - 1) if tl.turns else {k: 0 for k, _ in TOKEN_KEYS} | {"total": 0},
        "history_reads": len(reads),
        "history_reads_by_kind": dict(sorted(kinds.items())),
        "history_reads_by_session": by_session,
        "history_read_chars": sum(len(c["result"] or "") for c in reads),
        "briefing_seen": bool(tl.briefings),
        "briefing_chars": sum(b["chars"] for b in tl.briefings),
        "history_read_before_first_edit": bool(reads_before_edit) or bool(briefing_before_edit),
        "room_history_used_unprompted": (bool(room_before_edit) or bool(briefing_before_edit)) if is_k else None,
        "agent_commits": len(replayed),
        "correct_commits": sum(1 for r in replayed if r.get("correct")),
        "first_correct": first_correct,
    }
