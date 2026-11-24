"""The harness self-test of #1519: a fake agent that tries every way out of its directories that is known, run through the
whole harness. Each attempt must be blocked by the operating system or detected by the post-run scan, and a detected
attempt makes the run invalid with a reason. Also the layout, the staged binary and the clean run."""
import contextlib
import io
import json
import os
import shutil
import stat
import sys
import tempfile
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import drivers
import fixture as fixture_mod
import run
import runcli

HERE = os.path.dirname(os.path.abspath(__file__))
ESCAPE = os.path.join(HERE, "escape_agent.py")
FAKE = os.path.join(BENCH, "fixtures", "trivial", "fake.json")


class EscapeDriver(drivers.FakeDriver):
    """Starts the escape agent and tells it the leaked paths: the other agents' directories and the guarded ones."""
    name = "fake"
    lists_logs = False  # this agent does not poll the log directory, so it is hidden as for a real agent

    def __init__(self):
        super().__init__(FAKE)
        self.isos = {}

    def command(self, name, arm, prompt, cwd, out_dir, **kw):
        isolation = kw["isolation"]
        self.isos[name] = isolation
        argv = [sys.executable, ESCAPE, "--name", name, "--out", out_dir, "--clone", cwd,
                "--others", ",".join(isolation["others"]), "--guard", ",".join(isolation["guard"]),
                "--origin", [p for p in isolation["write"] if p.endswith("origin.git")][0],
                "--logfiles", ",".join(isolation.get("log_files", []))]
        return argv, {}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-iso-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.home = os.path.join(self.tmp, "home")
        os.mkdir(self.home)
        p = mock.patch.dict(os.environ, {"HOME": self.home})
        p.start()
        self.addCleanup(p.stop)
        self.results, self.logs, self.scratch = (os.path.join(self.tmp, d) for d in ("results", "logs", "scratch"))
        os.mkdir(self.scratch)

    def argv(self, arm="B0", **kw):
        a = ["--fixture", "trivial", "--arm", arm, "--runs", "1", "--driver", "fake", "--suite-id", "t1", "--results-dir", self.results,
             "--logs-dir", self.logs, "--tmp-root", self.scratch, "--seed", "100", "--poll-s", "0.2", "--wall-s", "60"]
        for k, v in kw.items():
            a += ["--" + k.replace("_", "-"), str(v)]
        return a

    def record(self):
        path = os.path.join(self.results, "t1", "trivial", "B0", "t1-trivial-b0-r01.json")
        with open(path) as f:
            return json.load(f)

    def run_with(self, driver):
        a = runcli.parse_args(self.argv())
        a.arm_list = ["B0"]
        a.klatch_bin = None
        a.tmp_root = os.path.realpath(a.tmp_root)
        fx = fixture_mod.Fixture("trivial", a.fixtures_dir, a.checks_dir)
        with contextlib.redirect_stdout(io.StringIO()):
            return run.Suite(a, fx, driver, run.plan_runs(["B0"], 1, a.seed)).go()


class Escapes(Base):
    def setUp(self):
        super().setUp()
        self.driver = EscapeDriver()
        self.run_with(self.driver)
        self.rec = self.record()
        self.logdir = self.rec["artifacts"]["streams"]
        with open(os.path.join(self.logdir, "a.escape.json")) as f:
            self.report = {r["label"]: r for r in json.load(f)}
        self.violations = self.rec["isolation"]["violations"]

    def detected(self, label):
        return any(v["session"] == "a" and v["call"] == self.report[label]["call"] for v in self.violations)

    def test_every_attempt_is_blocked_by_the_operating_system_or_detected_and_the_allowed_ones_are_not_flagged(self):
        self.assertGreaterEqual(len(self.report), 14)
        for label, r in self.report.items():
            if r["allowed"]:
                self.assertFalse(self.detected(label), label)
            else:
                self.assertTrue(r["outcome"] == "blocked" or self.detected(label), "%s: %s" % (label, r))

    def test_every_attempt_that_names_a_forbidden_place_is_detected_even_when_the_system_blocked_it(self):
        for label, r in self.report.items():
            if not r["allowed"]:
                self.assertTrue(self.detected(label), label)

    def test_the_system_blocks_what_the_agent_cannot_name(self):
        for label in ("ls ../..", "old layout clones/b", "symlink to the scratch root", "cat ../b/SPEC-ROUTING.md", "python open ../c/x"):
            self.assertEqual(self.report[label]["outcome"], "blocked", label)
        self.assertEqual(self.report["list the log directory"]["outcome"], "blocked")
        # the scratch root and the logs root are listable (they hold nothing private); the scan flags naming them
        for label in ("list the scratch root", "list the logs root"):
            self.assertEqual(self.report[label]["outcome"], "read", label)

    def test_what_the_system_cannot_block_once_a_path_leaks_is_reported_honestly(self):
        # the owner can read what it can name: the leaked-path attempts get through the permissions and only the scan sees them
        self.assertEqual(self.report["read the log by its leaked random name"]["outcome"], "read")
        self.assertEqual(self.report["git log of origin (pushed work only)"]["outcome"], "read")
        self.assertTrue(self.detected("read the log by its leaked random name"))

    def test_the_run_is_invalid_with_a_reason_and_the_record_has_the_scan(self):
        self.assertFalse(self.rec["valid"])
        self.assertIn("isolation violation (SPEC 11)", self.rec["notes"])
        iso = self.rec["isolation"]
        self.assertEqual(iso["mode"], "sandbox")
        self.assertGreater(iso["checked_calls"], 14)
        self.assertEqual(iso["violation_count"], len(self.violations))
        self.assertEqual({v["agent"] for v in self.violations}, {"a", "b"})
        for k in ("agent", "session", "call", "tool", "text", "path", "why"):
            self.assertIn(k, self.violations[0])

    def test_the_driver_was_told_who_the_others_are_and_never_the_agents_own_directory_as_one(self):
        iso = self.driver.isos["a"]
        self.assertEqual(len(iso["others"]), 1)
        self.assertNotIn(os.path.dirname(iso["own"][0]), iso["others"])
        self.assertEqual(iso["settings_dir"], os.path.dirname(iso["own"][0]))

    def test_everything_is_listable_again_and_the_logs_are_under_their_public_name(self):
        self.assertEqual(os.path.basename(self.logdir), "t1-trivial-b0-r01")
        self.assertEqual(stat.S_IMODE(os.stat(self.logdir).st_mode), 0o700)
        self.assertEqual(os.listdir(self.scratch), [])
        self.assertEqual([n for n in os.listdir(self.logs) if not n.startswith(".")], ["t1-trivial-b0-r01"])


class Clean(Base):
    def test_a_run_whose_agents_stay_in_their_clones_is_valid_and_scanned(self):
        with contextlib.redirect_stdout(io.StringIO()):
            run.main(self.argv("B0", max_run_usd=5, max_suite_usd=50))
        rec = self.record()
        self.assertTrue(rec["valid"], rec["notes"])
        self.assertEqual(rec["isolation"]["violations"], [])
        self.assertEqual(rec["isolation"]["violation_count"], 0)
        self.assertGreater(rec["isolation"]["checked_calls"], 0)

    def test_agents_have_per_agent_home_and_tmp_inside_their_own_directory(self):
        d = EscapeDriver()
        seen = {}
        orig = run.RunExecution.agent_env
        def spy(me, name, extra, role):
            env = orig(me, name, extra, role)
            seen[name] = (env["HOME"], env["TMPDIR"], me.clones[name])
            return env
        with mock.patch.object(run.RunExecution, "agent_env", spy):
            self.run_with(d)
        self.assertEqual(len(seen), 2)
        for name, (home, tmpdir, clone) in seen.items():
            self.assertEqual(os.path.dirname(home), os.path.dirname(clone))
            self.assertEqual(os.path.dirname(tmpdir), os.path.dirname(clone))
        self.assertNotEqual(os.path.dirname(seen["a"][0]), os.path.dirname(seen["b"][0]))


class StagedBinary(Base):
    def test_the_agents_are_given_a_copy_of_the_binary_and_no_prompt_names_the_original(self):
        src_dir = os.path.join(self.tmp, "klatch-repo", "proto", "target", "debug")
        os.makedirs(src_dir)
        src = os.path.join(src_dir, "klatch")
        with open(src, "w") as f:
            f.write("#!/bin/sh\necho klatch 0.0.0 store schema 1\n")
        os.chmod(src, 0o755)
        with contextlib.redirect_stdout(io.StringIO()):
            run.main(self.argv("K1", klatch_bin=src, max_run_usd=5, max_suite_usd=50))
        logdir = os.path.join(self.logs, "t1-trivial-k1-r01")
        text = "".join(open(os.path.join(logdir, p)).read() for p in os.listdir(logdir) if p.endswith(".prompt.md"))
        self.assertNotIn(src_dir, text)
        self.assertRegex(text, r"/cb-bin-[^/]+/klatch")
        with open(os.path.join(self.results, "t1", "trivial", "K1", "t1-trivial-k1-r01.json")) as f:
            rec = json.load(f)
        self.assertEqual(rec["klatch"]["binary_sha256"], __import__("hashlib").sha256(open(src, "rb").read()).hexdigest())
        self.assertEqual([n for n in os.listdir(self.scratch) if n.startswith("cb-bin-")], [])  # removed with the suite


if __name__ == "__main__":
    unittest.main()
