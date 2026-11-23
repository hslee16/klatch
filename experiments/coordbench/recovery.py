"""Scenario S6 (#1371): the kill trigger and the recovery metrics.

Two parts, both standard library only.

1. The kill trigger. A scenario file may carry `kill_agent` (scenarios/s6.json): the agent to kill, the `after` conditions that
   must all hold, and a fallback. The harness evaluates them every poll from the victim's own stream events and from the
   run's git history, never from a clock, and when they hold it sends SIGKILL to the victim's process group (no hook runs,
   no claim is released). `Killer` is the evaluation; run.py does the killing and records when.

2. The metrics. From the agents' streams, the timeline of when each stream line arrived (written by the harness), the kill
   record and the git history: how long until a survivor first acted on the victim's work, how the survivors noticed (a
   marker list taken from klatch's own output text), when the victim's module and the orphaned ask were finished, and false
   alarms. The definitions are in scenarios/s6.json, metrics.definitions.

A turn is one assistant message (streamed duplicates of a message id are one turn). Nothing here reads the environment.
"""
import json
import os
import re
import shlex
import subprocess
import sys
import time

WRITE_TOOLS = ("Write", "Edit", "MultiEdit", "NotebookEdit")
SHELL_WRITES = re.compile(r"(>|\btee\b|\bcp\s|\bmv\s|\bsed\s+-i|\binstall\s)")
KLATCH = r"(?:\S*/)?klatch\s+(?:-\S+\s+)*"
MSG_KEYS = ("message", "content", "text", "body", "prompt")
FLAGS_WITH_VALUE = ("--reason", "--ttl", "--scope", "--for", "--lease")


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and isinstance(b.get("text"), str))
    return ""


def read_events(lines):
    """The stream as ordered events: {"i": line index, "turn": n, "kind": use|result|hook|text, "name", "input", "text",
    "use": the tool_use event a result belongs to}. The turn of a result or a hook is the turn in progress."""
    evs, ids, tools, turn = [], {}, {}, 0
    for idx, line in enumerate(lines):
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if not isinstance(e, dict):
            continue
        t = e.get("type")
        if t == "assistant":
            msg = e.get("message") or {}
            mid = msg.get("id") or "anon-%d" % idx
            if mid not in ids:
                turn += 1
                ids[mid] = turn
            for b in msg.get("content") or []:
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "tool_use" and b.get("id") not in tools:
                    ev = {"i": idx, "turn": ids[mid], "kind": "use", "name": b.get("name", "?"), "input": b.get("input") or {}}
                    tools[b.get("id")] = ev
                    evs.append(ev)
                elif b.get("type") == "text" and isinstance(b.get("text"), str):
                    evs.append({"i": idx, "turn": ids[mid], "kind": "text", "text": b["text"]})
        elif t == "user":
            for b in (e.get("message") or {}).get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    evs.append({"i": idx, "turn": turn, "kind": "result", "text": _text_of(b.get("content")),
                                "use": tools.get(b.get("tool_use_id"))})
        elif t == "system" and e.get("subtype") == "hook_response":
            text = e.get("output") or e.get("stdout") or ""
            if isinstance(text, str):
                evs.append({"i": idx, "turn": turn, "kind": "hook", "text": text})
    return evs


def _cmd(ev):
    inp = ev["input"] if isinstance(ev["input"], dict) else {}
    return inp.get("command", "") if ev["name"] == "Bash" else ""


def sent_messages(ev):
    """The messages one tool_use sends: [{"verb", "text"}] (verb is send for SendMessage)."""
    inp = ev["input"] if isinstance(ev["input"], dict) else {}
    if ev["name"] == "SendMessage":
        for k in MSG_KEYS:
            if isinstance(inp.get(k), str):
                return [{"verb": "send", "text": inp[k]}]
        return [{"verb": "send", "text": json.dumps(inp, sort_keys=True)}]
    out = []
    for m in re.finditer(KLATCH + r"(post|dm|ask|answer)\b([^\n;&|]*(?:\"[^\"]*\"|'[^']*')?[^\n;&|]*)", _cmd(ev)):
        try:
            args = shlex.split(m.group(2))
        except ValueError:
            args = m.group(2).split()
        args = [a for a in args if not a.startswith("-")]
        if args:
            out.append({"verb": m.group(1), "text": args[-1], "args": args})
    return out


def is_answer(msg):
    return msg["verb"] == "answer" or msg["text"].strip().lower().startswith("answer")


def claim_ops(ev):
    """[(op, path)] of the klatch claim and release commands in one tool_use (op is claim or release)."""
    out = []
    for m in re.finditer(KLATCH + r"(claim|release)\b([^\n;&|]*(?:\"[^\"]*\"|'[^']*')?[^\n;&|]*)", _cmd(ev)):
        try:
            toks = shlex.split(m.group(2))
        except ValueError:
            toks = m.group(2).split()
        pos, skip = [], False
        for t in toks:
            if skip:
                skip = False
            elif t in FLAGS_WITH_VALUE:
                skip = True
            elif not t.startswith("-"):
                pos.append(t)
        if pos:
            out.append((m.group(1), pos[1] if len(pos) >= 2 else pos[0]))
    return out


def written_paths(ev):
    """File paths a tool_use writes: the file_path of Write, Edit and friends; for a shell command, the words that look like
    paths when the command writes at all."""
    inp = ev["input"] if isinstance(ev["input"], dict) else {}
    if ev["name"] in WRITE_TOOLS:
        p = inp.get("file_path") or inp.get("path") or inp.get("notebook_path")
        return [p] if isinstance(p, str) else []
    cmd = _cmd(ev)
    if ev["name"] == "Bash" and SHELL_WRITES.search(cmd):
        return [w for w in re.split(r"[\s'\";<>|&]+", cmd) if "/" in w or "." in w]
    return []


def touches(paths, unit_paths):
    return any(p == u or p.endswith("/" + u) for p in paths for u in unit_paths)


# 1. the kill trigger ---------------------------------------------------------------------------------------------------


def victim_state(lines, unit_paths, ask_re):
    """What the victim's stream so far shows: turns, whether it edited its module, the claims it holds (path claims and
    ask claims apart), and whether a question matching ask_re reached it and is still unanswered."""
    st = {"turns": 0, "edited": False, "claims": set(), "ask_claims": set(), "ask_seen": False, "ask_open": False}
    pat = re.compile(ask_re, re.I) if ask_re else None
    for ev in read_events(lines):
        st["turns"] = max(st["turns"], ev["turn"])
        if ev["kind"] == "use":
            if touches(written_paths(ev), unit_paths):
                st["edited"] = True
            for op, path in claim_ops(ev):
                target = st["ask_claims"] if path.startswith("ask:") else st["claims"]
                (target.add if op == "claim" else target.discard)(path)
            msgs = sent_messages(ev)
            for m in msgs:
                if is_answer(m):
                    st["ask_open"] = False
                    st["ask_claims"].clear()
        elif ev["kind"] in ("result", "hook") and pat and pat.search(ev["text"]):
            if ev["kind"] == "result" and ev["use"] is not None and sent_messages(ev["use"]):
                continue  # the echo of the victim's own message is not the question arriving
            st["ask_seen"] = st["ask_open"] = True
    return st


class GitProbe:
    """The two git facts a trigger may use, read from the run's bare origin (branch main)."""

    def __init__(self, origin, env):
        self.origin, self.env = origin, env

    def _git(self, args):
        p = subprocess.run(["git", "-C", self.origin] + args, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=self.env)
        return p.returncode, p.stdout.decode("utf-8", "replace")

    def board_working(self, victim):
        rc, out = self._git(["show", "main:TASKS.md"])
        return rc == 0 and any(re.match(r"^-\s*%s\s*\|\s*working\b" % re.escape(victim), l) for l in out.splitlines())

    def committed(self, victim, paths):
        rc, out = self._git(["log", "main", "--no-merges", "--format=%an", "--"] + list(paths))
        return rc == 0 and victim in out.split()


def default_ask_re(spec):
    return spec.get("ask_pattern", "")


class Killer:
    """Evaluates scenario['kill_agent'] against the victim's stream and the git history. poll() returns None until the kill
    is due, then a dict {"trigger": "conditions" or "fallback", "turns", "conditions"}."""

    def __init__(self, spec, arm, unit_paths, probe, git_every_s=0.3):
        self.spec, self.arm, self.unit_paths, self.probe = spec, arm, unit_paths, probe
        self.victim = spec["agent"]
        self.git_every_s, self._last_git, self._git_ok = git_every_s, 0.0, {}

    def _applies(self, cond):
        return not cond.get("arms") or self.arm in cond["arms"]

    def conditions(self, st):
        """{kind: bool} for the conditions that apply to this arm, from the stream state and (throttled) git."""
        out = {}
        now = time.monotonic()
        stream_ok = True
        for cond in self.spec["after"]:
            if not self._applies(cond):
                continue
            k = cond["kind"]
            if k == "edited":
                out[k] = st["edited"]
            elif k == "claimed":
                out[k] = bool(st["claims"])
            elif k == "ask_taken":
                out[k] = bool(st["ask_claims"])
            elif k == "ask_open":
                out[k] = st["ask_open"]
            elif k == "turns":
                out[k] = st["turns"] >= int(cond.get("min", 1))
            elif k in ("board_working", "no_commit"):
                continue
            else:
                raise ValueError("unknown kill condition %r" % k)
            stream_ok = stream_ok and out[k]
        for cond in self.spec["after"]:
            k = cond["kind"]
            if not self._applies(cond) or k not in ("board_working", "no_commit"):
                continue
            if not stream_ok:
                out[k] = self._git_ok.get(k, False)
                continue
            if now - self._last_git >= self.git_every_s or k not in self._git_ok:
                self._git_ok[k] = (self.probe.board_working(self.victim) if k == "board_working"
                                   else not self.probe.committed(self.victim, self.unit_paths))
            out[k] = self._git_ok[k]
        return out

    def poll(self, lines):
        st = victim_state(lines, self.unit_paths, self.spec.get("ask_pattern", ""))
        conds = self.conditions(st)
        self._last_git = time.monotonic()
        if conds and all(conds.values()):
            return {"trigger": "conditions", "turns": st["turns"], "conditions": conds}
        fb = self.spec.get("fallback_turns")
        if fb and st["turns"] >= fb:
            return {"trigger": "fallback", "turns": st["turns"], "conditions": conds}
        return None


def kill_spec(scn, variant=None):
    """The kill_agent option of a scenario for a fixture variant (a variant may replace it), or None."""
    v = (scn.get("scenario_variants") or {}).get(variant or "", {})
    return v.get("kill_agent", scn.get("kill_agent"))


# 2. the metrics --------------------------------------------------------------------------------------------------------

# What a survivor can see in klatch's own output that says the victim's work is stale. The text is klatch's: stuck.rs
# (StuckEvent::line), mediator.rs (the dead holder proposal), hook.rs (the digest line for an ask whose claim lapsed).
# {V} is the victim's name. NOTICES say a holder or an ask is dead or lapsed; SIGHTINGS only show the victim's record.
NOTICES = {
    "stuck_dead_holder": [r"dead_holder room \S+ claim \S+ holder {V}\b",
                          r"@{V} holds \S+ in \S+ but its process has exited"],
    "stuck_stale_ask": [r"stale_ask ask #\d+ asker \S+ age \d+s"],
    "ask_lease_lapsed": [r"{V}'s claim lapsed without an answer; the ask is open again"],
}
SIGHTINGS = {
    "claim_held_by_victim": [r"is claimed by {V} in room \S+ \(\d+s left"],
    "stale_board_entry": [r"^-\s*{V}\s*\|\s*working\b"],
}
BOARD_READ = re.compile(r"TASKS\.md")


def _patterns(table, victim):
    v = re.escape(victim)
    return {k: [re.compile(p.replace("{V}", v), re.M) for p in ps] for k, ps in table.items()}


def _arr(timeline, agent, i):
    row = (timeline or {}).get(agent)
    return row[i] if row is not None and i < len(row) else None


def _dt(t, t0):
    return None if t is None or t0 is None else round(max(0.0, t - t0), 3)


def analyze(sessions, timeline, scn, arm, kill):
    """The stream-derived S6 metrics. sessions: {agent: [stream lines]}; timeline: {agent: [seconds since the agents started,
    one per stream line]}; kill: the record the harness wrote {"fired", "agent", "t", "lines": {agent: lines parsed at the
    kill}, ...}. Returns a dict (see scenarios/s6.json for the definitions)."""
    if not kill or not kill.get("fired"):
        return {"kill_fired": False}
    victim, kt = kill["agent"], kill.get("t")
    vunit = scn["agents"][victim]["units"][0]
    vpath = scn["units"][vunit]["path"]
    owner = {u: a for a, v in scn["agents"].items() for u in v["units"]}
    unit_of = {v["path"]: u for u, v in scn["units"].items()}
    survivors = sorted(a for a in sessions if a != victim)
    notices, sightings = _patterns(NOTICES, victim), _patterns(SIGHTINGS, victim)
    events = {a: read_events(sessions[a]) for a in sessions}
    cut = kill.get("lines", {})

    base_turn = {a: max([e["turn"] for e in events[a] if e["i"] < cut.get(a, 0)] or [0]) for a in survivors}
    # turn starts after the kill, over all survivors, for "survivor turns until"
    starts = []
    for a in survivors:
        first = {}
        for e in events[a]:
            if e["i"] >= cut.get(a, 0) and e["turn"] > base_turn[a] and e["turn"] not in first:
                first[e["turn"]] = (_arr(timeline, a, e["i"]), a)
        starts += list(first.values())

    def turns_until(t):
        return None if t is None else sum(1 for s, _ in starts if s is not None and s <= t)

    def stamp(a, e):
        t = _arr(timeline, a, e["i"])
        return {"agent": a, "turn_after_kill": e["turn"] - base_turn[a], "t_after_kill_s": _dt(t, kt),
                "survivor_turns": turns_until(t), "_t": t}

    def strip(d):
        return None if d is None else {k: v for k, v in d.items() if k != "_t"}

    def first_of(items):
        return min(items, key=lambda d: (d["_t"] if d["_t"] is not None else float("inf"), d["agent"])) if items else None

    # what the survivors did to the victim's work after the kill
    actions, answers = [], []
    for a in survivors:
        for e in events[a]:
            if e["i"] < cut.get(a, 0) or e["kind"] != "use":
                continue
            for op, path in claim_ops(e):
                if op == "claim" and (path.startswith("ask:") or touches([path], [vpath])):
                    actions.append(dict(stamp(a, e), kind="claim", detail=path))
            if touches(written_paths(e), [vpath]):
                actions.append(dict(stamp(a, e), kind="edit", detail=vpath))
            for m in sent_messages(e):
                if is_answer(m):
                    d = dict(stamp(a, e), kind="answer", detail=m["text"][:80])
                    actions.append(d)
                    answers.append(d)
                elif re.search(r"(?<![\w/.-])%s(?![\w/.-])|%s" % (re.escape(victim), re.escape(vunit)), m["text"]):
                    actions.append(dict(stamp(a, e), kind="message", detail=m["text"][:80]))
    first_action = first_of(actions)

    # how they noticed
    found, seen = [], []
    for a in survivors:
        for e in events[a]:
            if e["i"] < cut.get(a, 0) or e["kind"] not in ("result", "hook"):
                continue
            for name, pats in notices.items():
                if any(p.search(e["text"]) for p in pats):
                    found.append(dict(stamp(a, e), marker=name))
            for name, pats in sightings.items():
                if any(p.search(e["text"]) for p in pats):
                    seen.append(dict(stamp(a, e), marker=name))
            if e["kind"] == "result" and e["use"] is not None and e["use"]["name"] == "Bash":
                # a claim of the victim's path that succeeds after the kill: its lease had run out
                for op, path in claim_ops(e["use"]):
                    if op == "claim" and touches([path], [vpath]) and re.search(r"^claimed ", e["text"], re.M):
                        found.append(dict(stamp(a, e), marker="claim_taken_after_lapse"))
    first_notice = first_of(found)
    first_per_marker = {}
    for d in sorted(found, key=lambda d: (d["_t"] if d["_t"] is not None else float("inf"), d["agent"])):
        first_per_marker.setdefault(d["marker"], d)
    noticed_by = [m for m in first_per_marker]
    if first_action is None:
        how = None
    elif first_notice is not None and (first_notice["_t"] is None or first_action["_t"] is None
                                       or first_notice["_t"] <= first_action["_t"]):
        how = first_notice["marker"]
    else:
        how = "no-marker"

    # false alarms: a survivor taking over a module whose agent is alive (before the kill, for the victim's own module)
    alarms = set()
    detail = []
    for a in sorted(sessions):
        for e in events[a]:
            if e["kind"] != "use":
                continue
            hit = []
            for op, path in claim_ops(e):
                if op == "claim" and path in unit_of:
                    hit.append(("claim", unit_of[path]))
            for p in written_paths(e):
                for path, u in unit_of.items():
                    if touches([p], [path]):
                        hit.append(("edit", u))
            for how_, u in hit:
                if a == owner[u]:
                    continue
                t = _arr(timeline, a, e["i"])
                if u != vunit or kt is None:
                    alive = True  # another agent's module: its owner is alive (running, or finished and not dead)
                elif t is not None:
                    alive = t < kt  # the victim's module before the kill: the victim was alive
                else:
                    alive = e["i"] < cut.get(a, 0)
                if alive and (a, u) not in alarms:
                    alarms.add((a, u))
                    detail.append({"agent": a, "unit": u, "how": how_, "owner": owner[u], "victim_unit": u == vunit})
    return {
        "kill_fired": True, "victim": victim, "victim_unit": vunit, "kill_trigger": kill.get("trigger"),
        "kill_t_s": kt, "victim_turns_at_kill": kill.get("turns"),
        "first_survivor_action": strip(first_action),
        "time_to_first_survivor_action_s": None if first_action is None else first_action["t_after_kill_s"],
        "survivor_turns_to_first_action": None if first_action is None else first_action["survivor_turns"],
        "time_to_ask_answer_s": None if not answers else first_of(answers)["t_after_kill_s"],
        "survivor_turns_to_ask_answer": None if not answers else first_of(answers)["survivor_turns"],
        "first_notice": strip(first_notice),
        "noticed_by": noticed_by if arm.startswith("K") else None,
        "how_noticed": how if arm.startswith("K") else None,
        "notice_markers": [strip(d) for d in first_per_marker.values()] if arm.startswith("K") else None,
        "sightings": sorted({d["marker"] for d in seen}),
        "survivor_turns_after_kill": len(starts),
        "false_alarms": len(alarms), "false_alarm_detail": detail,
    }


def git_recovery(origin, clones, scn, victim, kill_epoch, env, check_py):
    """From git: the first commit by someone other than the victim that completes the victim's module (replayed against the
    hidden checker's --module mode), the time from the kill to it (commit time has one second resolution), and the work
    done twice on the victim's module. Returns a dict; the keys are None when there is no such commit."""
    vunit = scn["agents"][victim]["units"][0]
    vpath = scn["units"][vunit]["path"]
    rc = subprocess.run(["git", "-C", origin, "log", "main", "--no-merges", "--full-history", "--reverse",
                         "--format=%H%x01%an%x01%ct%x01%P",
                         "--", vpath], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env)
    commits = []
    for line in rc.stdout.decode("utf-8", "replace").splitlines():
        sha, author, ct, parents = (line.split("\x01") + ["", "", "", ""])[:4]
        if sha and parents.strip():
            commits.append((sha, author, int(ct or 0)))
    first = None
    import tempfile
    for sha, author, ct in commits:
        if author == victim:
            continue
        p = subprocess.run(["git", "-C", origin, "show", "%s:%s" % (sha, vpath)], stdout=subprocess.PIPE,
                           stderr=subprocess.DEVNULL, env=env)
        if p.returncode != 0:
            continue
        with tempfile.TemporaryDirectory() as d:
            f = os.path.join(d, os.path.basename(vpath))
            with open(f, "wb") as fh:
                fh.write(p.stdout)
            try:
                q = subprocess.run([sys.executable, "-I", check_py, "--module", vunit, f], stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, env=env, timeout=60)
                ok = json.loads(q.stdout.decode().strip().splitlines()[-1]).get("ok") is True
            except (subprocess.TimeoutExpired, ValueError, IndexError, OSError):
                ok = False
        if ok:
            first = (sha, author, ct)
            break
    everyone = {}
    for repo in [origin] + list(clones):
        out = subprocess.run(["git", "-C", repo, "log", "--all", "--no-merges", "--full-history", "--format=%H%x01%an%x01%P",
                              "--", vpath], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env)
        for line in out.stdout.decode("utf-8", "replace").splitlines():
            sha, author, parents = (line.split("\x01") + ["", "", ""])[:3]
            if sha and parents.strip():  # not the root commit, which is the harness's initial tree
                everyone[sha] = author
    authors = sorted(set(everyone.values()))
    others = [a for a in authors if a != victim]
    return {
        "recovery_commit": None if first is None else first[0], "recovered_by_commit_author": None if first is None else first[1],
        "time_to_module_recovery_s": None if first is None or kill_epoch is None else max(0, first[2] - int(kill_epoch)),
        "victim_module_authors": authors,
        "duplicated_recovery": max(0, len(others) - 1),
        "redone_after_victim_commit": victim in authors and bool(others),
    }
