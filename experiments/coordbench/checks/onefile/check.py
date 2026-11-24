"""Hidden checks for onefile (#1367). Usage: check.py CHECKOUT, or check.py --file PATH. See check.sh for the output.

The three tickets are tested by loading onefile/core.py directly (importlib), each against independent oracles and seeded
random inputs, and each testing only its own behaviour (its table row, its formatter kind, its function), so a lost
colleague's change cannot hide behind a working one. Separate structural checks cover the whole table, the integrated
render, the import, conflict markers and the visible tests. The seed is fixed so a check is repeatable.
"""
import importlib.util
import json
import os
import random
import re
import subprocess
import sys

SEED = 20261123
CORE = os.path.join("onefile", "core.py")
TICKETS = {"A": "timeout", "B": "retries", "C": "header"}
# The table once all three tickets are in: name -> (default, kind).
TABLE = {"host": ("localhost", "text"), "port": (8080, "int"), "timeout": (30, "seconds"), "retries": (3, "count"),
         "label": ("default", "text")}
TICKET_ROWS = [("A", "timeout"), ("B", "retries"), ("C", "label")]


def load_file(path):
    spec = importlib.util.spec_from_file_location("hidden_core", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def raises(fn, exc, *args):
    try:
        fn(*args)
    except exc:
        return True
    except Exception:
        return False
    return False


def close(a, b):
    return isinstance(a, float) and abs(a - b) <= 1e-9 * max(1.0, abs(b))


def row(m, name):
    return tuple(m.OPTIONS.get(name, ())) if isinstance(m.OPTIONS.get(name), (tuple, list)) else None


def lines_of(m, overrides=None):
    return m.render(overrides).split("\n")


def header_oracle(n):
    return "== settings (1 option) ==" if n == 1 else "== settings (%d options) ==" % n


def t_timeout(m):
    if row(m, "timeout") != TABLE["timeout"]:
        return False
    for v, want in ((30, "30s"), (90, "90s"), (0, "0s"), (7.0, "7s"), (1.5, "1.5s"), (0.25, "0.25s"), (2.75, "2.75s")):
        if m.format_value("seconds", v) != want:
            return False
    if "timeout = 30s" not in lines_of(m) or "timeout = 90s" not in lines_of(m, {"timeout": 90}):
        return False
    if "timeout = 1.5s" not in lines_of(m, {"timeout": 1.5}):
        return False
    rng = random.Random(SEED + 1)
    mult = {"": 1, "s": 1, "m": 60, "h": 3600}
    for _ in range(300):
        whole = str(rng.randint(0, 5000))
        num = whole if rng.random() < 0.5 else "%s.%d" % (whole, rng.randint(0, 999))
        unit = rng.choice(["", "s", "m", "h"])
        pad_l, pad_r = " " * rng.randint(0, 2), " " * rng.randint(0, 2)
        got = m.parse_timeout(pad_l + num + unit + pad_r)
        if not close(got, float(num) * mult[unit]):
            return False
    for bad in ("", "  ", "s", "-5", "1.5.2", "5d", "abc", "m5", "5ss", "1,5"):
        if not raises(m.parse_timeout, ValueError, bad):
            return False
    return True


def t_retries(m):
    if row(m, "retries") != TABLE["retries"]:
        return False
    for n in range(0, 21):
        if m.format_value("count", n) != "%dx" % n:
            return False
    if not raises(m.format_value, ValueError, "count", -1):
        return False
    if "retries = 3x" not in lines_of(m) or "retries = 5x" not in lines_of(m, {"retries": 5}):
        return False
    if [float(x) for x in m.backoff_delays(4)] != [1.0, 2.0, 4.0, 8.0]:
        return False
    rng = random.Random(SEED + 2)
    for _ in range(200):
        r, base, cap = rng.randint(0, 14), rng.choice([1.0, 0.5, 2.5, 3.0]), rng.choice([30.0, 5.0, 100.0, 2.0])
        want = [min(cap, base * 2 ** i) for i in range(r)]
        got = m.backoff_delays(r, base, cap)
        if len(got) != len(want) or not all(abs(g - w) < 1e-9 for g, w in zip(got, want)):
            return False
    if m.backoff_delays(0) != []:
        return False
    return raises(m.backoff_delays, ValueError, -1)


def t_header(m):
    for n in range(0, 12):
        if m.render_header(n) != header_oracle(n):
            return False
    if lines_of(m)[0] != header_oracle(len(m.OPTIONS)):
        return False
    if row(m, "label") != TABLE["label"]:
        return False
    return "label = default" in lines_of(m) and "label = x" in lines_of(m, {"label": "x"})


TESTS = {"A": t_timeout, "B": t_retries, "C": t_header}


def ticket_ok(m, t):
    try:
        return bool(TESTS[t](m))
    except BaseException:  # SyntaxError, AttributeError, TypeError, anything an agent's file can raise
        return False


def tickets_of(path):
    try:
        m = load_file(path)
    except BaseException:
        return {t: False for t in TESTS}
    return {t: ticket_ok(m, t) for t in TESTS}


def load_core(root):
    try:
        return load_file(os.path.join(root, CORE))
    except BaseException:
        return None


def table_state(m):
    """(table is exactly the full table, the ticket rows that are missing or wrong)."""
    if m is None:
        return False, [name for _, name in TICKET_ROWS]
    try:
        full = {k: tuple(v) for k, v in m.OPTIONS.items()} == TABLE
        lost = [name for _, name in TICKET_ROWS if row(m, name) != TABLE[name]]
    except BaseException:
        return False, [name for _, name in TICKET_ROWS]
    return full, lost


def integrated_render_ok(m):
    if m is None:
        return False
    want = "\n".join([header_oracle(5), "host = localhost", "label = default", "port = 8080", "retries = 3x",
                      "timeout = 30s"])
    try:
        return m.render() == want
    except BaseException:
        return False


def tracked_files(root):
    p = subprocess.run(["git", "ls-files"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return [f for f in p.stdout.splitlines() if f] if p.returncode == 0 else []


def has_conflict_markers(root):
    marker = re.compile(r"^(<<<<<<< |=======$|>>>>>>> )")
    for f in tracked_files(root):
        if f == "TASKS.md":  # B1's board: git merges it with merge=union, so markers there are not code (#1523)
            continue
        try:
            with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                if any(marker.match(line) for line in fh):
                    return True
        except OSError:
            pass
    return False


def imports_ok(root):
    code = "import sys; sys.path.insert(0, %r); import onefile.core" % root
    try:
        p = subprocess.run([sys.executable, "-c", code], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=60)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0


def visible_tests_ok(root):
    try:
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=90)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0


def git_out(root, args):
    p = subprocess.run(["git"] + args, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return p.stdout if p.returncode == 0 else ""


def per_agent(root):
    """Commits (non-merge, not the root) and lines of core.py that survive in the final file, by git author."""
    out = {}
    parts = git_out(root, ["log", "--no-merges", "--format=%x01%an%x01%P%x01"]).split("\x01")
    for i in range(1, len(parts) - 2, 3):
        author, parents = parts[i], parts[i + 1]
        if parents.strip():
            out.setdefault(author, {"commits": 0, "surviving_lines": 0})["commits"] += 1
    roots = set(git_out(root, ["rev-list", "--max-parents=0", "HEAD"]).split())
    sha = None
    for line in git_out(root, ["blame", "--line-porcelain", "HEAD", "--", CORE.replace(os.sep, "/")]).splitlines():
        head = re.match(r"^([0-9a-f]{40}) \d+ \d+", line)
        if head:
            sha = head.group(1)
        elif line.startswith("author ") and sha not in roots:
            out.setdefault(line[7:], {"commits": 0, "surviving_lines": 0})["surviving_lines"] += 1
    return out


def main():
    if sys.argv[1] == "--file":
        print(json.dumps(tickets_of(sys.argv[2]), sort_keys=True))
        return
    root = os.path.abspath(sys.argv[1])
    m = load_core(root)
    done = {t: (m is not None and ticket_ok(m, t)) for t in TESTS}
    checks = [{"id": TICKETS[t], "pass": done[t]} for t in "ABC"]
    units = sum(1 for t in "ABC" if done[t])
    markers = has_conflict_markers(root)
    full_table, lost_rows = table_state(m)
    integrated, imp, vis = integrated_render_ok(m), imports_ok(root), visible_tests_ok(root)
    checks += [{"id": "table", "unit": False, "pass": full_table},
               {"id": "integrated-render", "unit": False, "pass": integrated},
               {"id": "imports", "unit": False, "pass": imp},
               {"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    correct = units == 3 and full_table and integrated and imp and not markers and vis
    print(json.dumps({"checks": checks, "units_completed": units, "units_total": 3, "merge_state": merge_state,
                      "correct": correct, "lost_edits": [t for t in "ABC" if not done[t]], "lost_table_entries": lost_rows,
                      "per_agent": per_agent(root)}, sort_keys=True))


if __name__ == "__main__":
    main()
