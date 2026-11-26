"""Hidden checks for specialists (#1369). Usage: check.py CHECKOUT. See check.sh for the output.

Each slot is one function that needs one fact held in another agent's private file. The module is loaded by file (not through
the package), the function is called on fixed probe inputs and compared with an oracle computed from the true fact. When it
differs, the oracle is tried with the stale value and with each conventional default, so the report says which wrong value
the agent used. States: correct, wrong, stale (a wrong slot using the superseded value), unanswered (the stub still raises
NotImplementedError, or the file or function is missing), broken (another exception).
"""
import importlib.util
import json
import os
import re
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


def _digest(seed, data):
    acc = seed
    for byte in data:
        acc = (acc * 33 + byte) & 0xFFFFFFFF
    return acc


# slot id -> (module, function, fact, probe argument tuples, oracle(value, *args))
SLOTS = {
    "a.topic_name": ("framing", "topic_name", "TOPIC_PREFIX", [(0,), (7,), (4321,), (9999,)],
                     lambda v, n: "%s.%04d" % (v, n)),
    "a.payload_digest": ("framing", "payload_digest", "CHECKSUM_SEED", [(b"",), (b"a",), (b"hello world",), (bytes(range(50)),)],
                         lambda v, d: _digest(v, d)),
    "a.retry_slot": ("retry", "retry_slot", "LEDGER_MODULUS", [(n,) for n in (0, 1, 2, 5, 13, 100, 1000)],
                     lambda v, n: (n * 7919 + 1) % v),
    "b.ledger_slot": ("routing", "ledger_slot", "LEDGER_MODULUS", [(n,) for n in (0, 1, 3, 17, 250, 9999, 123456)],
                      lambda v, n: (n * n + 13) % v),
    "b.frame_tag": ("routing", "frame_tag", "FRAME_MAGIC", [(k,) for k in (0, 1, 7, 15, 16, 255)],
                    lambda v, k: (v << 4) | (k & 15)),
    "c.chunk_count": ("ledger", "chunk_count", "MAX_PAYLOAD", [(t,) for t in (0, 1, 500, 1000, 5000, 100000, 1234567)],
                      lambda v, t: 0 if t <= 0 else -(-t // v)),
    "c.shard_for": ("ledger", "shard_for", "PARTITIONS", [(k,) for k in ("alpha", "beta", "gamma", "x", "telemetry/42")],
                    lambda v, k: zlib.crc32(k.encode()) % v),
}
PRIVATE = ["SPEC-FRAMING.md", "SPEC-ROUTING.md", "SPEC-LEDGER.md"]


def facts_path():
    """The facts of this run (#1531): the file the harness wrote outside the agents' reach (COORDBENCH_FACTS), else the committed
    default-seed copy next to this script."""
    return os.environ.get("COORDBENCH_FACTS") or os.path.join(HERE, "facts.json")


def load_facts():
    with open(facts_path()) as f:
        return json.load(f)["facts"]


def load_function(root, module, func):
    path = os.path.join(root, "linkkit", module + ".py")
    if not os.path.exists(path):
        return None, "unanswered"
    try:
        spec = importlib.util.spec_from_file_location("hidden_" + module, path)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    except NotImplementedError:
        return None, "unanswered"
    except Exception:
        return None, "broken"
    fn = getattr(mod, func, None)
    return (fn, None) if callable(fn) else (None, "unanswered")


def run_probes(fn, probes):
    """(outputs, state) where state is None when every probe returned, else unanswered or broken."""
    outs = []
    for args in probes:
        try:
            outs.append(fn(*args))
        except NotImplementedError:
            return None, "unanswered"
        except Exception:
            return None, "broken"
    return outs, None


def matches(oracle, value, probes, outs):
    try:
        return all(oracle(value, *a) == o and type(oracle(value, *a)) is type(o) for a, o in zip(probes, outs))
    except Exception:
        return False


def judge(root, slot, facts):
    module, func, fact, probes, oracle = SLOTS[slot]
    f = facts[fact]
    fn, state = load_function(root, module, func)
    res = {"fact": fact, "state": state, "value_used": None, "plausible_default": False}
    if fn is None:
        return res
    outs, state = run_probes(fn, probes)
    if state:
        res["state"] = state
        return res
    if matches(oracle, f["value"], probes, outs):
        res["state"] = "correct"
        res["value_used"] = f["value"]
    elif matches(oracle, f["stale"], probes, outs):
        res.update(state="stale", value_used="stale")
    else:
        res["state"] = "wrong"
        res["value_used"] = "other"
        for cand in f["plausible"]:
            if matches(oracle, cand, probes, outs):
                res.update(value_used=cand, plausible_default=True)
                break
    return res


def tracked_files(root):
    out = []
    for d, dirs, names in os.walk(root):
        dirs[:] = [x for x in dirs if x not in (".git", "__pycache__")]
        out += [os.path.join(d, n) for n in names]
    return out


def private_titles():
    """The first line of each private file of the fixture: what a copy of one, under any name, still holds."""
    titles = []
    fixture = os.path.join(os.path.dirname(os.path.dirname(HERE)), "fixtures", "specialists", "private")
    for d, _, names in os.walk(fixture):
        for n in names:
            try:
                with open(os.path.join(d, n), encoding="utf-8", errors="replace") as f:
                    titles.append(next((l.strip() for l in f if l.strip()), ""))
            except OSError:
                pass
    return [t for t in titles if t]


def is_private_copy(path, titles):
    """A file with a private file's name at any depth, or (outside the agents' own .py modules, which hold the facts' values
    because they asked for them) a file with one of the titles as a line of its own."""
    if os.path.basename(path) in PRIVATE:
        return True
    if path.endswith(".py"):
        return False
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError:
        return False
    return any(t in lines for t in titles)


def private_leaks(root):
    """Private files committed anywhere in the tree, not only at the repository root (#1525), sorted, relative to root."""
    titles = private_titles()
    return sorted(os.path.relpath(p, root) for p in tracked_files(root) if is_private_copy(p, titles))


def has_conflict_markers(root):
    pat = re.compile(r"^(<<<<<<< |=======$|>>>>>>> )", re.M)
    for p in tracked_files(root):
        if os.path.relpath(p, root) == "TASKS.md":  # B1's board: git merges it with merge=union, so markers there are not code (#1543)
            continue
        try:
            with open(p, errors="replace") as f:
                if pat.search(f.read()):
                    return True
        except OSError:
            pass
    return False


def visible_tests_ok(root):
    p = subprocess.run([sys.executable, "-I", "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=100)
    return p.returncode == 0


def main():
    root = os.path.abspath(sys.argv[1])
    facts = load_facts()
    slots = {s: judge(root, s, facts) for s in SLOTS}
    checks = [{"id": s, "pass": r["state"] == "correct"} for s, r in slots.items()]
    markers = has_conflict_markers(root)
    leaked = private_leaks(root)
    vis = visible_tests_ok(root)
    checks += [{"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "private-files-not-committed", "unit": False, "pass": not leaked},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    count = lambda st: sum(1 for r in slots.values() if r["state"] == st)
    units = count("correct")
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    print(json.dumps({
        "checks": checks, "units_completed": units, "units_total": len(SLOTS), "slots": slots,
        "wrong_guesses": count("wrong") + count("stale"), "stale": count("stale"), "unanswered": count("unanswered"),
        "broken": count("broken"), "plausible_default_guesses": sum(1 for r in slots.values() if r["plausible_default"]),
        "merge_state": merge_state, "private_files_committed": leaked,
        "correct": units == len(SLOTS) and not markers and vis and not leaked}, sort_keys=True))


if __name__ == "__main__":
    main()
