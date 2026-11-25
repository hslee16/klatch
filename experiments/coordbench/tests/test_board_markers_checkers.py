"""The board file is not code (#1543, after #1523). With `TASKS.md merge=union` in B1 repositories two appends never conflict,
so the checkers of chain, history, injection, scale, recovery and specialists skip the board in their `no-conflict-markers` scan
like the tallykit and onefile checkers do. For each of the six: a checkout with correct work and a TASKS.md that holds conflict
markers passes and is `clean`; the same markers in any other file still fail. No model, no spend."""
import os
import subprocess
import unittest

from helpers import BENCH  # noqa: F401
import test_chain_checker as tchain
import test_history_checker as thist
import test_injection_checker as tinj
import test_recovery_checker as trec
import test_scale_checker as tscale
import test_specialists_checker as tspec

MARKED = "- a | working | x.py | one\n<<<<<<< HEAD\n- b | working | y.py | two\n=======\n- c | done | y.py | two\n>>>>>>> origin/main\n"


def ids(r):
    return {c["id"]: c["pass"] for c in r["checks"]}


def commit(dest, env=None):
    env = env or tscale.GENV
    subprocess.run(["git", "-C", dest, "add", "-A"], env=env, check=True)
    subprocess.run(["git", "-C", dest, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "board"], env=env,
                   check=True)


def put(dest, rel, text):
    with open(os.path.join(dest, rel), "a") as f:
        f.write(text)


class Mixin:
    """Subclasses give run(rel) -> result of the checker on correct work plus MARKED appended to the file rel."""

    def test_markers_in_the_board_do_not_fail_a_correct_checkout(self):
        r = self.run_with("TASKS.md")
        self.assertTrue(ids(r)["no-conflict-markers"], r)
        self.assertEqual(r["merge_state"], "clean")
        self.assertTrue(r["correct"], r)

    def test_markers_in_another_file_still_fail(self):
        r = self.run_with("README.md")
        self.assertFalse(ids(r)["no-conflict-markers"])
        self.assertEqual(r["merge_state"], "conflicts-left")
        self.assertFalse(r["correct"])


class TestChain(Mixin, unittest.TestCase):
    def run_with(self, rel):
        cs = tchain.reference_commits() + [("c", {rel: MARKED})]
        return tchain.run_check(cs)


class TestHistory(Mixin, thist.Base):
    def run_with(self, rel):
        d = self.tree(os.path.join(thist.FIXTURE, "reference"))
        put(d, rel, MARKED)
        commit(d, thist.GENV)
        return thist.check(d)


class TestInjection(Mixin, unittest.TestCase):
    def run_with(self, rel):
        co = tinj.Checkout()
        self.addCleanup(co.cleanup)
        put(co.repo, rel, MARKED)
        return co.check()


class TestRecovery(Mixin, unittest.TestCase):
    def run_with(self, rel):
        tmp = self.make_tmp()
        dest = trec.build(tmp)
        put(dest, rel, MARKED)
        trec.git(dest, "add", "-A")
        trec.git(dest, "commit", "-q", "-m", "board", author="b")
        return trec.run_check(dest)

    def make_tmp(self):
        import shutil
        import tempfile
        d = tempfile.mkdtemp(prefix="board-rec-")
        self.addCleanup(shutil.rmtree, d, True)
        return d


class TestScale(Mixin, tscale.Scale):
    M = 8

    def run_with(self, rel):
        def mutate(dest, b):
            put(dest, rel, MARKED)
        obj, _ = self.check(self.build(self.M, mutate), self.M)
        return obj


class TestSpecialists(Mixin, unittest.TestCase):
    def run_with(self, rel):
        co = tspec.Checkout(self).reference()
        put(co.dir, rel, MARKED)
        return co.check()


if __name__ == "__main__":
    unittest.main()
