"""Was native messaging working in a B1 or B1-roles run? (#1520.) Standard library only; nothing here starts a process.

If cross-session discovery fails, B1 silently becomes B0 plus a board and a comparison against it says nothing about native
messaging. So the harness reads the run's own streams and judges, per run, one of three things:

  failed    an init event that carries a tools list lacks SendMessage or ListAgents; or, at least SETTLE_S seconds after EVERY
            agent of the run had started, a ListAgents result named no teammate (or a SendMessage said `No agent named ...`)
            while another agent's stream shows it still running, and no ListAgents result of the run ever named a teammate.
            The run is INVALID, with the reason in its notes.
  verified  some ListAgents result named a teammate. This means the teammate was found and a message could be accepted for it;
            it does not mean a message was read.
  unknown   neither: no such evidence (the first ListAgents of a real agent comes 3.6 to 4 s after it starts, so one that
            runs before every agent has been up for SETTLE_S is no test; agents that finished before the others had settled;
            fewer than two agents; streams without times). The run stays valid and is NOT evidence of working messaging:
            report.py labels a B1 group that holds such a run "messaging unverified".

The clock is the harness's reading of each stream line (seconds since the agents started), and the start and exit time of each
session. The agents of a harness run start together, so SETTLE_S after the latest start is what a fair test needs.

Result texts (real, claude 2.1.331, probes of 2026-11-23): a ListAgents result is `This session is a [id] - ...` then
`Peer sessions (1):` and one line per peer, `  b [eab75d]  .  interactive  .  busy  .  started 16s ago`, or `No reachable agents ...`;
a SendMessage result is JSON, `{"success":true,"message":...}` or `{"success":false,"message":"No agent named 'b' is reachable..."}`
(or `Failed to send to uds:...`), and is_error is NOT set on a failed send, so delivery is read from `success`.

Applies to the arms in MESSAGING_ARMS only; every other arm has the messaging tools denied and a record value of null."""
import json
import re

MESSAGING_ARMS = ("B1", "B1-roles")
TOOLS = ("SendMessage", "ListAgents")
SETTLE_S = 8.0
NOT_DELIVERED = re.compile(r"^\s*(?:Not sent|Not delivered|No agent named|Failed|Error)", re.I)
UNREACHABLE = re.compile(r"No agent named|Failed to send to")
PEER_ENTRY = re.compile(r"(?:^|\s)([\w.-]+) \[[0-9a-f]+\]", re.M)
MAX_LISTED = 20


def tool_names(tools):
    """The names in an init event's tools list (strings or objects with a name), or None when the event carries no list."""
    if not isinstance(tools, list):
        return None
    return [t if isinstance(t, str) else t.get("name") for t in tools if isinstance(t, str) or isinstance(t, dict)]


def peers_listed(text):
    """The names of the peers a ListAgents result lists: the `NAME [id]` entries after the `Peer sessions (N):` header (the
    asker's own `This session is NAME [id]` comes before it and is not a peer). [] when the header is missing."""
    m = re.search(r"Peer sessions \(\d+\):", text or "")
    return PEER_ENTRY.findall(text[m.end():]) if m else []


def names_peer(text, agent, team):
    """True when a ListAgents result lists an agent of the team other than the one that asked (exact names: a longer name that
    contains a teammate's name does not count)."""
    return any(p in team and p != agent for p in peers_listed(text))


def _json(text):
    try:
        v = json.loads(text)
    except (TypeError, ValueError):
        return None
    return v if isinstance(v, dict) else None


def delivered(send):
    """A SendMessage result: the JSON's `success` field (a failed send has is_error unset), else the text for a non-JSON result."""
    if send.get("error"):
        return False
    j = _json(send.get("text"))
    if j is not None and isinstance(j.get("success"), bool):
        return j["success"]
    return not NOT_DELIVERED.match(send.get("text") or "")


def _key_text(text):
    """The sentence of a ListAgents or SendMessage result that says what happened, short."""
    j = _json(text)
    msg = str(j.get("message", "")) if j else (text or "")
    for key in ("No reachable agents", "No agent named", "Failed to send to"):
        i = msg.find(key)
        if i >= 0:
            return " ".join(msg[i:].split("\n")[0].split())[:100]
    return " ".join(msg.split())[:80] or "nothing"


def _facts(session):
    return session.get("messaging") or {}


def _tools_problem(sessions):
    """(tools_present, reason): None when no stream carries a tools list; False with the first agent that lacks a tool."""
    seen = False
    for key, s in sessions.items():
        names = _facts(s).get("tools")
        if names is None:
            continue
        seen = True
        lacking = [t for t in TOOLS if t not in names]
        if lacking:
            return False, "the init event of %s lacks %s" % (s.get("agent", key), " and ".join(lacking))
    return (True if seen else None), None


def _numbers(sessions, team):
    present, reason = _tools_problem(sessions)
    sends = [x for s in sessions.values() for x in _facts(s).get("sends", [])]
    calls = [(s.get("agent"), c) for s in sessions.values() for c in _facts(s).get("listagents", [])]
    named = any(names_peer(c.get("text"), agent, team) for agent, c in calls)
    return {"tools_present": present, "listagents_named_peer": named, "messages_sent": len(sends),
            "messages_delivered": sum(1 for x in sends if delivered(x)), "listagents_calls": len(calls)}, reason


def _clock(alive):
    return {"settle_s": SETTLE_S, "last_agent_start": max((a for a, _ in alive.values()), default=None),
            "alive": {k: [round(a, 3), round(b, 3)] for k, (a, b) in alive.items()}}


def _results(sessions, team):
    """[{"agent", "t", "named_peer", "text"}] of every ListAgents result with a time, in time order."""
    rows = [{"agent": s.get("agent"), "t": c["t"], "named_peer": names_peer(c.get("text"), s.get("agent"), team),
             "text": c.get("text")} for s in sessions.values() for c in _facts(s).get("listagents", []) if c.get("t") is not None]
    return sorted(rows, key=lambda r: r["t"])


def _first_after_all_started(rows, last_start):
    for r in rows:
        if last_start is not None and r["t"] >= last_start:
            return {"agent": r["agent"], "t": r["t"], "seconds_after_last_start": round(r["t"] - last_start, 3),
                    "named_peer": r["named_peer"]}
    return None


def _probes(sessions, rows):
    """(agent, t, what was shown) of every observation that no teammate was reachable: a ListAgents result that named no one,
    and a SendMessage that failed because the name or socket was not there."""
    out = [(r["agent"], r["t"], _key_text(r["text"])) for r in rows if not r["named_peer"]]
    for s in sessions.values():
        out += [(s.get("agent"), x["t"], _key_text(x["text"])) for x in _facts(s).get("sends", [])
                if x.get("t") is not None and not delivered(x) and UNREACHABLE.search(x.get("text") or "")]
    return out


def _peer_running(sessions, alive, agent, t):
    return next((sessions[k].get("agent") for k, (a, b) in alive.items() if sessions[k].get("agent") != agent and a <= t <= b), None)


def _failure_reason(sessions, alive, rows):
    """The reason, or None: at least SETTLE_S after the latest start, a probe showed no teammate while another agent's stream
    shows it still running."""
    last = _clock(alive)["last_agent_start"]
    for agent, t, shown in sorted(_probes(sessions, rows), key=lambda p: p[1]):
        peer = _peer_running(sessions, alive, agent, t) if last is not None and t >= last + SETTLE_S else None
        if peer:
            seen = sorted({_key_text(r["text"]) for r in rows if not r["named_peer"]})
            return ("at %.0f s (%.0f s after the last agent started) %s got %r while %s was still running, and no ListAgents "
                    "result of the run named a teammate (ListAgents showed: %s)" % (
                        t, t - last, agent, shown, peer, "; ".join(seen) or "never called"))
    return None


def _unknown_reason(rows, first, last):
    if not rows:
        return "no ListAgents result in the run (no evidence either way)"
    if first is None:
        return "no ListAgents result came after every agent had started"
    return ("no ListAgents result came %.0f s after every agent had started while a teammate was still running (the first came "
            "%.1f s after the last start, at %.1f s)" % (SETTLE_S, first["seconds_after_last_start"], first["t"]))


def judge(arm, sessions, alive):
    """The messaging_verified object for a run, or None for an arm without messaging. sessions: session key -> stream summary
    (with "agent" and "messaging", see streams.StreamState.summary); alive: session key -> (start, end) seconds on the clock the
    streams' times use. The object has the four numbers of the record plus status, reason, listagents_calls, the clock facts
    (settle_s, last_agent_start, the alive window of each session) and first_listagents_after_all_started."""
    if arm not in MESSAGING_ARMS:
        return None
    alive = {k: v for k, v in alive.items() if k in sessions}
    team = sorted({s.get("agent") for s in sessions.values() if s.get("agent")})
    out, tools_reason = _numbers(sessions, team)
    clock, rows = _clock(alive), _results(sessions, team)
    first = _first_after_all_started(rows, clock["last_agent_start"])
    out.update(clock=clock, first_listagents_after_all_started=first,
               listagents_results=[{k: r[k] for k in ("agent", "t", "named_peer")} for r in rows[:MAX_LISTED]])
    if out["tools_present"] is False:
        return dict(out, status="failed", reason=tools_reason)
    if len(team) < 2:
        return dict(out, status="unknown", reason="fewer than two agents")
    if out["listagents_named_peer"]:
        return dict(out, status="verified", reason=None)
    why = _failure_reason(sessions, alive, rows)
    if why:
        return dict(out, status="failed", reason=why)
    return dict(out, status="unknown", reason=_unknown_reason(rows, first, clock["last_agent_start"]))


def note_for(mv):
    """The line for the record's notes, or None when there is nothing to say (verified)."""
    if mv is None or mv["status"] == "verified":
        return None
    if mv["status"] == "failed":
        return "messaging not working (#1520), the run is invalid: " + mv["reason"]
    return "messaging_verified is unknown (#1520), the run proves nothing about messaging: " + mv["reason"]
