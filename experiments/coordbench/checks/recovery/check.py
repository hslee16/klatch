"""Hidden checks for recovery (#1371, scenario S6). Usage: check.py CHECKOUT, or check.py --module NAME FILE.

CHECKOUT is a clone of the final origin (branch main, full history). The output is one JSON object on the last line, see
check.sh. --module NAME FILE tests one module's file on its own, with no repository (the harness uses it to find the commit
that completed the victim's module) and prints {"ok": true or false}.

Agents: a owns ranges, b owns wrap and pairs, c owns bits. c is the victim: the harness kills it while its module is edited
and not committed. The checks:

  ranges, wrap, pairs, bits   one unit each (0 to 4): the module behaves, tested by loading its file directly
  recovered                   the victim's module is correct on the final main AND a commit by someone other than the
                              victim, on main's history, changed it (the victim's own module finished by a survivor)
  ask-answered                a's mask numbering is right (ranges.to_mask): the orphaned question's answer reached a
  no-duplicate-work           the victim's module was committed by at most one author other than the victim, and nobody
                              but its owner committed a live agent's module
  no-conflict-markers, visible-tests
"""
import importlib.util
import json
import os
import random
import re
import subprocess
import sys

SEED = 20261123
OWNER = {"ranges": "a", "wrap": "b", "pairs": "b", "bits": "c"}
VICTIM = "c"
MODULES = ["ranges", "wrap", "pairs", "bits"]


def load_file(path, name):
    spec = importlib.util.spec_from_file_location("hidden_" + name, path)
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


def t_ranges_mask(m):
    """The orphaned ask's answer: index 0 is the most significant bit of the 16 bit mask."""
    return (m.to_mask([]) == 0 and m.to_mask([(0, 0)]) == 0x8000 and m.to_mask([(15, 15)]) == 0x0001
            and m.to_mask([(0, 3)]) == 0xF000 and m.to_mask([(2, 2), (4, 5)]) == 0x2000 | 0x0800 | 0x0400
            and m.to_mask([(14, 15)]) == 0x0003 and raises(m.to_mask, ValueError, [(0, 16)])
            and raises(m.to_mask, ValueError, [(-1, 2)]))


def t_ranges(m):
    rng = random.Random(SEED + 1)
    if m.compress([]) != [] or m.compress([5, 1, 2, 3, 7, 7]) != [(1, 3), (5, 5), (7, 7)]:
        return False
    for _ in range(200):
        nums = [rng.randint(-20, 40) for _ in range(rng.randint(0, 25))]
        want, run = [], None
        for n in sorted(set(nums)):
            if run and n == run[1] + 1:
                run[1] = n
            else:
                run = [n, n]
                want.append(run)
        want = [tuple(r) for r in want]
        if m.compress(nums) != want or m.expand(m.compress(nums)) != sorted(set(nums)):
            return False
    if m.expand([(1, 3), (2, 5), (9, 9)]) != [1, 2, 3, 4, 5, 9] or m.expand([]) != []:
        return False
    if not raises(m.expand, ValueError, [(5, 3)]):
        return False
    for _ in range(100):
        idx = sorted(set(rng.randint(0, 15) for _ in range(rng.randint(0, 10))))
        want = 0
        for i in idx:
            want |= 0x8000 >> i
        if m.to_mask(m.compress(idx)) != want:
            return False
    return t_ranges_mask(m)


def t_wrap(m):
    rng = random.Random(SEED + 2)
    if m.wrap("", 5) != [] or m.wrap("   ", 5) != [] or m.wrap("aa bb cc", 5) != ["aa bb", "cc"]:
        return False
    if m.wrap("abcdefgh ij", 4) != ["abcdefgh", "ij"] or m.wrap("a  b\t\nc", 10) != ["a b c"]:
        return False
    for _ in range(300):
        words = ["x" * rng.randint(1, 9) for _ in range(rng.randint(0, 14))]
        width = rng.randint(1, 14)
        lines, cur = [], ""
        for w in words:
            if cur and len(cur) + 1 + len(w) <= width:
                cur += " " + w
            else:
                if cur:
                    lines.append(cur)
                cur = w
        if cur:
            lines.append(cur)
        if m.wrap(" ".join(words), width) != lines or m.unwrap(lines) != " ".join(words):
            return False
    return m.unwrap([]) == "" and raises(m.wrap, ValueError, "a b", 0)


def t_pairs(m):
    rng = random.Random(SEED + 3)
    if m.pairs([]) != [] or m.pairs([1]) != [] or m.pairs([1, 2, 3]) != [(1, 2), (2, 3)]:
        return False
    for _ in range(200):
        seq = [rng.randint(0, 9) for _ in range(rng.randint(0, 12))]
        n = rng.randint(1, 6)
        if m.pairs(seq) != list(zip(seq, seq[1:])):
            return False
        if m.chunk(seq, n) != [seq[i:i + n] for i in range(0, len(seq), n)]:
            return False
        if m.window(seq, n) != [seq[i:i + n] for i in range(len(seq) - n + 1)]:
            return False
    orig = [1, 2, 3, 4]
    snap = list(orig)
    m.pairs(orig), m.chunk(orig, 2), m.window(orig, 2)
    return (orig == snap and m.chunk([], 3) == [] and m.window([1], 2) == [] and raises(m.chunk, ValueError, [1], 0)
            and raises(m.window, ValueError, [1], 0))


def t_bits(m):
    rng = random.Random(SEED + 4)
    for _ in range(300):
        mask = rng.randint(0, 0xFFFF)
        i = rng.randint(0, 15)
        bit = 0x8000 >> i
        if m.set_bit(mask, i) != mask | bit or m.clear_bit(mask, i) != mask & ~bit & 0xFFFF:
            return False
        if m.test_bit(mask, i) is not bool(mask & bit):
            return False
        if m.popcount(mask) != bin(mask).count("1"):
            return False
        if m.bit_indexes(mask) != [k for k in range(16) if mask & (0x8000 >> k)]:
            return False
    if m.set_bit(0, 0) != 0x8000 or m.set_bit(0, 15) != 1 or m.bit_indexes(0x8001) != [0, 15] or m.bit_indexes(0) != []:
        return False
    return (m.popcount(0) == 0 and m.popcount(0xFFFF) == 16 and raises(m.set_bit, ValueError, 0, 16)
            and raises(m.clear_bit, ValueError, 0, -1) and raises(m.test_bit, ValueError, 0, 99))


TESTS = {"ranges": t_ranges, "wrap": t_wrap, "pairs": t_pairs, "bits": t_bits}


def file_ok(path, name):
    try:
        return bool(TESTS[name](load_file(path, name)))
    except BaseException:  # NotImplementedError, SyntaxError, anything an agent's module can raise
        return False


def module_ok(root, name):
    return file_ok(os.path.join(root, "recoverkit", name + ".py"), name)


def mask_ok(root):
    try:
        return bool(t_ranges_mask(load_file(os.path.join(root, "recoverkit", "ranges.py"), "ranges")))
    except BaseException:
        return False


def git(root, *args):
    p = subprocess.run(["git"] + list(args), cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return p.stdout if p.returncode == 0 else ""


def authors_by_module(root):
    """{module: [authors in the order they first committed it]} from the non-merge, non-root commits of main's history."""
    out = git(root, "log", "main", "--reverse", "--no-merges", "--no-renames", "--format=%x01%an%x01%P", "--name-only")
    seen = {m: [] for m in MODULES}
    parts = out.split("\x01")
    for i in range(1, len(parts) - 1, 2):
        author, rest = parts[i], parts[i + 1]
        parents, _, files = rest.partition("\n")
        if not parents.strip():
            continue  # the root commit is the harness's initial tree
        for fn in files.split():
            m = re.fullmatch(r"recoverkit/(\w+)\.py", fn)
            if m and m.group(1) in seen and author not in seen[m.group(1)]:
                seen[m.group(1)].append(author)
    return seen


def duplicate_work(authors):
    """Modules with duplicated or foreign work: [{module, authors, why}]."""
    bad = []
    for mod, who in authors.items():
        if mod == "bits":
            survivors = [x for x in who if x != VICTIM]
            if len(survivors) > 1:
                bad.append({"module": mod, "authors": who, "why": "recovered by more than one survivor"})
        else:
            foreign = [x for x in who if x != OWNER[mod]]
            if foreign:
                bad.append({"module": mod, "authors": who, "why": "a live agent's module changed by %s" % ",".join(foreign)})
    return bad


def has_conflict_markers(root):
    marker = re.compile(r"^(<<<<<<< |=======$|>>>>>>> )")
    for f in git(root, "ls-files").splitlines():
        try:
            with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                if any(marker.match(line) for line in fh):
                    return True
        except OSError:
            pass
    return False


def visible_tests_ok(root):
    try:
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=90)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0


def per_agent(authors):
    out = {}
    for mod, who in authors.items():
        for a in who:
            out.setdefault(a, {"modules": []})["modules"].append(mod)
    for a in out.values():
        a["modules"].sort()
    return out


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--module":
        print(json.dumps({"ok": file_ok(sys.argv[3], sys.argv[2])}))
        return
    root = os.path.abspath(sys.argv[1])
    ok = {m: module_ok(root, m) for m in MODULES}
    authors = authors_by_module(root)
    dups = duplicate_work(authors)
    recovered_by = [a for a in authors["bits"] if a != VICTIM]
    recovered = ok["bits"] and bool(recovered_by)
    answered = mask_ok(root)
    markers = has_conflict_markers(root)
    vis = visible_tests_ok(root)
    checks = [{"id": m, "pass": ok[m]} for m in MODULES]
    checks += [{"id": "recovered", "unit": False, "pass": recovered}, {"id": "ask-answered", "unit": False, "pass": answered},
               {"id": "no-duplicate-work", "unit": False, "pass": not dups},
               {"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    units = sum(1 for m in MODULES if ok[m])
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    correct = units == len(MODULES) and recovered and answered and not dups and not markers and vis
    print(json.dumps({"checks": checks, "units_completed": units, "units_total": len(MODULES), "merge_state": merge_state,
                      "correct": correct, "recovered": recovered, "recovered_by": recovered_by, "ask_answered": answered,
                      "duplicates": dups, "per_agent": per_agent(authors)}, sort_keys=True))


if __name__ == "__main__":
    main()
