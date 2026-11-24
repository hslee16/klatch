"""#1524: B1's claim conflicts, computed from the task board TASKS.md (SPEC section 5: a board line already `working` for the
same file when another agent adds its own). Real git repositories in a temp directory; no model."""
import os
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import board

ENV = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")


def git(cwd, *args, author="bench"):
    env = dict(ENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@x", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@x")
    p = subprocess.run(["git", "-C", cwd] + list(args), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    if p.returncode != 0:
        raise AssertionError("git %s: %s" % (" ".join(args), p.stdout))
    return p.stdout


class Parse(unittest.TestCase):
    def test_lines_are_name_status_files_title(self):
        rows = board.parse_board("# board\n- a | working | x.py, y.py | do x\n- b | done | z.py | did z\nnot a line\n")
        self.assertEqual([(r["name"], r["status"], r["files"]) for r in rows],
                         [("a", "working", ["x.py", "y.py"]), ("b", "done", ["z.py"])])

    def test_a_malformed_line_is_skipped_not_an_error(self):
        self.assertEqual(board.parse_board("- a | working\n- | | |\n"), [])


class Conflicts(unittest.TestCase):
    def test_two_agents_working_on_the_same_file_conflict_once(self):
        v = ["- a | working | x.py | t\n", "- a | working | x.py | t\n- b | working | x.py | u\n",
             "- a | working | x.py | t\n- b | working | x.py | u\n- c | done | q.py | v\n"]
        got = board.conflicts(v)
        self.assertEqual(got, [{"file": "x.py", "agents": ["a", "b"]}])

    def test_a_finished_claim_does_not_conflict_with_a_later_one(self):
        self.assertEqual(board.conflicts(["- a | working | x.py | t\n", "- a | done | x.py | t\n",
                                          "- a | done | x.py | t\n- b | working | x.py | u\n"]), [])

    def test_overlapping_file_lists_conflict_on_the_shared_file(self):
        got = board.conflicts(["- a | working | x.py, y.py | t\n- b | working | y.py | u\n"])
        self.assertEqual(got, [{"file": "y.py", "agents": ["a", "b"]}])

    def test_one_agent_with_two_lines_on_a_file_is_not_a_conflict(self):
        self.assertEqual(board.conflicts(["- a | working | x.py | t\n- a | working | x.py | again\n"]), [])

    def test_different_files_do_not_conflict(self):
        self.assertEqual(board.conflicts(["- a | working | x.py | t\n- b | working | y.py | u\n"]), [])

    def test_the_same_pair_on_the_same_file_in_many_versions_counts_once(self):
        line = "- a | working | x.py | t\n- b | working | x.py | u\n"
        self.assertEqual(len(board.conflicts([line, line + "- c | done | z.py | w\n"])), 1)


class FromGit(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="coordbench-board-")
        self.addCleanup(shutil.rmtree, self.d, True)
        self.origin = os.path.join(self.d, "origin.git")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.origin], env=ENV, check=True)
        self.a = self.clone("a")
        with open(os.path.join(self.a, "TASKS.md"), "w") as f:
            f.write("# board\n")
        git(self.a, "add", ".")
        git(self.a, "commit", "-qm", "seed")
        git(self.a, "push", "-q", "origin", "main")
        self.b = self.clone("b")

    def clone(self, name):
        p = os.path.join(self.d, name)
        subprocess.run(["git", "clone", "-q", self.origin, p], env=ENV, check=True, stderr=subprocess.DEVNULL)
        return p

    def claim(self, repo, who, line, push=True):
        with open(os.path.join(repo, "TASKS.md"), "a") as f:
            f.write(line + "\n")
        git(repo, "add", "TASKS.md")
        git(repo, "commit", "-qm", "claim", author=who)

    def test_two_unpulled_claims_that_meet_in_a_merge_are_a_conflict(self):
        self.claim(self.a, "a", "- a | working | x.py | t")
        self.claim(self.b, "b", "- b | working | x.py | u")
        git(self.a, "push", "-q", "origin", "main")
        with open(os.path.join(self.b, ".git", "info", "attributes"), "w") as f:
            f.write("TASKS.md merge=union\n")  # what a careful team would set (#1461 I1)
        git(self.b, "pull", "-q", "--no-rebase", "--no-edit", "origin", "main")
        got = board.claim_conflicts([self.origin, self.a, self.b], ENV)
        self.assertEqual(got["claim_conflicts"], 1)
        self.assertEqual(got["detail"], [{"file": "x.py", "agents": ["a", "b"]}])

    def test_a_claim_on_a_file_the_other_already_finished_is_clean(self):
        self.claim(self.a, "a", "- a | working | x.py | t")
        with open(os.path.join(self.a, "TASKS.md")) as f:
            text = f.read().replace("- a | working |", "- a | done |")  # the agent edits its own line in place
        with open(os.path.join(self.a, "TASKS.md"), "w") as f:
            f.write(text)
        git(self.a, "commit", "-qam", "done", author="a")
        git(self.a, "push", "-q", "origin", "main")
        git(self.b, "pull", "-q", "origin", "main")
        self.claim(self.b, "b", "- b | working | x.py | u")
        self.assertEqual(board.claim_conflicts([self.origin, self.a, self.b], ENV)["claim_conflicts"], 0)

    def test_a_repository_without_a_board_gives_zero_not_an_error(self):
        empty = os.path.join(self.d, "empty")
        subprocess.run(["git", "init", "-q", empty], env=ENV, check=True)
        self.assertEqual(board.claim_conflicts([empty], ENV)["claim_conflicts"], 0)

    def test_a_missing_repository_is_a_reported_failure(self):
        with self.assertRaises(RuntimeError):
            board.claim_conflicts([os.path.join(self.d, "nope")], ENV)


if __name__ == "__main__":
    unittest.main()
