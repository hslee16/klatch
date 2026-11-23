"""The hidden checker of fixture F7 `scale` (#1372) against the generator's reference at N = 2, 8 and 16 agents (4, 16 and 32
modules) and against failing variants: missing modules, a conflicted merge, a lost registry entry, a lost changelog line, a
wrong module, an unfinished ticket. The reference never goes into a checkout; each variant is a git repository built from the
generator's template plus the reference, committed by one author per ticket."""
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
from test_scale_gen import load_gen

CHECK = os.path.join(BENCH, "checks", "scale", "check.sh")
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
SEED = 424242


def git(cwd, *args, author="bench"):
    env = dict(GENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@bench.invalid")
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), p.stdout))
    return p.stdout


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def read(path):
    with open(path) as f:
        return f.read()


class Scale(unittest.TestCase):
    gen = load_gen()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="scale-check-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def build(self, modules, mutate=None, done=None, seed=SEED):
        """A repository: the template (what the agents get), then one commit per ticket with the reference's modules, its
        registry and changelog lines for that ticket. done: a set of module names that get implemented (default all)."""
        b = self.gen.build(modules, seed)
        ref = self.gen.reference_files(modules, seed)
        dest = os.path.join(self.tmp, "co%d" % modules)
        for rel, text in self.gen.template_files(modules, seed).items():
            write(os.path.join(dest, rel), text)
        git(dest, "init", "-q", "-b", "main")
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "-m", "initial")
        reg, chg = [], []
        for a in b["agents"]:
            for m in a["modules"]:
                if done is None or m in done:
                    write(os.path.join(dest, "scalekit", m + ".py"), ref["scalekit/%s.py" % m])
                    reg.append([l for l in ref["scalekit/__init__.py"].splitlines() if l.startswith("from scalekit.%s " % m)][0])
            chg.append("- ticket %s: %s" % (a["ticket"], ", ".join(a["modules"])))
            with open(os.path.join(dest, "scalekit", "__init__.py"), "w") as f:
                f.write(self.gen.template_files(modules, seed)["scalekit/__init__.py"] + "".join(l + "\n" for l in reg))
            with open(os.path.join(dest, "CHANGELOG.md"), "w") as f:
                f.write(self.gen.template_files(modules, seed)["CHANGELOG.md"] + "".join(l + "\n" for l in chg))
            git(dest, "add", "-A")
            git(dest, "commit", "-q", "--allow-empty", "-m", "ticket " + a["ticket"], author=a["name"])
        if mutate:
            mutate(dest, b)
            git(dest, "add", "-A")
            git(dest, "commit", "-q", "--allow-empty", "-m", "mutate")
        return dest

    def check(self, dest, modules, seed=SEED):
        env = dict(GENV, COORDBENCH_GEN_MODULES=str(modules), COORDBENCH_GEN_SEED=str(seed))
        p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.assertEqual(p.returncode, 0, p.stderr)
        obj = json.loads(p.stdout.strip().splitlines()[-1])
        return obj, {c["id"]: c["pass"] for c in obj["checks"]}


class TestReferencePasses(Scale):
    def test_the_reference_passes_at_full_marks_for_two_eight_and_sixteen_agents(self):
        for agents in (2, 8, 16):
            m = agents * 2
            obj, ids = self.check(self.build(m), m)
            self.assertEqual((obj["units_completed"], obj["units_total"]), (m, m), agents)
            self.assertTrue(all(ids.values()), (agents, [k for k, v in ids.items() if not v]))
            self.assertTrue(obj["correct"])
            self.assertEqual(obj["merge_state"], "clean")
            self.assertEqual(obj["tickets_completed"], agents)
            self.assertEqual(obj["tickets_total"], agents)
            self.assertEqual(sorted(obj["per_agent"]), [chr(97 + i) for i in range(agents)])
            for a in obj["per_agent"].values():
                self.assertEqual(len(a["modules"]), 2)
                self.assertEqual(a["units_correct"], 2)

    def test_the_checks_list_every_module_then_the_merge_checks(self):
        obj, _ = self.check(self.build(16), 16)
        unit = [c for c in obj["checks"] if c.get("unit", True)]
        self.assertEqual(len(unit), 16)
        self.assertEqual([c["id"] for c in obj["checks"] if not c.get("unit", True)],
                         ["registry", "changelog", "no-conflict-markers", "visible-tests"])

    def test_the_stubs_alone_score_zero(self):
        dest = self.build(16, done=set())
        obj, ids = self.check(dest, 16)
        self.assertEqual(obj["units_completed"], 0)
        self.assertFalse(obj["correct"])

    def test_the_checker_is_repeatable(self):
        dest = self.build(8)
        self.assertEqual(self.check(dest, 8)[0], self.check(dest, 8)[0])

    def test_the_checker_needs_the_size_and_says_so(self):
        dest = self.build(4)
        p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=dict(GENV))
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("COORDBENCH_GEN_MODULES", p.stderr)

    def test_the_wrong_seed_does_not_pass(self):
        dest = self.build(16)
        obj, _ = self.check(dest, 16, seed=SEED + 1)
        self.assertFalse(obj["correct"])
        self.assertLess(obj["units_completed"], 16)


class TestFailingVariants(Scale):
    M = 16

    def names(self):
        return [x["name"] for x in self.gen.build(self.M, SEED)["modules"]]

    def test_missing_modules_count_zero_and_the_ticket_is_not_complete(self):
        names = self.names()
        keep = set(names[3:])  # the first three modules are never implemented
        obj, ids = self.check(self.build(self.M, done=keep), self.M)
        self.assertEqual(obj["units_completed"], 13)
        self.assertFalse(obj["correct"])
        for m in names[:3]:
            self.assertFalse(ids[m])
        self.assertEqual(obj["tickets_completed"], 8 - 2)  # modules 0,1 are ticket A, 2 is the first of ticket B

    def test_a_conflicted_merge_is_conflicts_left(self):
        def mutate(dest, b):
            p = os.path.join(dest, "scalekit", "__init__.py")
            text = read(p)
            write(p, "<<<<<<< HEAD\n" + text + "=======\n" + text + ">>>>>>> origin/main\n")
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertEqual(obj["merge_state"], "conflicts-left")
        self.assertFalse(ids["no-conflict-markers"])
        self.assertFalse(obj["correct"])
        self.assertEqual(obj["units_completed"], self.M, "the modules themselves are fine")

    def test_a_lost_registry_entry_fails_the_registry_not_the_unit(self):
        def mutate(dest, b):
            p = os.path.join(dest, "scalekit", "__init__.py")
            lines = read(p).splitlines(keepends=True)
            drop = [i for i, l in enumerate(lines) if l.startswith("from scalekit.")][5]
            del lines[drop]
            write(p, "".join(lines))
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids["registry"])
        self.assertTrue(ids["changelog"])
        self.assertEqual(obj["units_completed"], self.M)
        self.assertFalse(obj["correct"])

    def test_a_doubled_registry_entry_fails_the_registry(self):
        def mutate(dest, b):
            p = os.path.join(dest, "scalekit", "__init__.py")
            lines = [l for l in read(p).splitlines(keepends=True) if l.startswith("from scalekit.")]
            with open(p, "a") as f:
                f.write(lines[0])
        _, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids["registry"])

    def test_a_lost_changelog_line_fails_the_changelog(self):
        def mutate(dest, b):
            p = os.path.join(dest, "CHANGELOG.md")
            write(p, "".join(l for l in read(p).splitlines(keepends=True) if not l.startswith("- ticket C:")))
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids["changelog"])
        self.assertTrue(ids["registry"])
        self.assertFalse(obj["correct"])

    def test_a_wrong_module_fails_that_unit_and_the_visible_tests_pass_or_not(self):
        name = self.names()[4]

        def mutate(dest, b):
            fn = [x["fn"] for x in b["modules"] if x["name"] == name][0]
            write(os.path.join(dest, "scalekit", name + ".py"), "def %s(*args):\n    return None\n" % fn)
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids[name])
        self.assertEqual(obj["units_completed"], self.M - 1)
        self.assertEqual(obj["merge_state"], "tests-broken")
        self.assertFalse(ids["visible-tests"])

    def test_a_module_that_only_passes_the_visible_examples_fails_the_hidden_check(self):
        name = self.names()[0]

        def mutate(dest, b):
            m = [x for x in b["modules"] if x["name"] == name][0]
            files = self.gen.template_files(self.M, SEED)
            cases = re.search(r"CASES = (\[[^\n]*\])\n", files["tests/test_%s.py" % name])
            self.assertIsNotNone(cases)
            table = eval(cases.group(1))
            write(os.path.join(dest, "scalekit", name + ".py"),
                  "TABLE = %r\n\n\ndef %s(*args):\n    for a, w in TABLE:\n        if tuple(a) == tuple(args):\n            return w\n"
                  "    return None\n" % (table, m["fn"]))
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertTrue(ids["visible-tests"], "the visible examples are all it satisfies")
        self.assertFalse(ids[name])
        self.assertEqual(obj["units_completed"], self.M - 1)

    def test_a_module_that_hangs_on_unseen_inputs_fails_without_hanging_the_checker(self):
        name = self.names()[2]

        def mutate(dest, b):
            m = [x for x in b["modules"] if x["name"] == name][0]
            files = self.gen.template_files(self.M, SEED)
            table = eval(re.search(r"CASES = (\[[^\n]*\])\n", files["tests/test_%s.py" % name]).group(1))
            write(os.path.join(dest, "scalekit", name + ".py"),
                  "TABLE = %r\n\n\ndef %s(*args):\n    for a, w in TABLE:\n        if tuple(a) == tuple(args):\n            return w\n"
                  "    while True:\n        pass\n" % (table, m["fn"]))
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids[name])
        self.assertTrue(ids["visible-tests"])
        self.assertEqual(obj["units_completed"], self.M - 1)

    def test_a_module_with_a_syntax_error_fails_that_unit_only(self):
        name = self.names()[1]

        def mutate(dest, b):
            write(os.path.join(dest, "scalekit", name + ".py"), "def (:\n")
        obj, ids = self.check(self.build(self.M, mutate), self.M)
        self.assertFalse(ids[name])
        self.assertEqual(obj["units_completed"], self.M - 1)

    def test_a_foreign_edit_shows_in_per_agent_contribution(self):
        names = self.names()
        ref = self.gen.reference_files(self.M, SEED)

        def mutate(dest, b):  # agent h (the last) also rewrites a module of ticket A
            write(os.path.join(dest, "scalekit", names[0] + ".py"), ref["scalekit/%s.py" % names[0]] + "\n# touched\n")
            git(dest, "add", "-A")
            git(dest, "commit", "-q", "-m", "foreign", author="h")
        dest = self.build(self.M, mutate)
        obj, _ = self.check(dest, self.M)
        self.assertIn(names[0], obj["per_agent"]["h"]["modules"])
        self.assertEqual(obj["units_completed"], self.M)


if __name__ == "__main__":
    unittest.main()
