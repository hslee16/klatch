"""The scripted room history of fixture F9 `history` (#1374), for gen_facts.py: about 60 messages by ada, bo and cy, in the
order of the commits. A generator source: its output (room.json) is committed and checked by `gen_facts.py --check`."""


class Room:
    """The steps {as, argv, key?}. argv is the klatch command line after the binary; {{ask:KEY}} in a text is the number the
    ask got."""

    def __init__(self):
        self.steps = []

    def step(self, who, *argv, key=None):
        d = {"as": who, "argv": list(argv)}
        if key:
            d["key"] = key
        self.steps.append(d)

    def post(self, who, text):
        self.step(who, "post", "lobby", text)

    def claim(self, who, path, reason):
        self.step(who, "claim", "lobby", path, "--reason", reason)

    def release(self, who, path, note):
        self.step(who, "release", "lobby", path, "--note", note)

    def ask(self, who, key, text):
        self.step(who, "ask", "lobby", text, key=key)

    def answer(self, who, key, text):
        self.step(who, "answer", "lobby", "{{ask:%s}}" % key, text)

    def close(self, who, key, extra=""):
        self.post(who, ("closed #{{ask:%s}}" % key) + (" " + extra if extra else ""))


def _start(r, v):
    """Ticket 1, and the start of ticket 2."""
    r.post("ada", "working: ticket 1 weights, parcelkit/weights.py")
    r.claim("ada", "parcelkit/weights.py", "ticket 1")
    r.post("ada", "checkpoint: weights.to_grams done, block_count next")
    r.post("ada", "finished: ticket 1 weights, tests pass")
    r.post("ada", "merged: ticket 1 on main")
    r.release("ada", "parcelkit/weights.py", "weights done")
    r.post("ada", "working: ticket 2 zones, parcelkit/zones.py")
    r.claim("ada", "parcelkit/zones.py", "ticket 2")


def _middle(r, v):
    """Tickets 3 to 6, with the first versions of the decisions."""
    tare, ps = v["tare"], v["ps"]
    r.post("bo", "working: ticket 3 rates, parcelkit/rates.py")
    r.claim("bo", "parcelkit/rates.py", "ticket 3")
    r.ask("bo", "units", "what unit are weights in inside parcelkit?")
    r.answer("ada", "units", "whole grams, weights.to_grams converts from kilograms")
    r.close("bo", "units")
    r.ask("cy", "zonearg", "does zone_for take the postcode as a string?")
    r.answer("ada", "zonearg", "yes, a string of five digits; I will make it raise ValueError for anything else")
    r.close("cy", "zonearg")
    r.post("ada", "zone 4 is missing in my first cut, fixing it now: 9xxxx is zone 4")
    r.post("ada", "finished: ticket 2 zones, four zones")
    r.post("ada", "merged: ticket 2 on main")
    r.release("ada", "parcelkit/zones.py", "zones done")
    r.post("cy", "working: ticket 5 labels, parcelkit/labels.py")
    r.claim("cy", "parcelkit/labels.py", "ticket 5")
    r.ask("bo", "tare", "decision: the scanner reports GROSS weights; the manifest subtracts a tare of TARE_G = %d g from each "
        "parcel, never below 0 g, before rate_cents (which takes the net weight). Agree?" % tare)
    r.answer("ada", "tare", "ok: agreed, TARE_G = %d g, floor at 0 g" % tare)
    r.close("bo", "tare", "ok")
    r.post("bo", "checkpoint: rates has base plus per-block pricing, the tare rule is decision #{{ask:tare}}")
    r.ask("cy", "pagesold", "decision: a manifest page holds exactly PAGE_ROWS = %d rows because the label sheet layout assumes "
        "it. Agree?" % ps)
    r.answer("bo", "pagesold", "ok: agreed, %d rows per page" % ps)
    r.close("cy", "pagesold", "ok")
    r.post("bo", "finished: ticket 3 rates, tests pass")
    r.post("bo", "merged: ticket 3 on main")
    r.release("bo", "parcelkit/rates.py", "rates done")
    r.post("ada", "working: ticket 4 parcels, parcelkit/parcels.py")
    r.claim("ada", "parcelkit/parcels.py", "ticket 4")
    r.post("cy", "checkpoint: labels.paginate done, render_label next")
    r.post("ada", "finished: ticket 4 parcels, tests pass")
    r.post("ada", "merged: ticket 4 on main")
    r.release("ada", "parcelkit/parcels.py", "parcels done")
    r.ask("bo", "order", "does paginate keep the order of the rows and never return an empty page?")
    r.answer("cy", "order", "yes to both; it takes the page size as an argument")
    r.close("bo", "order")
    r.post("cy", "finished: ticket 5 labels, tests pass")
    r.post("cy", "merged: ticket 5 on main")
    r.release("cy", "parcelkit/labels.py", "labels done")
    r.post("bo", "working: ticket 6 ledger, parcelkit/ledger.py")
    r.claim("bo", "parcelkit/ledger.py", "ticket 6")
    r.post("bo", "finished: ticket 6 ledger, tests pass")
    r.post("bo", "merged: ticket 6 on main")
    r.release("bo", "parcelkit/ledger.py", "ledger done")


def _late(r, v):
    """The revised decisions and the handoff."""
    tare, pn, ps, tn, ts = v["tare"], v["pn"], v["ps"], v["tn"], v["ts"]
    r.ask("bo", "nameold", "decision: a manifest file is named TAG-YYYYMMDD-SEQ.mf with TAG = %s, the date as 8 digits and SEQ "
        "zero padded to 3 digits. Agree?" % ts)
    r.answer("ada", "nameold", "ok: agreed, %s-YYYYMMDD-SEQ.mf" % ts)
    r.close("bo", "nameold", "ok")
    r.post("ada", "status: tickets 1 to 6 are being finished, README updated")
    r.ask("cy", "eight", "is ticket 8 (cli) blocked on ticket 7?")
    r.answer("bo", "eight", "yes, the cli calls build_manifest, so it waits for ticket 7")
    r.close("cy", "eight")
    r.ask("cy", "pagesnew", "decision: PAGE_ROWS is now %d, not %d: the printer test shows %d rows overflow the sheet. This "
        "supersedes #{{ask:pagesold}}. Agree?" % (pn, ps, ps))
    r.answer("bo", "pagesnew", "ok: agreed, %d rows per page, supersedes #{{ask:pagesold}}" % pn)
    r.close("cy", "pagesnew", "ok")
    r.post("bo", "rates now refuses a negative net weight: the manifest must floor the net weight at 0 g after the tare")
    r.post("ada", "zones now refuses a postcode that is not five digits")
    r.ask("bo", "namenew", "decision: the file name tag is now TAG = %s, not %s: the archive tool already uses %s as a prefix. "
        "This supersedes #{{ask:nameold}}. Agree?" % (tn, ts, ts))
    r.answer("ada", "namenew", "ok: agreed, TAG = %s, supersedes #{{ask:nameold}}" % tn)
    r.close("bo", "namenew", "ok")
    r.post("cy", "labels: render_label pads the reference to 12 characters now")
    r.post("ada", "parcels: parse_line tolerates spaces around each field now")
    r.post("bo", "checkpoint: all six tickets merged on main; the manifest is not started")
    r.post("bo", "handoff: ticket 7 manifest is open and not started; the decisions on tare, page size and file name are in the "
         "room (klatch asks, klatch history); later decisions supersede earlier ones")
    r.post("cy", "finished: README status updated, tickets 1 to 6 done")


def room(f):
    """The scripted room history (a list of steps) of the facts f."""
    v = {"tare": f["TARE_G"]["value"], "pn": f["PAGE_ROWS"]["value"], "ps": f["PAGE_ROWS"]["stale"],
         "tn": f["TAG"]["value"], "ts": f["TAG"]["stale"]}
    r = Room()
    for who in ("ada", "bo", "cy"):
        r.step(who, "join", "lobby")
    for part in (_start, _middle, _late):
        part(r, v)
    for who in ("ada", "bo", "cy"):
        r.post(who, "status: done")
        r.step(who, "status", "lobby", "done")
    return r.steps
