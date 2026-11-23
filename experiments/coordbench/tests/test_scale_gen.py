"""The F7 `scale` generator (#1372): deterministic from a seed, valid (the reference passes the hidden checker at full marks
for N = 2, 8 and 16), module and function names unique, one stub and one visible test per module, nothing of the reference or
the oracles in the checkout. Standard library only."""
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH

FIXTURE = os.path.join(BENCH, "fixtures", "scale")
GEN_PATH = os.path.join(FIXTURE, "gen.py")


def load_gen():
    spec = importlib.util.spec_from_file_location("scale_gen_under_test", GEN_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def tree(root):
    out = {}
    for d, dirs, names in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x != "__pycache__")
        for n in sorted(names):
            p = os.path.join(d, n)
            with open(p, "rb") as f:
                out[os.path.relpath(p, root)] = f.read()
    return out


class TestDeterminism(unittest.TestCase):
    def setUp(self):
        self.gen = load_gen()

    def test_same_seed_same_output_in_process(self):
        g = self.gen
        for m in (4, 16, 32):
            self.assertEqual(g.template_files(m, 7), g.template_files(m, 7))
            self.assertEqual(g.reference_files(m, 7), g.reference_files(m, 7))
            self.assertEqual(g.tasks(m, 7), g.tasks(m, 7))
            self.assertEqual(g.build(m, 7), g.build(m, 7))

    def test_same_seed_same_bytes_from_two_processes(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = os.path.join(tmp, "a"), os.path.join(tmp, "b")
            for dest in (a, b):
                subprocess.run([sys.executable, "-I", GEN_PATH, "write", dest, "--modules", "16", "--seed", "11"], check=True,
                               stdout=subprocess.DEVNULL)
            self.assertEqual(tree(a), tree(b))
            self.assertGreater(len(tree(a)), 32)

    def test_a_different_seed_changes_the_parameters(self):
        g = self.gen
        self.assertNotEqual(g.template_files(16, 1), g.template_files(16, 2))
        self.assertNotEqual(g.reference_files(16, 1), g.reference_files(16, 2))

    def test_the_cli_and_the_library_agree(self):
        g = self.gen
        p = subprocess.run([sys.executable, "-I", GEN_PATH, "tasks", "--modules", "8", "--seed", "5"], check=True,
                           stdout=subprocess.PIPE, text=True)
        self.assertEqual(json.loads(p.stdout), g.tasks(8, 5))


class TestStructure(unittest.TestCase):
    def setUp(self):
        self.gen = load_gen()

    def test_module_and_function_names_are_unique_at_every_size(self):
        for m in (2, 4, 16, 32, 52):
            b = self.gen.build(m, 3)
            names = [x["name"] for x in b["modules"]]
            self.assertEqual(len(names), m)
            self.assertEqual(len(set(names)), m, names)
            fns = [x["fn"] for x in b["modules"]]
            self.assertEqual(len(set(fns)), m)
            for n in names:
                self.assertRegex(n, r"^[a-z][a-z0-9_]*$")

    def test_unique_names_for_every_seed_tried(self):
        for seed in range(30):
            names = [x["name"] for x in self.gen.build(32, seed)["modules"]]
            self.assertEqual(len(set(names)), 32)

    def test_agents_are_modules_over_two_with_two_modules_each(self):
        for m, n in ((4, 2), (16, 8), (32, 16)):
            b = self.gen.build(m, 9)
            self.assertEqual(len(b["agents"]), n)
            self.assertEqual([a["name"] for a in b["agents"]], [chr(97 + i) for i in range(n)])
            self.assertEqual([a["ticket"] for a in b["agents"]], [chr(65 + i) for i in range(n)])
            owned = [u for a in b["agents"] for u in a["modules"]]
            self.assertEqual(sorted(owned), sorted(x["name"] for x in b["modules"]))
            self.assertTrue(all(len(a["modules"]) == 2 for a in b["agents"]))

    def test_an_odd_or_too_large_module_count_is_refused(self):
        for bad in (0, 3, 54, -2):
            with self.assertRaises(ValueError):
                self.gen.build(bad, 1)

    def test_every_kind_is_used_when_there_are_enough_modules(self):
        kinds = {x["kind"] for x in self.gen.build(32, 1)["modules"]}
        self.assertEqual(len(kinds), min(32, len(self.gen.KINDS)))

    def test_the_template_has_stubs_visible_tests_registry_and_changelog(self):
        b = self.gen.build(16, 4)
        files = self.gen.template_files(16, 4)
        for x in b["modules"]:
            stub = files["scalekit/%s.py" % x["name"]]
            self.assertIn("def %s(" % x["fn"], stub)
            self.assertIn("raise NotImplementedError", stub)
            self.assertIn("tests/test_%s.py" % x["name"], files)
        self.assertEqual(files["CHANGELOG.md"], "# Changelog\n\n## Unreleased\n\n")
        self.assertIn("scalekit/__init__.py", files)
        self.assertNotIn("import scalekit.", files["scalekit/__init__.py"])
        self.assertFalse(re.search(r"^from scalekit\.", files["scalekit/__init__.py"], re.M))
        self.assertIn("tests/__init__.py", files)

    def test_the_checkout_has_no_reference_and_no_oracle(self):
        files = self.gen.template_files(16, 4)
        ref = self.gen.reference_files(16, 4)
        for path, text in files.items():
            self.assertNotIn("oracle", text.lower(), path)
            self.assertNotIn("reference", path.lower())
            if path.startswith("scalekit/") and path != "scalekit/__init__.py":
                self.assertNotEqual(text, ref[path], path)

    def test_the_reference_has_every_module_registry_line_and_changelog_line(self):
        b = self.gen.build(16, 4)
        ref = self.gen.reference_files(16, 4)
        reg = [l for l in ref["scalekit/__init__.py"].splitlines() if l.startswith("from scalekit.")]
        self.assertEqual(len(reg), 16)
        cl = [l for l in ref["CHANGELOG.md"].splitlines() if l.startswith("- ticket ")]
        self.assertEqual(len(cl), 8)
        for a in b["agents"]:
            self.assertIn("- ticket %s: %s" % (a["ticket"], ", ".join(a["modules"])), cl)

    def test_each_task_names_its_own_modules_and_none_of_the_others(self):
        b = self.gen.build(16, 4)
        tasks = self.gen.tasks(16, 4)
        self.assertEqual([t["name"] for t in tasks], [a["name"] for a in b["agents"]])
        for a, t in zip(b["agents"], tasks):
            for m in a["modules"]:
                self.assertIn("scalekit/%s.py" % m, t["task"])
            others = [m for o in b["agents"] if o is not a for m in o["modules"]]
            for m in others:
                self.assertNotIn("scalekit/%s.py" % m, t["task"])
            self.assertIn("ticket %s: %s" % (a["ticket"], ", ".join(a["modules"])), t["task"])
            self.assertNotIn("\u2014", t["task"])

    def test_the_scenario_map_covers_every_unit_agent_and_shared_file(self):
        mp = self.gen.scenario_map(16, 4)
        self.assertEqual(len(mp["units"]), 16)
        self.assertEqual(len(mp["agents"]), 8)
        self.assertEqual(sorted(mp["shared_files"]), ["CHANGELOG.md", "scalekit/__init__.py"])
        self.assertEqual(mp["shared_files"]["scalekit/__init__.py"]["expected_entries"], 16)
        self.assertEqual(mp["shared_files"]["CHANGELOG.md"]["expected_entries"], 8)
        for name, u in mp["units"].items():
            self.assertEqual(u["path"], "scalekit/%s.py" % name)
        self.assertTrue(re.match(mp["shared_files"]["scalekit/__init__.py"]["entry"], "from scalekit.x_1 import x_1"))
        self.assertTrue(re.match(mp["shared_files"]["CHANGELOG.md"]["entry"], "- ticket A: x_1, y_1"))

    def test_the_fake_script_has_one_agent_per_ticket(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = self.gen.fake_script(16, 4, tmp)
            self.assertEqual(sorted(script["agents"]), [chr(97 + i) for i in range(8)])
            self.assertTrue(os.path.exists(os.path.join(tmp, "scalekit")))
            ops = [s["op"] for s in script["agents"]["a"]]
            self.assertIn("sync", ops)
            self.assertEqual(ops.count("copy_from"), 2)


class TestVisibleTests(unittest.TestCase):
    """The visible tests fail on the stubs and pass on the reference: agents get a real, weaker check."""

    def setUp(self):
        self.gen = load_gen()
        self.tmp = tempfile.mkdtemp(prefix="scale-gen-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def write(self, files, dest):
        for rel, text in files.items():
            p = os.path.join(dest, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                f.write(text)

    def unittest_rc(self, dest):
        return subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=dest,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env={"PATH": os.environ.get("PATH", "")}).returncode

    def test_stubs_fail_and_the_reference_passes(self):
        stubs, ref = os.path.join(self.tmp, "stubs"), os.path.join(self.tmp, "ref")
        self.write(self.gen.template_files(8, 6), stubs)
        self.assertNotEqual(self.unittest_rc(stubs), 0)
        files = dict(self.gen.template_files(8, 6))
        files.update(self.gen.reference_files(8, 6))
        self.write(files, ref)
        self.assertEqual(self.unittest_rc(ref), 0)


if __name__ == "__main__":
    unittest.main()
