"""The hidden checker of fixture F1 `tallykit` (#1366) against the reference solution and against broken variants.

The reference lives in fixtures/tallykit/reference (never in the agent-visible checkout). Each variant is a git repository
built from the fixture's setup.sh plus the reference, with commits by four authors, then one thing broken.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401

FIXTURE = os.path.join(BENCH, "fixtures", "tallykit")
CHECK = os.path.join(BENCH, "checks", "tallykit", "check.sh")
REFERENCE = os.path.join(FIXTURE, "reference")
MODULES = ["units", "tokenizer", "dates", "checksum", "slug", "roman", "rle", "matrix"]
TICKETS = {"a": ["units", "tokenizer"], "b": ["dates", "checksum"], "c": ["slug", "roman"], "d": ["rle", "matrix"]}
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


def build(tmp, mutate=None, done=None):
    """A repository: the fixture's initial tree, then each author's modules (the reference), registry and changelog."""
    dest = os.path.join(tmp, "co")
    os.makedirs(dest)
    subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=GENV)
    git(dest, "init", "-q", "-b", "main")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "initial")
    for agent, mods in TICKETS.items():
        for m in mods:
            if done is None or m in done:
                shutil.copy(os.path.join(REFERENCE, "tallykit", m + ".py"), os.path.join(dest, "tallykit", m + ".py"))
        if agent == "d" and done != []:
            for f in ("tallykit/__init__.py", "CHANGELOG.md"):
                shutil.copy(os.path.join(REFERENCE, f), os.path.join(dest, f))
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "--allow-empty", "-m", "ticket " + agent, author=agent)
    if mutate:
        mutate(dest)
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "--allow-empty", "-m", "break", author="d")
    return dest


def check(dest):
    p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=GENV)
    assert p.returncode == 0, p.stdout + p.stderr
    return json.loads(p.stdout.strip().splitlines()[-1])


def by_id(result):
    return {c["id"]: c for c in result["checks"]}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="tallykit-test-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def run_variant(self, mutate=None, done=None):
        return check(build(self.tmp, mutate, done))


class TestFixtureShape(Base):
    def test_setup_makes_stubs_a_registry_and_a_changelog_and_no_hidden_files(self):
        dest = os.path.join(self.tmp, "x")
        os.makedirs(dest)
        subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=GENV)
        for m in MODULES:
            src = read(os.path.join(dest, "tallykit", m + ".py"))
            self.assertIn("NotImplementedError", src, m)
            self.assertTrue(os.path.exists(os.path.join(dest, "tests", "test_%s.py" % m)), m)
        self.assertTrue(os.path.exists(os.path.join(dest, "CHANGELOG.md")))
        self.assertEqual(sorted(MODULES), sorted(m for m in re.findall(r"`tallykit/(\w+)\.py`", read(os.path.join(dest, "README.md")))
                                          if m != "__init__"))
        all_text = "".join(read(os.path.join(d, n)) for d, _, ns in os.walk(dest) for n in ns)
        self.assertNotIn("hidden", all_text.lower().replace("hidden by", ""))
        self.assertFalse(os.path.exists(os.path.join(dest, "reference")))
        self.assertFalse(any(n in ("check.py", "check.sh") for d, _, ns in os.walk(dest) for n in ns))

    def test_four_task_files_each_name_two_modules(self):
        with open(os.path.join(FIXTURE, "fixture.json")) as f:
            spec = json.load(f)
        self.assertEqual([a["name"] for a in spec["agents"]], ["a", "b", "c", "d"])
        for a in spec["agents"]:
            text = read(os.path.join(FIXTURE, a["task"]))
            for m in TICKETS[a["name"]]:
                self.assertIn("`tallykit/%s.py`" % m, text)
            others = [m for n, ms in TICKETS.items() if n != a["name"] for m in ms]
            for m in others:
                self.assertNotIn("tallykit/%s.py" % m, text)

    def test_the_initial_checkout_fails_its_visible_tests_honestly_and_scores_zero(self):
        r = self.run_variant(done=[])
        ids = by_id(r)
        self.assertEqual(r["units_completed"], 0)
        self.assertFalse(ids["registry"]["pass"])
        self.assertFalse(ids["visible-tests"]["pass"])


class TestCheckerOnReference(Base):
    def test_reference_scores_eight_of_eight_and_is_clean(self):
        r = self.run_variant()
        ids = by_id(r)
        units = [c for c in r["checks"] if c.get("unit") is not False]
        self.assertEqual(sorted(c["id"] for c in units), sorted(MODULES))
        self.assertTrue(all(c["pass"] for c in units), r)
        self.assertEqual(r["units_completed"], 8)
        for k in ("registry", "changelog", "no-conflict-markers", "visible-tests"):
            self.assertIs(ids[k]["pass"], True, k)
            self.assertIs(ids[k]["unit"], False, k)
        self.assertEqual(r["merge_state"], "clean")
        self.assertTrue(r["correct"])

    def test_per_agent_contribution_is_by_git_author(self):
        r = self.run_variant()
        pa = r["per_agent"]
        self.assertEqual(sorted(pa), ["a", "b", "c", "d"])
        for name, mods in TICKETS.items():
            self.assertEqual(sorted(pa[name]["modules"]), sorted(mods), name)
            self.assertGreaterEqual(pa[name]["commits"], 1)
        self.assertNotIn("bench", pa)


class TestRegistryAlternatives(Base):
    """The independent review (#1461, M1): the registry text asks for every public function of a module, so a registered
    public helper is right; the checker used to want the exact names. Kept as regression tests (#1525)."""

    HELPER = "\n\ndef known_units():\n    return ['mm', 'cm', 'm', 'km']\n"

    def registry(self, edit, module_edit=None):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "__init__.py")
            write(p, edit(read(p)))
            if module_edit:
                mp = os.path.join(dest, "tallykit", "units.py")
                write(mp, read(mp) + module_edit)
        return by_id(self.run_variant(mutate))["registry"]["pass"]

    def test_a_registered_public_helper_is_accepted(self):
        self.assertTrue(self.registry(lambda t: t.replace("import convert_length, convert_temperature",
                                                          "import convert_length, convert_temperature, known_units"),
                                      self.HELPER))

    def test_a_helper_defined_but_not_registered_is_still_accepted(self):
        self.assertTrue(self.registry(lambda t: t, self.HELPER))

    def test_a_registered_name_the_module_does_not_define_is_rejected(self):
        self.assertFalse(self.registry(lambda t: t.replace("import convert_length, convert_temperature",
                                                           "import convert_length, convert_temperature, known_units")))

    def test_a_helper_registered_in_place_of_a_required_function_is_rejected(self):
        self.assertFalse(self.registry(lambda t: t.replace("import convert_length, convert_temperature", "import convert_length, known_units"),
                                       self.HELPER))

    def test_the_registry_as_parenthesised_multi_line_imports_is_accepted(self):
        def paren(t):
            out = []
            for line in t.splitlines():
                if line.startswith("from tallykit.") and " import " in line:
                    head, names = line.split(" import ")
                    out.append("%s import (\n    %s,\n)" % (head, ",\n    ".join(n.strip() for n in names.split(","))))
                else:
                    out.append(line)
            return "\n".join(out) + "\n"
        self.assertTrue(self.registry(paren))

    def test_a_module_registered_twice_is_rejected(self):
        self.assertFalse(self.registry(lambda t: t + "from tallykit.slug import slugify\n"))

    def test_a_star_import_is_not_a_registry_line(self):
        self.assertFalse(self.registry(lambda t: t.replace("from tallykit.units import convert_length, convert_temperature",
                                                           "from tallykit.units import *")))


class TestCheckerOnBrokenVariants(Base):
    def test_a_module_missing(self):
        r = self.run_variant(done=[m for m in MODULES if m != "roman"])
        ids = by_id(r)
        self.assertEqual(r["units_completed"], 7)
        self.assertFalse(ids["roman"]["pass"])
        self.assertTrue(all(ids[m]["pass"] for m in MODULES if m != "roman"))
        self.assertFalse(r["correct"])
        self.assertEqual(r["merge_state"], "tests-broken")

    def test_a_broken_module_passes_the_visible_tests_but_fails_hidden(self):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "roman.py")
            src = read(p)
            self.assertIn(" or to_roman(total) != text", src)
            write(p, src.replace(" or to_roman(total) != text", ""))
        r = self.run_variant(mutate)
        ids = by_id(r)
        self.assertEqual(r["units_completed"], 7)
        self.assertFalse(ids["roman"]["pass"])
        self.assertTrue(ids["visible-tests"]["pass"], "the visible tests are weaker than the hidden ones")
        self.assertFalse(r["correct"])

    def test_a_conflicted_merge_left_in_the_registry(self):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "__init__.py")
            lines = read(p).splitlines(keepends=True)
            keep = [l for l in lines if "matrix" not in l]
            lost = [l for l in lines if "matrix" in l]
            write(p, "".join(keep) + "<<<<<<< HEAD\n" + "".join(lost) + "=======\n>>>>>>> origin/main\n")
        r = self.run_variant(mutate)
        ids = by_id(r)
        self.assertEqual(r["merge_state"], "conflicts-left")
        self.assertFalse(ids["no-conflict-markers"]["pass"])
        self.assertFalse(r["correct"])
        self.assertEqual(r["units_completed"], 8, "the modules themselves are fine")

    def test_a_lost_registry_entry(self):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "__init__.py")
            write(p, "".join(l for l in read(p).splitlines(keepends=True) if "slug" not in l))
        r = self.run_variant(mutate)
        ids = by_id(r)
        self.assertFalse(ids["registry"]["pass"])
        self.assertTrue(ids["changelog"]["pass"])
        self.assertEqual(r["units_completed"], 8)
        self.assertFalse(r["correct"])
        self.assertEqual(r["merge_state"], "clean")

    def test_a_registry_entry_missing_a_function(self):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "__init__.py")
            write(p, read(p).replace("import is_leap_year, days_in_month, days_between",
                                     "import is_leap_year, days_in_month"))
        r = self.run_variant(mutate)
        self.assertFalse(by_id(r)["registry"]["pass"])

    def test_a_lost_changelog_line(self):
        def mutate(dest):
            p = os.path.join(dest, "CHANGELOG.md")
            write(p, "".join(l for l in read(p).splitlines(keepends=True) if "ticket B" not in l))
        r = self.run_variant(mutate)
        ids = by_id(r)
        self.assertFalse(ids["changelog"]["pass"])
        self.assertTrue(ids["registry"]["pass"])

    def test_a_duplicated_changelog_line_is_not_four_entries(self):
        def mutate(dest):
            p = os.path.join(dest, "CHANGELOG.md")
            write(p, read(p) + "- ticket B: dates, checksum\n")
        self.assertFalse(by_id(self.run_variant(mutate))["changelog"]["pass"])

    def test_a_syntax_error_in_the_registry_is_tests_broken_and_registry_fails(self):
        def mutate(dest):
            p = os.path.join(dest, "tallykit", "__init__.py")
            write(p, read(p) + "from tallykit.nowhere import oops\n")
        r = self.run_variant(mutate)
        self.assertFalse(by_id(r)["registry"]["pass"])
        self.assertEqual(r["units_completed"], 8)


if __name__ == "__main__":
    unittest.main()
