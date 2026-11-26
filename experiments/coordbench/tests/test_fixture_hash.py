"""#1516, #1510: what the fixture hash covers (fixture hash version 2), the checker hash, the prompt hashes, and the report
never pooling records of different hash versions. A mutation test works on a copy of the fixtures and the checkers, never on
the repository's own."""
import copy
import os
import py_compile
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH, make_record
import fixhash
import fixture as fixture_mod
import report

FIXTURES = os.path.join(BENCH, "fixtures")
CHECKS = os.path.join(BENCH, "checks")
GENERATORS = {"chain": ["gen.py"], "specialists": ["gen_facts.py"], "history": ["build_history.py", "gen_code.py", "gen_facts.py", "gen_room.py"],
              "recovery": ["gen_facts.py"], "scale": ["gen.py"]}


def _ignore(_d, names):
    return [n for n in names if n in ("__pycache__", ".DS_Store") or n.endswith(".pyc")]


class Copy(unittest.TestCase):
    """A private copy of the fixtures and the checkers to mutate."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cb-fixhash-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.fixtures = os.path.join(self.tmp, "fixtures")
        self.checks = os.path.join(self.tmp, "checks")
        shutil.copytree(FIXTURES, self.fixtures, ignore=_ignore)
        shutil.copytree(CHECKS, self.checks, ignore=_ignore)

    def fx(self, name, variant=None):
        return fixture_mod.Fixture(name, fixtures_dir=self.fixtures, checks_dir=self.checks, variant=variant)

    def edit(self, *parts, append=None, replace=None):
        path = os.path.join(self.tmp, *parts)
        with open(path) as f:
            text = f.read()
        if replace:
            assert replace[0] in text, replace[0]
            text = text.replace(*replace, 1)
        with open(path, "w") as f:
            f.write(text + (append or ""))
        return path


class WhatTheHashCovers(Copy):
    def test_the_version_is_three(self):
        self.assertEqual(fixhash.FIXTURE_HASH_VERSION, 3)  # #1531: the rule for fixtures with seeded facts

    def test_bytecode_and_editor_leftovers_do_not_change_the_hash(self):
        for name, variant in (("trivial", None), ("chain", None), ("scale", "n2")):
            before = self.fx(name, variant).sha256()
            d = os.path.join(self.tmp, "fixtures", name)
            os.makedirs(os.path.join(d, "__pycache__"), exist_ok=True)
            with open(os.path.join(d, "stray.py"), "w") as f:
                f.write("x = 1\n")
            py_compile.compile(os.path.join(d, "stray.py"), cfile=os.path.join(d, "__pycache__", "stray.cpython-399.pyc"))
            os.unlink(os.path.join(d, "stray.py"))
            for leftover in (".DS_Store", "task-a.md~", ".task-a.md.swp", "loose.pyc"):
                with open(os.path.join(d, leftover), "wb") as f:
                    f.write(b"junk")
            self.assertEqual(self.fx(name, variant).sha256(), before, name)

    def test_the_hash_is_the_same_under_python_B_and_after_bytecode_was_written(self):
        code = ("import sys; sys.path.insert(0, %r); import fixture; "
                "print(fixture.Fixture('scale', fixtures_dir=%r, variant='n2').sha256(), "
                "fixture.Fixture('chain', fixtures_dir=%r).sha256())" % (BENCH, self.fixtures, self.fixtures))
        scale_dir = os.path.join(self.fixtures, "scale")
        a = subprocess.run([sys.executable, "-B", "-c", code], stdout=subprocess.PIPE, text=True, check=True).stdout.split()
        self.assertFalse(os.path.exists(os.path.join(scale_dir, "__pycache__")))
        # a process that writes bytecode: it imports the generator from the fixture directory
        env = {k: v for k, v in os.environ.items() if k not in ("PYTHONDONTWRITEBYTECODE", "PYTHONPATH")}
        subprocess.run([sys.executable, "-c", "import sys; sys.path.insert(0, %r); import gen" % scale_dir], check=True, env=env)
        self.assertTrue(os.path.isdir(os.path.join(scale_dir, "__pycache__")), "the import wrote no bytecode")
        b = subprocess.run([sys.executable, "-c", code], stdout=subprocess.PIPE, text=True, check=True, env=env).stdout.split()
        self.assertEqual(a, b)
        self.assertEqual(a[0], self.fx("scale", "n2").sha256())

    def test_loading_the_generator_writes_no_bytecode_into_the_fixture(self):
        fx = self.fx("scale", "n2")
        fx.gen()
        self.assertFalse(os.path.exists(os.path.join(self.fixtures, "scale", "__pycache__")))

    def test_a_comment_edit_in_a_generator_source_does_not_change_the_hash(self):
        # the three fixtures with seeded facts hash their generator sources instead (#1531, test_seeded_facts.py)
        for name, variant, sources in (("chain", None, ["gen.py"]), ("scale", "n2", ["gen.py"])):
            before = self.fx(name, variant).sha256()
            for s in sources:
                self.edit("fixtures", name, s, append="\n# a comment that changes no output\n")
            self.assertEqual(self.fx(name, variant).sha256(), before, name)

    def test_changing_a_committed_generated_file_changes_the_hash(self):
        for name, rel in (("chain", "template/data.json"), ("specialists", "private/a/facts.json"),
                          ("recovery", "fake.json.in")):  # history.json and the other seeded files are not hashed (#1531)
            path = os.path.join(self.fixtures, name, rel)
            if not os.path.exists(path):
                continue
            before = self.fx(name).sha256()
            self.edit("fixtures", name, rel, append=" ")
            self.assertNotEqual(self.fx(name).sha256(), before, name)

    def test_a_generated_fixture_hashes_the_generator_output(self):
        # scale commits no generated file: a change of what gen.py writes (not a comment) must show in the hash
        before = self.fx("scale", "n2").sha256()
        self.edit("fixtures", "scale", "gen.py", replace=("MAX_AGENTS = 26", "MAX_AGENTS = 26\nTASK_EXTRA = 1"))
        self.assertEqual(self.fx("scale", "n2").sha256(), before)  # an unused constant: same output, same hash
        self.edit("fixtures", "scale", "gen.py", replace=('PACKAGE = "scalekit"', 'PACKAGE = "scalekit2"'))
        self.assertNotEqual(self.fx("scale", "n2").sha256(), before)

    def test_the_history_hash_follows_the_commits_build_history_makes(self):
        before = self.fx("history").sha256()
        self.edit("fixtures", "history", "build_history.py",
                  replace=('"--allow-empty", "-m", c["message"]', '"--allow-empty", "-m", c["message"] + "!"'))
        self.assertNotEqual(self.fx("history").sha256(), before)

    def test_reference_and_task_texts_are_in_the_hash(self):
        before = self.fx("tallykit").sha256()
        self.edit("fixtures", "tallykit", "task-a.md", append="\nmore\n")
        self.assertNotEqual(self.fx("tallykit").sha256(), before)
        ref = [os.path.join(d, n) for d, _, ns in os.walk(os.path.join(self.fixtures, "tallykit", "reference")) for n in ns][0]
        before = self.fx("tallykit").sha256()
        with open(ref, "a") as f:
            f.write("\n")
        self.assertNotEqual(self.fx("tallykit").sha256(), before)

    def test_two_instances_and_two_variants(self):
        self.assertEqual(self.fx("scale", "n2").sha256(), self.fx("scale", "n2").sha256())
        self.assertNotEqual(self.fx("scale", "n2").sha256(), self.fx("scale", "n8").sha256())

    def test_every_generator_source_is_declared_for_exclusion(self):
        for name in sorted(os.listdir(FIXTURES)):
            fx = fixture_mod.Fixture(name, variant="n2" if name == "scale" else None)
            found = sorted(n for n in os.listdir(os.path.join(FIXTURES, name))
                           if n.endswith(".py") and (n.startswith("gen") or n.startswith("build_")))
            self.assertEqual(sorted(fx.generator_sources), found, name)
            self.assertEqual(sorted(fx.generator_sources), sorted(GENERATORS.get(name, [])), name)


class GeneratorsCheckTheirOutput(unittest.TestCase):
    """The committed output of a generator is in the hash, so the generator's own --check must hold."""

    def test_check_passes(self):
        for name, script in (("chain", "gen.py"), ("specialists", "gen_facts.py"), ("history", "gen_facts.py"),
                             ("recovery", "gen_facts.py")):
            p = subprocess.run([sys.executable, "-B", "-I", os.path.join(FIXTURES, name, script), "--check"],
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            self.assertEqual(p.returncode, 0, "%s/%s --check: %s" % (name, script, p.stdout[-300:]))


class CheckerHash(Copy):
    def test_a_checker_change_changes_the_checker_hash_only(self):
        fx = self.fx("trivial")
        h, c = fx.sha256(), fx.checker_sha256()
        self.assertRegex(c, "^[0-9a-f]{64}$")
        self.edit("checks", fx.spec["checker"], "check.sh", append="\n# changed\n")
        fx2 = self.fx("trivial")
        self.assertEqual(fx2.sha256(), h)
        self.assertNotEqual(fx2.checker_sha256(), c)

    def test_bytecode_in_the_checker_directory_is_not_hashed(self):
        fx = self.fx("scale", "n2")
        c = fx.checker_sha256()
        d = os.path.join(self.checks, "scale", "__pycache__")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "check.cpython-399.pyc"), "wb") as f:
            f.write(b"x")
        self.assertEqual(self.fx("scale", "n2").checker_sha256(), c)

    def test_two_fixtures_with_one_checker_have_one_checker_hash(self):
        self.assertEqual(self.fx("history").checker_sha256(), self.fx("history", "resume").checker_sha256())


class RecordFields(unittest.TestCase):
    def run_fake(self, arms):
        import test_run_fake
        t = test_run_fake.Base("setUp")
        t.setUp()
        self.addCleanup(t.tearDown) if hasattr(t, "tearDown") else None
        code, _, _ = t.go(t.argv(arms=arms, runs=2))
        self.assertEqual(code, 0)
        recs = []
        for d, _, names in os.walk(t.results):
            for n in names:
                if n.endswith(".json") and n != "ledger.json":
                    recs.append(test_run_fake.read_json(os.path.join(d, n)))
        self.addCleanup(shutil.rmtree, t.tmp, ignore_errors=True)
        return [r for r in recs if "run_id" in r]

    def test_records_carry_the_version_the_checker_hash_and_the_prompt_hashes(self):
        recs = self.run_fake("B0,B1")
        self.assertEqual(len(recs), 4)
        for r in recs:
            self.assertEqual(r["fixture_hash_version"], fixhash.FIXTURE_HASH_VERSION)
            self.assertRegex(r["fixture"]["sha256"], "^[0-9a-f]{64}$")
            self.assertRegex(r["fixture"]["checker_sha256"], "^[0-9a-f]{64}$")
            self.assertEqual(sorted(r["prompt_sha256"]), ["a", "b"])
            for v in r["prompt_sha256"].values():
                self.assertRegex(v, "^[0-9a-f]{64}$")
        self.assertEqual(len({r["fixture"]["sha256"] for r in recs}), 1)  # two suites, two arms: one fixture hash
        self.assertEqual(len({r["fixture"]["checker_sha256"] for r in recs}), 1)
        by_arm = {}
        for r in recs:
            by_arm.setdefault(r["arm"], set()).add(tuple(sorted(r["prompt_sha256"].items())))
        self.assertTrue(all(len(v) == 1 for v in by_arm.values()))  # the same arm and agent: the same prompt hash
        self.assertNotEqual(by_arm["B0"], by_arm["B1"])  # another arm text, another prompt hash


def rec(version=None, **kw):
    r = make_record(**kw)
    if version is not None:
        r["fixture_hash_version"] = version
    return r


class ReportGroupsByHashVersion(unittest.TestCase):
    def test_a_record_without_the_field_is_version_one(self):
        self.assertEqual(fixhash.version_of(rec()), 1)
        self.assertEqual(fixhash.version_of(rec(version=2)), 2)

    def test_old_only_records_leave_the_analysis_unchanged(self):
        self.assertNotIn("fixture_hash_versions", report.analyse([rec(run="r1"), rec(run="r2")], resamples=50))

    def test_mixed_hash_versions_are_never_pooled(self):
        rs = [rec(run="r1", version=1, turns=10), rec(run="r2", version=1, turns=12),
              rec(run="r3", version=2, turns=40), rec(run="r4", version=2, turns=44)]
        a = report.analyse(rs, resamples=50)
        self.assertEqual(a["fixture_hash_versions"], {"1": 2, "2": 2})
        self.assertEqual(sorted(a["scenarios"]), ["trivial", "trivial [fixture hash v1]"])
        self.assertEqual(a["scenarios"]["trivial"]["arms"]["B0"]["n"], 2)
        md = report.render_markdown(a)
        self.assertIn("fixture hash versions", md.lower())
        self.assertIn("not pooled", md)

    def test_metric_and_hash_versions_both_split(self):
        rs = [rec(run="r1", version=1), rec(run="r2", version=2)]
        rs[0]["metric_version"] = 2
        rs[1]["metric_version"] = 1
        a = report.analyse(rs, resamples=50)
        self.assertEqual(len(a["scenarios"]), 2)
        self.assertTrue(any("[metric v1]" in s for s in a["scenarios"]))

    def test_the_conditions_list_the_hash_versions_present(self):
        self.assertEqual(report.analyse([rec(run="r1", version=2)], resamples=50)["conditions"]["fixture hash version"], [2])

    def test_comparing_analyses_of_different_hash_versions_says_so(self):
        new = report.analyse([rec(run="r1", version=2), rec(run="r2", version=2)], resamples=50)
        old = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        c = report.compare(new, old, resamples=50)
        self.assertIn("fixture_hash_version_mismatch", c)
        self.assertIn("different fixture hash versions", report.render_markdown(new, c, "OLD", "NEW"))

    def test_comparing_the_same_version_adds_nothing(self):
        new = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        self.assertNotIn("fixture_hash_version_mismatch", report.compare(new, copy.deepcopy(new), resamples=50))


if __name__ == "__main__":
    unittest.main()
