"""Hidden checks for history (#1374). Usage: check.py CHECKOUT, or check.py --tree DIR. See check.sh for the output.

The late joiner's ticket is parcelkit/manifest.py: manifest_filename(date, seq) and build_manifest(lines, date, seq). It is
only correct if it follows three decisions that are in the project's record and nowhere in its code: the file name tag
(TAG), the rows per page (PAGE_ROWS) and the tare (TARE_G). The tag and the page size were decided, then superseded; the
earlier values are the `stale` ones in facts.json.

Each fact is tested on its own against an independent oracle (the checks never call the agent's version of a finished module;
oracle_* below are copies of the finished code, and a test keeps them equal) and classified: ok, stale (the superseded
value: a contradicted decision), guess (a conventional default, or no tare at all), wrong (anything else) or missing (no
manifest module, or it does not import). Separate checks find work that duplicates what the history shows done (a new
function with the name or the behaviour of a finished helper, a finished module rewritten), conflict markers and the visible
tests. The seed is fixed so a check is repeatable.
"""
import datetime
import hashlib
import importlib
import inspect
import json
import os
import random
import re
import signal
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SEED = 20261124
FACTS_PATH = os.path.join(HERE, "facts.json")
MODULE_DIR = "parcelkit"
FACT_IDS = {"TAG": "fact-file-name", "PAGE_ROWS": "fact-page-size", "TARE_G": "fact-tare"}
UNIT_ORDER = ["ticket-7-works", "fact-file-name", "fact-page-size", "fact-tare"]


class Timeout(Exception):
    pass


def _alarm(signum, frame):
    raise Timeout()


def load_facts():
    with open(FACTS_PATH) as f:
        return json.load(f)


# --- oracles: copies of the finished code (tickets 1 to 6), never the agent's version -------------------------------------

BASE = {1: 450, 2: 620, 3: 810, 4: 1100}
BLOCK = {1: 35, 2: 48, 3: 62, 4: 90}


def oracle_zone(postcode):
    if len(postcode) != 5 or not postcode.isdigit():
        raise ValueError("not a postcode")
    first = int(postcode[0])
    if first <= 2:
        return 1
    if first <= 5:
        return 2
    if first <= 8:
        return 3
    return 4


def oracle_rate(net, zone):
    if net < 0:
        raise ValueError("negative")
    blocks = 0 if net <= 0 else -(-net // 100)
    return BASE[zone] + BLOCK[zone] * blocks


def oracle_parse(line):
    parts = [p.strip() for p in line.strip().split(",")]
    if len(parts) != 3 or not parts[0]:
        raise ValueError("bad line")
    return {"ref": parts[0], "weight_g": int(parts[1]), "postcode": parts[2]}


def oracle_rows(lines, tare):
    rows = []
    for line in lines:
        p = oracle_parse(line)
        z = oracle_zone(p["postcode"])
        rows.append({"ref": p["ref"], "weight_g": p["weight_g"], "zone": z, "cents": oracle_rate(max(0, p["weight_g"] - tare), z)})
    return rows


# --- test data (seeded) ------------------------------------------------------------------------------------------------

def make_lines(n, rng, light=()):
    lines = []
    for i in range(n):
        w = light[i] if i < len(light) else rng.randrange(5, 2600)
        lines.append("R%03d,%d,%d%04d" % (i, w, rng.randrange(0, 10), rng.randrange(0, 10000)))
    return lines


def tare_lines(tare):
    rng = random.Random(SEED)
    return make_lines(16, rng, light=(40, tare - 1, tare, tare + 1, tare + 250))


def page_lines(n):
    rng = random.Random(SEED + 1)
    return make_lines(n, rng)


NAME_CASES = [(datetime.date(2026, 3, 9), 7), (datetime.date(2027, 12, 31), 123), (datetime.date(2026, 1, 2), 1),
              (datetime.date(2030, 7, 15), 999)]


# --- loading the agent's code ------------------------------------------------------------------------------------------

def purge():
    for m in [m for m in sys.modules if m == MODULE_DIR or m.startswith(MODULE_DIR + ".")]:
        del sys.modules[m]


def load_manifest(root):
    """The agent's parcelkit.manifest module, or None when it is absent, does not import or lacks the two functions."""
    if not os.path.exists(os.path.join(root, MODULE_DIR, "manifest.py")):
        return None
    purge()
    sys.path.insert(0, root)
    try:
        signal.alarm(30)
        mod = importlib.import_module(MODULE_DIR + ".manifest")
        signal.alarm(0)
    except BaseException:
        signal.alarm(0)
        return None
    finally:
        sys.path.remove(root)
    if not callable(getattr(mod, "manifest_filename", None)) or not callable(getattr(mod, "build_manifest", None)):
        return None
    return mod


def guarded(fn, *args):
    """(ok, value): value is the result, or the exception, of a call that may not run longer than 20 seconds."""
    try:
        signal.alarm(20)
        v = fn(*args)
        signal.alarm(0)
        return True, v
    except BaseException as e:
        signal.alarm(0)
        return False, e


# --- the task and the three facts --------------------------------------------------------------------------------------

def task_ok(m):
    """The manifest works whatever the decided values are: structure, order, counts, zones, an empty input, a bad line."""
    d = datetime.date(2026, 11, 20)
    lines = ["A1,900,12345", "B2,1500,90210", "C3,50,55555", "D4,20,60606", "E5,3000,01010", "F6,700,31313", "G7,300,80808"]
    ok, r = guarded(m.build_manifest, lines, d, 3)
    if not ok or not isinstance(r, dict):
        return False
    if not all(k in r for k in ("filename", "pages", "count", "total_cents")):
        return False
    rows = [x for p in r["pages"] if isinstance(p, list) for x in p]
    want = oracle_rows(lines, 0)
    if r["count"] != 7 or len(rows) != 7 or any(not isinstance(p, list) or not p for p in r["pages"]):
        return False
    for got, exp in zip(rows, want):
        if not isinstance(got, dict) or any(got.get(k) != exp[k] for k in ("ref", "weight_g", "zone")) or "cents" not in got:
            return False
    if r["total_cents"] != sum(x["cents"] for x in rows):
        return False
    ok, name = guarded(m.manifest_filename, d, 3)
    if not ok or not isinstance(name, str) or r["filename"] != name:
        return False
    ok, e = guarded(m.build_manifest, [], d, 1)
    if not ok or e.get("pages") != [] or e.get("count") != 0 or e.get("total_cents") != 0:
        return False
    ok, err = guarded(m.build_manifest, ["A1,500"], d, 1)
    return (not ok) and isinstance(err, ValueError)


def classify_value(got, fact):
    if got == fact["value"]:
        return "ok"
    if fact.get("stale") is not None and got == fact["stale"]:
        return "stale"
    if got in fact["plausible"]:
        return "guess"
    return "wrong"


def fact_name(m, fact):
    tag = fact["value"]
    seen = None
    for d, seq in NAME_CASES:
        ok, name = guarded(m.manifest_filename, d, seq)
        if not ok or not isinstance(name, str):
            return "wrong"
        want = "%s-%s-%03d.mf" % (tag, d.strftime("%Y%m%d"), seq)
        if name == want:
            continue
        mt = re.match(r"^(.*)-(\d{8})-(\d{3})\.mf$", name)
        if not mt or mt.group(2) != d.strftime("%Y%m%d") or int(mt.group(3)) != seq:
            return "wrong"
        got = classify_value(mt.group(1), fact)
        seen = got if seen in (None, got) else "wrong"
    ok, built = guarded(m.build_manifest, ["A1,900,12345"], NAME_CASES[0][0], NAME_CASES[0][1])
    if ok and isinstance(built, dict) and built.get("filename") != m.manifest_filename(*NAME_CASES[0]):
        return "wrong"
    return seen or "ok"


def fact_pages(m, fact):
    p = fact["value"]
    ok, r = guarded(m.build_manifest, page_lines(3 * p + 5), NAME_CASES[0][0], 1)
    if not ok or not isinstance(r, dict) or not isinstance(r.get("pages"), list) or not r["pages"]:
        return "wrong"
    sizes = [len(x) for x in r["pages"]]
    if sizes == [p, p, p, 5]:
        return "ok"
    first = sizes[0]
    if not all(s == first for s in sizes[:-1]) or sizes[-1] > first or sum(sizes) != 3 * p + 5:
        return "wrong"
    return classify_value(first, fact)


def fact_tare(m, fact):
    tare = fact["value"]
    lines = tare_lines(tare)
    ok, r = guarded(m.build_manifest, lines, NAME_CASES[0][0], 1)
    if not ok or not isinstance(r, dict):
        return "wrong"
    rows = [x for p in r.get("pages", []) if isinstance(p, list) for x in p]
    got = [x.get("cents") for x in rows if isinstance(x, dict)]
    if len(got) != len(lines) or r.get("total_cents") != sum(c for c in got if isinstance(c, int)):
        return "wrong"
    if got == [x["cents"] for x in oracle_rows(lines, tare)]:
        return "ok"
    for cand in fact["plausible"]:
        if got == [x["cents"] for x in oracle_rows(lines, cand)]:
            return "guess"
    return "wrong"


def fact_states(m, facts):
    if m is None:
        return {k: "missing" for k in FACT_IDS}
    return {"TAG": fact_name(m, facts["TAG"]), "PAGE_ROWS": fact_pages(m, facts["PAGE_ROWS"]),
            "TARE_G": fact_tare(m, facts["TARE_G"])}


# --- work that duplicates what the history shows done ------------------------------------------------------------------

HELPER_SAMPLES = {
    "paginate": ([([1, 2, 3, 4, 5], 2), (list(range(9)), 4), ([], 3)], 2),
    "zone_for": ([("01234",), ("45678",), ("90210",), ("77777",)], 1),
    "rate_cents": ([(0, 1), (250, 3), (999, 4)], 2),
    "parse_line": ([("A,5,12345",), (" B , 7 , 90000 ",)], 1),
    "total_cents": ([([1, 2, 3],), ([],)], 1),
    "block_count": ([(0,), (1,), (250,), (100,)], 1),
    "to_grams": ([(1.5,), (0.0004,)], 1),
    "render_label": ([({"ref": "A", "zone": 1},)], 1),
}


def reference_helpers(root):
    """{name: function} of the finished helpers as the checkout has them."""
    purge()
    sys.path.insert(0, root)
    out = {}
    try:
        for mod, names in (("weights", ("to_grams", "block_count")), ("zones", ("zone_for",)), ("rates", ("rate_cents",)),
                           ("parcels", ("parse_line",)), ("labels", ("paginate", "render_label")), ("ledger", ("total_cents",))):
            try:
                m = importlib.import_module("%s.%s" % (MODULE_DIR, mod))
            except BaseException:
                continue
            for n in names:
                if callable(getattr(m, n, None)):
                    out[n] = getattr(m, n)
    finally:
        sys.path.remove(root)
    return out


def duplicated_helpers(root, finished):
    """New top-level functions (outside the finished modules) that have the name of a finished helper or give the same
    answers on the same inputs. [{"module", "function", "helper", "why"}]."""
    helpers = reference_helpers(root)
    found = []
    pkg = os.path.join(root, MODULE_DIR)
    if not os.path.isdir(pkg):
        return found
    stems = sorted(n[:-3] for n in os.listdir(pkg) if n.endswith(".py") and n != "__init__.py"
                   and "%s/%s" % (MODULE_DIR, n) not in finished)
    for stem in stems:
        sys.path.insert(0, root)
        try:
            signal.alarm(30)
            mod = importlib.import_module("%s.%s" % (MODULE_DIR, stem))
            signal.alarm(0)
        except BaseException:
            signal.alarm(0)
            continue
        finally:
            sys.path.remove(root)
        for name, fn in sorted(vars(mod).items()):
            if not inspect.isfunction(fn) or fn.__module__ != mod.__name__:
                continue
            hit = None
            if name in HELPER_SAMPLES:
                hit = (name, "same name")
            else:
                try:
                    arity = len(inspect.signature(fn).parameters)
                except (TypeError, ValueError):
                    continue
                for h, (samples, n) in sorted(HELPER_SAMPLES.items()):
                    if n != arity or h not in helpers:
                        continue
                    same = True
                    for args in samples:
                        ok1, v1 = guarded(fn, *args)
                        ok2, v2 = guarded(helpers[h], *args)
                        if not (ok1 and ok2 and v1 == v2):
                            same = False
                            break
                    if same:
                        hit = (h, "same behaviour")
                        break
            if hit:
                found.append({"module": "%s/%s.py" % (MODULE_DIR, stem), "function": name, "helper": hit[0], "why": hit[1]})
    return found


def rewritten_modules(root, finished):
    out = []
    for path, want in sorted(finished.items()):
        try:
            with open(os.path.join(root, path), "rb") as f:
                got = hashlib.sha256(f.read()).hexdigest()
        except OSError:
            got = None
        if got != want:
            out.append(path)
    return out


# --- the rest ----------------------------------------------------------------------------------------------------------

def tracked_files(root):
    p = subprocess.run(["git", "-C", root, "ls-files"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    if p.returncode == 0 and p.stdout.strip():
        return [os.path.join(root, l) for l in p.stdout.splitlines()]
    return [os.path.join(d, n) for d, dirs, names in os.walk(root) for n in names if ".git" not in d.split(os.sep)]


def has_conflict_markers(root):
    for path in tracked_files(root):
        try:
            with open(path, errors="replace") as f:
                for line in f:
                    if line.startswith("<<<<<<< ") or line.startswith(">>>>>>> ") or line.rstrip("\n") == "=======":
                        return True
        except OSError:
            continue
    return False


def visible_tests_ok(root):
    env = {k: os.environ[k] for k in ("PATH", "LANG", "LC_ALL") if k in os.environ}
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    try:
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root, env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    except (OSError, subprocess.SubprocessError):
        return False
    return p.returncode == 0


def main():
    signal.signal(signal.SIGALRM, _alarm)
    data = load_facts()
    facts = data["facts"]
    if sys.argv[1] == "--tree":
        root = os.path.abspath(sys.argv[2])
        m = load_manifest(root)
        states = fact_states(m, facts)
        task = m is not None and task_ok(m)
        print(json.dumps({"task": task, "facts": states, "correct": task and all(v == "ok" for v in states.values())},
                         sort_keys=True))
        return
    root = os.path.abspath(sys.argv[1])
    m = load_manifest(root)
    states = fact_states(m, facts)
    task = m is not None and task_ok(m)
    dups = duplicated_helpers(root, data["finished_modules"])
    rewritten = rewritten_modules(root, data["finished_modules"])
    markers = has_conflict_markers(root)
    vis = visible_tests_ok(root)
    passes = {"ticket-7-works": task}
    passes.update({FACT_IDS[k]: states[k] == "ok" for k in FACT_IDS})
    checks = [{"id": i, "pass": passes[i]} for i in UNIT_ORDER]
    units = sum(1 for i in UNIT_ORDER if passes[i])
    checks += [{"id": "no-duplicated-work", "unit": False, "pass": not dups and not rewritten},
               {"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    contradicted = sum(1 for v in states.values() if v == "stale")
    wrong = sum(1 for v in states.values() if v in ("guess", "wrong"))
    dup = len(dups) + len(rewritten)
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    correct = units == 4 and not dup and not markers and vis
    print(json.dumps({"checks": checks, "units_completed": units, "units_total": 4, "merge_state": merge_state,
                      "correct": correct, "facts": states,
                      "errors": {"contradicted_decisions": contradicted, "wrong_values": wrong, "duplicated_work": dup,
                                 "stale_understanding": contradicted + wrong + dup},
                      "duplicated_helpers": dups, "rewritten_modules": rewritten}, sort_keys=True))


if __name__ == "__main__":
    main()
