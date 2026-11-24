#!/usr/bin/env python3
"""Generator of fixture F7 `scale` (#1372): a tallykit-like project of M independent modules for N = M / 2 agents.

Standard library only; deterministic: the same (modules, seed) gives the same bytes in every process (every random choice
comes from a `random.Random` seeded with a string, which does not depend on hash randomisation). The generator writes:

  template   what an agent's checkout holds: one stub module per unit (docstring, signature, NotImplementedError), one visible
             test per module (a few fixed examples), the registry scalekit/__init__.py, CHANGELOG.md and README.md
  reference  a full solution (every module, the registry with every line, the changelog with every ticket line); it is never
             part of a checkout: setup.sh writes the template only, and only the fake agents and the tests read the reference
  tasks      the text of each agent's job (two modules per agent: ticket A for agent a, and so on)
  map        the unit-to-path and agent-to-units map the scenario file needs (scenario.py git metrics)

Every module is one small pure function drawn from the table KINDS with seeded parameters (the parameters are stated in the
stub's docstring, so an agent can implement it from the text). The hidden oracles are NOT here: they are an independent
implementation in checks/scale/oracles.py. The shared files are the same as in S1 (a registry every ticket appends to and a
changelog every ticket appends to), so the chance of a real merge conflict grows with the number of agents.

CLI:
  gen.py write DEST --modules M --seed S        write the template into DEST (what setup.sh does)
  gen.py reference DEST --modules M --seed S    write the reference solution into DEST
  gen.py tasks --modules M --seed S             the agents' tasks as JSON [{"name", "task"}]
  gen.py map --modules M --seed S               the scenario map as JSON
  gen.py fake --modules M --seed S --reference DIR [--chain]   a fake-agent script as JSON (writes the reference into DIR)
"""
import argparse
import json
import os
import random
import sys

PACKAGE = "scalekit"
MAX_AGENTS = 26  # tickets are the letters A to Z


def _pick(rng, seq):
    return seq[rng.randrange(len(seq))]


# Each kind: sig (the parameters), params(rng), doc(p) (the contract, in words), body(p) (the lines of the reference
# function, after the def line), cases (argument tuples for the visible examples).
KINDS = [
    {"name": "affine", "sig": "x",
     "params": lambda r: {"a": r.randint(2, 9), "b": r.randint(0, 20), "m": _pick(r, [7, 11, 13, 17, 19, 23, 29])},
     "doc": lambda p: "Return (%(a)d * x + %(b)d) mod %(m)d for an integer x (Python's %% operator, so the result is 0 to %(m1)d)."
     % dict(p, m1=p["m"] - 1),
     "body": lambda p: ["return (%(a)d * x + %(b)d) %% %(m)d" % p],
     "cases": [(0,), (1,), (5,), (-3,), (100,)]},
    {"name": "clampk", "sig": "x",
     "params": lambda r: {"lo": r.randint(-20, 0), "hi": r.randint(1, 30), "k": r.randint(2, 5)},
     "doc": lambda p: "Clamp the integer x into the range %(lo)d to %(hi)d (inclusive) and multiply the result by %(k)d." % p,
     "body": lambda p: ["return min(max(x, %(lo)d), %(hi)d) * %(k)d" % p],
     "cases": [(0,), (-50,), (50,), (7,)]},
    {"name": "digitsum", "sig": "n",
     "params": lambda r: {"b": r.randint(2, 9)},
     "doc": lambda p: "Return the sum of the digits of abs(n) written in base %(b)d (0 for n = 0)." % p,
     "body": lambda p: ["n = abs(n)", "total = 0", "while n:", "    total += n %% %(b)d" % p, "    n //= %(b)d" % p, "return total"],
     "cases": [(0,), (1,), (255,), (-1000,), (99999,)]},
    {"name": "popmask", "sig": "n",
     "params": lambda r: {"mask": r.randint(0x0F, 0xFFF)},
     "doc": lambda p: "For a non-negative integer n return the number of 1 bits in (n AND %(mask)d)." % p,
     "body": lambda p: ['return bin(n & %(mask)d).count("1")' % p],
     "cases": [(0,), (1,), (4095,), (123456,)]},
    {"name": "caesar", "sig": "s",
     "params": lambda r: {"k": r.randint(1, 25)},
     "doc": lambda p: "Shift every ASCII letter of the text s forward by %(k)d places in the alphabet, wrapping around and keeping "
     "its case; every other character stays as it is." % p,
     "body": lambda p: ["out = []", "for ch in s:", '    if "a" <= ch <= "z":',
                        "        out.append(chr((ord(ch) - 97 + %(k)d) %% 26 + 97))" % p, '    elif "A" <= ch <= "Z":',
                        "        out.append(chr((ord(ch) - 65 + %(k)d) %% 26 + 65))" % p, "    else:", "        out.append(ch)",
                        'return "".join(out)'],
     "cases": [("",), ("abc",), ("Hello, World!",), ("xyz XYZ 123",)]},
    {"name": "padfit", "sig": "s",
     "params": lambda r: {"w": r.randint(4, 10), "c": _pick(r, "0*#.-")},
     "doc": lambda p: "Return the text s as exactly %(w)d characters: shorter text is padded on the left with %(c)r, longer text "
     "keeps only its last %(w)d characters." % p,
     "body": lambda p: ["if len(s) >= %(w)d:" % p, "    return s[len(s) - %(w)d:]" % p, "return %(c)r * (%(w)d - len(s)) + s" % p],
     "cases": [("",), ("ab",), ("abcdefghijklmnop",), ("exactly10!",)]},
    {"name": "rotate", "sig": "xs",
     "params": lambda r: {"k": r.randint(1, 5)},
     "doc": lambda p: "Return a new list: the list xs rotated left by %(k)d places (the first %(k)d items move to the end, "
     "counted modulo the length); an empty list gives []. The input is not changed." % p,
     "body": lambda p: ["if not xs:", "    return []", "k = %(k)d %% len(xs)" % p, "return xs[k:] + xs[:k]"],
     "cases": [([],), ([1],), ([1, 2, 3, 4, 5, 6, 7],), (["a", "b", "c"],)]},
    {"name": "winmax", "sig": "xs",
     "params": lambda r: {"w": r.randint(2, 4)},
     "doc": lambda p: "Return the list of the maximum of every window of %(w)d consecutive items of xs, left to right ([] when xs "
     "has fewer than %(w)d items)." % p,
     "body": lambda p: ["return [max(xs[i:i + %(w)d]) for i in range(len(xs) - %(w)d + 1)]" % p],
     "cases": [([],), ([5],), ([1, 3, 2, 5, 4, 1, 9],), ([-4, -2, -9, -1],)]},
    {"name": "runmod", "sig": "xs",
     "params": lambda r: {"m": r.randint(5, 50)},
     "doc": lambda p: "Return the running sums of the integers in xs, each reduced modulo %(m)d (Python's %% operator)." % p,
     "body": lambda p: ["out, total = [], 0", "for x in xs:", "    total = (total + x) %% %(m)d" % p, "    out.append(total)",
                        "return out"],
     "cases": [([],), ([7],), ([10, 20, 30, 40],), ([-5, 3, -8],)]},
    {"name": "countdiv", "sig": "xs",
     "params": lambda r: {"d": r.randint(2, 9)},
     "doc": lambda p: "Return how many of the integers in xs are divisible by %(d)d (0 counts as divisible)." % p,
     "body": lambda p: ["return sum(1 for x in xs if x %% %(d)d == 0)" % p],
     "cases": [([],), ([0],), ([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12],), ([-9, -6, 11],)]},
    {"name": "strhash", "sig": "s",
     "params": lambda r: {"m": _pick(r, [97, 101, 251, 509, 1009])},
     "doc": lambda p: "Return the sum of (i + 1) * ord(c) over the characters c of s at positions i = 0, 1, 2, ..., reduced "
     "modulo %(m)d (0 for the empty text)." % p,
     "body": lambda p: ["return sum((i + 1) * ord(c) for i, c in enumerate(s)) %% %(m)d" % p],
     "cases": [("",), ("a",), ("klatch",), ("The quick brown fox",)]},
    {"name": "label", "sig": "n",
     "params": lambda r: {"p": r.randint(2, 4), "q": _pick(r, [5, 7]), "w1": _pick(r, ["tick", "bolt", "ping"]),
                          "w2": _pick(r, ["tock", "nut", "pong"])},
     "doc": lambda p: "For an integer n return %(w12)r when n is divisible by both %(p)d and %(q)d, %(w1)r when only by "
     "%(p)d, %(w2)r when only by %(q)d, and str(n) otherwise." % dict(p, w12=p["w1"] + p["w2"]),
     "body": lambda p: ["if n %% %(p)d == 0 and n %% %(q)d == 0:" % p, "    return %(w1)r + %(w2)r" % p, "if n %% %(p)d == 0:" % p,
                        "    return %(w1)r" % p, "if n %% %(q)d == 0:" % p, "    return %(w2)r" % p, "return str(n)"],
     "cases": [(0,), (1,), (3,), (4,), (6,), (10,), (12,), (14,), (35,), (70,)]},
    {"name": "poly", "sig": "x",
     "params": lambda r: {"c0": r.randint(-9, 9), "c1": r.randint(-9, 9), "c2": r.randint(-5, 5)},
     "doc": lambda p: "Return the integer (%(c0)d) + (%(c1)d) * x + (%(c2)d) * x * x for an integer x." % p,
     "body": lambda p: ["return (%(c0)d) + (%(c1)d) * x + (%(c2)d) * x * x" % p],
     "cases": [(0,), (1,), (-1,), (12,), (-7,)]},
    {"name": "longrun", "sig": "s",
     "params": lambda r: {"c": _pick(r, "abcde")},
     "doc": lambda p: "Return the length of the longest run of consecutive %(c)r characters in the text s (0 if there is none)."
     % p,
     "body": lambda p: ["best = run = 0", "for ch in s:", "    if ch == %(c)r:" % p, "        run += 1", "        best = max(best, run)",
                        "    else:", "        run = 0", "return best"],
     "cases": [("",), ("xyz",), ("aabaaabbaa",), ("eeeeeaeee",), ("abcdeabcde",)]},
    {"name": "collatz", "sig": "n",
     "params": lambda r: {"cap": r.randint(5, 40)},
     "doc": lambda p: "For an integer n of at least 1 count the Collatz steps (n becomes n // 2 when even, 3 * n + 1 when odd) "
     "until n reaches 1, but never more than %(cap)d: return min(steps, %(cap)d)." % p,
     "body": lambda p: ["steps = 0", "while n != 1 and steps < %(cap)d:" % p, "    n = n // 2 if n % 2 == 0 else 3 * n + 1",
                        "    steps += 1", "return steps"],
     "cases": [(1,), (2,), (7,), (27,), (97,)]},
    {"name": "bucket", "sig": "x",
     "params": lambda r: {"t": sorted(r.sample(range(-30, 60), r.randint(3, 5)))},
     "doc": lambda p: "Return how many of the thresholds %(t)s are less than or equal to the integer x." % p,
     "body": lambda p: ["return sum(1 for t in %(t)s if t <= x)" % p],
     "cases": [(-100,), (0,), (10,), (30,), (100,)]},
    {"name": "gcdk", "sig": "n",
     "params": lambda r: {"k": r.randint(12, 360)},
     "doc": lambda p: "Return the greatest common divisor of the integer n and %(k)d (for n = 0 that is %(k)d; the result is never "
     "negative)." % p,
     "body": lambda p: ["from math import gcd", "return gcd(n, %(k)d)" % p],
     "cases": [(0,), (1,), (-36,), (360,), (97,)]},
    {"name": "revadd", "sig": "n",
     "params": lambda r: {"k": r.randint(1, 3)},
     "doc": lambda p: "Reverse the decimal digits of abs(n) (so 120 gives 21), multiply that number by %(k)d and add it to n."
     % p,
     "body": lambda p: ["return n + %(k)d * int(str(abs(n))[::-1])" % p],
     "cases": [(0,), (5,), (120,), (-45,), (12345,)]},
]
KIND_BY_NAME = {k["name"]: k for k in KINDS}


def _valid_size(modules):
    if not isinstance(modules, int):
        return False
    return modules >= 2 and modules % 2 == 0 and modules // 2 <= MAX_AGENTS


def _make_modules(modules, seed):
    """One entry per unit: the kinds in a seeded order, repeated with a counter, each with seeded parameters."""
    order = list(range(len(KINDS)))
    random.Random("order:%d" % seed).shuffle(order)
    mods = []
    for i in range(modules):
        kind = KINDS[order[i % len(KINDS)]]
        name = "%s_%d" % (kind["name"], i // len(KINDS) + 1)
        params = kind["params"](random.Random("param:%d:%s" % (seed, name)))
        mods.append({"name": name, "fn": name, "kind": kind["name"], "params": params})
    names = [m["name"] for m in mods]
    if len(set(names)) != len(names):
        raise AssertionError("module names are not unique: %r" % names)
    return mods


def _make_agents(mods):
    """Two modules per agent, in order: agent a has ticket A, and so on; each module learns its ticket and owner."""
    by_name = {m["name"]: m for m in mods}
    agents = []
    for i in range(len(mods) // 2):
        a = {"name": chr(97 + i), "ticket": chr(65 + i), "modules": [mods[2 * i]["name"], mods[2 * i + 1]["name"]]}
        agents.append(a)
        for name in a["modules"]:
            by_name[name].update(ticket=a["ticket"], owner=a["name"])
    return agents


def build(modules, seed):
    """The structure of the project: {"modules": [{"name", "fn", "kind", "params", "ticket", "owner"}], "agents": [{"name",
    "ticket", "modules"}]}. modules must be even (two per agent), from 2 to 52 (26 agents)."""
    if not _valid_size(modules):
        raise ValueError("modules must be an even number from 2 to %d, got %r" % (2 * MAX_AGENTS, modules))
    mods = _make_modules(modules, seed)
    return {"modules": mods, "agents": _make_agents(mods)}


def _kind(m):
    return KIND_BY_NAME[m["kind"]]


def reference_source(m):
    k = _kind(m)
    lines = ['"""%s."""' % m["name"], "", "", "def %s(%s):" % (m["fn"], k["sig"])]
    lines += ["    " + l for l in k["body"](m["params"])]  # a body line carries its own relative indentation
    return "\n".join(lines) + "\n"


def stub_source(m):
    k = _kind(m)
    doc = k["doc"](m["params"])
    return '"""%s: unit of ticket %s."""\n\n\ndef %s(%s):\n    """%s"""\n    raise NotImplementedError\n' % (
        m["name"], m["ticket"], m["fn"], k["sig"], doc)


def _exec_reference(m):
    ns = {}
    exec(compile(reference_source(m), "<reference %s>" % m["name"], "exec"), ns)
    return ns[m["fn"]]


def visible_test_source(m):
    fn = _exec_reference(m)
    cases = [(a, fn(*[x[:] if isinstance(x, list) else x for x in a])) for a in _kind(m)["cases"]]
    return ("import unittest\n\nfrom %s.%s import %s\n\nCASES = %r\n\n\nclass Test%s(unittest.TestCase):\n"
            "    def test_examples(self):\n        for args, want in CASES:\n            with self.subTest(args=args):\n"
            "                self.assertEqual(%s(*args), want)\n") % (
        PACKAGE, m["name"], m["fn"], cases, "".join(w.capitalize() for w in m["name"].split("_")), m["fn"])


REGISTRY_DOC = '''"""%s: small, independent helpers.

Registry. When you finish a module, add one line at the END of this file, exactly:

    from %s.<module> import <function>

naming the public function of that module. Do not reorder or delete other lines: other people add theirs here too.
"""
''' % (PACKAGE, PACKAGE)
CHANGELOG_HEAD = "# Changelog\n\n## Unreleased\n\n"


def registry_line(m):
    return "from %s.%s import %s" % (PACKAGE, m["name"], m["fn"])


def changelog_line(a):
    return "- ticket %s: %s" % (a["ticket"], ", ".join(a["modules"]))


def readme(b):
    lines = ["# %s" % PACKAGE, "", "A small Python package (standard library only) of %d independent helper modules, one small "
             "function each. Run the tests with `python3 -m unittest discover -s tests -t .`. Each function's contract is in "
             "the docstring of its stub:" % len(b["modules"]), ""]
    lines += ["- `%s/%s.py`: `%s(%s)`." % (PACKAGE, m["name"], m["fn"], _kind(m)["sig"]) for m in b["modules"]]
    lines += ["", "Shared files: `%s/__init__.py` is the registry (every function is exported from the package; the file says "
              "how to add yours) and `CHANGELOG.md` has one line per ticket under \"Unreleased\". Everyone who finishes a "
              "ticket edits both." % PACKAGE, ""]
    return "\n".join(lines)


def template_files(modules, seed):
    b = build(modules, seed)
    files = {"%s/__init__.py" % PACKAGE: REGISTRY_DOC, "CHANGELOG.md": CHANGELOG_HEAD, "README.md": readme(b),
             "tests/__init__.py": ""}
    for m in b["modules"]:
        files["%s/%s.py" % (PACKAGE, m["name"])] = stub_source(m)
        files["tests/test_%s.py" % m["name"]] = visible_test_source(m)
    return files


def reference_files(modules, seed):
    """The solution: every module, the full registry and the full changelog."""
    b = build(modules, seed)
    files = {"%s/%s.py" % (PACKAGE, m["name"]): reference_source(m) for m in b["modules"]}
    files["%s/__init__.py" % PACKAGE] = REGISTRY_DOC + "".join(registry_line(m) + "\n" for m in b["modules"])
    files["CHANGELOG.md"] = CHANGELOG_HEAD + "".join(changelog_line(a) + "\n" for a in b["agents"])
    return files


def tasks(modules, seed):
    b = build(modules, seed)
    n = len(b["agents"])
    out = []
    for a in b["agents"]:
        p1, p2 = a["modules"]
        text = (
            "%d tickets are open on the %s repository (a small Python package, standard library only; see README.md). You own "
            "the ticket named %s: implement `%s/%s.py` and `%s/%s.py` so that their tests pass (the visible tests are "
            "`tests/test_%s.py` and `tests/test_%s.py`; each function is described in the docstring of its stub, which raises "
            "NotImplementedError today). Your ticket also has two shared-file changes, which every ticket makes:\n\n"
            "- Registry: add one line at the end of `%s/__init__.py` for each of your two modules, as that file describes.\n"
            "- Changelog: add one line at the end of `CHANGELOG.md` under \"Unreleased\": `%s`.\n\n"
            "Publish your work to main, and make sure main passes the full test run when you finish. Work only on your own "
            "ticket; the other tickets belong to the other agents." % (
                n, PACKAGE, a["ticket"], PACKAGE, p1, PACKAGE, p2, p1, p2, PACKAGE, changelog_line(a)))
        out.append({"name": a["name"], "task": text})
    return out


def scenario_map(modules, seed):
    b = build(modules, seed)
    return {
        "units": {m["name"]: {"path": "%s/%s.py" % (PACKAGE, m["name"])} for m in b["modules"]},
        "agents": {a["name"]: {"ticket": a["ticket"], "units": list(a["modules"])} for a in b["agents"]},
        "shared_files": {
            "%s/__init__.py" % PACKAGE: {"entry": "^from %s\\.[a-z0-9_]+ import \\w" % PACKAGE, "expected_entries": modules},
            "CHANGELOG.md": {"entry": "^- ticket [A-Z]:", "expected_entries": modules // 2}},
    }


def write_files(files, dest):
    for rel, text in files.items():
        path = os.path.join(dest, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)


def fake_script(modules, seed, reference_dir, chain=False):
    """A fake-agent script (fake_agent.py) in which every agent does its ticket correctly: the room join and post in the K
    arms, copy its two modules from the reference, add its registry and changelog lines, sync (resolving a conflicted pull by
    keeping both sides), post, read the room, finish. chain=True makes agent i wait until ticket i-1 is on origin before it
    syncs, so every agent after the first pulls a conflicted registry (a collision at every size). The reference is written
    into reference_dir, which must be outside any checkout."""
    b = build(modules, seed)
    ref = reference_files(modules, seed)
    write_files(ref, reference_dir)
    agents = {}
    for i, a in enumerate(b["agents"]):
        mods = [next(m for m in b["modules"] if m["name"] == n) for n in a["modules"]]
        steps = [{"op": "klatch", "args": ["join", "{{ROOM}}"], "arms": ["K1", "K2"]},
                 {"op": "klatch", "args": ["post", "{{ROOM}}", "working: ticket %s (%s)" % (a["ticket"], ", ".join(a["modules"]))],
                  "arms": ["K1", "K2"]}]
        for m in mods:
            steps.append({"op": "copy_from", "from": os.path.join(reference_dir, PACKAGE, m["name"] + ".py"),
                          "path": "%s/%s.py" % (PACKAGE, m["name"])})
        for m in mods:
            steps.append({"op": "append", "path": "%s/__init__.py" % PACKAGE, "content": registry_line(m) + "\n"})
        steps.append({"op": "append", "path": "CHANGELOG.md", "content": changelog_line(a) + "\n"})
        if chain and i > 0:
            steps.append({"op": "wait_origin_has", "path": "CHANGELOG.md", "contains": "- ticket %s:" % chr(64 + i), "timeout": 60})
        steps += [{"op": "sync", "message": "ticket %s: %s" % (a["ticket"], ", ".join(a["modules"])), "resolve": "union",
                   "retries": 80},
                  {"op": "klatch", "args": ["post", "{{ROOM}}", "finished: ticket %s" % a["ticket"]], "arms": ["K1", "K2"]},
                  {"op": "klatch", "args": ["read", "{{ROOM}}"], "arms": ["K1", "K2"]},
                  {"op": "final", "text": "ticket %s is published to main. DONE %s" % (a["ticket"], a["name"])}]
        agents[a["name"]] = steps
    return {"comment": "Scripted agents for the S7 tests (#1372), generated by gen.py: each copies the reference solution of its "
                       "two modules, adds its registry and changelog lines and syncs.", "agents": agents}


def _parser():
    p = argparse.ArgumentParser(description="Generate fixture F7 scale (#1372).")
    p.add_argument("command", choices=("write", "reference", "tasks", "map", "fake"))
    p.add_argument("dest", nargs="?")
    p.add_argument("--modules", type=int, required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--reference", help="fake: the directory the reference is written to")
    p.add_argument("--chain", action="store_true")
    return p


def _write_command(a, p):
    if not a.dest:
        p.error("%s needs DEST" % a.command)
    os.makedirs(a.dest, exist_ok=True)
    write_files(template_files(a.modules, a.seed) if a.command == "write" else reference_files(a.modules, a.seed), a.dest)


def _fake_command(a, p):
    if not a.reference:
        p.error("fake needs --reference DIR")
    print(json.dumps(fake_script(a.modules, a.seed, a.reference, a.chain), indent=1))


def _run(a, p):
    if a.command in ("write", "reference"):
        _write_command(a, p)
    elif a.command == "tasks":
        print(json.dumps(tasks(a.modules, a.seed), indent=1, sort_keys=True))
    elif a.command == "map":
        print(json.dumps(scenario_map(a.modules, a.seed), indent=1, sort_keys=True))
    else:
        _fake_command(a, p)


def main(argv=None):
    p = _parser()
    a = p.parse_args(argv)
    try:
        _run(a, p)
    except ValueError as e:
        print("gen.py: %s" % e, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
