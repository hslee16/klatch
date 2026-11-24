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
if HERE not in sys.path:  # the generator is also loaded from its path by the tests
    sys.path.insert(0, HERE)
from gen_code import (  # noqa: E402
    DUP_MANIFEST_EXTRA, EMPTY, INIT, LABELS_V1, LABELS_V2, LABELS_V3, LEDGER, MANIFEST_TEST, PARCELS_V1, PARCELS_V2,
    RATES_V1, RATES_V2, RATES_V3, T_LABELS, T_LEDGER, T_PARCELS, T_RATES, T_WEIGHTS, T_ZONES, WEIGHTS, ZONES_V1,
    ZONES_V2, ZONES_V3, manifest, readme)
from gen_room import room  # noqa: E402

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


_DRAWS = {
    "tag": lambda rng: rng.choice(LETTERS) + rng.choice(LETTERS) + str(rng.randrange(2, 10)) + rng.choice(LETTERS),
    "rows": lambda rng: rng.randrange(31, 98),
    "tare": lambda rng: rng.randrange(121, 480),
}


def draw(rng, kind, avoid):
    if kind not in _DRAWS:
        raise ValueError(kind)
    while True:
        v = _DRAWS[kind](rng)
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


def _values(f):
    """The seeded values the record talks about: the tare, the page size (now and before), the tag (now and before)."""
    return {"tare": f["TARE_G"]["value"], "pn": f["PAGE_ROWS"]["value"], "ps": f["PAGE_ROWS"]["stale"],
            "tn": f["TAG"]["value"], "ts": f["TAG"]["stale"]}



def _commits_first(add, v):
    """The scaffold, tickets 1 and 2, and the start of ticket 3."""
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


def _commits_middle(add, v):
    """Tickets 3 to 6 and the first versions of the decisions."""
    tare, ps = v["tare"], v["ps"]
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
    add("bo", "ledger: total_cents refuses floats (ticket 6)", {"parcelkit/ledger.py": LEDGER},
        board=["- bo | working | parcelkit/ledger.py | ticket 6 ledger"])
    add("bo", "ledger: tests for total_cents", {"tests/test_ledger.py": T_LEDGER},
        board=["- bo | done | parcelkit/ledger.py | ticket 6 ledger, tests pass"])


def _commits_late(add, v):
    """The later decisions, the late fixes and the handoff."""
    tare, pn, ps, tn, ts = v["tare"], v["pn"], v["ps"], v["tn"], v["ts"]
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

    def add(author, message, files, board=(), decisions=()):
        c.append({"author": author, "message": message, "files": files, "board": list(board), "decisions": list(decisions)})

    v = _values(f)
    for part in (_commits_first, _commits_middle, _commits_late):
        part(add, v)
    assert len(c) == 25, len(c)
    return c


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
