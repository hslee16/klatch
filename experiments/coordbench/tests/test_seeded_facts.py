"""Hidden facts are drawn from the run seed (#1531): fixtures specialists, recovery and history. Two seeds give different facts
and the hidden checker accepts the reference solution of the matching seed and rejects one built from another seed's facts; one
seed is deterministic; the committed default-seed copy is unchanged; the run record says which facts a run had; and in a fake run
the facts are never in a path an agent can read. No model, no API, no credential, no network."""
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import fixture as fixture_mod
import runisolation
import schema
from test_run_fake import Base, read_json

NAMES = ("specialists", "recovery", "history")
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_TERMINAL_PROMPT": "0"}
SEED_A, SEED_B = 31, 32


def fx(name):
    return fixture_mod.Fixture(name)


def facts_of(name, seed):
    return json.loads(fx(name).facts_files(seed)["checks/%s/facts.json" % name])


def write_tree(files, root):
    for rel, text in files.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)


def git(cwd, *args, author="bench"):
    env = dict(GENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@bench.invalid")
    p = subprocess.run(["git", "-C", cwd] + list(args), env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    assert p.returncode == 0, p.stdout


class Tmp(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="cb-seeded-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def sub(self, name):
        d = os.path.join(self.tmp, name)
        os.makedirs(d)
        return d

    def solved(self, name, seed):
        """(checkout, facts file): the fixture's setup for the seed with the reference solution of the seed's facts on top."""
        files = fx(name).facts_files(seed)
        facts = os.path.join(self.sub("facts-%s-%d" % (name, seed)), "facts.json")
        with open(facts, "w") as f:
            f.write(files["checks/%s/facts.json" % name])
        dest = os.path.join(self.sub("co-%s-%d" % (name, seed)), "co")
        os.makedirs(dest)
        subprocess.run(["bash", os.path.join(BENCH, "fixtures", name, "setup.sh"), dest, str(seed)], check=True, env=GENV,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        prefix = "fixtures/%s/reference/" % name
        ref = {k[len(prefix):]: v for k, v in files.items() if k.startswith(prefix)}
        if name == "recovery":  # the seeded module only; the others are the same for every seed
            base = os.path.join(BENCH, "fixtures", "recovery", "reference")
            for m in ("bits", "pairs", "wrap"):
                with open(os.path.join(base, "recoverkit", m + ".py")) as f:
                    ref["recoverkit/%s.py" % m] = f.read()
        write_tree(ref, dest)
        if name == "recovery":
            self.commit_recovery(dest)
        return dest, facts

    def commit_recovery(self, dest):
        git(dest, "init", "-q", "-b", "main")
        git(dest, "add", "-A")
        git(dest, "commit", "-q", "-m", "initial")
        for author, mods in (("a", ["ranges"]), ("b", ["wrap", "pairs"]), ("b", ["bits"])):
            for m in mods:  # a commit by each author that changes the module the author owns or recovers
                path = os.path.join(dest, "recoverkit", m + ".py")
                with open(path, "a") as f:
                    f.write("\n")
            git(dest, "add", "-A")
            git(dest, "commit", "-q", "--allow-empty", "-m", "%s work" % author, author=author)

    def check(self, name, dest, facts):
        env = dict(GENV, COORDBENCH_FACTS=facts)
        p = subprocess.run(["bash", os.path.join(BENCH, "checks", name, "check.sh"), dest], env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, text=True, timeout=120)
        self.assertEqual(p.returncode, 0, p.stderr)
        return json.loads(p.stdout.strip().splitlines()[-1])


class TestSeedsGiveDifferentFacts(unittest.TestCase):
    def test_each_fixture_draws_different_hidden_facts_from_different_seeds(self):
        for name in NAMES:
            a, b = fx(name).facts_files(SEED_A), fx(name).facts_files(SEED_B)
            self.assertEqual(sorted(a), sorted(b), name)
            self.assertNotEqual(a["checks/%s/facts.json" % name], b["checks/%s/facts.json" % name], name)
            private = [k for k in a if "/private/" in k or k.endswith("history.json") or k.endswith("seed.json")]
            self.assertTrue(private, name)
            for k in private:
                self.assertNotEqual(a[k], b[k], k)
            self.assertNotEqual(fx(name).facts_sha256(SEED_A), fx(name).facts_sha256(SEED_B), name)

    def test_the_same_seed_is_deterministic_in_process_and_through_the_command_line(self):
        for name in NAMES:
            self.assertEqual(fx(name).facts_files(SEED_A), fx(name).facts_files(SEED_A), name)
            self.assertEqual(fx(name).facts_sha256(SEED_A), fx(name).facts_sha256(SEED_A), name)
            out = tempfile.mkdtemp(prefix="cb-seeded-out-")
            self.addCleanup(shutil.rmtree, out, ignore_errors=True)
            script = os.path.join(BENCH, "fixtures", name, "gen_facts.py")
            subprocess.run([sys.executable, "-B", "-I", script, "--seed", str(SEED_A), "--out", out], check=True)
            for rel, text in fx(name).facts_files(SEED_A).items():
                with open(os.path.join(out, rel)) as f:
                    self.assertEqual(f.read(), text, rel)
            bad = subprocess.run([sys.executable, "-B", "-I", script, "--seed", str(SEED_B), "--out", out, "--check"],
                                 stdout=subprocess.PIPE, text=True)
            self.assertEqual(bad.returncode, 1, "another seed's files differ from these")

    def test_the_default_seed_is_the_committed_copy(self):
        for name in NAMES:
            f = fx(name)
            for rel, text in f.facts_files(f.default_seed).items():
                with open(os.path.join(BENCH, rel)) as fh:
                    self.assertEqual(fh.read(), text, rel)

    def test_no_seeded_fact_is_a_plausible_default_or_its_stale_value(self):
        sp = facts_of("specialists", SEED_B)["facts"]
        for k, v in sp.items():
            self.assertNotIn(v["value"], v["plausible"], k)
            self.assertNotEqual(v["value"], v["stale"], k)
        rc = facts_of("recovery", SEED_B)
        self.assertNotIn([rc["K"], rc["R"]], rc["plausible"])
        self.assertNotEqual([rc["K"], rc["R"]], [rc["stale"]["K"], rc["stale"]["R"]])
        hs = facts_of("history", SEED_B)["facts"]
        for k, v in hs.items():
            self.assertNotIn(v["value"], v["plausible"], k)


class TestCheckerFollowsTheSeed(Tmp):
    def test_the_checker_accepts_the_reference_solution_of_the_matching_seed(self):
        for name in NAMES:
            for seed in (SEED_A, SEED_B):
                dest, facts = self.solved(name, seed)
                out = self.check(name, dest, facts)
                self.assertTrue(out["correct"], (name, seed, out))
                self.assertEqual(out["units_completed"], out["units_total"], (name, seed))

    def test_the_checker_rejects_a_solution_built_from_another_seeds_facts(self):
        for name in NAMES:
            dest, _ = self.solved(name, SEED_A)
            _, other = self.solved(name, SEED_B)
            out = self.check(name, dest, other)
            self.assertFalse(out["correct"], (name, out))
            self.assertLess(out["units_completed"], out["units_total"], name)

    def test_without_the_environment_the_checker_reads_its_committed_default_facts(self):
        for name in NAMES:
            f = fx(name)
            dest, _ = self.solved(name, f.default_seed)
            p = subprocess.run(["bash", os.path.join(BENCH, "checks", name, "check.sh"), dest], env=GENV, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, timeout=120)
            self.assertTrue(json.loads(p.stdout.strip().splitlines()[-1])["correct"], name)


class TestHashAndIdentity(Tmp):
    def test_the_fixture_hash_does_not_depend_on_the_seeded_files_but_on_the_generator(self):
        base = os.path.join(self.tmp, "bench")
        shutil.copytree(os.path.join(BENCH, "fixtures"), os.path.join(base, "fixtures"), ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copytree(os.path.join(BENCH, "checks"), os.path.join(base, "checks"), ignore=shutil.ignore_patterns("__pycache__"))

        def make(name):
            return fixture_mod.Fixture(name, os.path.join(base, "fixtures"), os.path.join(base, "checks"))
        for name in NAMES:
            f = fx(name)
            before = make(name)
            h, c = before.sha256(), before.checker_sha256()
            for rel in f.seeded["outputs"]:
                p = os.path.join(base, "fixtures", name, rel)
                target = p if os.path.isfile(p) else next(os.path.join(d, n) for d, _, ns in os.walk(p) for n in sorted(ns))
                with open(target, "a") as fh:
                    fh.write(" ")
            for rel in f.seeded["checker_files"]:
                with open(os.path.join(base, "checks", name, rel), "a") as fh:
                    fh.write(" ")
            self.assertEqual(make(name).sha256(), h, name)
            self.assertEqual(make(name).checker_sha256(), c, name)
            with open(os.path.join(base, "fixtures", name, "gen_facts.py"), "a") as fh:
                fh.write("\n# a comment\n")
            self.assertNotEqual(make(name).sha256(), h, "the generator is part of what ran")

    def test_the_hash_version_says_the_seeded_rule_applies(self):
        import fixhash
        self.assertEqual(fixhash.FIXTURE_HASH_VERSION, 3)

    def test_a_fixture_without_seeded_facts_has_no_seed_or_facts_hash_in_its_record_object(self):
        import runidentity
        t = fixture_mod.Fixture("trivial")
        self.assertEqual(sorted(runidentity.fixture_identity(t, 5, None)), ["checker_sha256", "name", "sha256"])
        f = fx("recovery")
        ident = runidentity.fixture_identity(f, 5, None)
        self.assertEqual((ident["seed"], ident["facts_sha256"]), (5, f.facts_sha256(5)))

    def test_the_validator_tolerates_the_fields_missing_and_refuses_a_wrong_shape(self):
        from helpers import make_record
        r = make_record()
        self.assertEqual(schema.validate_record(r), [])
        r["fixture"] = dict(r["fixture"], seed=7, facts_sha256="a" * 64)
        self.assertEqual(schema.validate_record(r), [])
        r["fixture"] = dict(r["fixture"], seed="7")
        self.assertIn("fixture.seed is not an integer or null", schema.validate_record(r))
        r["fixture"] = dict(r["fixture"], seed=7, facts_sha256="zz")
        self.assertIn("fixture.facts_sha256 is not a sha256 or null", schema.validate_record(r))


class FakeRun(Base):
    """History through the whole harness with its own fake agent: the seed's facts are in its private place only."""

    def argv(self, arms="B0", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "history"
        return a

    def records(self):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "history")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return sorted(out, key=lambda r: r["run_id"])

    def run_and_watch(self, flags=()):
        seen = {}
        original = runisolation.IsolationSupport.hide_run

        def watch(this):
            original(this)
            readable = [this.clone_path(n) for n in this.fx.names] + [os.path.join(this.agent_dir(n), s) for n in this.fx.names
                                                                       for s in ("home", "tmp")] + [this.origin, this.khome, this.kkeys]
            if this.driver.lists_logs:
                readable.append(this.log_dir)
            seen.update(facts_dir=this.facts_dir, facts_path=this.fx.facts_path, readable=readable,
                        mode=stat.S_IMODE(os.stat(this.facts_dir).st_mode), scratch=this.scratch)
            seen["marked"] = [p for root in readable for d, _, ns in os.walk(root) for n in ns
                              for p in [os.path.join(d, n)] if self.holds_hidden_data(p)]
        with mock.patch.object(runisolation.IsolationSupport, "hide_run", watch):
            code, out, err = self.go(self.argv("B0", 1, seed=SEED_A) + list(flags))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        return recs[0], seen

    @staticmethod
    def holds_hidden_data(path):
        try:
            with open(path, errors="replace") as f:
                return "Hidden checker data" in f.read()
        except OSError:
            return False

    def test_the_record_names_the_seed_and_hash_of_the_facts_and_the_run_passes_with_them(self):
        r, _ = self.run_and_watch()
        f = fx("history")
        self.assertEqual(schema.validate_record(r), [])
        self.assertEqual(r["seed"], SEED_A + 1, "the run seed is the suite seed plus the block number")
        self.assertEqual((r["fixture"]["seed"], r["fixture"]["facts_sha256"]), (r["seed"], f.facts_sha256(r["seed"])))
        self.assertEqual(r["fixture"]["sha256"], f.sha256())
        self.assertEqual(r["fixture_hash_version"], 3)
        self.assertEqual(r["outcome"], "ok")
        self.assertTrue(r["scenario_metrics"]["correct"], r["scenario_metrics"])
        self.assertTrue(r["valid"])
        self.assertEqual(r["isolation"]["violation_count"], 0)

    def test_the_facts_are_in_no_path_an_agent_can_read(self):
        r, seen = self.run_and_watch()
        self.assertTrue(seen["facts_dir"].startswith(seen["scratch"]), "under the run's scratch, which the sandbox and the scan guard")
        self.assertEqual(seen["mode"], 0o300, "unlistable while the agents run")
        for root in seen["readable"]:
            real_root, real_facts = os.path.realpath(root), os.path.realpath(seen["facts_dir"])
            self.assertFalse(real_facts == real_root or real_facts.startswith(real_root + os.sep), root)
            self.assertFalse(real_root.startswith(real_facts + os.sep), root)
        self.assertEqual(seen["marked"], [], "no file an agent can read holds the checker's facts")
        self.assertEqual(r["isolation"]["violation_count"], 0)

    def test_the_facts_of_a_run_are_not_left_in_the_log_directory_or_the_results(self):
        r, seen = self.run_and_watch()
        for root in (self.logs, self.results):
            for d, _, names in os.walk(root):
                for n in names:
                    with open(os.path.join(d, n), errors="replace") as f:
                        self.assertNotIn("Hidden checker data", f.read(), os.path.join(d, n))

    def test_recovery_runs_through_the_harness_on_the_seeds_facts(self):
        for name in ("recovery",):  # specialists has no fake script of its own
            a = self.argv("B0", 1, seed=SEED_B)
            a[a.index("--fixture") + 1] = name
            self.go(a)
            recs = []
            for d, _, names in os.walk(os.path.join(self.results, "t1", name)):
                recs += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
            self.assertEqual(len(recs), 1, name)
            r = recs[0]
            self.assertEqual((r["fixture"]["seed"], r["outcome"]), (SEED_B + 1, "ok"), name)
            self.assertTrue(r["scenario_metrics"]["correct"], r["scenario_metrics"])
            self.assertTrue(r["scenario_metrics"]["ask_answered"], "the survivor answered with the seeded map")

    def test_the_committed_facts_flag_is_refused_with_the_real_driver(self):
        code, out, err = self.go(self.argv("B0", 1) + ["--committed-facts", "--driver", "claude", "--model", "m",
                                                       "--budget-approved-in", os.path.join(self.tmp, "none")])
        self.assertNotEqual(code, 0)
        self.assertIn("committed-facts", out + err)

    def test_with_committed_facts_the_record_names_the_default_seed(self):
        r, _ = self.run_and_watch(["--committed-facts"])
        f = fx("history")
        self.assertEqual((r["seed"], r["fixture"]["seed"]), (SEED_A + 1, f.default_seed))
        self.assertEqual(r["fixture"]["facts_sha256"], f.facts_sha256(f.default_seed))
        self.assertTrue(r["scenario_metrics"]["correct"], r["scenario_metrics"])


if __name__ == "__main__":
    unittest.main()
