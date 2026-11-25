"""Which agents stopped on their own spend or turn cap (#1553).

`claude -p` ends an agent that used its --max-budget-usd with a result of subtype error_max_budget_usd (terminal_reason
budget_exhausted) and one that used its --max-turns with error_max_turns, both with exit code 1. That is the cap the SPEC
names, not a crash: the run outcome is `spend-cap` or `turn-cap` and the record's notes say which agent stopped."""

BUDGET_SUBTYPE, TURNS_SUBTYPE, BUDGET_REASON = "error_max_budget_usd", "error_max_turns", "budget_exhausted"
# outcome -> the order of precedence is the order here (a spend stop is reported before a turn stop)
OUTCOME_OF = (("spend-cap", BUDGET_SUBTYPE), ("turn-cap", TURNS_SUBTYPE))


def stop_of(summary):
    """spend-cap, turn-cap or None for one agent's stream summary."""
    if summary.get("result_subtype") == BUDGET_SUBTYPE or summary.get("terminal_reason") == BUDGET_REASON:
        return "spend-cap"
    return "turn-cap" if summary.get("result_subtype") == TURNS_SUBTYPE else None


def agent_caps(sums):
    """{outcome: [session keys]} for the sessions in sums (key -> stream summary) that stopped on a cap."""
    caps = {}
    for key, s in sums.items():
        stop = stop_of(s)
        if stop:
            caps.setdefault(stop, []).append(key)
    return caps


def cap_outcome(caps):
    """The run outcome for agent_caps' result: spend-cap first, then turn-cap, else None."""
    for outcome, _ in OUTCOME_OF:
        if outcome in caps:
            return outcome
    return None


def notes(caps, summaries, agent_of):
    """One note per capped agent: its name, the result subtype and what it had spent."""
    out = []
    for outcome, keys in sorted(caps.items()):
        for k in keys:
            s = summaries[k]
            out.append("agent %s stopped on its own %s cap (%s, terminal_reason %s, spent %.4f USD, %d turns)" % (
                agent_of.get(k, k), "spend" if outcome == "spend-cap" else "turn", s.get("result_subtype"),
                s.get("terminal_reason"), s["cost_usd"], s["turns"]))
    return out
