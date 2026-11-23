"""Hidden checks for chain (#1368). Usage: check.py CHECKOUT (a clone of the final origin, with its history). See check.sh.

Four steps are the units: schema (a), bins (a then c), loader (b), report (c). Each is tested on its own, so one agent's
failure cannot hide behind another's: the loader runs with the reference schema file in place, the report with the reference
loader, normaliser and schema. A second, structural group says what the steps alone cannot:
  quirk          the legacy bins are normalised by the rule only a was told (right prefix, right offset), on every legacy
                 record of the real data and on seeded cases
  handoff-state  none of a's published pieces (the schema file, parse_bin with its Bin(pfx, num) result) was dropped or
                 redone differently later: a piece that met its contract in some commit and fails on the final main is lost
                 work; one that never met it is a step that failed, not lost work
  integrated     the whole pipeline on the final main, nothing substituted: bin_totals() equals the expected totals
  order-*        the loader was first committed with the schema file in its tree; the report with the loader in its tree
  no-conflict-markers, visible-tests   merge state
All facts are in expected.json, made from a fixed seed by fixtures/chain/gen.py.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REFERENCE = os.path.join(os.path.dirname(os.path.dirname(HERE)), "fixtures", "chain", "reference", "chainkit")
EXPECTED = os.path.join(HERE, "expected.json")
MARKERS = re.compile(r"^(<<<<<<<|>>>>>>>) ", re.M)

BINS_SNIPPET = r'''
import json, re, sys
sys.path.insert(0, sys.argv[1])
exp = json.load(open(sys.argv[2]))
out = {"import": False, "parse": False, "plain": False, "quirk": False, "how": None}
try:
    from chainkit import bins
    out["import"] = True
except BaseException:
    print(json.dumps(out)); sys.exit(0)

def attempt(fn):
    try:
        return fn()
    except BaseException:
        return False

def parse_ok():
    for raw, pfx, num in exp["parse_cases"]:
        b = bins.parse_bin(raw)
        if not (hasattr(b, "pfx") and hasattr(b, "num") and b.pfx == pfx and b.num == num and type(b.num) is int):
            return False
    try:
        bins.parse_bin("not a bin")
    except ValueError:
        return True
    return False

def plain_ok():
    return all(bins.canonical_bin(raw) == want for raw, want in exp["canon_cases"])

def quirk_ok():
    q = exp["quirk"]
    cases = [tuple(c) for c in exp["quirk_cases"]]
    for r in exp["records"]:
        m = re.fullmatch(r"([A-Z]{2})[-/]?([0-9]+)", r["bin"])
        if m.group(1) == q["legacy"]:
            cases.append((r["bin"], "%s-%05d" % (q["new"], int(m.group(2)) + q["offset"])))
    return all(bins.canonical_bin(raw) == want for raw, want in cases)

out["parse"] = attempt(parse_ok)
out["plain"] = attempt(plain_ok)
out["quirk"] = attempt(quirk_ok)
if not out["parse"]:
    try:
        bins.parse_bin("QX-1")
        out["how"] = "redone-differently"
    except NotImplementedError:
        out["how"] = "dropped"
    except AttributeError:
        out["how"] = "dropped"
    except BaseException:
        out["how"] = "redone-differently"
print(json.dumps(out))
'''

LOADER_SNIPPET = r'''
import json, os, sys, tempfile
sys.path.insert(0, sys.argv[1])
exp = json.load(open(sys.argv[2]))
out = {"real": False, "other_layout": False}
try:
    from chainkit import loader
    out["real"] = loader.load() == exp["records"]
    d = tempfile.mkdtemp()
    with open(os.path.join(d, "alt.dat"), "w") as f:
        f.write("\n".join(exp["alt_lines"]) + "\n")
    with open(os.path.join(d, "alt.json"), "w") as f:
        json.dump(exp["alt_schema"], f)
    out["other_layout"] = loader.load(os.path.join(d, "alt.dat"), os.path.join(d, "alt.json")) == exp["alt_records"]
except BaseException:
    pass
print(json.dumps(out))
'''

REPORT_SNIPPET = r'''
import json, os, re, sys, tempfile
sys.path.insert(0, sys.argv[1])
exp = json.load(open(sys.argv[2]))
q = exp["quirk"]

def canon(raw):
    m = re.fullmatch(r"([A-Z]{2})[-/]?([0-9]+)", raw)
    n = int(m.group(2))
    return "%s-%05d" % (q["new"], n + q["offset"]) if m.group(1) == q["legacy"] else "%s-%05d" % (m.group(1), n)

out = {"real": False, "trimmed": False}
try:
    from chainkit import report
    out["real"] = report.bin_totals() == exp["totals"]
    d = tempfile.mkdtemp()
    with open(os.path.join(d, "part.dat"), "w") as f:
        f.write("\n".join(exp["lines"][:10]) + "\n")
    want = {}
    for r in exp["records"][:10]:
        want[canon(r["bin"])] = want.get(canon(r["bin"]), 0) + r["qty"]
    out["trimmed"] = report.bin_totals(os.path.join(d, "part.dat")) == want
except BaseException:
    pass
print(json.dumps(out))
'''


def read_json(path):
    with open(path) as f:
        return json.load(f)


def snippet(code, root, expected_path, timeout=60):
    env = {"PATH": os.environ.get("PATH", ""), "PYTHONDONTWRITEBYTECODE": "1"}
    try:
        p = subprocess.run([sys.executable, "-I", "-c", code, root, expected_path], cwd=root, env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=timeout)
        return json.loads(p.stdout.strip().splitlines()[-1])
    except (subprocess.TimeoutExpired, ValueError, IndexError, OSError):
        return {}


def git(root, *args):
    p = subprocess.run(["git", "-C", root] + list(args), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return p.returncode, p.stdout


def overlay(root, names):
    """A copy of the checkout with the reference files for `names` put over it."""
    d = tempfile.mkdtemp(prefix="chain-check-")
    dest = os.path.join(d, "w")
    shutil.copytree(root, dest, ignore=shutil.ignore_patterns(".git", "__pycache__"))
    os.makedirs(os.path.join(dest, "chainkit"), exist_ok=True)
    for n in names:
        shutil.copyfile(os.path.join(REFERENCE, n), os.path.join(dest, "chainkit", n))
    return dest


def first_commit_tree_has(root, path, needs):
    """True when the oldest non-root, non-merge commit that touched `path` has every file of `needs` in its tree; False
    when `path` was never committed after the initial commit."""
    rc, out = git(root, "log", "--no-merges", "--reverse", "--format=%H", "--", path)
    for sha in out.split():
        rc, parents = git(root, "rev-list", "--parents", "-n", "1", sha)
        if len(parents.split()) < 2:
            continue  # the harness's initial commit
        return all(git(root, "cat-file", "-e", "%s:%s" % (sha, n))[0] == 0 for n in needs)
    return False


def versions(root, path):
    """The text of `path` in every non-merge commit that touched it, oldest first."""
    rc, out = git(root, "log", "--no-merges", "--reverse", "--format=%H", "--", path)
    texts = []
    for sha in out.split():
        rc, text = git(root, "show", "%s:%s" % (sha, path))
        if rc == 0:
            texts.append(text)
    return texts


def had_contract(root, path, test):
    """True when some commit's version of `path` passed test(version_text)."""
    return any(test(t) for t in versions(root, path))


def main(argv):
    root = os.path.abspath(argv[0])
    exp = read_json(EXPECTED)
    checks = []

    def add(cid, ok, unit=True):
        c = {"id": cid, "pass": bool(ok)}
        if not unit:
            c["unit"] = False
        checks.append(c)

    # unit: schema
    try:
        schema_ok = read_json(os.path.join(root, "chainkit", "schema.json")) == exp["schema"]
    except (OSError, ValueError):
        schema_ok = False
    schema_exists = os.path.exists(os.path.join(root, "chainkit", "schema.json"))

    # unit: bins (parse_bin contract and the plain canonical cases)
    bins = snippet(BINS_SNIPPET, root, EXPECTED)
    bins_ok = bool(bins.get("parse") and bins.get("plain"))
    # unit: loader, with the reference schema file in place
    ov = overlay(root, ["schema.json"])
    loader = snippet(LOADER_SNIPPET, ov, EXPECTED)
    shutil.rmtree(os.path.dirname(ov), ignore_errors=True)
    loader_ok = bool(loader.get("real") and loader.get("other_layout"))
    # unit: report, with the reference loader, normaliser and schema in place
    ov = overlay(root, ["schema.json", "loader.py", "bins.py"])
    report = snippet(REPORT_SNIPPET, ov, EXPECTED)
    shutil.rmtree(os.path.dirname(ov), ignore_errors=True)
    report_ok = bool(report.get("real") and report.get("trimmed"))
    add("schema", schema_ok)
    add("bins", bins_ok)
    add("loader", loader_ok)
    add("report", report_ok)

    # structural
    quirk_ok = bool(bins.get("quirk"))
    integrated = snippet(REPORT_SNIPPET, root, EXPECTED).get("real") is True

    def parse_contract(text):
        d = tempfile.mkdtemp(prefix="chain-hist-")
        try:
            os.makedirs(os.path.join(d, "chainkit"))
            open(os.path.join(d, "chainkit", "__init__.py"), "w").close()
            with open(os.path.join(d, "chainkit", "bins.py"), "w") as f:
                f.write(text)
            return bool(snippet(BINS_SNIPPET, d, EXPECTED).get("parse"))
        finally:
            shutil.rmtree(d, ignore_errors=True)

    lost = []
    if not schema_ok and had_contract(root, "chainkit/schema.json", lambda t: _json_equal(t, exp["schema"])):
        lost.append({"piece": "schema", "how": "redone-differently" if schema_exists else "dropped"})
    if not bins.get("parse") and had_contract(root, "chainkit/bins.py", parse_contract):
        lost.append({"piece": "parse_bin", "how": bins.get("how") or "dropped"})
    order = {"loader_after_schema": first_commit_tree_has(root, "chainkit/loader.py", ["chainkit/schema.json"]),
             "report_after_loader": first_commit_tree_has(root, "chainkit/report.py", ["chainkit/loader.py"])}
    markers = False
    rc, files = git(root, "ls-files")
    for rel in files.splitlines():
        if rel.endswith((".py", ".json", ".md", ".txt", ".dat")):
            try:
                with open(os.path.join(root, rel), errors="replace") as f:
                    if MARKERS.search(f.read()):
                        markers = True
            except OSError:
                pass
    env = {"PATH": os.environ.get("PATH", ""), "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([sys.executable, "-I", "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root, env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tests_ok = p.returncode == 0
    add("quirk", quirk_ok, False)
    add("handoff-state", not lost, False)
    add("integrated", integrated, False)
    add("order-loader-after-schema", order["loader_after_schema"], False)
    add("order-report-after-loader", order["report_after_loader"], False)
    add("no-conflict-markers", not markers, False)
    add("visible-tests", tests_ok, False)
    merge_state = "conflicts-left" if markers else ("clean" if tests_ok else "tests-broken")
    units = [c for c in checks if c.get("unit") is not False]
    done = sum(1 for c in units if c["pass"])
    correct = (done == len(units) and quirk_ok and not lost and integrated and all(order.values())
               and merge_state == "clean")
    print(json.dumps({"checks": checks, "units_completed": done, "units_total": len(units), "steps_correct": done,
                      "quirk_honoured": quirk_ok, "lost_at_handoff": lost, "order": order, "merge_state": merge_state,
                      "correct": correct}))
    return 0


def _json_equal(text, want):
    try:
        return json.loads(text) == want
    except ValueError:
        return False


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
