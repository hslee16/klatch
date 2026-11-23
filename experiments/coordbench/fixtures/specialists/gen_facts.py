#!/usr/bin/env python3
"""Generate the seeded facts of fixture F4 `specialists` (#1369) and every file that carries them.

    gen_facts.py            write the files (run once; the output is committed)
    gen_facts.py --check    exit 1 if a committed file differs from what the seed generates

The facts are drawn from a fixed seed with a fixed order of draws, so every run of the bench sees the same facts and the
output is reproducible byte for byte. Each fact is arbitrary: it is never inferable from a name, and it is drawn so that it
is not one of the conventional defaults an agent would guess (the `plausible` list, kept in the hidden checker's
facts.json so the checker can say which default a wrong guess was). Each fact also has a `stale` value, the one an earlier
revision of the private spec had; the spec file lists it as superseded, so a holder who answers from the wrong line gives a
stale value.

Files written (paths relative to experiments/coordbench):
  fixtures/specialists/private/<agent>/SPEC-*.md   the private knowledge file of each agent (never in origin, never in
                                                   another agent's clone)
  fixtures/specialists/reference/linkkit/*.py      the reference solution (never in a checkout)
  fixtures/specialists/seed.json                   the seeded earlier exchange (room history for K arms, board line for B1)
  checks/specialists/facts.json                    the hidden checker's facts (values, stale values, plausible guesses)
"""
import json
import os
import random
import sys

SEED = 20261123
HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(os.path.dirname(HERE))

# fact key -> (holder, kind, plausible defaults). The order is the order of the draws: do not reorder.
FACTS = [
    ("FRAME_MAGIC", "a", "hex16", [0xCAFE, 0xBEEF, 0xDEAD, 0xFACE, 0x7E7E, 0xABCD, 0x1234, 0xAA55, 0x55AA, 0xFFFF, 0xF00D]),
    ("MAX_PAYLOAD", "a", "size", [1024, 1400, 1500, 2048, 4096, 512, 1000, 2000, 3000, 256, 65535]),
    ("TOPIC_PREFIX", "b", "word", ["topic", "msg", "evt", "event", "kl", "stream", "linkkit", "lk", "t"]),
    ("PARTITIONS", "b", "count", [8, 10, 12, 16, 24, 32, 64, 50, 100, 4, 128]),
    ("CHECKSUM_SEED", "c", "seed", [5381, 0, 1, 31, 33, 17, 65599, 1000, 12345, 65521, 7, 42]),
    ("LEDGER_MODULUS", "c", "prime", [65521, 65536, 65537, 1000003, 997, 255, 10007, 1000000007, 4096, 1000]),
]
DISTRACTORS = {"a": [("FRAME_VERSION", "count", 3), ("HEARTBEAT_SECONDS", "count", 0)],
               "b": [("ROUTE_TTL_SECONDS", "count", 0), ("TOPIC_SEPARATOR_NOTE", "word", 0)],
               "c": [("LEDGER_BLOCK_SIZE", "size", 0), ("AUDIT_INTERVAL_MINUTES", "count", 0)]}
SPECS = {"a": ("SPEC-FRAMING.md", "Framing specification", "linkkit/framing.py"),
         "b": ("SPEC-ROUTING.md", "Routing specification", "linkkit/routing.py"),
         "c": ("SPEC-LEDGER.md", "Ledger specification", "linkkit/ledger.py")}
LETTERS = "bcdfghjkmnpqrstvwxz"


def is_prime(n):
    if n < 2:
        return False
    i = 2
    while i * i <= n:
        if n % i == 0:
            return False
        i += 1
    return True


def draw(rng, kind, avoid):
    while True:
        if kind == "hex16":
            v = rng.randrange(0x1000, 0xF000)
        elif kind == "size":
            v = rng.randrange(600, 4000)
        elif kind == "word":
            v = "".join(rng.choice(LETTERS) for _ in range(3)) + str(rng.randrange(2, 10))
        elif kind == "count":
            v = rng.randrange(11, 98)
        elif kind == "seed":
            v = rng.randrange(1000, 1000000)
        elif kind == "prime":
            v = rng.randrange(20011, 99991)
            if not is_prime(v):
                continue
        else:
            raise ValueError(kind)
        if v not in avoid:
            return v


def show(kind, v):
    return "0x%04x" % v if kind == "hex16" else str(v)


def generate():
    """{path relative to experiments/coordbench: text}"""
    rng = random.Random(SEED)
    facts = {}
    for key, holder, kind, plausible in FACTS:
        value = draw(rng, kind, plausible)
        stale = draw(rng, kind, plausible + [value])
        facts[key] = {"holder": holder, "kind": kind, "value": value, "stale": stale, "plausible": plausible}
    extras = {}
    for holder, items in DISTRACTORS.items():
        for key, kind, _ in items:
            extras[key] = draw(rng, "count" if kind == "count" else kind if kind != "word" else "word", [])
    out = {}
    notes = {
        "FRAME_MAGIC": "the 16-bit marker that starts every frame",
        "MAX_PAYLOAD": "the largest payload in one frame, in bytes",
        "TOPIC_PREFIX": "the prefix of every stream topic name",
        "PARTITIONS": "the number of shards keys are spread over",
        "CHECKSUM_SEED": "the starting value of the payload digest",
        "LEDGER_MODULUS": "the modulus of every ledger slot number",
    }
    for holder, (fname, title, module) in SPECS.items():
        own = [k for k in facts if facts[k]["holder"] == holder]
        lines = ["# %s (internal, revision 3)" % title, "",
                 "Held by agent %s only. This file is not in the repository and no colleague's checkout has it; do not "
                 "commit it. A colleague who needs a value from it asks you, and you may answer from it." % holder, "",
                 "## Current constants (revision 3)", ""]
        for k in own:
            f = facts[k]
            lines.append("- %s = %s (%s)" % (k, show(f["kind"], f["value"]), notes[k]))
        for k, kind, _ in DISTRACTORS[holder]:
            lines.append("- %s = %s (not used by any module of this repository)" % (k, extras[k] if kind != "word" else extras[k]))
        lines += ["", "## Superseded in revision 3 (do not use)", ""]
        for k in own:
            f = facts[k]
            lines.append("- %s was %s in revision 2" % (k, show(f["kind"], f["stale"])))
        lines += ["", "The module you own, %s, is yours to write; the constants above are used by other modules." % module, ""]
        out["fixtures/specialists/private/%s/%s" % (holder, fname)] = "\n".join(lines)
    v = {k: facts[k]["value"] for k in facts}
    ref = {
        "linkkit/__init__.py": '"""linkkit: framing, routing and ledger helpers (reference solution)."""\n',
        "linkkit/framing.py": '''"""Framing helpers."""

TOPIC_PREFIX = %(TOPIC_PREFIX)r
CHECKSUM_SEED = %(CHECKSUM_SEED)d


def topic_name(stream_id):
    return "%%s.%%04d" %% (TOPIC_PREFIX, stream_id)


def payload_digest(data):
    acc = CHECKSUM_SEED
    for byte in data:
        acc = (acc * 33 + byte) & 0xFFFFFFFF
    return acc
''' % v,
        "linkkit/retry.py": '''"""Retry slots."""

LEDGER_MODULUS = %(LEDGER_MODULUS)d


def retry_slot(attempt):
    return (attempt * 7919 + 1) %% LEDGER_MODULUS
''' % v,
        "linkkit/routing.py": '''"""Routing helpers."""

LEDGER_MODULUS = %(LEDGER_MODULUS)d
FRAME_MAGIC = %(FRAME_MAGIC)d


def ledger_slot(n):
    return (n * n + 13) %% LEDGER_MODULUS


def frame_tag(kind):
    return (FRAME_MAGIC << 4) | (kind & 15)
''' % v,
        "linkkit/ledger.py": '''"""Ledger helpers."""
import zlib

MAX_PAYLOAD = %(MAX_PAYLOAD)d
PARTITIONS = %(PARTITIONS)d


def chunk_count(total_bytes):
    if total_bytes <= 0:
        return 0
    return -(-total_bytes // MAX_PAYLOAD)


def shard_for(key):
    return zlib.crc32(key.encode()) %% PARTITIONS
''' % v,
    }
    for path, text in ref.items():
        out["fixtures/specialists/reference/" + path] = text
    mp = facts["MAX_PAYLOAD"]["value"]
    seed = {
        "comment": "The seeded earlier exchange (SPEC S4): MAX_PAYLOAD was asked and answered before the run started, so "
                   "agent c asking for it again is a repeat. K arms: posted into the room by two seed members before any "
                   "agent starts. B1: one line appended to TASKS.md.",
        "fact": "MAX_PAYLOAD",
        "room": [
            {"as": "seed-p", "text": "seeking: what is MAX_PAYLOAD in the framing spec, in bytes?"},
            {"as": "seed-q", "text": "answer to #1: MAX_PAYLOAD is %d bytes (framing spec revision 3, held by a)" % mp},
            {"as": "seed-p", "text": "closed #1"},
        ],
        "board": ["- seed-q | done | - | answered seed-p: MAX_PAYLOAD is %d bytes (framing spec revision 3, held by a)" % mp],
    }
    out["fixtures/specialists/seed.json"] = json.dumps(seed, indent=2, sort_keys=True) + "\n"
    hidden = {"comment": "Hidden checker data for F4 (#1369). Never in a checkout. Generated by fixtures/specialists/gen_facts.py.",
              "seed": SEED, "facts": facts}
    out["checks/specialists/facts.json"] = json.dumps(hidden, indent=2, sort_keys=True) + "\n"
    return out


def main(argv):
    files = generate()
    bad = 0
    for rel, text in sorted(files.items()):
        path = os.path.join(BENCH, rel)
        if "--check" in argv:
            try:
                with open(path) as f:
                    same = f.read() == text
            except OSError:
                same = False
            if not same:
                print("differs: " + rel)
                bad += 1
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(text)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
