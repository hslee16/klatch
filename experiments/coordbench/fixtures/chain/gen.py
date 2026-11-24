#!/usr/bin/env python3
"""Generate the seeded facts of fixture F3 `chain` (#1368). Standard library only; deterministic from SEED.

    gen.py            write every generated file
    gen.py --check    exit 1 when a committed file differs from what the seed makes (a test runs this)

Generated: template/data/inventory.dat (the fixed-width records; the ruler is not in the repository, only in task-a.md), task-a.md (from task-a.md.in,
with the quirk filled in), fake.json (the scripted agents' steps), reference/chainkit/schema.json, bins.py and
bins_partial.py (the solution and the state a is told to leave), and checks/chain/expected.json (the hidden checker's
facts). The quirk is the arbitrary part: which prefix is legacy, which prefix it moves to and by how much; nothing in the
repository an agent can see states it, and it cannot be guessed.
"""
import json
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(os.path.dirname(HERE))
SEED = 1368
FIELDS = ["sku", "qty", "bin", "note"]
WORDS = ["crate", "pallet", "fragile", "returns", "overstock", "damaged", "recount", "inbound", "outbound", "seasonal"]
SEPS = ["-", "/", ""]
N_RECORDS = 48


def build():
    rng = random.Random(SEED)
    order = FIELDS[:]
    rng.shuffle(order)
    width = {"sku": rng.randint(6, 8), "qty": rng.randint(4, 5), "bin": rng.randint(8, 10), "note": rng.randint(10, 14)}
    letters = "ABCDEFGHJKLMNPQRSTUVWXYZ"
    codes = []
    while len(codes) < 8:
        c = rng.choice(letters) + rng.choice(letters)
        if c not in codes:
            codes.append(c)
    normal, legacy, new = codes[:6], codes[6], codes[7]
    offset = rng.randint(101, 899)
    records = []
    while True:
        records = []
        for i in range(N_RECORDS):
            pfx = legacy if rng.random() < 0.2 else rng.choice(normal)
            num = rng.randint(1, 999)
            digits = str(num).zfill(rng.choice([1, 3, 4]))
            raw = pfx + rng.choice(SEPS) + digits
            note = rng.choice(WORDS)
            records.append({"sku": "S%05d" % (10000 + rng.randint(0, 8999)), "qty": rng.randint(1, 999), "bin": raw,
                            "note": note[:width["note"]].strip()})
        if 6 <= sum(1 for r in records if r["bin"].startswith(legacy)) <= 14:
            break

    def line(rec, order, width):
        parts = []
        for f in order:
            v = ("%0*d" % (width[f], rec[f])) if f == "qty" else str(rec[f]).ljust(width[f])
            parts.append(v)
        return "".join(parts)

    def schema(order, width):
        pos, fields = 0, []
        for f in order:
            fields.append({"name": f, "start": pos, "end": pos + width[f]})
            pos += width[f]
        return {"fields": fields, "record_length": pos}

    def canon(raw):
        pfx, num = raw[:2], int(raw[2:].lstrip("-/"))
        if pfx == legacy:
            return "%s-%05d" % (new, num + offset)
        return "%s-%05d" % (pfx, num)

    totals = {}
    for r in records:
        totals[canon(r["bin"])] = totals.get(canon(r["bin"]), 0) + r["qty"]
    alt_order = list(reversed(order))
    alt_width = {k: v + 2 for k, v in width.items()}
    alt_records = records[:6]
    parse_cases, canon_cases, quirk_cases = [], [], []
    for pfx in normal[:4]:
        for sep in SEPS:
            n = rng.randint(1, 999)
            raw = "%s%s%d" % (pfx, sep, n)
            parse_cases.append([raw, pfx, n])
            canon_cases.append([raw, "%s-%05d" % (pfx, n)])
    for sep in SEPS:
        n = rng.randint(1, 500)
        raw = "%s%s%d" % (legacy, sep, n)
        parse_cases.append([raw, legacy, n])
        quirk_cases.append([raw, "%s-%05d" % (new, n + offset)])
    quirk_cases.append(["%s-%04d" % (legacy, 7), "%s-%05d" % (new, 7 + offset)])
    return {
        "seed": SEED, "order": order, "width": width, "ruler": ",".join("%s=%d" % (f, width[f]) for f in order),
        "lines": [line(r, order, width) for r in records], "records": records, "schema": schema(order, width),
        "alt_schema": schema(alt_order, alt_width), "alt_lines": [line(r, alt_order, alt_width) for r in alt_records],
        "alt_records": alt_records, "totals": totals,
        "quirk": {"legacy": legacy, "new": new, "offset": offset},
        "parse_cases": parse_cases, "canon_cases": canon_cases, "quirk_cases": quirk_cases,
    }


BINS_HEAD = '''"""Bin codes for the inventory (step 2)."""
import collections
import re

Bin = collections.namedtuple("Bin", ["pfx", "num"])
_BIN = re.compile(r"([A-Z]{2})[-/]?([0-9]+)")


def parse_bin(raw):
    """Split a raw bin code (two capital letters, an optional - or /, digits) into Bin(pfx, num); num is an int."""
    m = _BIN.fullmatch(raw.strip())
    if m is None:
        raise ValueError("not a bin code: %r" % (raw,))
    return Bin(m.group(1), int(m.group(2)))


'''
BINS_PARTIAL = BINS_HEAD + '''def canonical_bin(raw):
    """The canonical code of a bin."""
    b = parse_bin(raw)
    # TODO(a): unfinished, see the handoff
    return "%s-%05d" % (b.pfx, b.num)
'''
BINS_FINAL_TAIL = '''LEGACY = "%(legacy)s"
MOVED_TO = "%(new)s"
OFFSET = %(offset)d


def canonical_bin(raw):
    """The canonical code of a bin: PREFIX-NNNNN. A legacy bin moves to another prefix and number."""
    b = parse_bin(raw)
    if b.pfx == LEGACY:
        return "%%s-%%05d" %% (MOVED_TO, b.num + OFFSET)
    return "%%s-%%05d" %% (b.pfx, b.num)
'''


def fake(facts):
    q = facts["quirk"]
    note = ("# Handoff from a\nDone: chainkit/schema.json (step 1) and parse_bin in chainkit/bins.py.\n"
            "Not done: canonical_bin is the plain format only.\n"
            "Quirk (from the vendor, not in the repo): bins with prefix %s are legacy; their canonical code has prefix %s "
            "and the number plus %d, five digits.\n" % (q["legacy"], q["new"], q["offset"]))
    room = "{{ROOM}}"
    sync = lambda m: {"op": "sync", "message": m, "resolve": "union"}  # noqa: E731
    cp = lambda f, t: {"op": "copy_from", "from": "reference/chainkit/" + f, "path": "chainkit/" + t}  # noqa: E731
    wait = lambda p, c="": {"op": "wait_origin_has", "path": p, "contains": c, "timeout": 60}  # noqa: E731
    pull = {"op": "git", "args": ["pull", "--no-rebase", "--no-edit", "origin", "main"]}
    k = ["K1", "K2"]
    kl = lambda *args: {"op": "klatch", "args": ["post", room] + list(args), "arms": k}  # noqa: E731
    steps = {
        "a": [{"op": "klatch", "args": ["join", room], "arms": k}, kl("working: step 1 schema"),
              {"op": "klatch", "args": ["claim", room, "chainkit/bins.py", "--reason", "step 2 start"], "arms": k},
              cp("schema.json", "schema.json"), cp("bins_partial.py", "bins.py"),
              {"op": "write", "path": "HANDOFF.md", "content": note},
              sync("step 1 and the start of step 2"),
              {"op": "klatch", "args": ["release", room, "chainkit/bins.py"], "arms": k},
              kl("checkpoint: step 1 done, parse_bin done, canonical_bin unfinished; quirk in HANDOFF.md"),
              kl("handoff: c"), {"op": "final", "text": "handed over to c. DONE a"}],
        "b": [{"op": "klatch", "args": ["join", room], "arms": k}, wait("chainkit/schema.json"), pull,
              cp("loader.py", "loader.py"), sync("step 3 loader"), kl("finished: step 3 loader"),
              {"op": "final", "text": "loader published. DONE b"}],
        "c": [{"op": "klatch", "args": ["join", room], "arms": k}, wait("HANDOFF.md"), pull,
              {"op": "klatch", "args": ["claim", room, "chainkit/bins.py", "--reason", "finish step 2"], "arms": k},
              cp("bins.py", "bins.py"), sync("step 2 canonical_bin"),
              wait("chainkit/loader.py", "def load"), pull, cp("report.py", "report.py"), sync("step 4 report"),
              {"op": "klatch", "args": ["release", room, "chainkit/bins.py"], "arms": k},
              {"op": "final", "text": "report published. DONE c"}],
    }
    return {"comment": "Scripted agents for the harness tests (#1368): a does step 1 and the start of step 2 and hands over "
                       "with the quirk in HANDOFF.md (and, in the K arms, a checkpoint and a handoff message); b waits for "
                       "the schema, then does the loader; c waits for the handoff, finishes bins.py, waits for the loader "
                       "and does the report. Generated by gen.py; reference/ is never in a checkout.",
            "agents": steps}


def outputs(facts):
    task = open(os.path.join(HERE, "task-a.md.in")).read()
    q = facts["quirk"]
    for k, v in (("{{RULER}}", facts["ruler"]), ("{{LEGACY}}", q["legacy"]), ("{{NEW}}", q["new"]), ("{{OFFSET}}", str(q["offset"])),
                 ("{{EXAMPLE}}", "%s-0042 becomes %s-%05d" % (q["legacy"], q["new"], 42 + q["offset"]))):
        task = task.replace(k, v)
    expected = {k: facts[k] for k in ("seed", "ruler", "records", "schema", "alt_schema", "alt_lines", "alt_records", "totals",
                                      "quirk", "parse_cases", "canon_cases", "quirk_cases")}
    expected["lines"] = facts["lines"]
    jd = lambda o: json.dumps(o, indent=1, sort_keys=True) + "\n"  # noqa: E731
    return {
        "template/data/inventory.dat": "\n".join(facts["lines"]) + "\n",
        "task-a.md": task,
        "fake.json": jd(fake(facts)),
        "reference/chainkit/schema.json": jd(facts["schema"]),
        "reference/chainkit/bins.py": BINS_HEAD + BINS_FINAL_TAIL % q,
        "reference/chainkit/bins_partial.py": BINS_PARTIAL,
        os.path.join("..", "..", "checks", "chain", "expected.json"): jd(expected),
    }


def main(argv):
    outs = outputs(build())
    bad = []
    for rel, text in sorted(outs.items()):
        path = os.path.normpath(os.path.join(HERE, rel))
        if "--check" in argv:
            try:
                with open(path) as f:
                    same = f.read() == text
            except OSError:
                same = False
            if not same:
                bad.append(rel)
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(text)
    if bad:
        print("out of date: " + ", ".join(bad), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
