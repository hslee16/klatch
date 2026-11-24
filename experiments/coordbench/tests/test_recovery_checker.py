"""The hidden checker of fixture F6 `recovery` (#1371, scenario S6) against the reference solution and broken variants.

Each variant is a git repository built from the fixture's setup.sh plus the reference, with commits by the authors a, b and
c (c is the victim), then one thing wrong: the victim's module never recovered, the orphaned ask unanswered, the recovery
done twice, a conflicted merge, a live agent's module changed by someone else.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401

FIXTURE = os.path.join(BENCH, "fixtures", "recovery")
CHECK = os.path.join(BENCH, "checks", "recovery", "check.sh")
CHECK_PY = os.path.join(BENCH, "checks", "recovery", "check.py")
REF = os.path.join(FIXTURE, "reference", "recoverkit")
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull}


def git(cwd, *args, author="bench"):
    env = dict(GENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@bench.invalid")
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), p.stdout))
    return p.stdout


def ref(name, dest):
    shutil.copy(os.path.join(REF, name + ".py"), dest)


def build(tmp, steps=None):
    """The initial tree, then each (author, {module: source file or text}) step as one commit. Default: a does ranges, b does
    wrap and pairs, b recovers bits (the reference run)."""
    dest = os.path.join(tmp, "co")
    os.makedirs(dest)
    subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=GENV)
    git(dest, "init", "-q", "-b", "main")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "initial")
    steps = steps if steps is not None else [("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "ref"})]
    for author, mods in steps:
        for m, src in mods.items():
            path = os.path.join(dest, "recoverkit", m + ".py")
            if src == "ref":
                ref(m, path)
            elif src == "ref-again":  # the same solution written a second time, so the commit has a change in it
                ref(m, path)
                with open(path, "a") as f:
                    f.write("\n# written again by %s\n" % author)
            else:
                with open(path, "w") as f:
                    f.write(src)
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "--allow-empty", "-m", "%s work" % author, author=author)
    return dest


def run_check(dest):
    p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout.strip().splitlines()[-1])


def passed(out):
    return {c["id"]: c["pass"] for c in out["checks"]}


def ranges_with(position_expr):
    """A ranges module whose to_mask maps index n to the bit position given by position_expr (a Python expression of n)."""
    return '''"""a guess at the index map"""


def compress(numbers):
    out = []
    for n in sorted(set(numbers)):
        if out and out[-1][1] == n - 1:
            out[-1] = (out[-1][0], n)
        else:
            out.append((n, n))
    return out


def expand(ranges):
    seen = set()
    for lo, hi in ranges:
        if hi < lo:
            raise ValueError("reversed")
        seen.update(range(lo, hi + 1))
    return sorted(seen)


def to_mask(ranges):
    mask = 0
    for n in expand(ranges):
        if not 0 <= n <= 15:
            raise ValueError("out of range")
        mask |= 1 << (%s)
    return mask
''' % position_expr


FACTS = json.load(open(os.path.join(BENCH, "checks", "recovery", "facts.json")))
LSB_RANGES = ranges_with("n")
MSB_RANGES = ranges_with("15 - n")
STALE_RANGES = ranges_with("(n * %d + %d) %% 16" % (FACTS["stale"]["K"], FACTS["stale"]["R"]))


class TestChecker(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="recovery-check-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def check(self, steps=None):
        return run_check(build(self.tmp, steps))

    def test_the_reference_solution_passes_everything(self):
        out = self.check()
        self.assertEqual(out["units_completed"], 4, out)
        self.assertEqual(out["units_total"], 4)
        self.assertTrue(all(passed(out).values()), out)
        self.assertTrue(out["correct"])
        self.assertTrue(out["recovered"])
        self.assertEqual(out["recovered_by"], ["b"])
        self.assertTrue(out["ask_answered"])
        self.assertEqual(out["merge_state"], "clean")
        self.assertEqual(out["duplicates"], [])
        self.assertEqual(out["per_agent"]["b"]["modules"], ["bits", "pairs", "wrap"])

    def test_the_four_unit_checks_are_the_only_units(self):
        out = self.check()
        units = [c["id"] for c in out["checks"] if c.get("unit") is not False]
        self.assertEqual(units, ["ranges", "wrap", "pairs", "bits"])

    def test_the_victims_module_never_recovered(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"})])
        p = passed(out)
        self.assertEqual(out["units_completed"], 3)
        self.assertFalse(p["bits"])
        self.assertFalse(p["recovered"])
        self.assertFalse(out["correct"])
        self.assertTrue(p["ask-answered"], "a's own mask is right even though c's module is not")

    def test_the_victim_finishing_its_own_module_is_not_a_recovery(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("c", {"bits": "ref"})])
        p = passed(out)
        self.assertTrue(p["bits"])
        self.assertFalse(p["recovered"], "no survivor changed the module")
        self.assertEqual(out["recovered_by"], [])
        self.assertFalse(out["correct"])

    def test_a_victim_that_pushed_a_stub_and_a_survivor_that_finished_it_is_a_recovery(self):
        stub = '"""stub"""\n\n\ndef set_bit(mask, index):\n    return mask\n'
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("c", {"bits": stub}),
                          ("a", {"bits": "ref"})])
        self.assertTrue(passed(out)["recovered"])
        self.assertTrue(passed(out)["no-duplicate-work"])
        self.assertEqual(out["recovered_by"], ["a"])

    def test_the_orphaned_ask_unanswered_a_guesses_the_numbering(self):
        out = self.check([("a", {"ranges": LSB_RANGES}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "ref"})])
        p = passed(out)
        self.assertFalse(p["ask-answered"])
        self.assertFalse(p["ranges"], "the mask is part of the module")
        self.assertTrue(p["bits"] and p["recovered"])
        self.assertEqual(out["units_completed"], 3)
        self.assertFalse(out["correct"])

    def test_the_orphaned_ask_answered_wrongly_with_the_repository_docs_value_or_the_stale_value(self):
        for name, src in (("msb legacy value in docs/FORMATS.md", MSB_RANGES), ("superseded revision", STALE_RANGES)):
            sub = os.path.join(self.tmp, name[:4])
            os.makedirs(sub)
            out = run_check(build(sub, [("a", {"ranges": src}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "ref"})]))
            self.assertFalse(passed(out)["ask-answered"], name)
            self.assertFalse(passed(out)["ranges"], name)

    def test_the_seeded_value_passes_and_is_the_only_one_that_does(self):
        good = ranges_with("(n * %d + %d) %% 16" % (FACTS["K"], FACTS["R"]))
        out = self.check([("a", {"ranges": good}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "ref"})])
        self.assertTrue(passed(out)["ask-answered"])
        self.assertTrue(passed(out)["ranges"])

    def test_the_victims_module_is_checked_without_the_private_map(self):
        # recovering bits needs only the repository: the module check has no use for the seeded values
        import re
        with open(os.path.join(REF, "bits.py")) as f:
            src = f.read()
        self.assertNotIn(str(FACTS["K"]), re.findall(r"\d+", src))
        self.assertNotIn(str(FACTS["R"]), re.findall(r"\d+", src))
        p = subprocess.run(["python3", "-I", CHECK_PY, "--module", "bits", os.path.join(REF, "bits.py")], stdout=subprocess.PIPE, text=True)
        self.assertTrue(json.loads(p.stdout)["ok"])
        with open(os.path.join(FIXTURE, "template", "docs", "FORMATS.md")) as f:
            text = f.read()
        self.assertIn("position 0 is the least significant bit", text)

    def test_duplicated_work_two_survivors_both_recover_the_module(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "ref"}),
                          ("a", {"bits": "ref-again"})])
        p = passed(out)
        self.assertEqual(out["units_completed"], 4)
        self.assertFalse(p["no-duplicate-work"])
        self.assertEqual(out["duplicates"][0]["module"], "bits")
        self.assertEqual(sorted(out["duplicates"][0]["authors"]), ["a", "b"])
        self.assertFalse(out["correct"])

    def test_a_live_agents_module_changed_by_someone_else_is_foreign_work(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"ranges": "ref-again"}),
                          ("b", {"bits": "ref"})])
        p = passed(out)
        self.assertFalse(p["no-duplicate-work"])
        self.assertIn("ranges", [d["module"] for d in out["duplicates"]])

    def test_a_conflicted_merge_is_reported(self):
        dest = build(self.tmp)
        with open(os.path.join(dest, "recoverkit", "wrap.py"), "a") as f:
            f.write("<<<<<<< HEAD\nx = 1\n=======\nx = 2\n>>>>>>> other\n")
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "-m", "oops", author="b")
        out = run_check(dest)
        self.assertEqual(out["merge_state"], "conflicts-left")
        self.assertFalse(passed(out)["no-conflict-markers"])
        self.assertFalse(out["correct"])

    def test_broken_visible_tests_are_reported(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}),
                          ("b", {"bits": "ref"}), ("b", {"wrap": "def wrap(t, w):\n    raise SystemExit(3)\n"})])
        self.assertEqual(out["merge_state"], "tests-broken")
        self.assertFalse(passed(out)["visible-tests"])

    def test_nothing_done_scores_zero(self):
        out = self.check([])
        self.assertEqual(out["units_completed"], 0)
        self.assertFalse(out["correct"])
        self.assertFalse(out["recovered"])
        self.assertFalse(out["ask_answered"])

    def test_the_module_mode_tests_one_file_and_the_stubs_fail(self):
        def mod(name, path):
            p = subprocess.run(["python3", "-I", CHECK_PY, "--module", name, path], stdout=subprocess.PIPE, text=True)
            return json.loads(p.stdout)["ok"]
        for name in ("ranges", "wrap", "pairs", "bits"):
            self.assertTrue(mod(name, os.path.join(REF, name + ".py")), name)
            self.assertFalse(mod(name, os.path.join(FIXTURE, "template", "recoverkit", name + ".py")), name)

    def test_a_bad_module_that_raises_at_import_fails_instead_of_crashing_the_checker(self):
        out = self.check([("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": "this is not python("})])
        self.assertFalse(passed(out)["bits"])
        self.assertEqual(out["units_completed"], 3)


BITS_REF = open(os.path.join(REF, "bits.py")).read()

BITS_SHIFT = '''"""Bit operations by shifting; test_bit returns the 0 or 1 of the shift, not a bool."""


def _chk(index):
    if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index <= 15:
        raise ValueError(index)
    return index


def set_bit(mask, index):
    return mask | (1 << _chk(index))


def clear_bit(mask, index):
    return (mask & ~(1 << _chk(index))) & 0xFFFF


def test_bit(mask, index):
    return (mask >> _chk(index)) & 1


def popcount(mask):
    n = 0
    mask &= 0xFFFF
    while mask:
        mask &= mask - 1
        n += 1
    return n


def bit_indexes(mask):
    return [i for i in range(16) if (mask >> i) & 1]
'''


class TestBitsAlternatives(unittest.TestCase):
    """Structurally different correct bits modules and gaming attempts (the independent review of #1461, kept as regression
    tests, #1525). The module is the victim's unit and is tested on its own."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="recovery-alt-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def bits(self, text):
        out = run_check(build(self.tmp, [("a", {"ranges": "ref"}), ("b", {"wrap": "ref", "pairs": "ref"}), ("b", {"bits": text})]))
        return out, passed(out)

    def test_a_test_bit_that_returns_the_masked_int_passes_and_recovers(self):
        out, p = self.bits(BITS_REF.replace("return bool(mask & _bit(index))", "return mask & _bit(index)"))
        self.assertTrue(p["bits"], "a truthy int is a right answer for test_bit")
        self.assertTrue(p["recovered"])
        self.assertEqual(out["units_completed"], 4)
        self.assertTrue(out["correct"])

    def test_a_test_bit_that_returns_zero_or_one_passes(self):
        out, p = self.bits(BITS_SHIFT)
        self.assertTrue(p["bits"])
        self.assertTrue(out["correct"])

    def test_a_test_bit_that_returns_a_bool_still_passes(self):
        out, p = self.bits(BITS_REF)
        self.assertTrue(p["bits"])

    def test_a_loop_based_popcount_and_a_comprehension_for_bit_indexes_pass(self):
        out, p = self.bits(BITS_SHIFT.replace("return (mask >> _chk(index)) & 1", "return bool((mask >> _chk(index)) & 1)"))
        self.assertTrue(p["bits"])

    def test_a_test_bit_with_the_wrong_sense_fails(self):
        out, p = self.bits(BITS_SHIFT.replace("& 1\n\n\ndef popcount", "^ 1\n\n\ndef popcount"))
        self.assertFalse(p["bits"], "the inverted answer is wrong whatever its type")
        self.assertFalse(out["correct"])

    def test_a_test_bit_that_is_always_true_fails(self):
        out, p = self.bits(BITS_REF.replace("return bool(mask & _bit(index))", "_bit(index)\n    return True"))
        self.assertFalse(p["bits"])

    def test_a_test_bit_that_accepts_an_out_of_range_position_fails(self):
        out, p = self.bits(BITS_REF.replace("return bool(mask & _bit(index))", "return bool(mask & (1 << index))"))
        self.assertFalse(p["bits"], "the contract raises ValueError outside 0 to 15")

    def test_hard_coded_answers_for_the_visible_examples_fail(self):
        stub = ("def set_bit(mask, index):\n    return {(0, 0): 1, (0, 15): 0x8000}.get((mask, index), mask)\n"
                "def clear_bit(mask, index):\n    return mask\n\ndef test_bit(mask, index):\n    return True\n"
                "def popcount(mask):\n    return 0\n\ndef bit_indexes(mask):\n    return []\n")
        out, p = self.bits(stub)
        self.assertFalse(p["bits"])


class TestFixture(unittest.TestCase):
    def test_setup_copies_the_template_and_never_the_reference_or_the_checker(self):
        with tempfile.TemporaryDirectory() as d:
            dest = os.path.join(d, "w")
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "7"], check=True)
            seen = set()
            for root, _, names in os.walk(dest):
                for n in names:
                    seen.add(os.path.relpath(os.path.join(root, n), dest))
        self.assertIn("docs/FORMATS.md", seen)
        self.assertIn("recoverkit/bits.py", seen)
        self.assertFalse([s for s in seen if s.startswith("reference") or "check" in s], seen)

    def test_the_stubs_fail_the_visible_tests_and_the_reference_passes_them(self):
        with tempfile.TemporaryDirectory() as d:
            dest = os.path.join(d, "w")
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True)

            def visible():
                return subprocess.run(["python3", "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=dest,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode
            self.assertNotEqual(visible(), 0)
            for name in ("ranges", "wrap", "pairs", "bits"):
                ref(name, os.path.join(dest, "recoverkit", name + ".py"))
            self.assertEqual(visible(), 0)

    def private_texts(self):
        """{agent: [text of each private file]} for the agents that have any."""
        holders = {}
        for agent in ("a", "b", "c"):
            for d, _, names in os.walk(os.path.join(FIXTURE, "private", agent)):
                for n in names:
                    with open(os.path.join(d, n)) as f:
                        holders.setdefault(agent, []).append(f.read())
        return holders

    def repository_files(self):
        """(relative path, text) of every fixture file an agent could be given (not private, reference, fake or compiled)."""
        for d, _, names in os.walk(FIXTURE):
            for n in names:
                rel = os.path.relpath(os.path.join(d, n), FIXTURE)
                if rel.startswith(("private", "reference", "fake")) or n.endswith(".pyc") or "__pycache__" in rel:
                    continue
                with open(os.path.join(d, n)) as f:
                    yield rel, f.read()

    def test_the_index_map_is_in_private_files_only_and_the_repository_value_is_a_stale_guess(self):
        K, R = FACTS["K"], FACTS["R"]
        want = "position = (index * %d + %d) mod 16" % (K, R)
        holders = self.private_texts()
        self.assertNotIn("a", holders, "the asker has no private file")
        self.assertTrue(any(want in t for t in holders["c"]), "the owner holds it")
        self.assertTrue(any(want in t for t in holders["b"]), "the deputy holds a copy")
        for rel, text in self.repository_files():
            self.assertNotIn(want, text, rel)
        with open(os.path.join(FIXTURE, "template", "docs", "FORMATS.md")) as f:
            doc = f.read()
        self.assertIn("15 - index", doc)
        self.assertIn("no longer used", doc)
        for name in ("task-a.md", "task-b.md", "task-c.md"):
            with open(os.path.join(FIXTURE, name)) as f:
                self.assertNotRegex(f.read(), r"\* ?%d" % K, name)

    def test_the_seeded_map_is_not_a_conventional_default_nor_its_own_stale_value(self):
        K, R = FACTS["K"], FACTS["R"]
        self.assertEqual(K % 2, 1, "a bijection on 16 positions")
        self.assertNotIn([K, R], FACTS["plausible"])
        self.assertNotEqual((K, R), (FACTS["stale"]["K"], FACTS["stale"]["R"]))
        self.assertEqual(FACTS["seed"], 20261123)
        for default in ([1, 0], [15, 15]):
            self.assertIn(default, FACTS["plausible"])

    def test_the_facts_are_reproducible_from_the_seed(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("gen_facts", os.path.join(FIXTURE, "gen_facts.py"))
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        for rel, text in gen.generate().items():
            with open(os.path.join(BENCH, rel)) as f:
                self.assertEqual(f.read(), text, rel)
        p = subprocess.run(["python3", "-I", os.path.join(FIXTURE, "gen_facts.py"), "--check"], stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)


if __name__ == "__main__":
    unittest.main()
