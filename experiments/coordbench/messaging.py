"""Was native messaging working in a B1 or B1-roles run? (#1520.) Standard library only; nothing here starts a process.

If cross-session discovery fails, B1 silently becomes B0 plus a board and a comparison against it says nothing about native
messaging. So the harness reads the run's own streams and judges, per run, one of three things:

  failed    an init event that carries a tools list lacks SendMessage or ListAgents; or a ListAgents result came back while another
            agent had been alive for at least ALIVE_GRACE_S seconds, and no ListAgents result of the run ever named a teammate.
            The run is INVALID, with the reason in its notes.
  verified  some ListAgents result named a teammate.
  unknown   neither: no ListAgents ran while a teammate had been alive for the grace period (a run in which every agent finished
            before the next started is no evidence of failure), the run has fewer than two agents, or the streams carry no times.
            The run stays valid and says so; it is not evidence of working messaging either.

The result texts are the shapes of the two-session probe of 2026-11-23 (claude 2.1.331): `Peer sessions (1): b [eab75d] ... busy`
when a peer is visible and `No reachable agents` when none is; a failed SendMessage says `No agent named b is reachable`.
The grace period is there because a session binds its inbox a few seconds after it starts: a ListAgents run at the very
moment a colleague started is not a fair test.

Applies to the arms in drivers.MESSAGING_ARMS only; every other arm has the messaging tools denied and a record value of null."""
import re

MESSAGING_ARMS = ("B1", "B1-roles")
TOOLS = ("SendMessage", "ListAgents")
ALIVE_GRACE_S = 15.0
NOT_DELIVERED = re.compile(r"^\s*(?:Not sent|Not delivered|No agent named|Failed|Error)", re.I)
RECORD_KEYS = ("tools_present", "listagents_named_peer", "messages_sent", "messages_delivered")


def tool_names(tools):
    """The names in an init event's tools list (strings or objects with a name), or None when the event carries no list."""
    if not isinstance(tools, list):
        return None
    return [t if isinstance(t, str) else t.get("name") for t in tools if isinstance(t, str) or isinstance(t, dict)]


def _lists(text, name):
    """True when a ListAgents result has an entry `NAME [session id]` (at the start of a line or after the header's colon)."""
    return re.search(r"(?:^|[:\s])%s \[" % re.escape(name), text or "", re.M) is not None


def names_peer(text, agent, team):
    """True when a ListAgents result lists an agent of the team other than the one that asked. A longer name that merely
    contains a teammate's name does not count."""
    return any(_lists(text, name) for name in team if name != agent)


def delivered(send):
    return not send.get("error") and not NOT_DELIVERED.match(send.get("text") or "")


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


def _peer_alive_at(sessions, alive, agent, t):
    """(name, start) of an agent other than `agent` that had been alive for the grace period at time t, or None."""
    for other, (start, end) in alive.items():
        if sessions[other].get("agent") != agent and start + ALIVE_GRACE_S <= t <= end:
            return sessions[other].get("agent"), start
    return None


def _failure_reason(sessions, alive):
    """The reason, or None: a ListAgents result with no teammate in it, given while a teammate had been alive for the grace period."""
    calls = [(s.get("agent"), c) for s in sessions.values() for c in _facts(s).get("listagents", []) if c.get("t") is not None]
    for agent, call in calls:
        peer = _peer_alive_at(sessions, alive, agent, call["t"])
        if peer:
            shown = " ".join((call.get("text") or "").split())[:80] or "nothing"
            return ("a ListAgents result at %.0f s showed %r although %s had been alive since %.0f s, and no ListAgents "
                    "result of the run named a teammate" % (call["t"], shown, peer[0], peer[1]))
    return None


def _numbers(sessions, team):
    present, reason = _tools_problem(sessions)
    sends = [x for s in sessions.values() for x in _facts(s).get("sends", [])]
    calls = [c for s in sessions.values() for c in _facts(s).get("listagents", [])]
    named = any(names_peer(c.get("text"), s.get("agent"), team) for s in sessions.values() for c in _facts(s).get("listagents", []))
    return {"tools_present": present, "listagents_named_peer": named, "messages_sent": len(sends),
            "messages_delivered": sum(1 for x in sends if delivered(x)), "listagents_calls": len(calls)}, reason


def judge(arm, sessions, alive):
    """The messaging_verified object for a run, or None for an arm without messaging. sessions: session key -> stream summary
    (with "agent" and "messaging", see streams.StreamState.summary); alive: session key -> (start, end) seconds on the clock the
    streams' times use. The object has the four numbers of the record plus status, reason and listagents_calls."""
    if arm not in MESSAGING_ARMS:
        return None
    team = sorted({s.get("agent") for s in sessions.values() if s.get("agent")})
    out, tools_reason = _numbers(sessions, team)
    status, reason = "unknown", "no ListAgents result was given while a teammate had been alive for %.0f s" % ALIVE_GRACE_S
    if out["tools_present"] is False:
        status, reason = "failed", tools_reason
    elif len(team) < 2:
        reason = "fewer than two agents"
    elif out["listagents_named_peer"]:
        status, reason = "verified", None
    else:
        why = _failure_reason(sessions, {k: v for k, v in alive.items() if k in sessions})
        status, reason = ("failed", why) if why else (status, reason)
    return dict(out, status=status, reason=reason)


def note_for(mv):
    """The line for the record's notes, or None when there is nothing to say (verified)."""
    if mv is None or mv["status"] == "verified":
        return None
    if mv["status"] == "failed":
        return "messaging not working (#1520), the run is invalid: " + mv["reason"]
    return "messaging_verified is unknown (#1520): " + mv["reason"]
