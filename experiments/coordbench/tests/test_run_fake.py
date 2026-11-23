"""The whole harness end to end with fake agents on the trivial fixture: no model, no spend, no network.

B arms and the isolation, cap and dry-run tests need no binary. The K arms need a built klatch: set KLATCH_BIN to it
(`cargo build -p klatch --locked`, then KLATCH_BIN=proto/target/debug/klatch) or those tests are skipped with a message.
"""
import contextlib
import io
import json
import os
import re
import stat
import tempfile
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import run
import schema

KLATCH_BIN = os.environ.get("KLATCH_BIN")
NEEDS_BIN = unittest.skipUnless(KLATCH_BIN and os.path.exists(KLATCH_BIN or ""),
                                "KLATCH_BIN is unset: build klatch (cargo build -p klatch --locked) and set KLATCH_BIN "
                                "to the binary to run the K-arm tests")
DEFAULT_FAKE = os.path.join(BENCH, "fixtures", "trivial", "fake.json")


def read_json(path):
    with open(path) as f:
        return json.load(f)


def files_under(root):
    for d, _, names in os.walk(root):
        for n in names:
            yield os.path.join(d, n)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-test-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))
        self.fakehome = os.path.join(self.tmp, "fakehome")
        os.mkdir(self.fakehome)
        self.results = os.path.join(self.tmp, "results")
        self.logs = os.path.join(self.tmp, "logs")
        self.scratch = os.path.join(self.tmp, "scratch")
        os.mkdir(self.scratch)
        env = {"HOME": self.fakehome, "ANTHROPIC_API_KEY": "sk-sentinel-123"}
        p = mock.patch.dict(os.environ, env)
        p.start()
        self.addCleanup(p.stop)

    def argv(self, arms="B0", runs=1, **kw):
        a = ["--fixture", "trivial", "--arm", arms, "--runs", str(runs), "--driver", "fake", "--suite-id", "t1",
             "--results-dir", self.results, "--logs-dir", self.logs, "--tmp-root", self.scratch, "--seed", "100",
             "--poll-s", "0.2", "--max-run-usd", "5", "--max-suite-usd", "50", "--wall-s", "60"]
        if KLATCH_BIN:
            a += ["--klatch-bin", KLATCH_BIN]
        for k, v in kw.items():
            a += ["--" + k.replace("_", "-"), str(v)]
        return a

    def go(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = run.main(argv)
        return code, out.getvalue(), err.getvalue()

    def script(self, **agents):
        """The default fake script with some agents' steps replaced."""
        s = read_json(DEFAULT_FAKE)
        s["agents"].update(agents)
        path = os.path.join(self.tmp, "fake-%d.json" % len(os.listdir(self.tmp)))
        with open(path, "w") as f:
            json.dump(s, f)
        return path

    def records(self, arm=None):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "trivial")):
            for n in names:
                if n.endswith(".json") and (arm is None or os.path.basename(d) == arm):
                    out.append(read_json(os.path.join(d, n)))
        return sorted(out, key=lambda r: r["run_id"])


class EndToEndBArms(Base):
    def test_b0_and_b1_run_the_trivial_scenario_to_a_valid_record(self):
        code, out, err = self.go(self.argv("B0,B1", runs=2))
        self.assertEqual(code, 0, out + err)
        recs = self.records()
        self.assertEqual(len(recs), 4)
        for r in recs:
            self.assertEqual(schema.validate_record(r), [], r["run_id"])
            self.assertEqual(r["outcome"], "ok")
            self.assertTrue(r["valid"])
            self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (2, 2))
            self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b"])

    def test_records_live_at_the_spec_path_and_say_what_ran(self):
        self.go(self.argv("B0", runs=1))
        path = os.path.join(self.results, "t1", "trivial", "B0", "t1-trivial-b0-r01.json")
        self.assertTrue(os.path.exists(path), os.listdir(self.results))
        r = read_json(path)
        self.assertEqual((r["scenario"], r["arm"], r["milestone"], r["topology"]), ("trivial", "B0", "t1", "T-clone"))
        self.assertTrue(re.fullmatch(r"[0-9a-f]{64}", r["fixture"]["sha256"]))
        self.assertTrue(re.fullmatch(r"[0-9a-f]{40}", r["klatch"]["commit"]))
        self.assertEqual(r["models"]["agent"], "fake")
        self.assertEqual(r["claude_code_version"], "fake")
        self.assertEqual(r["caps"], {"per_run_usd": 5.0, "suite_usd": 50.0, "wall_s": 60, "max_turns_per_agent": 60})
        self.assertEqual(r["temperature"], "default")
        self.assertEqual(r["spend"], {"usd": 0.0, "estimated": False})
        self.assertGreater(r["tool_calls"]["total"], 0)
        self.assertIsNotNone(r["time_to_first_pass_s"])
        self.assertLessEqual(r["time_to_first_pass_s"], r["wall_s"] + 5)
        ids = [c["id"] for c in r["outcome_checks"]]
        self.assertEqual(ids, ["alpha", "beta", "no-conflict-markers", "visible-tests"])

    def test_the_seed_is_per_block_and_shared_by_the_arms_of_a_block(self):
        self.go(self.argv("B0,B1", runs=2))
        seeds = {(r["arm"], r["run_id"][-3:]): r["seed"] for r in self.records()}
        self.assertEqual(seeds[("B0", "r01")], seeds[("B1", "r01")])
        self.assertNotEqual(seeds[("B0", "r01")], seeds[("B0", "r02")])

    def test_ledger_and_suite_files(self):
        self.go(self.argv("B0,B1", runs=2))
        with open(os.path.join(self.results, "LEDGER.md")) as f:
            lines = [l for l in f.read().splitlines() if l and not l.startswith("#")]
        self.assertEqual(len(lines), 4)
        self.assertIn("t1-trivial-b0-r01", "\n".join(lines))
        s = read_json(os.path.join(self.results, "t1", "suite.json"))
        self.assertEqual((s["suite_id"], s["executed"], s["cap_hit"]), ("t1", 4, None))
        self.assertEqual(s["arms"], ["B0", "B1"])

    def test_stream_logs_go_to_logs_dir_outside_the_results(self):
        self.go(self.argv("B0", runs=1))
        streams = [p for p in files_under(self.logs) if p.endswith(".stream.jsonl")]
        self.assertEqual(len(streams), 2)
        self.assertFalse(any(p.endswith(".jsonl") for p in files_under(self.results)))
        r = self.records()[0]
        self.assertEqual(len(r["artifacts"]["stream_sha256"]), 2)

    def test_prompts_are_kept_with_the_logs(self):
        self.go(self.argv("B1", runs=1))
        prompts = [p for p in files_under(self.logs) if p.endswith(".prompt.md")]
        self.assertEqual(len(prompts), 2)
        with open(prompts[0]) as f:
            text = f.read()
        self.assertIn("Arm B1", text)
        self.assertNotIn("{{", text)

    def test_the_run_scratch_is_removed_afterwards(self):
        self.go(self.argv("B0", runs=1))
        self.assertEqual(os.listdir(self.scratch), [])


class Failures(Base):
    def test_an_agent_that_exits_nonzero_is_a_crash_and_the_run_is_kept(self):
        s = self.script(a=[{"op": "exit", "code": 1}])
        code, out, err = self.go(self.argv("B0", runs=1, fake_script=s))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "crash")
        self.assertTrue(r["valid"])
        self.assertEqual(r["metrics"]["units_completed"], 1)  # b still finished
        self.assertEqual(code, 1)

    def test_a_failing_git_step_ends_the_agent_with_an_error_exit(self):
        s = self.script(a=[{"op": "git", "args": ["checkout", "no-such-branch"]}])
        self.go(self.argv("B0", runs=1, fake_script=s))
        self.assertEqual(self.records()[0]["outcome"], "crash")

    def test_wall_time_cap_kills_the_team_and_records_a_timeout(self):
        s = self.script(a=[{"op": "hang"}])
        self.go(self.argv("B0", runs=1, fake_script=s, wall_s=2))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "timeout")
        self.assertGreaterEqual(r["wall_s"], 2)
        self.assertLess(r["wall_s"], 30)
        self.assertEqual(r["metrics"]["units_completed"], 1)

    def test_per_run_spend_cap_kills_and_records_spend_cap(self):
        s = self.script(a=[{"op": "cost", "usd": 0.5}, {"op": "hang"}])
        self.go(self.argv("B0", runs=1, fake_script=s, max_run_usd=0.3))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "spend-cap")
        self.assertGreaterEqual(r["spend"]["usd"], 0.5)
        self.assertTrue(r["spend"]["estimated"])

    def test_suite_cap_stops_the_suite_and_records_the_cap_hit(self):
        s = self.script(a=[{"op": "cost", "usd": 0.9}, {"op": "final", "text": "DONE a"}])
        code, out, err = self.go(self.argv("B0", runs=5, fake_script=s, max_run_usd=1.0, max_suite_usd=2.5))
        self.assertEqual(code, 3, out + err)
        self.assertEqual(len(self.records()), 2)
        meta = read_json(os.path.join(self.results, "t1", "suite.json"))
        self.assertEqual(meta["cap_hit"]["reason"], "suite-cap")
        self.assertEqual(meta["executed"], 2)
        self.assertEqual(len(meta["not_run"]), 3)
        self.assertAlmostEqual(meta["spent_usd"], 1.8, places=6)
        self.assertIn("spend cap", out)

    def test_git_conflicts_are_counted_from_the_stream(self):
        s = self.script(
            a=[{"op": "write", "path": "NOTES.md", "content": "from a\n"}, {"op": "sync", "message": "notes a"},
               {"op": "write", "path": "alpha.py", "content": "def alpha():\n    return 'alpha-done'\n"},
               {"op": "sync", "message": "alpha"}],
            b=[{"op": "write", "path": "NOTES.md", "content": "from b\n"},
               {"op": "wait_origin_has", "path": "NOTES.md", "timeout": 20},
               {"op": "sync", "message": "notes b", "allow_fail": True},
               {"op": "git", "args": ["merge", "--abort"], "allow_fail": True},
               {"op": "git", "args": ["reset", "--hard", "origin/main"]},
               {"op": "write", "path": "beta.py", "content": "def beta():\n    return 'beta-done'\n"},
               {"op": "sync", "message": "beta"}])
        self.go(self.argv("B0", runs=1, fake_script=s))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 1)

    def test_a_broken_fixture_setup_is_an_invalid_error_record(self):
        fx = os.path.join(self.tmp, "fixtures")
        os.makedirs(os.path.join(fx, "trivial"))
        import shutil
        for n in os.listdir(os.path.join(BENCH, "fixtures", "trivial")):
            shutil.copy(os.path.join(BENCH, "fixtures", "trivial", n), os.path.join(fx, "trivial", n))
        with open(os.path.join(fx, "trivial", "setup.sh"), "w") as f:
            f.write("#!/usr/bin/env bash\nexit 7\n")
        self.go(self.argv("B0", runs=1, fixtures_dir=fx))
        r = self.records()[0]
        self.assertEqual((r["outcome"], r["valid"]), ("error", False))
        self.assertIn("setup", r["notes"])
        self.assertEqual(schema.validate_record(r), [])


class Isolation(Base):
    def dump_script(self):
        return self.script(a=[{"op": "dump_env", "path": "env-a.json"}] + read_json(DEFAULT_FAKE)["agents"]["a"])

    def test_every_store_path_is_a_private_temp_directory_and_home_is_untouched(self):
        self.go(self.argv("B0", runs=1, fake_script=self.dump_script()))
        dump = read_json(next(p for p in files_under(self.logs) if p.endswith("env-a.json")))
        for key in ("HOME", "KLATCH_HOME", "KLATCH_KEY_DIR", "cwd"):
            self.assertTrue(os.path.realpath(dump[key]).startswith(self.scratch + os.sep), (key, dump[key]))
        self.assertNotEqual(os.path.realpath(dump["KLATCH_HOME"]), os.path.realpath(os.path.join(self.fakehome, ".klatch")))
        self.assertEqual(dump["modes"]["KLATCH_HOME"], 0o700)
        self.assertEqual(dump["modes"]["KLATCH_KEY_DIR"], 0o700)
        self.assertEqual(os.listdir(self.fakehome), [])  # the user's home was never written

    def test_credentials_never_reach_a_fake_agent_or_any_file(self):
        self.go(self.argv("B0", runs=1, fake_script=self.dump_script()))
        dump = read_json(next(p for p in files_under(self.logs) if p.endswith("env-a.json")))
        self.assertFalse(dump["has_anthropic_key"])
        for root in (self.results, self.logs):
            for p in files_under(root):
                with open(p, errors="replace") as f:
                    self.assertNotIn("sk-sentinel-123", f.read(), p)

    def test_guard_refuses_a_store_under_the_live_klatch_directory(self):
        live = os.path.join(self.fakehome, ".klatch")
        with self.assertRaises(run.GuardError):
            run.guard_paths(os.path.join(live, "h"), os.path.join(self.scratch, "k"), self.scratch)
        with self.assertRaises(run.GuardError):
            run.guard_paths(live, os.path.join(live, "k"), live)

    def test_guard_refuses_paths_outside_the_temp_root(self):
        with self.assertRaises(run.GuardError):
            run.guard_paths(os.path.join(self.tmp, "elsewhere"), os.path.join(self.scratch, "k"), self.scratch)

    def test_guard_refuses_a_tmp_root_under_the_claude_directory(self):
        root = os.path.join(self.fakehome, ".claude", "x")
        os.makedirs(root)
        code, out, err = self.go(self.argv("B0", runs=1, tmp_root=root))
        self.assertEqual(code, 2)
        self.assertEqual(os.listdir(os.path.join(self.fakehome, ".claude", "x")), [])

    def test_a_symlink_into_the_live_store_is_refused(self):
        live = os.path.join(self.fakehome, ".klatch")
        os.mkdir(live)
        link = os.path.join(self.scratch, "sneaky")
        os.symlink(live, link)
        with self.assertRaises(run.GuardError):
            run.guard_paths(os.path.join(link, "h"), os.path.join(self.scratch, "k"), self.scratch)


class Planning(Base):
    def test_interleaved_blocks_are_deterministic_and_complete(self):
        plan = run.plan_runs(["B0", "B1", "K1", "K2"], 10, seed=7)
        self.assertEqual(plan, run.plan_runs(["B0", "B1", "K1", "K2"], 10, seed=7))
        self.assertEqual(len(plan), 40)
        for b in range(10):
            self.assertEqual(sorted(a for blk, a in plan if blk == b + 1), ["B0", "B1", "K1", "K2"])
        orders = {tuple(a for blk, a in plan if blk == b + 1) for b in range(10)}
        self.assertGreater(len(orders), 1)  # not all of one order, not all of one arm first

    def test_dry_run_prints_the_plan_and_the_cost_range_and_runs_nothing(self):
        code, out, err = self.go(self.argv("B0,B1,K1,K2", runs=2) + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("DRY RUN", out)
        self.assertIn("8 runs", out)
        self.assertIn("estimated cost", out)
        self.assertIn("0.30", out)
        self.assertIn("0.50", out)
        self.assertFalse(os.path.exists(self.results))
        self.assertFalse(os.path.exists(self.logs))
        self.assertEqual(os.listdir(self.scratch), [])

    def test_dry_run_cost_scales_with_the_model_price_ratio(self):
        code, out, err = self.go(self.argv("B0,B1,K1,K2", runs=2, model="claude-sonnet-4-5") + ["--dry-run"])
        self.assertIn("0.90", out)
        self.assertIn("1.50", out)

    def test_real_agents_are_refused_without_a_recorded_budget_approval(self):
        argv = self.argv("B0", runs=1)
        argv[argv.index("--driver") + 1] = "claude"
        code, out, err = self.go(argv)
        self.assertEqual(code, 2)
        self.assertIn("budget", err)
        self.assertFalse(os.path.exists(self.results))

    def test_k_arms_need_the_klatch_binary_path(self):
        argv = self.argv("K1", runs=1)
        if "--klatch-bin" in argv:
            i = argv.index("--klatch-bin")
            del argv[i:i + 2]
        code, out, err = self.go(argv)
        self.assertEqual(code, 2)
        self.assertIn("--klatch-bin", err)

    def test_prompts_fill_every_placeholder_and_drop_the_draft_comment(self):
        for arm in ("B0", "B1", "K1", "K2"):
            text = run.build_prompt(os.path.join(BENCH, "arms"), arm, name="a", team=["a", "b"], task="DO THE THING",
                                    workdir="/w/a", test_command="python3 -m unittest", klatch_bin="/opt/k/klatch",
                                    room="lobby", max_turns=60, max_minutes=30)
            self.assertNotIn("{{", text, arm)
            self.assertNotIn("DRAFT", text, arm)
            self.assertIn("DO THE THING", text)
            self.assertIn("Arm %s" % arm, text)
        self.assertIn("/opt/k/klatch", run.build_prompt(os.path.join(BENCH, "arms"), "K1", "a", ["a", "b"], "T", "/w",
                                                        "t", "/opt/k/klatch", "lobby", 1, 1))

    def test_the_common_part_is_identical_across_arms(self):
        def common(arm):
            t = run.build_prompt(os.path.join(BENCH, "arms"), arm, "a", ["a", "b"], "T", "/w", "t", "/k", "lobby", 1, 1)
            return t.split("## How you coordinate with the others")[0]
        self.assertEqual(common("B0"), common("K2"))
        self.assertEqual(common("B1"), common("K1"))


@NEEDS_BIN
class KArms(Base):
    def test_k1_and_k2_run_end_to_end_with_a_real_klatch_in_a_private_store(self):
        s = self.script(a=[{"op": "dump_env", "path": "env-a.json"}] + read_json(DEFAULT_FAKE)["agents"]["a"])
        code, out, err = self.go(self.argv("K1,K2", runs=1, fake_script=s))
        self.assertEqual(code, 0, out + err)
        for r in self.records():
            self.assertEqual(schema.validate_record(r), [])
            self.assertEqual(r["outcome"], "ok", r["notes"])
            self.assertEqual(r["metrics"]["units_completed"], 2)
            self.assertEqual(r["metrics"]["messages"], 4)  # two posts each
            self.assertTrue(re.fullmatch(r"[0-9a-f]{64}", r["klatch"]["binary_sha256"]))
            self.assertIsInstance(r["klatch"]["store_schema"], int)

    def test_k2_installs_the_hooks_in_the_clone_and_k1_does_not(self):
        s = self.script(a=[{"op": "dump_env", "path": "env-a.json"}] + read_json(DEFAULT_FAKE)["agents"]["a"])
        self.go(self.argv("K1,K2", runs=1, fake_script=s))
        seen = {}
        for p in files_under(self.logs):
            if p.endswith("env-a.json"):
                seen["K2" if "-k2-" in p else "K1"] = read_json(p)["settings_local_exists"]
        self.assertEqual(seen, {"K1": False, "K2": True})

    def test_the_store_is_under_the_temp_root_not_the_live_one(self):
        s = self.script(a=[{"op": "dump_env", "path": "env-a.json"}] + read_json(DEFAULT_FAKE)["agents"]["a"])
        self.go(self.argv("K1", runs=1, fake_script=s))
        dump = read_json(next(p for p in files_under(self.logs) if p.endswith("env-a.json")))
        self.assertTrue(os.path.realpath(dump["KLATCH_HOME"]).startswith(self.scratch + os.sep))
        self.assertEqual(os.listdir(self.fakehome), [])


if __name__ == "__main__":
    unittest.main()
