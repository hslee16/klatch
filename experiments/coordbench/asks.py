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


def _send_event(inp):
    for k in MSG_KEYS:
        if isinstance(inp.get(k), str):
            return {"verb": "send", "text": inp[k]}
    return {"verb": "send", "text": json.dumps(inp, sort_keys=True)}


def _words(rest):
    try:
        args = shlex.split(rest)
    except ValueError:
        args = rest.split()
    return [a for a in args if not a.startswith("-")]


def _bash_messages(command, klatch_bin):
    binre = re.escape(klatch_bin) if klatch_bin else r"\S*klatch"
    out = []
    for m in re.finditer(binre + r"\s+(?:-\S+\s+)*(" + "|".join(VERBS) + r")\b([^\n;&|]*(?:\"[^\"]*\"|'[^']*')?[^\n;&|]*)", command):
        args = _words(m.group(2))
        if args:
            out.append({"verb": m.group(1), "text": args[-1]})
    return out


def extract(tool_name, tool_input, klatch_bin=None):
    """The messages one tool call sends: a list of {"verb", "text"} (verb is send for SendMessage)."""
    inp = tool_input if isinstance(tool_input, dict) else {}
    if tool_name == "SendMessage":
        return [_send_event(inp)]
    if tool_name != "Bash":
        return []
    return _bash_messages(inp.get("command", ""), klatch_bin)


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


def _after(x, y):
    """x is not before y: by the harness clock when both have one, else by the order the events were read."""
    if x["t"] is not None and y["t"] is not None:
        return x["t"] >= y["t"]
    return x["i"] > y["i"]


def _sort_events(events, facts):
    """(asks, answers, need_info count) of the events, in the order they were read."""
    asks, answers, need_info = [], [], 0
    for i, e in enumerate(events):
        kind = classify(e["text"], e.get("verb"))
        if kind == "ask":
            names = _names(e["text"], facts)
            asks.append({"i": i, "agent": e["agent"], "t": e.get("t"), "fact": names[0] if names else None})
        elif kind == "answer":
            keys, stale = _answer_facts(e["text"], facts)
            answers.append({"i": i, "agent": e["agent"], "t": e.get("t"), "facts": keys, "stale": stale})
        elif kind == "need_info":
            need_info += 1
    return asks, answers, need_info


def _answers_this_ask(ask, answer, next_same):
    """The answer names the ask's fact, came after the ask and not after the next ask for the same fact."""
    return (ask["fact"] in answer["facts"] and _after(answer, ask)
            and (next_same is None or not _after(answer, next_same)))


def _attach_matching(asks, answers):
    """Give each ask with a fact the answers that name it between it and the next ask for the same fact; the set of
    indexes of the answers used."""
    used = set()
    for k, ask in enumerate(asks):
        nxt = next((o for o in asks[k + 1:] if o["fact"] == ask["fact"]), None)
        ask["answers"] = [] if ask["fact"] is None else [a for a in answers if _answers_this_ask(ask, a, nxt)]
        used.update(a["i"] for a in ask["answers"])
    return used


def _link_answers(asks, answers):
    """Give every ask the answers it got (ask["answers"]); an ask with no fact takes the first spare answer after it."""
    used = _attach_matching(asks, answers)
    spare = [a for a in answers if a["i"] not in used]
    for ask in asks:
        if ask["fact"] is None:
            mine = [a for a in spare if _after(a, ask)]
            if mine:
                ask["answers"].append(mine[0])
                spare.remove(mine[0])


def _answered_earlier(ask, a):
    """The answer a (another event) names the ask's fact and was given before the ask."""
    return (ask["fact"] in a["facts"] and a["i"] != ask["i"] and _after(ask, a)
            and not (a["t"] == ask["t"] and a["i"] > ask["i"]))


def _is_repeat(ask, answers, scn):
    fact = ask["fact"]
    if fact is None:
        return False
    return fact in scn.get("seeded_answers", []) or any(_answered_earlier(ask, a) for a in answers)


def _is_late(ask, holders, done_t):
    """The ask came after the member who holds the fact had finished."""
    h = holders.get(ask["fact"])
    return bool(h and ask["t"] is not None and done_t.get(h) is not None and done_t[h] < ask["t"])


def _ask_timings(asks, answers, scn, done_t):
    """(answer delays, repeat asks, late asks {n, answered, lost}); sets ask["delay"] and ask["late"]."""
    holders = {k: v["holder"] for k, v in scn["facts"].items()}
    delays, repeat_asks, late = [], 0, {"n": 0, "answered": 0, "lost": 0}
    for ask in asks:
        if ask["answers"]:
            first = min(ask["answers"], key=lambda a: (a["t"] if a["t"] is not None else 0, a["i"]))
            ask["delay"] = _dt(first["t"], ask["t"])
            delays.append(ask["delay"])
        ask["late"] = _is_late(ask, holders, done_t)
        if ask["late"]:
            late["n"] += 1
            late["answered" if ask["answers"] else "lost"] += 1
        if _is_repeat(ask, answers, scn):
            repeat_asks += 1
    return delays, repeat_asks, late


def _could_repeat(spec, scn, answers):
    """The slot is of a kind that can repeat an ask, and the answer already existed (seeded, or given to another agent)."""
    if spec.get("kind") not in ("seeded-repeat", "late"):
        return False
    fact, agent = spec["fact"], spec["agent"]
    return fact in scn.get("seeded_answers", []) or any(fact in a["facts"] and a["agent"] != agent for a in answers)


def _repeat_opportunities(scn, answers, asked, slot_results):
    """(opportunities to repeat an ask, how many were avoided) over the slots of kind seeded-repeat and late."""
    opp = avoided = 0
    for s, spec in scn["slots"].items():
        if not _could_repeat(spec, scn, answers):
            continue
        opp += 1
        if (spec["agent"], spec["fact"]) not in asked and slot_results.get(s, {}).get("state") == "correct":
            avoided += 1
    return opp, avoided


def _answer_delays(delays):
    return [d for d in delays if d is not None] if all(d is not None for d in delays) else []


def _first_answer_delay(asks):
    """Seconds from the first ask to the first answer to any ask; None when nothing was answered."""
    answered = [a for a in asks if a["answers"]]
    if not answered:
        return None
    first_ask_t = min((a["t"] for a in asks if a["t"] is not None), default=None)
    first_ans_t = min((x["t"] for a in answered for x in a["answers"] if x["t"] is not None), default=None)
    return _dt(first_ans_t, first_ask_t)


def _slot_counts(scn, slot_results, asked):
    wrong = [s for s, r in slot_results.items() if r["state"] in ("wrong", "stale")]
    return {"wrong_guesses": len(wrong),
            "wrong_without_ask": sum(1 for s in wrong if (scn["slots"][s]["agent"], scn["slots"][s]["fact"]) not in asked),
            "unanswered_dependencies": sum(1 for r in slot_results.values() if r["state"] == "unanswered")}


def compute(events, done_t, scn, facts, slot_results):
    """The S4 metrics. events: [{"agent", "t", "verb", "text"}] in the order they were read; done_t: {agent: t of its result
    event}; scn: the scenario file; facts: the checker's facts; slot_results: the checker's per-slot states."""
    asks, answers, need_info = _sort_events(events, facts)
    _link_answers(asks, answers)
    delays, repeat_asks, late = _ask_timings(asks, answers, scn, done_t)
    answered = [a for a in asks if a["answers"]]
    asked = {(a["agent"], a["fact"]) for a in asks if a["fact"]}
    opp, avoided = _repeat_opportunities(scn, answers, asked, slot_results)
    out = {
        "asks_made": len(asks), "asks_unclassified": sum(1 for a in asks if a["fact"] is None),
        "asks_answered": len(answered), "asks_lost": len(asks) - len(answered),
        "time_to_first_answer_s": _first_answer_delay(asks), "answer_delays_s": _answer_delays(delays),
        "duplicate_answers": sum(max(0, len(a["answers"]) - 1) for a in asks), "repeat_asks": repeat_asks,
        "repeat_opportunities": opp, "repeat_asks_avoided": avoided,
        "late_asks": late["n"], "late_asks_answered": late["answered"], "late_asks_lost": late["lost"],
        "answers_stale": sum(len(a["stale"]) for a in answers)}
    out.update(_slot_counts(scn, slot_results, asked))
    out["need_info_events"] = need_info
    return out


# Ask counts for the run store cross-check (#1433). Where `compute` above reads what agents SENT, these count what the
# commands DID, from their results: a `klatch ask` that printed `asked #N` made an ask, a `klatch answer` that printed
# `answered #N` answered one, and a lost race (`not answered: ...`) answered nothing. The store export (storeexport.py) has
# the same two numbers; run.py records both side by side and flags a difference.
ASKED = re.compile(r"\basked #\d+")
ANSWERED = re.compile(r"(?<!not )\banswered #\d+")


def _count_results(tool, pat):
    """(asks created, asks answered) by the klatch commands of one tool call, by what their result printed."""
    created = answered = 0
    res = tool["result"] or ""
    for m in pat.finditer(chain._command(tool)):
        if m.group(1) == "ask" and ASKED.search(res):
            created += 1
        elif m.group(1) == "answer" and ANSWERED.search(res):
            answered += 1
    return created, answered


def from_streams(streams, klatch_bin):
    """{"created", "answered"} counted over {name: stream lines}: successful ask and answer commands, by their results."""
    binre = re.escape(klatch_bin) if klatch_bin else r"(?:\S*/)?klatch"
    pat = re.compile(binre + r"\s+(?:-\S+\s+)*(ask|answer)\b")
    created = answered = 0
    for lines in streams.values():
        for turn in chain.parse_stream(lines):
            for tool in turn["tools"]:
                c, a = _count_results(tool, pat)
                created, answered = created + c, answered + a
    return {"created": created, "answered": answered}


def cross_check(stream_counts, store_counts):
    """Both sides and whether they agree: {"streams", "store", "match", "differences"}."""
    diffs = ["%s: streams %d, store %d" % (k, stream_counts[k], store_counts[k])
             for k in ("created", "answered") if stream_counts[k] != store_counts[k]]
    return {"streams": dict(stream_counts), "store": dict(store_counts), "match": not diffs, "differences": diffs}
