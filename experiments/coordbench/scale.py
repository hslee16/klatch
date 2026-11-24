"""Scenario S7 (#1372): the scale metrics of one run and the scaling table over team sizes.

stream_parts(states, n, messages) is called by run.py with the parsed streams of the agents and gives the stream-derived
part, kept under scenario_metrics.scale in the record. derive(rec) turns a record into the per-member and per-unit figures
the scaling table uses (messages per agent, tokens spent reading the room per member, units per agent per turn and per dollar,
and so on). scaling(records) groups the valid runs of the scenarios `<fixture>-n<N>` by arm and size, takes the median of every
metric at each size and the ratio of the largest size to the next one, and says whether adding agents still helps (units per
dollar). The sizes are separate scenarios in the records and are never pooled; this table only sets them side by side.
Standard library only; no process is started and nothing is read from the environment."""
import re

import stats

SIZE_SCENARIO = re.compile(r"^(?P<base>.+)-n(?P<n>\d+)$")
# (metric, label). The rows of the scaling table, in the order they are shown.
METRICS = (
    ("messages_per_agent", "messages posted per agent"),
    ("read_context_tokens_per_member", "tokens read from the room per member (klatch read, wait, digest, board, TASKS.md)"),
    ("coordination_context_tokens_per_member", "all coordination context tokens per member"),
    ("hook_chars_per_member", "characters injected by hooks per member (from the stream)"),
    ("inbound_tokens_per_member", "tokens of inbound messages per member (B1 SendMessage deliveries, from the stream)"),
    ("schema_tokens_per_member", "tokens of coordination tool schemas per member (pinned estimate unless the stream carries them)"),
    ("hook_digest_bytes_per_member", "hook bytes injected per member (store export, K arms)"),
    ("max_coord_result_chars", "largest single coordination tool result (characters)"),
    ("units_completed", "units completed"),
    ("units_per_agent", "units per agent"),
    ("units_per_agent_turn", "units per agent per turn (units over total turns)"),
    ("units_per_usd", "units per dollar"),
    ("tickets_per_agent_minute", "tickets per agent-minute"),
    ("cost_usd", "cost (USD)"),
    ("turns", "turns"),
    ("git_conflicts", "git conflicts"),
    ("overwrites", "overwrites"),
    ("duplicate_work", "duplicate work (units)"),
    ("wall_s", "wall time (s)"),
)
STILL_HELPS = "adding agents still helps (units per dollar rose)"
STOPPED = "adding agents stopped helping (units per dollar fell)"
NOT_COMPUTED = "not computed (no spend, or a size is missing)"


def stream_parts(states, n, messages=None):
    """The stream-derived part of the S7 metrics: states is {agent: streams.StreamState}, n the number of agents."""
    read = sum(s.read_chars for s in states.values())
    hook = sum(s.hook_chars for s in states.values())
    largest = max([s.max_coord_result for s in states.values()] or [0])
    inbound = sum(s.parts["inbound"] for s in states.values())
    schema = sum(s.parts["schema"] for s in states.values())
    return {
        "agents": n,
        "messages_per_agent": (messages / n) if messages is not None and n else None,
        "read_context_chars": read,
        "read_context_tokens": read // 4,
        "read_context_tokens_per_member": read / 4 / n if n else None,
        "hook_context_chars": hook,
        "hook_chars_per_member": hook / n if n else None,
        "inbound_context_chars": inbound,
        "schema_context_chars": schema,
        "context_method": "chars/4",
        "max_coord_result_chars": largest,
        "hook_bytes_injected": None,
        "hook_bytes_reason": "no store export in this record: a B arm has no klatch store, and the klatch-bench store export "
                             "(scenario_metrics.store.hook_bytes_injected, #1433) fills it for a K arm when the tool is available",
        "chars_per_token": 4,
    }


def _div(a, b):
    return None if a is None or not b else a / b


def derive(rec):
    """The scaling figures of one record, or None when the record has no scenario_metrics.scale. Every figure is None when
    its inputs are missing (never zero by default)."""
    sm = rec.get("scenario_metrics") or {}
    sc = sm.get("scale")
    if not isinstance(sc, dict) or not sc.get("agents"):
        return None
    n = sc["agents"]
    m = rec.get("metrics") or {}
    units = m.get("units_completed")
    turns = (rec.get("turns") or {}).get("total")
    usd = (rec.get("spend") or {}).get("usd")
    wall = rec.get("wall_s")
    store = (sm.get("store") or {}).get("hook_bytes_injected")
    hook_bytes = store if store is not None else sc.get("hook_bytes_injected")
    # K3 and B1-roles (#1421): the store counted every member's hooks, the role members and replacements too, so its figure is
    # divided by all the members the store saw; every other per-member figure counts the workers only (their streams)
    roles = rec.get("roles") or {}
    store_members = n + len(roles.get("members") or {}) + len(roles.get("replacements") or {}) if store is not None else n
    tickets = sm.get("tickets_completed")
    return {
        "agents": n,
        "messages_per_agent": _div(m.get("messages"), n),
        "read_context_tokens_per_member": _div(sc.get("read_context_chars"), 4 * n),
        "coordination_context_tokens_per_member": _div(m.get("coordination_context_tokens"), n),
        "hook_chars_per_member": _div(sc.get("hook_context_chars"), n),
        "inbound_tokens_per_member": _div(sc.get("inbound_context_chars"), 4 * n),
        "schema_tokens_per_member": _div(sc.get("schema_context_chars"), 4 * n),
        "hook_digest_bytes_per_member": _div(hook_bytes, store_members),
        "max_coord_result_chars": sc.get("max_coord_result_chars"),
        "units_completed": units,
        "units_per_agent": _div(units, n),
        "units_per_agent_turn": _div(units, turns),
        "units_per_usd": _div(units, usd),
        "tickets_per_agent_minute": _div(tickets, n * wall / 60.0) if wall else None,
        "cost_usd": usd,
        "turns": turns,
        "git_conflicts": m.get("git_conflicts"),
        "overwrites": m.get("overwrites"),
        "duplicate_work": m.get("duplicate_work"),
        "wall_s": wall,
    }


def _median(values):
    vals = [float(v) for v in values if isinstance(v, (int, float)) and not isinstance(v, bool)]
    if not vals:
        return None
    return stats.summary(vals)["median"]


def scaling(records):
    """The scaling table over the sizes found in the records, or None when no scenario `<name>-n<N>` has a scale record.
    {"sizes": [8, 16], "arms": {arm: {"rows": [{"metric", "label", "values": {"8": v, "16": v}, "n": {...}, "ratio"}],
    "units_per_usd": {"values", "ratio", "verdict"} (with two sizes or more)}}}. Invalid runs are left out; the ratio is the
    median at the largest size over the median at the next one down (8 to 16 for the scenario's two sizes)."""
    cells = {}  # (arm, size) -> list of derived dicts
    for r in records:
        if not r.get("valid", True):
            continue
        d = derive(r)
        if d is None or not SIZE_SCENARIO.match(r.get("scenario", "")):
            continue
        cells.setdefault((r["arm"], d["agents"]), []).append(d)
    if not cells:
        return None
    sizes = sorted({n for _, n in cells})
    out = {"sizes": sizes, "arms": {}, "ratio_between": [sizes[-2], sizes[-1]] if len(sizes) > 1 else None}
    for arm in sorted({a for a, _ in cells}):
        rows = []
        for metric, label in METRICS:
            values = {str(n): _median([d[metric] for d in cells.get((arm, n), [])]) for n in sizes}
            counts = {str(n): len(cells.get((arm, n), [])) for n in sizes}
            ratio = None
            if len(sizes) > 1:
                lo, hi = values[str(sizes[-2])], values[str(sizes[-1])]
                ratio = _div(hi, lo)
            rows.append({"metric": metric, "label": label, "values": values, "n": counts, "ratio": ratio})
        entry = {"rows": rows}
        if len(sizes) > 1:
            row = next(r for r in rows if r["metric"] == "units_per_usd")
            ratio = row["ratio"]
            entry["units_per_usd"] = {"values": row["values"], "ratio": ratio,
                                      "verdict": NOT_COMPUTED if ratio is None else (STOPPED if ratio < 1 else STILL_HELPS)}
        out["arms"][arm] = entry
    return out


def _f(x):
    if x is None:
        return "n/a"
    if isinstance(x, float) and x.is_integer() and abs(x) < 1e9:
        return "%d" % x
    return "%.3g" % x if abs(x) >= 1000 else "%.3f" % x


def render(sc):
    """Markdown lines of the scaling section ([] when there is nothing to show)."""
    if not sc:
        return []
    sizes = sc["sizes"]
    L = ["## Scaling curve", ""]
    L.append("Each metric against team size, as the median of the valid runs at that size (N per size in the last column). "
             "The sizes are separate scenarios in the records and are not pooled; this table only sets them side by side. "
             "Tokens are characters over 4.")
    L.append("")
    for arm, entry in sc["arms"].items():
        L += ["### Arm %s" % arm, ""]
        head = ["Metric"] + ["N=%d" % n for n in sizes]
        if len(sizes) > 1:
            head.append("Ratio %d over %d" % (sizes[-1], sizes[-2]))
        head.append("Runs per size")
        rows = []
        for r in entry["rows"]:
            row = [r["metric"]] + [_f(r["values"][str(n)]) for n in sizes]
            if len(sizes) > 1:
                row.append(_f(r["ratio"]))
            row.append("/".join(str(r["n"][str(n)]) for n in sizes))
            rows.append(row)
        L.append("| " + " | ".join(head) + " |")
        L.append("|" + "|".join("---" for _ in head) + "|")
        L += ["| " + " | ".join(str(c) for c in row) + " |" for row in rows]
        L.append("")
        if "units_per_usd" in entry:
            u = entry["units_per_usd"]
            L.append("Units per dollar, %d to %d agents: ratio %s. %s." % (sizes[-2], sizes[-1], _f(u["ratio"]), u["verdict"]))
            L.append("")
    L.append("Role arms (K3, B1-roles, #1421): cost, and so units per dollar, is the team total including the integrator and "
             "supervisor (and any replacement); turns, messages and the per-member context figures count the workers only; the "
             "store's hook bytes are divided by every member the store saw, the role members included.")
    L.append("")
    L.append("Metric definitions: scenarios/s7.json. hook_digest_bytes_per_member is n/a for a B arm (no klatch store) and for a "
             "K arm whose run had no store export.")
    L.append("")
    return L
