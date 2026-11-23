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


LSB_RANGES = '''"""wrong numbering: index 0 is the least significant bit"""


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
        mask |= 1 << n
    return mask
'''


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

    def test_the_numbering_is_in_the_repository_and_not_in_a_visible_test_or_a_task_text(self):
        with open(os.path.join(FIXTURE, "template", "docs", "FORMATS.md")) as f:
            self.assertIn("most significant bit", f.read())
        for d, _, names in os.walk(os.path.join(FIXTURE, "template", "tests")):
            for n in names:
                with open(os.path.join(d, n)) as f:
                    t = f.read()
                self.assertNotIn("0x8000", t, n)
        for name in ("task-a.md", "task-b.md", "task-c.md"):
            with open(os.path.join(FIXTURE, name)) as f:
                self.assertNotIn("most significant", f.read(), name)


if __name__ == "__main__":
    unittest.main()
