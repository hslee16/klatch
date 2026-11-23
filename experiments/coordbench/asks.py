"""Ask and answer metrics for scenario S4 (#1369), from the message events in the agents' streams.

Events are what an agent sent: a `SendMessage` call, or a klatch `post`, `ask`, `answer` or `dm` command in Bash. The text of
a message says what it is by its first word, the convention every arm teaches: `seeking:` is an ask, `answer` (`answer:` or
`answer to #N:`) an answer, `need-info` a clarifying question from a helper. A fact is named in a message by its key (case,
spaces, underscores and punctuation ignored), and an answer may instead carry the fact's true or stale value. Standard library
only; nothing here starts a process. The definitions are in scenarios/s4.json, metrics.definitions.
"""
import json
import re
import shlex

import chain

MSG_KEYS = ("message", "content", "text", "body", "prompt")
VERBS = ("post", "dm", "ask", "answer")


def norm(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def extract(tool_name, tool_input, klatch_bin=None):
    """The messages one tool call sends: a list of {"verb", "text"} (verb is send for SendMessage)."""
    inp = tool_input if isinstance(tool_input, dict) else {}
    if tool_name == "SendMessage":
        for k in MSG_KEYS:
            if isinstance(inp.get(k), str):
                return [{"verb": "send", "text": inp[k]}]
        return [{"verb": "send", "text": json.dumps(inp, sort_keys=True)}]
    if tool_name != "Bash":
        return []
    binre = re.escape(klatch_bin) if klatch_bin else r"\S*klatch"
    out = []
    for m in re.finditer(binre + r"\s+(?:-\S+\s+)*(" + "|".join(VERBS) + r")\b([^\n;&|]*(?:\"[^\"]*\"|'[^']*')?[^\n;&|]*)", inp.get("command", "")):
        try:
            args = shlex.split(m.group(2))
        except ValueError:
            args = m.group(2).split()
        args = [a for a in args if not a.startswith("-")]
        if args:
            out.append({"verb": m.group(1), "text": args[-1]})
    return out


def classify(text, verb=None):
    t = text.strip().lower()
    if t.startswith("need-info") or t.startswith("need info"):
        return "need_info"
    if verb == "ask" or t.startswith("seeking"):
        return "ask"
    if verb == "answer" or t.startswith("answer"):
        return "answer"
    return "other"


def _shown(kind, v):
    forms = [str(v)]
    if kind == "hex16":
        forms.append("0x%04x" % v)
    return forms


def _has_value(text, kind, v):
    low = text.lower()
    return any(re.search(r"(?<![0-9a-z])%s(?![0-9a-z])" % re.escape(f.lower()), low) for f in _shown(kind, v))


def _names(text, facts):
    n = norm(text)
    return [k for k in facts if norm(k) in n]


def _answer_facts(text, facts):
    """(set of fact keys the answer is about, set of keys whose stale value it carries)."""
    keys = set(_names(text, facts))
    stale = set()
    for k, f in facts.items():
        has_true, has_stale = _has_value(text, f["kind"], f["value"]), _has_value(text, f["kind"], f["stale"])
        if has_true or has_stale:
            keys.add(k)
        if has_stale and not has_true:
            stale.add(k)
    return keys, stale


def _dt(t1, t0):
    return None if t1 is None or t0 is None else round(t1 - t0, 3)


def compute(events, done_t, scn, facts, slot_results):
    """The S4 metrics. events: [{"agent", "t", "verb", "text"}] in the order they were read; done_t: {agent: t of its result
    event}; scn: the scenario file; facts: the checker's facts; slot_results: the checker's per-slot states."""
    holders = {k: v["holder"] for k, v in scn["facts"].items()}
    ordered = list(enumerate(events))
    asks, answers, need_info = [], [], 0
    for i, e in ordered:
        kind = classify(e["text"], e.get("verb"))
        if kind == "ask":
            names = _names(e["text"], facts)
            asks.append({"i": i, "agent": e["agent"], "t": e.get("t"), "fact": names[0] if names else None})
        elif kind == "answer":
            keys, stale = _answer_facts(e["text"], facts)
            answers.append({"i": i, "agent": e["agent"], "t": e.get("t"), "facts": keys, "stale": stale})
        elif kind == "need_info":
            need_info += 1

    def after(x, y):  # x is not before y
        if x["t"] is not None and y["t"] is not None:
            return x["t"] >= y["t"]
        return x["i"] > y["i"]

    used = set()
    for k, ask in enumerate(asks):
        ask["answers"] = []
        if ask["fact"] is None:
            continue
        nxt = next((o for o in asks[k + 1:] if o["fact"] == ask["fact"]), None)
        for a in answers:
            if ask["fact"] in a["facts"] and after(a, ask) and (nxt is None or not after(a, nxt)):
                ask["answers"].append(a)
                used.add(a["i"])
    spare = [a for a in answers if a["i"] not in used]
    for ask in asks:
        if ask["fact"] is None:
            mine = [a for a in spare if after(a, ask)]
            if mine:
                ask["answers"].append(mine[0])
                spare.remove(mine[0])

    delays, repeat_asks, late = [], 0, {"n": 0, "answered": 0, "lost": 0}
    for ask in asks:
        if ask["answers"]:
            first = min(ask["answers"], key=lambda a: (a["t"] if a["t"] is not None else 0, a["i"]))
            ask["delay"] = _dt(first["t"], ask["t"])
            delays.append(ask["delay"])
        h = holders.get(ask["fact"])
        ask["late"] = bool(h and ask["t"] is not None and done_t.get(h) is not None and done_t[h] < ask["t"])
        if ask["late"]:
            late["n"] += 1
            late["answered" if ask["answers"] else "lost"] += 1
        if ask["fact"] is not None and (ask["fact"] in scn.get("seeded_answers", []) or any(
                ask["fact"] in a["facts"] and a["i"] != ask["i"] and (after(ask, a) and not (a["t"] == ask["t"] and a["i"] > ask["i"]))
                for a in answers)):
            repeat_asks += 1
    answered = [a for a in asks if a["answers"]]
    first_ask_t = min((a["t"] for a in asks if a["t"] is not None), default=None)
    first_ans_t = min((x["t"] for a in answered for x in a["answers"] if x["t"] is not None), default=None)
    asked = {(a["agent"], a["fact"]) for a in asks if a["fact"]}
    wrong = [s for s, r in slot_results.items() if r["state"] in ("wrong", "stale")]
    opp = avoided = 0
    for s, spec in scn["slots"].items():
        if spec.get("kind") not in ("seeded-repeat", "late"):
            continue
        fact, agent = spec["fact"], spec["agent"]
        existed = fact in scn.get("seeded_answers", []) or any(fact in a["facts"] and a["agent"] != agent for a in answers)
        if not existed:
            continue
        opp += 1
        if (agent, fact) not in asked and slot_results.get(s, {}).get("state") == "correct":
            avoided += 1
    dups = sum(max(0, len(a["answers"]) - 1) for a in asks)
    return {
        "asks_made": len(asks), "asks_unclassified": sum(1 for a in asks if a["fact"] is None),
        "asks_answered": len(answered), "asks_lost": len(asks) - len(answered),
        "time_to_first_answer_s": _dt(first_ans_t, first_ask_t) if answered else None,
        "answer_delays_s": [d for d in delays if d is not None] if all(d is not None for d in delays) else [],
        "duplicate_answers": dups, "repeat_asks": repeat_asks, "repeat_opportunities": opp, "repeat_asks_avoided": avoided,
        "late_asks": late["n"], "late_asks_answered": late["answered"], "late_asks_lost": late["lost"],
        "answers_stale": sum(len(a["stale"]) for a in answers),
        "wrong_guesses": len(wrong), "wrong_without_ask": sum(1 for s in wrong if (scn["slots"][s]["agent"], scn["slots"][s]["fact"]) not in asked),
        "unanswered_dependencies": sum(1 for r in slot_results.values() if r["state"] == "unanswered"),
        "need_info_events": need_info,
    }


# Ask counts for the run store cross-check (#1433). Where `compute` above reads what agents SENT, these count what the
# commands DID, from their results: a `klatch ask` that printed `asked #N` made an ask, a `klatch answer` that printed
# `answered #N` answered one, and a lost race (`not answered: ...`) answered nothing. The store export (storeexport.py) has
# the same two numbers; run.py records both side by side and flags a difference.
ASKED = re.compile(r"\basked #\d+")
ANSWERED = re.compile(r"(?<!not )\banswered #\d+")


def from_streams(streams, klatch_bin):
    """{"created", "answered"} counted over {name: stream lines}: successful ask and answer commands, by their results."""
    binre = re.escape(klatch_bin) if klatch_bin else r"(?:\S*/)?klatch"
    pat = re.compile(binre + r"\s+(?:-\S+\s+)*(ask|answer)\b")
    created = answered = 0
    for lines in streams.values():
        for turn in chain.parse_stream(lines):
            for tool in turn["tools"]:
                cmd = chain._command(tool)
                res = tool["result"] or ""
                for m in pat.finditer(cmd):
                    if m.group(1) == "ask" and ASKED.search(res):
                        created += 1
                    elif m.group(1) == "answer" and ANSWERED.search(res):
                        answered += 1
    return {"created": created, "answered": answered}


def cross_check(stream_counts, store_counts):
    """Both sides and whether they agree: {"streams", "store", "match", "differences"}."""
    diffs = ["%s: streams %d, store %d" % (k, stream_counts[k], store_counts[k])
             for k in ("created", "answered") if stream_counts[k] != store_counts[k]]
    return {"streams": dict(stream_counts), "store": dict(store_counts), "match": not diffs, "differences": diffs}
