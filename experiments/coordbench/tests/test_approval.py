"""The budget approval file (#1399 item 7): validation, and its use by run.py. Fake driver, no model, nothing spent."""
import datetime
import json
import os
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import approval
import run
from test_run_fake import Base

TODAY = datetime.date(2026, 11, 23)


def good(**kw):
    d = {"approved_by": "the human", "approved_on": "2026-11-23", "suite_budget_usd": 2.0, "per_run_cap_usd": 0.5,
         "scope": "The B1 pilot, one run per arm on the trivial fixture.", "models": ["claude-haiku-4-5"],
         "expires_on": "2026-11-30"}
    d.update(kw)
    return d


class Load(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="approval-test-")
        self.addCleanup(lambda: __import__("shutil").rmtree(self.dir, ignore_errors=True))

    def write(self, data, raw=None):
        p = os.path.join(self.dir, "approval.json")
        with open(p, "w") as f:
            f.write(raw if raw is not None else json.dumps(data))
        return p

    def load(self, data, **kw):
        args = dict(model="claude-haiku-4-5", roles_model=None, run_cap=None, suite_cap=None, today=TODAY)
        args.update(kw)
        return approval.load(self.write(data), **args)

    def refused(self, data, text, **kw):
        with self.assertRaises(approval.ApprovalError) as cm:
            self.load(data, **kw)
        self.assertIn(text, str(cm.exception))

    def test_a_good_file_gives_its_caps(self):
        ap = self.load(good())
        self.assertEqual((ap["per_run_usd"], ap["suite_usd"]), (0.5, 2.0))
        self.assertEqual(ap["approved_by"], "the human")

    def test_missing_file_refuses(self):
        with self.assertRaises(approval.ApprovalError) as cm:
            approval.load(os.path.join(self.dir, "nope.json"), "m", None, None, None, TODAY)
        self.assertIn("cannot read", str(cm.exception))

    def test_malformed_json_refuses(self):
        with self.assertRaises(approval.ApprovalError):
            approval.load(self.write(None, raw="{not json"), "m", None, None, None, TODAY)
        with self.assertRaises(approval.ApprovalError):
            approval.load(self.write(["a list"]), "m", None, None, None, TODAY)

    def test_each_missing_field_refuses(self):
        for field in good():
            d = good()
            del d[field]
            self.refused(d, field)

    def test_bad_field_values_refuse(self):
        self.refused(good(suite_budget_usd="2"), "suite_budget_usd")
        self.refused(good(suite_budget_usd=True), "suite_budget_usd")
        self.refused(good(per_run_cap_usd=0), "per_run_cap_usd")
        self.refused(good(per_run_cap_usd=-1), "per_run_cap_usd")
        self.refused(good(per_run_cap_usd=3.0), "per_run_cap_usd")  # above the suite budget
        self.refused(good(models=[]), "models")
        self.refused(good(models="claude-haiku-4-5"), "models")
        self.refused(good(models=[1]), "models")
        self.refused(good(scope=" "), "scope")
        self.refused(good(approved_by=""), "approved_by")
        self.refused(good(approved_on="23/11/2026"), "approved_on")
        self.refused(good(expires_on="soon"), "expires_on")
        self.refused(good(approved_on="2026-12-01", expires_on="2026-12-05"), "approved_on")  # in the future
        self.refused(good(expires_on="2026-11-01"), "expires_on")  # before approval

    def test_expired_refuses_and_the_last_day_is_valid(self):
        self.refused(good(approved_on="2026-11-01", expires_on="2026-11-22"), "expired")
        self.assertTrue(self.load(good(expires_on="2026-11-23")))

    def test_a_model_not_listed_refuses(self):
        self.refused(good(), "claude-sonnet-4-5", model="claude-sonnet-4-5")
        self.refused(good(), "roles", roles_model="claude-opus-4-1")
        self.assertTrue(self.load(good(models=["claude-haiku-4-5", "claude-opus-4-1"]), roles_model="claude-opus-4-1"))

    def test_a_cli_cap_above_the_file_refuses(self):
        self.refused(good(), "per-run", run_cap=0.6)
        self.refused(good(), "suite", suite_cap=2.5)

    def test_cli_caps_at_or_below_the_file_stand(self):
        ap = self.load(good(), run_cap=0.25, suite_cap=1.0)
        self.assertEqual((ap["per_run_usd"], ap["suite_usd"]), (0.25, 1.0))
        ap = self.load(good(), run_cap=0.5, suite_cap=2.0)
        self.assertEqual((ap["per_run_usd"], ap["suite_usd"]), (0.5, 2.0))


class Main(Base):
    def claude_argv(self, path, **kw):
        argv = self.argv("B0", runs=1)
        argv[argv.index("--driver") + 1] = "claude"
        for flag in ("--max-run-usd", "--max-suite-usd"):
            i = argv.index(flag)
            del argv[i:i + 2]
        argv += ["--budget-approved-in", path, "--model", "claude-haiku-4-5"]
        for k, v in kw.items():
            argv += ["--" + k.replace("_", "-"), str(v)]
        return argv

    def write(self, **kw):
        p = os.path.join(self.tmp, "approval.json")
        with open(p, "w") as f:
            json.dump(good(approved_on="2020-01-01", expires_on="2999-01-01", **kw), f)
        return p

    def test_a_missing_or_bad_file_refuses_before_anything_is_written(self):
        for path in (os.path.join(self.tmp, "none.json"), self.write(models=[])):
            code, out, err = self.go(self.claude_argv(path))
            self.assertEqual(code, 2, err)
            self.assertIn("approval", err)
            self.assertFalse(os.path.exists(self.results))

    def test_a_model_outside_the_file_refuses(self):
        argv = self.claude_argv(self.write())
        argv[argv.index("--model") + 1] = "claude-sonnet-4-5"
        code, out, err = self.go(argv)
        self.assertEqual(code, 2)
        self.assertIn("claude-sonnet-4-5", err)

    def test_an_explicit_cap_above_the_file_refuses(self):
        code, out, err = self.go(self.claude_argv(self.write(), max_run_usd=1.0))
        self.assertEqual(code, 2)
        self.assertIn("per-run", err)

    def test_the_files_numbers_become_the_caps(self):
        args = run.parse_args(self.claude_argv(self.write()))
        self.assertEqual(run.resolve_caps(args), (0.5, 2.0))
        args = run.parse_args(self.claude_argv(self.write(), max_run_usd=0.25))
        self.assertEqual(run.resolve_caps(args), (0.25, 2.0))

    def test_the_default_caps_apply_to_fake_runs_without_a_file(self):
        args = run.parse_args(self.argv("B0", runs=1))
        self.assertEqual(run.resolve_caps(args), (5.0, 50.0))
        argv = self.argv("B0", runs=1)
        for flag in ("--max-run-usd", "--max-suite-usd"):
            i = argv.index(flag)
            del argv[i:i + 2]
        self.assertEqual(run.resolve_caps(run.parse_args(argv)), (3.0, 150.0))

if __name__ == "__main__":
    unittest.main()
