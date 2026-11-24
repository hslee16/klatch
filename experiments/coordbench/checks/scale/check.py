"""Hidden checks for the generated fixture F7 `scale` (#1372). Usage: check.py CHECKOUT. See check.sh for the output.

Generalises checks/tallykit to M modules (M = 2 x agents) taken from the generator (fixtures/scale/gen.py) for the size and
seed in COORDBENCH_GEN_MODULES and COORDBENCH_GEN_SEED. Each module is tested by loading its file directly (importlib, not
through the package), so a broken registry cannot hide a good module, against the independent oracles in oracles.py on seeded
inputs; a module that raises, hangs (10 seconds) or changes its argument fails. A ticket is complete when both of its modules
pass. The merge checks (registry, changelog, conflict markers, the visible tests) and the per-agent contribution follow S1.
"""
import ast
import copy
import importlib.util
import json
import os
import random
import re
import signal
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PACKAGE = "scalekit"
MODULE_SECONDS = 10


def load_path(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Timeout(BaseException):
    pass


def _alarm(signum, frame):
    raise Timeout()


def module_ok(root, m, oracles, seed):
    """True when module m (a dict of gen.build) behaves like its oracle on every seeded input."""
    path = os.path.join(root, PACKAGE, m["name"] + ".py")
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(MODULE_SECONDS)
    try:
        mod = load_path("hidden_" + m["name"], path)
        fn = getattr(mod, m["fn"], None)
        if not callable(fn):
            return False
        make, inputs = oracles.ORACLES[m["kind"]]
        want_fn = make(m["params"])
        rng = random.Random("check:%d:%s" % (seed, m["name"]))
        for args in inputs(m["params"], rng):
            given = copy.deepcopy(args)
            got = fn(*given)
            want = want_fn(*copy.deepcopy(args))
            if got != want or type(got) is not type(want) or given != args:
                return False
        return True
    except BaseException:  # NotImplementedError, SyntaxError, Timeout, anything an agent's module can raise
        return False
    finally:
        signal.alarm(0)


def tracked_files(root):
    p = subprocess.run(["git", "ls-files"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return [f for f in p.stdout.splitlines() if f] if p.returncode == 0 else []


def has_conflict_markers(root):
    marker = re.compile(r"^(<<<<<<< |=======$|>>>>>>> )")
    for f in tracked_files(root):
        try:
            with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                if any(marker.match(line) for line in fh):
                    return True
        except OSError:
            pass
    return False


def registry_lines(root):
    """{module: [names]} from the import lines of the package's __init__.py, or None when the file is missing, does not parse
    or has two lines for one module."""
    try:
        with open(os.path.join(root, PACKAGE, "__init__.py")) as f:
            tree = ast.parse(f.read())
    except (OSError, SyntaxError, ValueError):
        return None
    seen = {}
    for mod, names in import_lines(tree, PACKAGE + "."):
        if mod in seen:
            return None
        seen[mod] = names
    return seen


def import_lines(tree, prefix):
    """(module, sorted names) of each top level `from prefix.module import names` statement."""
    for node in tree.body:
        if not isinstance(node, ast.ImportFrom):
            continue
        mod = node.module or ""
        if mod.startswith(prefix):
            yield mod[len(prefix):], sorted(a.name for a in node.names)


def names_cover(seen, wanted):
    """The registry has a line for exactly the wanted modules and each line names at least the wanted functions."""
    if set(seen) != set(wanted):
        return False
    return all(set(fs) <= set(seen[m]) for m, fs in wanted.items())


def registry_ok(root, build):
    """scalekit/__init__.py imports and exports every function, with exactly one registry line per module. A line may also
    name a public helper of its module (the registry text asks for the public functions, #1525): the function the checker
    needs must be there and every name on a line must import."""
    seen = registry_lines(root)
    want = {m["name"]: [m["fn"]] for m in build["modules"]}
    if seen is None or not names_cover(seen, want):
        return False
    code = ("import %s, json; names = %r; print(json.dumps(all(callable(getattr(%s, n, None)) for n in names)))"
            % (PACKAGE, sorted({n for names in seen.values() for n in names}), PACKAGE))
    try:
        p = subprocess.run([sys.executable, "-c", code], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                           timeout=60)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0 and p.stdout.strip() == "true"


def changelog_ok(root, build):
    try:
        with open(os.path.join(root, "CHANGELOG.md")) as f:
            lines = f.read().splitlines()
    except OSError:
        return False
    entries = [l for l in lines if re.match(r"^- ticket [A-Z]:", l)]
    want = sorted("- ticket %s: %s" % (a["ticket"], ", ".join(a["modules"])) for a in build["agents"])
    return sorted(entries) == want


def visible_tests_ok(root):
    try:
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=90)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0


def per_agent(root, names):
    """Commits and modules touched, by git author, skipping the root commit and merge commits."""
    p = subprocess.run(["git", "log", "--no-merges", "--no-renames", "--format=%x01%an%x01%P", "--name-only"], cwd=root,
                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    out = {}
    parts = p.stdout.split("\x01") if p.returncode == 0 else []
    for i in range(1, len(parts) - 1, 2):
        author, rest = parts[i], parts[i + 1]
        parents, _, files = rest.partition("\n")
        if not parents.strip():
            continue  # the root commit is the harness's initial tree
        a = out.setdefault(author, {"commits": 0, "modules": []})
        a["commits"] += 1
        for fn in files.split():
            m = re.fullmatch(PACKAGE + r"/(\w+)\.py", fn)
            if m and m.group(1) in names and m.group(1) not in a["modules"]:
                a["modules"].append(m.group(1))
    for a in out.values():
        a["modules"].sort()
    return out


def main():
    root = os.path.abspath(sys.argv[1])
    try:
        modules, seed = int(os.environ["COORDBENCH_GEN_MODULES"]), int(os.environ["COORDBENCH_GEN_SEED"])
    except (KeyError, ValueError):
        print("check.py: COORDBENCH_GEN_MODULES and COORDBENCH_GEN_SEED must be set (the harness sets them from the "
              "fixture's generated key)", file=sys.stderr)
        return 2
    gen = load_path("scale_gen", os.path.join(HERE, "..", "..", "fixtures", "scale", "gen.py"))
    oracles = load_path("scale_oracles", os.path.join(HERE, "oracles.py"))
    build = gen.build(modules, seed)
    ok = {m["name"]: module_ok(root, m, oracles, seed) for m in build["modules"]}
    checks = [{"id": m["name"], "pass": ok[m["name"]]} for m in build["modules"]]
    units = sum(1 for v in ok.values() if v)
    tickets = {a["ticket"]: all(ok[n] for n in a["modules"]) for a in build["agents"]}
    markers = has_conflict_markers(root)
    reg, chg, vis = registry_ok(root, build), changelog_ok(root, build), visible_tests_ok(root)
    checks += [{"id": "registry", "unit": False, "pass": reg}, {"id": "changelog", "unit": False, "pass": chg},
               {"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    correct = units == len(build["modules"]) and reg and chg and not markers and vis
    agents = per_agent(root, set(ok))
    for a in agents.values():
        a["units_correct"] = sum(1 for n in a["modules"] if ok[n])
    print(json.dumps({"checks": checks, "units_completed": units, "units_total": len(build["modules"]),
                      "tickets": tickets, "tickets_completed": sum(1 for v in tickets.values() if v),
                      "tickets_total": len(tickets), "merge_state": merge_state, "correct": correct, "per_agent": agents},
                     sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
