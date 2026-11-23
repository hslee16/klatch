#!/usr/bin/env python3
"""Generate fixture F9 `history` (#1374): the seeded facts, the prebuilt git history, the prebuilt room history, the
board files, the reference solution, the failing variants and the hidden checker's facts.

    gen_facts.py            write the files (run once; the output is committed)
    gen_facts.py --check    exit 1 if a committed file differs from what the seed generates

Everything is drawn from a fixed seed with a fixed order of draws, so every run of the bench sees the same facts and the
output is reproducible byte for byte. The facts the late joiner needs are arbitrary (never inferable from a name or from
the code) and are drawn so that none is one of the conventional defaults an agent would guess (the `plausible` lists, kept
in the hidden checker's facts.json). Two facts were decided, then revised later in the history: the earlier value is the
`stale` one, so an agent that reads only the early part of the record gets a stale value. The code of the six finished
tickets does not contain any of the three values: they live only in the record (commit messages in every arm, plus the room
history in the K arms and TASKS.md and DECISIONS.md in B1).

Files written (paths relative to experiments/coordbench):
  fixtures/history/history.json        the 25 scripted commits (author, date, message, files, board and decision lines)
  fixtures/history/room.json           the scripted room history: about 60 messages by three earlier members (K arms)
  fixtures/history/reference/...       the reference solution (never in a checkout)
  fixtures/history/variants/<name>/... failing variants of the solution, only the files that differ (never in a checkout)
  checks/history/facts.json            the hidden checker's facts (values, stale values, plausible guesses, module hashes)
"""
import hashlib
import json
import os
import random
import re
import sys

SEED = 20261124
HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(os.path.dirname(HERE))
LETTERS = "bcdfghjkmnpqrstvwxz"
DAY0 = "2026-11-20T09:00:00Z"

# fact key -> (kind, plausible defaults). The order is the order of the draws: do not reorder.
FACTS = [
    ("TAG", "tag", ["mf", "pk", "man", "parcel", "manifest", "pkit", "mnf", "ship", "pm"]),
    ("PAGE_ROWS", "rows", [10, 20, 25, 30, 32, 40, 48, 50, 60, 64, 75, 80, 100, 128]),
    ("TARE_G", "tare", [0, 100, 120, 150, 200, 250, 300, 400, 450, 500]),
]
BOARD_HEADER = ("# Task board\n\nOne line per piece of work: `- WHO | STATUS | FILES | TITLE`. STATUS is `working`, `done`, "
                "`blocked` or `handoff`. Add your line at the end, commit it and push it.\n")
DECISIONS_HEADER = ("# Decisions\n\nOne line per decision, newest last: `- DN | WHO | DECISION | DATE`. A later decision that "
                    "replaces an earlier one says `supersedes DN` and the earlier line is not edited.\n")


def draw(rng, kind, avoid):
    while True:
        if kind == "tag":
            v = rng.choice(LETTERS) + rng.choice(LETTERS) + str(rng.randrange(2, 10)) + rng.choice(LETTERS)
        elif kind == "rows":
            v = rng.randrange(31, 98)
        elif kind == "tare":
            v = rng.randrange(121, 480)
        else:
            raise ValueError(kind)
        if v not in avoid:
            return v


def code_numbers():
    """Every whole number written anywhere in the finished code, tests and README: a seeded value is never one of them, so
    a value found in the code is never a coincidence."""
    texts = [INIT, WEIGHTS, ZONES_V1, ZONES_V2, ZONES_V3, RATES_V1, RATES_V2, RATES_V3, PARCELS_V1, PARCELS_V2, LABELS_V1,
             LABELS_V2, LABELS_V3, LEDGER, T_WEIGHTS, T_ZONES, T_RATES, T_PARCELS, T_LABELS, T_LEDGER, MANIFEST_TEST,
             readme("")]
    return set(int(n) for t in texts for n in re.findall(r"\d+", t))


def facts():
    rng = random.Random(SEED)
    out = {}
    nums = code_numbers()
    for key, kind, plausible in FACTS:
        avoid = plausible + sorted(nums)
        value = draw(rng, kind, avoid)
        stale = draw(rng, kind, avoid + [value]) if key != "TARE_G" else None
        out[key] = {"kind": kind, "value": value, "stale": stale, "plausible": plausible}
    return out


# --- the code of the six finished tickets, by version (no seeded value appears in any of it) ---------------------------

INIT = '"""parcelkit: shipping manifests for a small warehouse."""\n'

WEIGHTS = '''"""Weights. Inside parcelkit every weight is a whole number of grams."""


def to_grams(kg):
    """Kilograms (a float) to whole grams, rounded to the nearest gram."""
    return int(round(kg * 1000))


def block_count(weight_g, block_g=100):
    """How many started blocks of block_g grams the weight fills (0 for a weight of 0 or less)."""
    if weight_g <= 0:
        return 0
    return -(-weight_g // block_g)
'''

ZONES_V1 = '''"""Postage zones from postcodes."""


def zone_for(postcode):
    """The zone of a five digit postcode: first digit 0-2 is zone 1, 3-5 is zone 2, anything else is zone 3."""
    first = int(postcode[0])
    if first <= 2:
        return 1
    if first <= 5:
        return 2
    return 3
'''
ZONES_V2 = ZONES_V1.replace('''anything else is zone 3."""''', '''6-8 is zone 3, 9 is zone 4 (remote)."""''').replace(
    "    return 3\n", "    if first <= 8:\n        return 3\n    return 4\n")
ZONES_V3 = ZONES_V2.replace('''    first = int(postcode[0])''', '''    if len(postcode) != 5 or not postcode.isdigit():
        raise ValueError("not a postcode: %r" % (postcode,))
    first = int(postcode[0])''')

RATES_V1 = '''"""Prices."""

BASE_CENTS = {1: 450, 2: 620, 3: 810, 4: 1100}


def rate_cents(net_weight_g, zone):
    """Price in cents of one parcel in a zone. net_weight_g is the NET weight in whole grams."""
    return BASE_CENTS[zone]
'''
RATES_V2 = '''"""Prices."""
from parcelkit.weights import block_count

BASE_CENTS = {1: 450, 2: 620, 3: 810, 4: 1100}
BLOCK_CENTS = {1: 35, 2: 48, 3: 62, 4: 90}


def rate_cents(net_weight_g, zone):
    """Price in cents of one parcel in a zone: the zone's base plus its price for every started 100 g block of the NET
    weight (whole grams)."""
    return BASE_CENTS[zone] + BLOCK_CENTS[zone] * block_count(net_weight_g)
'''
RATES_V3 = RATES_V2.replace('''    return BASE_CENTS[zone]''', '''    if net_weight_g < 0:
        raise ValueError("a net weight cannot be negative")
    return BASE_CENTS[zone]''')

PARCELS_V1 = '''"""Parcels as they arrive from the scanner."""


def parse_line(line):
    """Read 'ref,weight_g,postcode' into {"ref", "weight_g" (an int), "postcode" (a string)}; ValueError when the line
    is not that."""
    parts = line.strip().split(",")
    if len(parts) != 3 or not parts[0]:
        raise ValueError("expected ref,weight_g,postcode: %r" % (line,))
    return {"ref": parts[0], "weight_g": int(parts[1]), "postcode": parts[2]}
'''
PARCELS_V2 = PARCELS_V1.replace('''    parts = line.strip().split(",")''', '''    parts = [p.strip() for p in line.strip().split(",")]''')

LABELS_V1 = '''"""Label sheets."""


def paginate(items, size):
    """Split items into consecutive lists of at most size items, in order, with no empty page. size must be at least 1."""
    if size < 1:
        raise ValueError("a page needs at least one row")
    return [list(items[i:i + size]) for i in range(0, len(items), size)]
'''
LABELS_V2 = LABELS_V1 + '''

def render_label(row):
    """One line of a label: the reference padded to 10 characters and the zone."""
    return "%-10s zone %d" % (row["ref"], row["zone"])
'''
LABELS_V3 = LABELS_V2.replace("%-10s", "%-12s").replace("padded to 10", "padded to 12")

LEDGER = '''"""Totals."""


def total_cents(amounts):
    """The sum of whole-cent amounts. A float is refused with TypeError: floats hide rounding errors."""
    total = 0
    for a in amounts:
        if not isinstance(a, int) or isinstance(a, bool):
            raise TypeError("an amount is a whole number of cents: %r" % (a,))
        total += a
    return total
'''

T_WEIGHTS = '''import unittest

from parcelkit.weights import block_count, to_grams


class TestWeights(unittest.TestCase):
    def test_to_grams_rounds(self):
        self.assertEqual(to_grams(1.2504), 1250)
        self.assertEqual(to_grams(0), 0)

    def test_block_count_counts_started_blocks(self):
        self.assertEqual(block_count(0), 0)
        self.assertEqual(block_count(1), 1)
        self.assertEqual(block_count(100), 1)
        self.assertEqual(block_count(101), 2)
'''
T_ZONES = '''import unittest

from parcelkit.zones import zone_for


class TestZones(unittest.TestCase):
    def test_zones_by_first_digit(self):
        self.assertEqual([zone_for(p) for p in ("01234", "29999", "30000", "55555", "60000", "88888", "90000")],
                         [1, 1, 2, 2, 3, 3, 4])

    def test_a_bad_postcode_is_refused(self):
        for bad in ("1234", "123456", "12a45", ""):
            with self.assertRaises(ValueError):
                zone_for(bad)
'''
T_RATES = '''import unittest

from parcelkit.rates import rate_cents


class TestRates(unittest.TestCase):
    def test_base_and_blocks(self):
        self.assertEqual(rate_cents(0, 1), 450)
        self.assertEqual(rate_cents(100, 1), 485)
        self.assertEqual(rate_cents(250, 3), 810 + 3 * 62)

    def test_a_negative_net_weight_is_refused(self):
        with self.assertRaises(ValueError):
            rate_cents(-1, 2)
'''
T_PARCELS = '''import unittest

from parcelkit.parcels import parse_line


class TestParcels(unittest.TestCase):
    def test_a_line(self):
        self.assertEqual(parse_line("A1,500,12345\\n"), {"ref": "A1", "weight_g": 500, "postcode": "12345"})
        self.assertEqual(parse_line("A1 , 500 , 12345 "), {"ref": "A1", "weight_g": 500, "postcode": "12345"})

    def test_a_bad_line_is_refused(self):
        for bad in ("", "A1,500", ",500,12345", "A1,heavy,12345"):
            with self.assertRaises(ValueError):
                parse_line(bad)
'''
T_LABELS = '''import unittest

from parcelkit.labels import paginate, render_label


class TestLabels(unittest.TestCase):
    def test_pages_keep_order_and_are_never_empty(self):
        self.assertEqual(paginate([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]])
        self.assertEqual(paginate([], 3), [])
        with self.assertRaises(ValueError):
            paginate([1], 0)

    def test_a_label_line(self):
        self.assertEqual(render_label({"ref": "A1", "zone": 2}), "A1           zone 2")
'''
T_LEDGER = '''import unittest

from parcelkit.ledger import total_cents


class TestLedger(unittest.TestCase):
    def test_sum_and_refusal_of_floats(self):
        self.assertEqual(total_cents([450, 485]), 935)
        self.assertEqual(total_cents([]), 0)
        with self.assertRaises(TypeError):
            total_cents([1.5])
'''
EMPTY = ""


def readme(status):
    return ("# parcelkit\n\nShipping manifests for a small warehouse. Eight tickets:\n\n"
            "1. weights (parcelkit/weights.py)\n2. zones (parcelkit/zones.py)\n3. rates (parcelkit/rates.py)\n"
            "4. parcels (parcelkit/parcels.py)\n5. labels (parcelkit/labels.py)\n6. ledger (parcelkit/ledger.py)\n"
            "7. manifest (parcelkit/manifest.py)\n8. command line (parcelkit/cli.py)\n\n"
            "Run the tests with `python3 -m unittest discover -s tests -t .`.\n\n" + status)


def _values(f):
    """(TARE_G, PAGE_ROWS, stale PAGE_ROWS, TAG, stale TAG) of this fixture's facts."""
    return f["TARE_G"]["value"], f["PAGE_ROWS"]["value"], f["PAGE_ROWS"]["stale"], f["TAG"]["value"], f["TAG"]["stale"]


def _adder(c):
    def add(author, message, files, board=(), decisions=()):
        c.append({"author": author, "message": message, "files": files, "board": list(board), "decisions": list(decisions)})
    return add


def _commits_tickets_1_to_5(add, f):
    tare, pn, ps, tn, ts = _values(f)
    add("ada", "scaffold: package parcelkit and the test layout\n\nThe job is eight tickets, listed in README.md with their "
        "modules. ada takes 1, 2 and 4, bo takes 3 and 6, cy takes 5. Ticket 7 (manifest) and 8 (cli) are not assigned yet.",
        {"parcelkit/__init__.py": INIT, "tests/__init__.py": EMPTY, "README.md": readme("Status: nothing is done yet.\n")},
        board=["- ada | working | parcelkit/weights.py | ticket 1 weights"])
    add("ada", "weights: to_grams and block_count (ticket 1)\n\nEvery weight inside parcelkit is a whole number of grams.",
        {"parcelkit/weights.py": WEIGHTS})
    add("ada", "weights: tests for to_grams and block_count", {"tests/test_weights.py": T_WEIGHTS},
        board=["- ada | done | parcelkit/weights.py | ticket 1 weights, tests pass"])
    add("ada", "zones: zone_for from the first digit of the postcode (ticket 2, first cut)\n\nThree zones for now.",
        {"parcelkit/zones.py": ZONES_V1}, board=["- ada | working | parcelkit/zones.py | ticket 2 zones"])
    add("ada", "zones: add zone 4 for postcodes starting with 9\n\nThe first cut had no remote zone; the rate table (ticket 3) "
        "has four zones, so 9xxxx is zone 4.", {"parcelkit/zones.py": ZONES_V2, "tests/test_zones.py": T_ZONES},
        board=["- ada | done | parcelkit/zones.py | ticket 2 zones, four zones"])
    add("bo", "rates: rate_cents returns the zone's base price (ticket 3, first cut)",
        {"parcelkit/rates.py": RATES_V1}, board=["- bo | working | parcelkit/rates.py | ticket 3 rates"])
    add("bo", "rates: price every started 100 g block; decide how parcels are weighed (ticket 3)\n\n"
        "DECISION D1 (bo, agreed with ada): the scanner reports GROSS weight. Before a parcel is priced the manifest "
        "subtracts a tare of TARE_G = %d g from the scanner weight, never going below 0 g. rate_cents takes the NET weight "
        "and does not know the tare, so the caller must subtract it. Pitfall: pricing the gross weight overcharges every "
        "parcel." % tare,
        {"parcelkit/rates.py": RATES_V2},
        decisions=["- D1 | bo | parcels are weighed gross by the scanner; the manifest subtracts TARE_G = %d g from each weight "
                   "(never below 0 g) before calling rate_cents, which takes the net weight | 2026-11-20" % tare])
    add("bo", "rates: tests for base, blocks and refusal", {"tests/test_rates.py": T_RATES},
        board=["- bo | done | parcelkit/rates.py | ticket 3 rates, tests pass"])
    add("cy", "labels: paginate (ticket 5, first cut)\n\nDECISION D2 (cy, agreed with bo): a manifest page holds exactly "
        "PAGE_ROWS = %d rows, because the label sheet layout assumes it. paginate takes the size as an argument; the "
        "manifest (ticket 7) passes PAGE_ROWS." % ps,
        {"parcelkit/labels.py": LABELS_V1},
        board=["- cy | working | parcelkit/labels.py | ticket 5 labels"],
        decisions=["- D2 | cy | a manifest page holds exactly PAGE_ROWS = %d rows (the label sheet layout assumes it) | 2026-11-20" % ps])
    add("cy", "labels: render_label (ticket 5)", {"parcelkit/labels.py": LABELS_V2})
    add("ada", "parcels: parse_line reads one scanner line (ticket 4)", {"parcelkit/parcels.py": PARCELS_V1},
        board=["- ada | working | parcelkit/parcels.py | ticket 4 parcels"])
    add("ada", "parcels: tests for parse_line", {"tests/test_parcels.py": T_PARCELS.replace(
        '        self.assertEqual(parse_line("A1 , 500 , 12345 "), {"ref": "A1", "weight_g": 500, "postcode": "12345"})\n', "")})
    add("cy", "labels: tests for paginate and render_label", {"tests/test_labels.py": T_LABELS})


def _commits_tickets_6_and_decisions(add, f):
    tare, pn, ps, tn, ts = _values(f)
    add("bo", "ledger: total_cents refuses floats (ticket 6)", {"parcelkit/ledger.py": LEDGER},
        board=["- bo | working | parcelkit/ledger.py | ticket 6 ledger"])
    add("bo", "ledger: tests for total_cents", {"tests/test_ledger.py": T_LEDGER},
        board=["- bo | done | parcelkit/ledger.py | ticket 6 ledger, tests pass"])
    add("bo", "naming: the manifest file name rule (for ticket 7)\n\nDECISION D3 (bo, agreed with ada): a manifest file is "
        "named TAG-YYYYMMDD-SEQ.mf with TAG = %s, the date as 8 digits, SEQ the run number zero padded to 3 digits and the "
        "extension .mf. Example: %s-20261120-007.mf." % (ts, ts),
        {},
        decisions=["- D3 | bo | a manifest file is named TAG-YYYYMMDD-SEQ.mf with TAG = %s, the date as 8 digits, SEQ zero padded "
                   "to 3 digits | 2026-11-20" % ts])
    add("ada", "docs: README status, tickets 1 to 6 are being finished", {"README.md": readme(
        "Status: tickets 1 to 6 are being finished.\n")})
    add("cy", "labels: the layout test on the real printer says %d rows overflow the sheet; PAGE_ROWS is now %d\n\n"
        "DECISION D4 (cy, agreed with bo and ada): a manifest page holds exactly PAGE_ROWS = %d rows. This supersedes D2 "
        "(%d rows). The code of paginate is unchanged: only the size the manifest passes changes." % (ps, pn, pn, ps),
        {"tests/test_labels.py": T_LABELS},
        decisions=["- D4 | cy | a manifest page holds exactly PAGE_ROWS = %d rows; supersedes D2 (%d rows overflowed the sheet "
                   "on the printer) | 2026-11-20" % (pn, ps)])
    add("bo", "rates: refuse a negative net weight, zero is base price only\n\nThis is why the manifest must floor the "
        "net weight at 0 g after subtracting the tare (decision D1).", {"parcelkit/rates.py": RATES_V3})
    add("ada", "zones: refuse a postcode that is not five digits", {"parcelkit/zones.py": ZONES_V3})
    add("bo", "naming: the tag changes to %s\n\nDECISION D5 (bo, agreed with ada and cy): the manifest file name tag is "
        "TAG = %s, not %s: the archive tool already uses %s as its prefix and would pick manifests up. This supersedes D3. "
        "The rest of the rule is unchanged (TAG-YYYYMMDD-SEQ.mf, date as 8 digits, SEQ zero padded to 3 digits)."
        % (tn, tn, ts, ts), {},
        decisions=["- D5 | bo | the manifest file name tag is TAG = %s; supersedes D3 (%s collides with the archive tool) | "
                   "2026-11-20" % (tn, ts)])
    add("cy", "labels: render_label pads the reference to 12 characters", {"parcelkit/labels.py": LABELS_V3,
                                                                          "tests/test_labels.py": T_LABELS})
    add("ada", "parcels: parse_line tolerates spaces around each field", {"parcelkit/parcels.py": PARCELS_V2,
                                                                        "tests/test_parcels.py": T_PARCELS})
    add("bo", "handoff: ticket 7 (manifest) is open and not started\n\nTickets 1 to 6 are merged. The manifest is built from "
        "parse_line, zone_for, rate_cents, paginate and total_cents. The decisions it must follow (the tare, the page size "
        "and the file name) are in the commit messages above; later decisions supersede earlier ones.",
        {}, board=["- bo | handoff | parcelkit/manifest.py | ticket 7 manifest is open and not started; the decisions on tare, "
                   "page size and file name are in DECISIONS.md, later ones supersede earlier ones"])
    add("cy", "docs: README status, tickets 1 to 6 done", {"README.md": readme(
        "Status: tickets 1 to 6 are done. Ticket 7 (manifest) is open and needs the team's earlier decisions. "
        "Ticket 8 (cli) waits for ticket 7.\n")},
        board=["- cy | done | parcelkit/labels.py | ticket 5 labels, tests pass"])


def commits(f):
    """The 25 scripted commits. Each: author, message, files {path: text}, board [lines], decisions [lines]. Dates are one
    hour apart from DAY0, set by the builder."""
    c = []
    add = _adder(c)
    _commits_tickets_1_to_5(add, f)
    _commits_tickets_6_and_decisions(add, f)
    assert len(c) == 25, len(c)
    return c



class _Room:
    """The scripted room history being built: one method per klatch command a member runs."""

    def __init__(self):
        self.s = []

    def step(self, who, *argv, key=None):
        d = {"as": who, "argv": list(argv)}
        if key:
            d["key"] = key
        self.s.append(d)

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


def _room_tickets_1_to_6(r, f):
    tare, pn, ps, tn, ts = _values(f)
    for who in ("ada", "bo", "cy"):
        r.step(who, "join", "lobby")
    r.post("ada", "working: ticket 1 weights, parcelkit/weights.py")
    r.claim("ada", "parcelkit/weights.py", "ticket 1")
    r.post("ada", "checkpoint: weights.to_grams done, block_count next")
    r.post("ada", "finished: ticket 1 weights, tests pass")
    r.post("ada", "merged: ticket 1 on main")
    r.release("ada", "parcelkit/weights.py", "weights done")
    r.post("ada", "working: ticket 2 zones, parcelkit/zones.py")
    r.claim("ada", "parcelkit/zones.py", "ticket 2")
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


def _room_decisions_and_wrap_up(r, f):
    tare, pn, ps, tn, ts = _values(f)
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
    for who in ("ada", "bo", "cy"):
        r.post(who, "status: done")
        r.step(who, "status", "lobby", "done")


def room(f):
    """The scripted room history: steps {as, argv, key?}. argv is the klatch command line after the binary; {{ask:KEY}} in
    a text is the number the ask got. About 60 messages by ada, bo and cy, in the order of the commits."""
    r = _Room()
    _room_tickets_1_to_6(r, f)
    _room_decisions_and_wrap_up(r, f)
    return r.s



# --- the reference solution and the failing variants -------------------------------------------------------------------

MANIFEST = '''"""Shipping manifests (ticket 7)."""
from parcelkit.labels import paginate
from parcelkit.ledger import total_cents
from parcelkit.parcels import parse_line
from parcelkit.rates import rate_cents
from parcelkit.zones import zone_for

TAG = @@TAG@@
PAGE_ROWS = @@PAGE@@
TARE_G = @@TARE@@


def manifest_filename(date, seq):
    """TAG-YYYYMMDD-SEQ.mf, SEQ zero padded to 3 digits."""
    return "%s-%s-%03d.mf" % (TAG, date.strftime("%Y%m%d"), seq)


def build_manifest(lines, date, seq):
    """The manifest of a list of scanner lines: its file name, the rows in pages, the count and the total in cents."""
    rows = []
    for line in lines:
        p = parse_line(line)
        zone = zone_for(p["postcode"])
        net = max(0, p["weight_g"] - TARE_G)
        rows.append({"ref": p["ref"], "weight_g": p["weight_g"], "zone": zone, "cents": rate_cents(net, zone)})
    return {"filename": manifest_filename(date, seq), "pages": paginate(rows, PAGE_ROWS), "count": len(rows),
            "total_cents": total_cents([r["cents"] for r in rows])}
'''

MANIFEST_TEST = '''import datetime
import unittest

from parcelkit.manifest import build_manifest, manifest_filename


class TestManifest(unittest.TestCase):
    def test_the_file_name_has_the_agreed_shape(self):
        self.assertRegex(manifest_filename(datetime.date(2026, 11, 20), 7), r"^[a-z0-9]+-20261120-007\\.mf$")

    def test_rows_are_in_order_and_counted(self):
        m = build_manifest(["A1,900,12345", "B2,1500,90210"], datetime.date(2026, 11, 20), 1)
        self.assertEqual(m["count"], 2)
        self.assertEqual([r["ref"] for p in m["pages"] for r in p], ["A1", "B2"])
        self.assertEqual(m["total_cents"], sum(r["cents"] for p in m["pages"] for r in p))
'''

DUP_MANIFEST_EXTRA = '''

def paginate(items, size):
    """Pages of at most size items (my own copy)."""
    return [list(items[i:i + size]) for i in range(0, len(items), size)]
'''


def manifest(tag, page, tare):
    return MANIFEST.replace("@@TAG@@", repr(tag)).replace("@@PAGE@@", str(page)).replace("@@TARE@@", str(tare))


def variants(f):
    v = {}
    t, p, r = f["TAG"], f["PAGE_ROWS"], f["TARE_G"]
    v["wrong_value"] = {"parcelkit/manifest.py": manifest("mf", 50, 0)}
    v["stale_decision"] = {"parcelkit/manifest.py": manifest(t["stale"], p["stale"], r["value"])}
    dup = manifest(t["value"], p["value"], r["value"]).replace("from parcelkit.labels import paginate\n", "")
    dup = dup.replace("from parcelkit.zones import zone_for\n", "").replace("\n\ndef manifest_filename", DUP_MANIFEST_EXTRA +
                                                                             '''

def zone_for(postcode):
    """My own zone rule."""
    first = int(postcode[0])
    return 1 if first <= 2 else 2 if first <= 5 else 3 if first <= 8 else 4


def manifest_filename''', 1)
    v["duplicated_work"] = {"parcelkit/manifest.py": dup}
    v["rewrote_finished"] = {"parcelkit/manifest.py": manifest(t["value"], p["value"], r["value"]),
                             "parcelkit/ledger.py": LEDGER.replace("total = 0", "total = 0  # rewritten")}
    return v


def module_hashes():
    mods = {"parcelkit/weights.py": WEIGHTS, "parcelkit/zones.py": ZONES_V3, "parcelkit/rates.py": RATES_V3,
            "parcelkit/parcels.py": PARCELS_V2, "parcelkit/labels.py": LABELS_V3, "parcelkit/ledger.py": LEDGER}
    return {k: hashlib.sha256(x.encode()).hexdigest() for k, x in sorted(mods.items())}


def generate():
    """{path relative to experiments/coordbench: text}"""
    f = facts()
    out = {}
    hist = {"comment": "Generated by fixtures/history/gen_facts.py (#1374). The scripted history of F9: 25 commits by ada, bo and "
            "cy, one hour apart from %s. board and decisions are applied only in arm B1 (TASKS.md, DECISIONS.md); the "
            "commit messages are the same in every arm." % DAY0,
            "start": DAY0, "board_header": BOARD_HEADER, "decisions_header": DECISIONS_HEADER, "commits": commits(f)}
    out["fixtures/history/history.json"] = json.dumps(hist, indent=1, sort_keys=True) + "\n"
    rm = {"comment": "Generated by fixtures/history/gen_facts.py (#1374). The prebuilt room history (K arms): posted into a "
          "temporary store by ada, bo and cy through the klatch binary before the late joiner starts. {{ask:KEY}} is the "
          "number the ask called KEY got.", "steps": room(f)}
    out["fixtures/history/room.json"] = json.dumps(rm, indent=1, sort_keys=True) + "\n"
    ref = {"parcelkit/manifest.py": manifest(f["TAG"]["value"], f["PAGE_ROWS"]["value"], f["TARE_G"]["value"]),
           "tests/test_manifest.py": MANIFEST_TEST}
    for path, text in ref.items():
        out["fixtures/history/reference/" + path] = text
    for name, files in variants(f).items():
        for path, text in files.items():
            out["fixtures/history/variants/%s/%s" % (name, path)] = text
    hidden = {"comment": "Hidden checker data for F9 (#1374). Never in a checkout. Generated by fixtures/history/gen_facts.py.",
              "seed": SEED, "facts": f, "finished_modules": module_hashes()}
    out["checks/history/facts.json"] = json.dumps(hidden, indent=2, sort_keys=True) + "\n"
    return out


def main(argv):
    files = generate()
    bad = 0
    for rel, text in sorted(files.items()):
        path = os.path.join(BENCH, rel)
        if "--check" in argv:
            try:
                with open(path) as fh:
                    same = fh.read() == text
            except OSError:
                same = False
            if not same:
                print("differs: " + rel)
                bad += 1
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as fh:
                fh.write(text)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
