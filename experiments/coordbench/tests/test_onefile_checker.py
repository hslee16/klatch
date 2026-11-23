"""The hidden checker of fixture F2 `onefile` (#1367) against the reference solution and against broken variants.

The reference lives in fixtures/onefile/reference (never in the agent-visible checkout): core.py is the full solution and
edits.json holds each ticket's edit as fake-agent steps (replace, append) that apply to the initial file one after the other.
Each variant is a git repository built from the fixture's setup.sh plus the three tickets' edits by three authors, then one
thing broken.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401

FIXTURE = os.path.join(BENCH, "fixtures", "onefile")
CHECK = os.path.join(BENCH, "checks", "onefile", "check.sh")
CHECK_PY = os.path.join(BENCH, "checks", "onefile", "check.py")
REFERENCE = os.path.join(FIXTURE, "reference")
CORE = "onefile/core.py"
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull}


def git(cwd, *args, author="bench"):
    env = dict(GENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@bench.invalid")
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), p.stdout))
    return p.stdout


def read(path):
    with open(path) as f:
        return f.read()


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


def edits():
    return json.loads(read(os.path.join(REFERENCE, "edits.json")))


def apply_steps(dest, steps):
    """Apply fake-agent replace and append steps to a checkout."""
    for st in steps:
        path = os.path.join(dest, st["path"])
        if st["op"] == "replace":
            text = read(path)
            assert text.count(st["old"]) == 1, "replace target not unique or absent: %r" % st["old"]
            write(path, text.replace(st["old"], st["new"]))
        elif st["op"] == "append":
            write(path, read(path) + st["content"])
        else:
            raise AssertionError(st["op"])


def build(tmp, mutate=None, done="abc"):
    """A repository: the fixture's initial tree, then each ticket's edit committed by its author, then a mutation."""
    dest = os.path.join(tmp, "co")
    os.makedirs(dest)
    subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=GENV)
    git(dest, "init", "-q", "-b", "main")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "initial")
    ed = edits()
    for agent in "abc":
        if agent in done:
            apply_steps(dest, ed[agent])
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "--allow-empty", "-m", "ticket " + agent, author=agent)
    if mutate:
        mutate(dest)
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "--allow-empty", "-m", "break", author="c")
    return dest


def check(dest):
    p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=GENV)
    assert p.returncode == 0, p.stdout + p.stderr
    return json.loads(p.stdout.strip().splitlines()[-1])


def by_id(result):
    return {c["id"]: c["pass"] for c in result["checks"]}


def sub(old, new):
    def mutate(dest):
        path = os.path.join(dest, CORE)
        text = read(path)
        assert text.count(old) == 1, old
        write(path, text.replace(old, new))
    return mutate


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="onefile-test-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def variant(self, mutate=None, done="abc"):
        return check(build(tempfile.mkdtemp(dir=self.tmp), mutate, done))


class TestFixtureShape(Base):
    def test_setup_makes_the_one_file_its_visible_tests_and_no_hidden_files(self):
        dest = os.path.join(self.tmp, "x")
        os.makedirs(dest)
        subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=GENV)
        found = sorted(os.path.relpath(os.path.join(d, n), dest) for d, _, ns in os.walk(dest) for n in ns)
        self.assertIn(CORE, found)
        self.assertIn("tests/test_core.py", found)
        self.assertIn("README.md", found)
        for f in found:
            self.assertNotIn("reference", f)
            self.assertNotIn("check", os.path.basename(f))
        lines = read(os.path.join(dest, CORE)).count("\n")
        self.assertTrue(80 <= lines <= 160, "the module is about 120 lines, got %d" % lines)

    def test_the_visible_tests_pass_on_the_initial_tree_and_on_the_full_solution(self):
        for done in ("", "abc"):
            work = tempfile.mkdtemp(dir=self.tmp)
            dest = build(work, None, done)
            p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=dest,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=GENV)
            self.assertEqual(p.returncode, 0, p.stdout)

    def test_the_three_edits_applied_in_order_make_the_reference_file(self):
        dest = build(self.tmp)
        self.assertEqual(read(os.path.join(dest, CORE)), read(os.path.join(REFERENCE, CORE)))

    def test_the_edits_are_tickets_that_overlap_on_the_table_and_the_helper(self):
        ed = edits()
        for agent in "abc":
            self.assertTrue(any("OPTIONS" in s.get("old", "") or '"' in s.get("new", "") for s in ed[agent]))
        table = [a for a in "abc" if any('"port": (8080, "int"),' in s.get("old", "") for s in ed[a])]
        helper = [a for a in "abc" if any("new kinds go above this line" in s.get("old", "") for s in ed[a])]
        self.assertEqual(table, ["a", "b", "c"], "every ticket adds a row to the shared table at the same place")
        self.assertEqual(helper, ["a", "b"], "two tickets adjust the shared helper at the same place")


class TestReference(Base):
    def test_the_reference_solution_gets_full_marks(self):
        r = self.variant()
        self.assertEqual((r["units_completed"], r["units_total"]), (3, 3))
        self.assertTrue(r["correct"], r)
        self.assertEqual(r["merge_state"], "clean")
        self.assertEqual(r["lost_edits"], [])
        self.assertEqual(r["lost_table_entries"], [])
        ids = by_id(r)
        for k in ("timeout", "retries", "header", "table", "integrated-render", "imports", "no-conflict-markers",
                  "visible-tests"):
            self.assertTrue(ids[k], k)

    def test_only_the_three_ticket_checks_are_units(self):
        r = self.variant()
        units = [c["id"] for c in r["checks"] if c.get("unit") is not False]
        self.assertEqual(sorted(units), ["header", "retries", "timeout"])

    def test_per_agent_is_commits_and_surviving_lines_by_git_author(self):
        pa = self.variant()["per_agent"]
        self.assertEqual(sorted(pa), ["a", "b", "c"])
        for a in "abc":
            self.assertEqual(pa[a]["commits"], 1)
            self.assertGreater(pa[a]["surviving_lines"], 0)
        self.assertNotIn("bench", pa)

    def test_file_mode_reports_which_tickets_a_single_file_has(self):
        def tickets(path):
            p = subprocess.run([sys.executable, "-I", CHECK_PY, "--file", path], stdout=subprocess.PIPE, text=True, env=GENV)
            self.assertEqual(p.returncode, 0)
            return json.loads(p.stdout.strip().splitlines()[-1])
        self.assertEqual(tickets(os.path.join(REFERENCE, CORE)), {"A": True, "B": True, "C": True})
        dest = build(self.tmp, None, "b")
        self.assertEqual(tickets(os.path.join(dest, CORE)), {"A": False, "B": True, "C": False})
        bad = os.path.join(self.tmp, "bad.py")
        write(bad, "<<<<<<< HEAD\n")
        self.assertEqual(tickets(bad), {"A": False, "B": False, "C": False})


class TestEachTicketAlone(Base):
    def test_a_single_ticket_is_one_unit_and_the_others_are_lost(self):
        for done, name, lost in (("a", "timeout", ["B", "C"]), ("b", "retries", ["A", "C"]), ("c", "header", ["A", "B"])):
            with self.subTest(done=done):
                r = self.variant(done=done)
                self.assertEqual(r["units_completed"], 1)
                self.assertTrue(by_id(r)[name])
                self.assertEqual(r["lost_edits"], lost)
                self.assertFalse(r["correct"])
                self.assertEqual(r["merge_state"], "clean", "the visible tests pass on a half-done file")
                self.assertFalse(by_id(r)["table"])

    def test_no_ticket_done_is_zero_units(self):
        r = self.variant(done="")
        self.assertEqual(r["units_completed"], 0)
        self.assertEqual(r["lost_edits"], ["A", "B", "C"])


class TestBrokenVariants(Base):
    def test_one_ticket_missing(self):
        r = self.variant(done="ac")
        self.assertEqual(r["units_completed"], 2)
        self.assertEqual(r["lost_edits"], ["B"])
        self.assertEqual(r["lost_table_entries"], ["retries"])
        self.assertFalse(r["correct"])
        self.assertEqual(r["merge_state"], "clean")
        self.assertFalse(by_id(r)["integrated-render"])

    def test_a_silent_overwrite_loses_a_colleagues_function_and_table_entry(self):
        """c copies a stale file (the initial file plus only c's edit) over main: a's and b's work vanish silently."""
        def stale(dest):
            fresh = os.path.join(self.tmp, "fresh")
            os.makedirs(fresh, exist_ok=True)
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), fresh, "1"], check=True, env=GENV)
            apply_steps(fresh, edits()["c"])
            shutil.copy(os.path.join(fresh, CORE), os.path.join(dest, CORE))
        r = self.variant(stale)
        self.assertEqual(r["units_completed"], 1)
        self.assertEqual(r["lost_edits"], ["A", "B"])
        self.assertEqual(sorted(r["lost_table_entries"]), ["retries", "timeout"])
        self.assertEqual(r["merge_state"], "clean", "nothing looks wrong to git or the visible tests")
        self.assertFalse(by_id(r)["table"])
        self.assertTrue(by_id(r)["no-conflict-markers"])

    def test_a_conflicted_merge_left_in_the_file(self):
        marked = sub('    "port": (8080, "int"),\n',
                     '    "port": (8080, "int"),\n<<<<<<< HEAD\n    "timeout": (30, "seconds"),\n=======\n'
                     '    "retries": (3, "count"),\n>>>>>>> origin/main\n')
        r = self.variant(marked)
        self.assertEqual(r["merge_state"], "conflicts-left")
        ids = by_id(r)
        self.assertFalse(ids["no-conflict-markers"])
        self.assertFalse(ids["imports"])
        self.assertFalse(r["correct"])
        self.assertEqual(r["units_completed"], 0, "a file that does not import has no working ticket")
        self.assertEqual(r["lost_edits"], ["A", "B", "C"])

    def test_a_broken_shared_helper_for_a_kind_a_ticket_added(self):
        r = self.variant(sub('return "%dx" % int(value)', 'return "%d" % int(value)'))
        self.assertEqual(r["lost_edits"], ["B"])
        self.assertEqual(r["units_completed"], 2)
        self.assertEqual(r["merge_state"], "clean")
        self.assertFalse(by_id(r)["integrated-render"])

    def test_a_broken_shared_helper_for_the_original_kinds_breaks_the_visible_tests(self):
        r = self.variant(sub("return str(int(value))", "return str(int(value)) + '!'"))
        self.assertEqual(r["merge_state"], "tests-broken")
        self.assertFalse(by_id(r)["visible-tests"])
        self.assertFalse(r["correct"])

    def test_a_lost_shared_table_entry_loses_that_tickets_behaviour(self):
        r = self.variant(sub('    "retries": (3, "count"),\n', ""))
        self.assertEqual(r["lost_edits"], ["B"])
        self.assertEqual(r["lost_table_entries"], ["retries"])
        self.assertFalse(by_id(r)["table"])
        self.assertTrue(by_id(r)["timeout"] and by_id(r)["header"])
        self.assertFalse(r["correct"])

    def test_a_wrong_table_default_is_not_the_reference_table(self):
        r = self.variant(sub('"timeout": (30, "seconds")', '"timeout": (60, "seconds")'))
        self.assertFalse(r["correct"])
        self.assertEqual(r["lost_edits"], ["A"])

    def test_a_ticket_function_with_a_wrong_result_fails_its_oracle(self):
        r = self.variant(sub("base * 2 ** i", "base * 3 ** i"))
        self.assertEqual(r["lost_edits"], ["B"])
        r = self.variant(sub("== settings (%d options) ==", "== settings (%d) =="))
        self.assertEqual(r["lost_edits"], ["C"])

    def test_an_agent_side_file_that_does_not_import_but_has_no_markers(self):
        r = self.variant(sub("def option_names", "def option_names(:"))
        self.assertEqual(r["merge_state"], "tests-broken")
        self.assertFalse(by_id(r)["imports"])
        self.assertEqual(r["units_completed"], 0)


if __name__ == "__main__":
    unittest.main()
