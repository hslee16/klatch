"""The B1 arm text and fixtures after the independent review (#1523): the board file merges without manufactured conflicts, the
checkers do not read the board for conflict markers, the waiting sentence is the same in every arm, and B1 agents record answers
where others find them. No model, no spend."""
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import runprompt
from test_messaging_verified import Harness, ProbeDriver

ARMS = os.path.join(BENCH, "arms")
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_TERMINAL_PROMPT": "0", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t.invalid", "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@t.invalid"}
WAIT_SENTENCE = ("Do not wait more than 5 minutes in total for any one result; if it has not come by then, do the part that "
                 "you can do without it and go on.")
UNION = "TASKS.md merge=union"


def git(cwd, *args):
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=GENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return p.returncode, p.stdout


def read(path):
    with open(path) as f:
        return f.read()


def write(path, text, mode="w"):
    with open(path, mode) as f:
        f.write(text)


def stripped(arm):
    return re.sub(r"<!--.*?-->", "", read(os.path.join(ARMS, arm + ".md")), flags=re.S)


class Waiting(unittest.TestCase):
    def test_every_arm_ends_its_waiting_paragraph_with_the_same_limit_in_minutes(self):
        for arm in ("B0", "B1", "K1", "K2", "B0-shared", "B1-shared", "K1-shared", "K2-shared"):
            text = " ".join(stripped(arm).split())
            self.assertEqual(text.count(WAIT_SENTENCE), 1, arm)
            para = [p for p in stripped(arm).split("\n\n") if p.startswith("Waiting:")][0]
            self.assertTrue(" ".join(para.split()).endswith(WAIT_SENTENCE), arm)
            self.assertNotIn("a few minutes", text, arm)

    def test_the_sentence_is_in_the_prompt_every_agent_gets(self):
        for arm in ("B0", "B1", "K1", "K2"):
            prompt = runprompt.build_prompt(ARMS, arm, "a", ["a", "b"], "T", "/w", "pytest", "/k/klatch", "lobby", 30, 10)
            self.assertIn(WAIT_SENTENCE, " ".join(prompt.split()), arm)

    def test_nothing_else_in_the_waiting_paragraphs_gives_one_arm_a_time_limit(self):
        for arm in ("B0", "B1", "K1", "K2"):
            para = " ".join([p for p in stripped(arm).split("\n\n") if p.startswith("Waiting:")][0].split())
            self.assertEqual(len(re.findall(r"\b\d+ (?:minutes?|seconds?)\b", para.replace(WAIT_SENTENCE, ""))), 0, para)


class Answers(unittest.TestCase):
    def test_b1_tells_agents_to_append_answers_to_the_board(self):
        for arm in ("B1", "B1-shared"):
            text = " ".join(stripped(arm).split())
            self.assertIn("`answer: FACT (from NAME)`", text, arm)
            i = text.index("answer: FACT (from NAME)")
            self.assertIn("`TASKS.md`", text[i:i + 80], arm)

    def test_no_other_arm_mentions_the_board_answer_line(self):
        for arm in ("B0", "K1", "K2", "K1-shared", "K2-shared", "B0-shared"):
            self.assertNotIn("FACT (from NAME)", stripped(arm), arm)

    def test_the_arm_texts_keep_parity(self):
        p = subprocess.run([sys.executable, os.path.join(ARMS, "check_parity.py")], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)


class Board(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-board-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def origin_with(self, attrs):
        seed = os.path.join(self.tmp, "seed")
        os.makedirs(seed)
        write(os.path.join(seed, "TASKS.md"), "# Task board\n")
        if attrs:
            write(os.path.join(seed, ".gitattributes"), UNION + "\n")
        git(seed, "init", "-q", "-b", "main")
        git(seed, "add", "-A")
        git(seed, "commit", "-q", "-m", "initial")
        origin = os.path.join(self.tmp, "origin.git")
        git(self.tmp, "clone", "-q", "--bare", seed, origin)
        return origin

    def appends_conflict(self, attrs):
        origin = self.origin_with(attrs)
        clones = {}
        for n in ("a", "b"):
            clones[n] = os.path.join(self.tmp, n)
            git(self.tmp, "clone", "-q", origin, clones[n])
            write(os.path.join(clones[n], "TASKS.md"), "- %s | working | f.py | piece %s\n" % (n, n), "a")
            git(clones[n], "commit", "-q", "-am", "board " + n)
        self.assertEqual(git(clones["a"], "push", "-q", "origin", "main")[0], 0)
        code, out = git(clones["b"], "pull", "-q", "--no-rebase", "origin", "main")
        return code != 0 or "CONFLICT" in out, read(os.path.join(clones["b"], "TASKS.md"))

    def test_without_the_union_attribute_two_appends_conflict_the_premise_of_the_ticket(self):
        conflicts, _ = self.appends_conflict(False)
        self.assertTrue(conflicts)

    def test_with_the_union_attribute_two_appends_merge_and_keep_both_lines(self):
        conflicts, text = self.appends_conflict(True)
        self.assertFalse(conflicts, text)
        self.assertIn("- a | working", text)
        self.assertIn("- b | working", text)
        self.assertNotIn("<<<<<<<", text)


class Seeded(Harness):
    def test_the_harness_seeds_the_attribute_in_b1_clones_only(self):
        for arm, want in (("B1", UNION + "\n"), ("B0", None)):
            d = ProbeDriver(stay=0.1)
            self.go(d, arm=arm)
            for root, _, files in os.walk(self.logs):
                for f in files:
                    if f.endswith(".env.json"):
                        self.assertEqual(__import__("json").loads(read(os.path.join(root, f)))["attrs"], want, (arm, f))
            shutil.rmtree(self.results)
            shutil.rmtree(self.logs)

    def test_the_history_fixture_carries_it_in_b1_only(self):
        for arm, want in (("B1", True), ("B0", False)):
            dest = os.path.join(self.tmp, "h-" + arm)
            os.makedirs(dest)
            subprocess.run(["bash", os.path.join(BENCH, "fixtures", "history", "setup.sh"), dest, "1"], check=True,
                           env=dict(GENV, COORDBENCH_ARM=arm), stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            has = os.path.exists(os.path.join(dest, ".gitattributes"))
            self.assertEqual(has, want, arm)
            if has:
                self.assertEqual(read(os.path.join(dest, ".gitattributes")), UNION + "\n")
                self.assertIn(".gitattributes", git(dest, "ls-files")[1])


def load_checker(name):
    spec = importlib.util.spec_from_file_location("chk_" + name, os.path.join(BENCH, "checks", name, "check.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class CheckersSkipTheBoard(unittest.TestCase):
    MARKED = "# Task board\n<<<<<<< HEAD\n- a | working\n=======\n- b | working\n>>>>>>> origin/main\n"

    def repo(self, files):
        tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-mark-"))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        for rel, text in files.items():
            os.makedirs(os.path.dirname(os.path.join(tmp, rel)) or tmp, exist_ok=True)
            write(os.path.join(tmp, rel), text)
        git(tmp, "init", "-q", "-b", "main")
        git(tmp, "add", "-A")
        git(tmp, "commit", "-q", "-m", "x")
        return tmp

    def test_markers_only_in_the_board_are_not_a_conflict_in_the_code(self):
        for name in ("tallykit", "onefile"):
            chk = load_checker(name)
            self.assertFalse(chk.has_conflict_markers(self.repo({"TASKS.md": self.MARKED, "a.py": "x = 1\n"})), name)

    def test_markers_in_any_other_file_still_count(self):
        for name in ("tallykit", "onefile"):
            chk = load_checker(name)
            self.assertTrue(chk.has_conflict_markers(self.repo({"TASKS.md": "# t\n", "pkg/a.py": self.MARKED})), name)
            self.assertTrue(chk.has_conflict_markers(self.repo({"sub/TASKS.md.bak": self.MARKED})), name)
            self.assertTrue(chk.has_conflict_markers(self.repo({"NOTES.md": self.MARKED})), name)


if __name__ == "__main__":
    unittest.main()
