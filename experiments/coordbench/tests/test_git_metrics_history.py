"""git_metrics must see every commit that touched a unit, including one that a merge makes invisible to a path-limited
`git log` (#1471): history simplification drops a side branch when the merge result equals the other side, so two agents
who each wrote the same module could count as one author, depending on who pushed first."""
import os
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import scenario

ENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}


def git(cwd, *args, author="bench"):
    env = dict(ENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@x.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@x.invalid")
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    assert p.returncode == 0, p.stdout
    return p.stdout


SCN = {"agents": {"a": {"units": ["u"]}, "b": {"units": []}}, "units": {"u": {"path": "m.py"}}, "shared_files": {}}


class TestHiddenSideBranch(unittest.TestCase):
    def test_two_authors_of_the_same_file_are_both_counted_whatever_the_merge_order(self):
        with tempfile.TemporaryDirectory() as d:
            git(d, "init", "-q", "-b", "main")
            with open(os.path.join(d, "m.py"), "w") as f:
                f.write("stub\n")
            git(d, "add", "-A")
            git(d, "commit", "-q", "-m", "initial")
            git(d, "checkout", "-q", "-b", "side")
            with open(os.path.join(d, "m.py"), "w") as f:
                f.write("done\n")
            git(d, "commit", "-qam", "b writes it", author="b")
            git(d, "checkout", "-q", "main")
            with open(os.path.join(d, "m.py"), "w") as f:
                f.write("done\n")
            git(d, "commit", "-qam", "a writes it too", author="a")
            git(d, "merge", "-q", "--no-edit", "side")  # identical content: the merge equals both parents
            git(d, "branch", "-q", "-D", "side")  # an origin has main only: b's commit is reachable through the merge alone
            gm = scenario.git_metrics(SCN, d, [], ENV)
            self.assertEqual(gm["duplicate_work"], 1, gm)
            self.assertEqual(gm["details"]["duplicated_units"], ["u"])


if __name__ == "__main__":
    unittest.main()
