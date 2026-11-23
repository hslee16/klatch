"""Scenario S7 (#1372): scale, 8 and 16 agents. The scenario file, the fixture variants, the dry-run cost estimate, the
metrics through the whole harness with fake agents (cooperative, a registry collision, chatty, silent) at both sizes, the
K2 room with 16 members through the real binary (needs KLATCH_BIN), and the scaling table of the report. No model, no
spend."""
import contextlib
import io
import json
import os
import re
import unittest

from helpers import BENCH, make_record
import fixture as fixture_mod
import report
import scale
import scenario
import schema
from test_run_fake import Base, NEEDS_BIN, read_json
from test_scale_gen import load_gen

GEN = load_gen()


def fx(variant):
    return fixture_mod.Fixture("scale", variant=variant)


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s7")

    def test_arms_n_caps_and_sizes_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["fixture"], "scale")
        self.assertEqual(s["arms"], ["B1", "K2"])
        self.assertEqual(sorted(s["variants"]), ["n16", "n8"])
        self.assertEqual(s["n_per_arm"]["m0"], 6)
        self.assertEqual(s["variants"]["n8"]["agents"], 8)
        self.assertEqual(s["variants"]["n16"]["agents"], 16)
        self.assertEqual(s["variants"]["n8"]["modules"], 16)
        self.assertEqual(s["variants"]["n16"]["modules"], 32)
        self.assertIn("never pooled", s["pooling"])
        self.assertEqual(s["metrics"]["primary"], "units_per_usd")
        self.assertGreater(s["variants"]["n16"]["caps"]["per_run_usd"], s["variants"]["n8"]["caps"]["per_run_usd"])

    def test_every_metric_is_defined_and_the_slots_come_from_the_generator(self):
        d = self.scn["metrics"]["definitions"]
        for k in ("messages_per_agent", "read_context_tokens_per_member", "hook_digest_bytes_per_member", "max_coord_result_chars",
                  "units_per_agent_turn", "units_per_usd", "git_conflicts", "overwrites", "wall_s", "units_per_usd_ratio"):
            self.assertIn(k, d)
        self.assertTrue(self.scn["scale_metrics"])
        self.assertEqual(self.scn["units"], {}, "the unit map is generated per size")

    def test_the_variants_match_the_fixture(self):
        for v, n in (("n8", 8), ("n16", 16)):
            f = fx(v)
            self.assertEqual(f.name, "scale-" + v)
            self.assertEqual(len(f.agents), n)
            self.assertEqual(f.generated["modules"], 2 * n)
            self.assertEqual(f.spec["est_cost_usd_haiku"], self.scn["variants"][v]["est_cost_usd_haiku"])
            self.assertEqual(f.spec["scenario"], "s7")

    def test_the_two_sizes_share_one_seed_so_the_fixtures_are_comparable(self):
        self.assertEqual(fx("n8").generated["seed"], fx("n16").generated["seed"])

    def test_a_scale_fixture_without_a_variant_is_refused(self):
        with self.assertRaises(fixture_mod.FixtureError):
            fixture_mod.Fixture("scale")


class TestFixtureVariants(unittest.TestCase):
    def test_tasks_are_generated_one_per_agent(self):
        f = fx("n16")
        self.assertEqual(f.names, [chr(97 + i) for i in range(16)])
        b = GEN.build(32, f.generated["seed"])
        for ag, a in zip(f.agents, b["agents"]):
            for m in a["modules"]:
                self.assertIn("scalekit/%s.py" % m, ag["task"])

    def test_materialise_writes_the_template_and_no_reference(self):
        import tempfile
        f = fx("n8")
        with tempfile.TemporaryDirectory() as tmp:
            dest = os.path.join(tmp, "d")
            f.materialise(dest, 999, {"PATH": os.environ.get("PATH", "")})
            files = sorted(os.listdir(os.path.join(dest, "scalekit")))
            self.assertEqual(len(files), 16 + 1)  # sixteen stubs and the registry
            self.assertFalse(os.path.exists(os.path.join(dest, "reference")))
            with open(os.path.join(dest, "CHANGELOG.md")) as fh:
                self.assertEqual(fh.read(), "# Changelog\n\n## Unreleased\n\n")

    def test_the_run_seed_does_not_change_the_fixture(self):
        import tempfile
        f = fx("n8")
        out = []
        for seed in (1, 2):
            with tempfile.TemporaryDirectory() as tmp:
                dest = os.path.join(tmp, "d")
                f.materialise(dest, seed, {"PATH": os.environ.get("PATH", "")})
                snap = {}
                for d, _, ns in os.walk(dest):
                    if "__pycache__" in d:
                        continue
                    for n in ns:
                        with open(os.path.join(d, n)) as fh:
                            snap[os.path.relpath(os.path.join(d, n), dest)] = fh.read()
                out.append(snap)
        self.assertEqual(out[0], out[1])


class S7Base(Base):
    def argv(self, arms="B1", runs=1, variant="n8", **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "scale"
        return a + ["--variant", variant]

    def records(self, variant="n8", arm=None):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "scale-" + variant)):
            for n in names:
                if n.endswith(".json") and (arm is None or os.path.basename(d) == arm):
                    out.append(read_json(os.path.join(d, n)))
        return sorted(out, key=lambda r: r["run_id"])

    def script(self, variant, chain=False, tweak=None):
        f = fx(variant)
        ref = os.path.join(self.tmp, "ref-" + variant)
        script = GEN.fake_script(f.generated["modules"], f.generated["seed"], ref, chain=chain)
        if tweak:
            tweak(script)
        path = os.path.join(self.tmp, "fake-s7-%d.json" % len(os.listdir(self.tmp)))
        with open(path, "w") as fh:
            json.dump(script, fh)
        return path

    def one(self, variant="n8", arms="B1", chain=False, tweak=None, **kw):
        code, out, err = self.go(self.argv(arms, 1, variant, fake_script=self.script(variant, chain, tweak), **kw))
        recs = self.records(variant)
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r


class TestDryRun(S7Base):
    def estimate(self, variant, arms="B1,K2", runs=6, model=None):
        a = self.argv(arms, runs, variant)
        a.remove("--driver")
        a.remove("fake")
        a += ["--dry-run"] + (["--model", model] if model else [])
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(__import__("run").main(a), 0)
        return out.getvalue()

    def test_dry_run_states_agents_runs_and_the_estimate_for_each_size(self):
        t8 = self.estimate("n8")
        self.assertIn("8 agents per run", t8)
        self.assertIn("12 runs", t8)
        self.assertIn("estimated cost: 46.80 to 78.00 USD for 12 runs", t8)
        t16 = self.estimate("n16")
        self.assertIn("16 agents per run", t16)
        self.assertIn("estimated cost: 136.80 to 228.00 USD for 12 runs", t16)

    def test_the_cost_per_run_grows_faster_than_the_agent_count(self):
        # 8 agents cost 5.20 a run and 16 agents 15.20 on Haiku: twice the agents, 2.9 times the cost
        self.assertIn("assumed 5.2000 USD a run", self.estimate("n8"))
        self.assertIn("assumed 15.2000 USD a run", self.estimate("n16"))

    def test_sonnet_costs_three_times_haiku_and_the_proposed_caps_follow(self):
        t = self.estimate("n16", model="claude-sonnet-4")
        self.assertIn("410.40 to 684.00 USD", t)
        self.assertIn("proposed caps: per run 136.80 USD", t)


class TestFakeAgentsAtBothSizes(S7Base):
    def test_cooperative_agents_complete_every_unit_at_eight_and_sixteen(self):
        for variant, n in (("n8", 8), ("n16", 16)):
            r = self.one(variant)
            self.assertEqual(r["scenario"], "scale-" + variant)
            self.assertEqual(r["outcome"], "ok", variant)
            self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (2 * n, 2 * n))
            self.assertEqual(sorted(r["turns"]["per_agent"]), [chr(97 + i) for i in range(n)])
            self.assertEqual(r["metrics"]["duplicate_work"], 0)
            self.assertEqual(r["metrics"]["overwrites"], 0)
            sm = r["scenario_metrics"]
            self.assertEqual(sm["merge_state"], "clean")
            self.assertEqual(sm["tickets_completed"], n)
            self.assertEqual(sm["foreign_edits"], [])
            sc = sm["scale"]
            self.assertEqual(sc["agents"], n)

    def test_a_registry_collision_is_resolved_and_loses_nothing_at_both_sizes(self):
        for variant, n in (("n8", 8), ("n16", 16)):
            r = self.one(variant, chain=True)
            self.assertEqual(r["metrics"]["units_completed"], 2 * n, variant)
            self.assertEqual(r["metrics"]["overwrites"], 0, variant)
            self.assertGreaterEqual(r["metrics"]["git_conflicts"], n - 2, "every agent after the first two pulls a conflicted registry")
            self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")
            self.assertTrue(all(c["pass"] for c in r["outcome_checks"]), variant)

    def test_conflicts_grow_with_the_team(self):
        c8 = self.one("n8", chain=True)["metrics"]["git_conflicts"]
        c16 = self.one("n16", chain=True)["metrics"]["git_conflicts"]
        self.assertGreater(c16, c8)

    def test_an_unresolved_collision_loses_the_ticket_of_the_agent_that_stopped(self):
        def tweak(script):
            for st in script["agents"]["h"]:
                if st["op"] == "sync":
                    st.pop("resolve", None)
        r = self.one("n8", chain=True, tweak=tweak)
        self.assertLess(r["metrics"]["units_completed"], 16)
        self.assertNotEqual(r["outcome"], "ok")
        self.assertFalse(r["scenario_metrics"]["correct"])

    def test_a_silent_run_has_no_messages_and_no_coordination_reads(self):
        def tweak(script):
            for steps in script["agents"].values():
                steps[:] = [s for s in steps if s["op"] not in ("klatch", "send", "hook", "read")]
        r = self.one("n8", tweak=tweak)
        sc = r["scenario_metrics"]["scale"]
        self.assertEqual(r["metrics"]["messages"], 0)
        self.assertEqual(sc["messages_per_agent"], 0)
        self.assertEqual(sc["read_context_chars"], 0)
        self.assertEqual(sc["read_context_tokens_per_member"], 0)
        self.assertEqual(sc["max_coord_result_chars"], 0)

    def test_a_chatty_run_counts_messages_and_reading_cost_per_member(self):
        def tweak(script):
            for name, steps in script["agents"].items():
                chat = [{"op": "send", "to": "all", "text": "status update %d from %s, still working on my ticket" % (i, name)}
                        for i in range(10)]
                chat += [{"op": "read", "path": "TASKS.md"} for _ in range(3)]
                steps[0:0] = chat
        r = self.one("n16", tweak=tweak)
        sc = r["scenario_metrics"]["scale"]
        self.assertEqual(r["metrics"]["messages"], 160)
        self.assertEqual(sc["messages_per_agent"], 10)
        self.assertGreater(sc["read_context_chars"], 0)
        self.assertEqual(sc["read_context_tokens_per_member"], sc["read_context_chars"] / 4 / 16)
        self.assertEqual(sc["read_context_tokens"], sc["read_context_chars"] // 4)
        self.assertGreater(sc["max_coord_result_chars"], 100, "the board, read in full, is the largest coordination result")
        self.assertEqual(r["metrics"]["units_completed"], 32)

    def test_the_hook_digest_bytes_and_the_largest_result_are_counted_from_the_stream(self):
        def tweak(script):
            script["agents"]["a"].insert(0, {"op": "hook", "text": "x" * 4000})
            script["agents"]["b"].insert(0, {"op": "hook", "text": "y" * 1000})
        r = self.one("n8", tweak=tweak)
        sc = r["scenario_metrics"]["scale"]
        self.assertEqual(sc["hook_context_chars"], 5000)
        self.assertEqual(sc["hook_chars_per_member"], 5000 / 8)

    def test_derived_metrics_from_the_record(self):
        r = self.one("n8", chain=True)
        d = scale.derive(r)
        self.assertEqual(d["agents"], 8)
        self.assertEqual(d["units_completed"], 16)
        self.assertEqual(d["units_per_agent"], 2.0)
        self.assertEqual(d["units_per_agent_turn"], 16 / r["turns"]["total"])
        self.assertIsNone(d["units_per_usd"], "fake agents spend nothing, so there is no per-dollar figure")
        self.assertEqual(d["wall_s"], r["wall_s"])
        self.assertEqual(d["tickets_per_agent_minute"], 8 / (8 * r["wall_s"] / 60.0))


class TestK2RoomWith16Members(S7Base):
    @NEEDS_BIN
    def test_sixteen_members_join_post_and_read_through_the_real_binary(self):
        r = self.one("n16", arms="K2")
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual(r["metrics"]["units_completed"], 32)
        self.assertEqual(r["metrics"]["messages"], 32, "each of 16 members posts working and finished")
        sc = r["scenario_metrics"]["scale"]
        self.assertEqual(sc["agents"], 16)
        self.assertGreater(sc["read_context_chars"], 0, "klatch read output is counted")
        self.assertEqual(sorted(r["turns"]["per_agent"]), [chr(97 + i) for i in range(16)])
        logs = os.path.join(self.logs, r["run_id"])
        for i in range(16):
            name = chr(97 + i)
            with open(os.path.join(logs, name + ".stream.jsonl")) as fh:
                text = fh.read()
            self.assertNotIn('"is_error": true', text, name)
            self.assertRegex(text, r"join", name)

    @NEEDS_BIN
    def test_eight_members_complete_in_k2_and_the_store_hook_bytes_are_null_without_the_export_tool(self):
        r = self.one("n8", arms="K2")
        self.assertEqual(r["metrics"]["units_completed"], 16)
        sc = r["scenario_metrics"]["scale"]
        self.assertIn("hook_bytes_injected", sc)
        if sc["hook_bytes_injected"] is None:
            self.assertIn("store", sc["hook_bytes_reason"])


class TestScalingTable(unittest.TestCase):
    def rec(self, variant, arm, n, units, cost, turns, run, msgs, read_tokens, wall=100.0, conflicts=0, hook=None):
        r = make_record("scale-" + variant, arm, run, units=(2 * n, units), cost=cost, turns=turns, wall=wall, messages=msgs)
        r["metrics"]["git_conflicts"] = conflicts
        r["metrics"]["overwrites"] = 0
        r["metrics"]["coordination_context_tokens"] = read_tokens
        r["scenario_metrics"] = {"scale": {"agents": n, "read_context_chars": read_tokens * 4, "hook_context_chars": 0,
                                           "max_coord_result_chars": 1000 * n, "hook_bytes_injected": hook},
                                 "tickets_completed": units // 2, "tickets_total": n}
        return r

    def records(self):
        rs = []
        for i in range(3):
            rs.append(self.rec("n8", "B1", 8, 16, 4.0, 160, "r%d" % i, 40, 800, conflicts=3))
            rs.append(self.rec("n16", "B1", 16, 30, 16.0, 420, "r%d" % i, 130, 4000, wall=200.0, conflicts=9))
            rs.append(self.rec("n8", "K2", 8, 16, 5.0, 170, "r%d" % i, 50, 600, hook=8000))
            rs.append(self.rec("n16", "K2", 16, 32, 12.0, 330, "r%d" % i, 110, 1800, wall=150.0, hook=40000))
        return rs

    def test_scaling_has_one_row_per_arm_and_metric_with_each_size_and_the_ratio(self):
        sc = scale.scaling(self.records())
        self.assertEqual(sc["sizes"], [8, 16])
        self.assertEqual(sorted(sc["arms"]), ["B1", "K2"])
        row = {r["metric"]: r for r in sc["arms"]["B1"]["rows"]}
        self.assertEqual(row["messages_per_agent"]["values"], {"8": 5.0, "16": 130 / 16.0})
        self.assertEqual(row["read_context_tokens_per_member"]["values"]["8"], 100.0)
        self.assertEqual(row["read_context_tokens_per_member"]["values"]["16"], 250.0)
        self.assertEqual(row["read_context_tokens_per_member"]["ratio"], 2.5)
        self.assertEqual(row["units_completed"]["values"], {"8": 16.0, "16": 30.0})
        self.assertEqual(row["git_conflicts"]["ratio"], 3.0)
        self.assertEqual(sc["arms"]["K2"]["rows"][0]["n"], {"8": 3, "16": 3})

    def test_the_units_per_dollar_ratio_says_whether_more_agents_still_help(self):
        sc = scale.scaling(self.records())
        b1, k2 = sc["arms"]["B1"]["units_per_usd"], sc["arms"]["K2"]["units_per_usd"]
        self.assertEqual(b1["values"], {"8": 4.0, "16": 30 / 16.0})
        self.assertAlmostEqual(b1["ratio"], (30 / 16.0) / 4.0)
        self.assertEqual(b1["verdict"], "adding agents stopped helping (units per dollar fell)")
        self.assertEqual(k2["values"], {"8": 16 / 5.0, "16": 32 / 12.0})
        self.assertEqual(k2["verdict"], "adding agents stopped helping (units per dollar fell)")

    def test_a_ratio_above_one_reads_as_still_helping(self):
        rs = [self.rec("n8", "K2", 8, 16, 8.0, 100, "r1", 10, 100), self.rec("n16", "K2", 16, 32, 8.0, 100, "r1", 10, 100)]
        self.assertEqual(scale.scaling(rs)["arms"]["K2"]["units_per_usd"]["verdict"], "adding agents still helps (units per dollar rose)")

    def test_hook_bytes_come_from_the_store_export_when_present_and_are_n_a_otherwise(self):
        sc = scale.scaling(self.records())
        k2 = {r["metric"]: r for r in sc["arms"]["K2"]["rows"]}
        self.assertEqual(k2["hook_digest_bytes_per_member"]["values"], {"8": 1000.0, "16": 2500.0})
        b1 = {r["metric"]: r for r in sc["arms"]["B1"]["rows"]}
        self.assertEqual(b1["hook_digest_bytes_per_member"]["values"], {"8": None, "16": None})
        self.assertIsNone(b1["hook_digest_bytes_per_member"]["ratio"])

    def test_records_of_other_scenarios_and_single_sizes_are_left_out(self):
        rs = self.records() + [make_record("trivial", "B0", "r9")]
        self.assertEqual(scale.scaling(rs)["sizes"], [8, 16])
        only8 = [r for r in self.records() if r["scenario"] == "scale-n8"]
        sc = scale.scaling(only8)
        self.assertEqual(sc["sizes"], [8])
        self.assertNotIn("units_per_usd", sc["arms"]["B1"])
        self.assertEqual(scale.scaling([make_record("trivial", "B0", "r9")]), None)

    def test_the_report_has_a_scaling_section_and_never_pools_the_two_sizes(self):
        an = report.analyse(self.records())
        self.assertIn("scale-n8", an["scenarios"])
        self.assertIn("scale-n16", an["scenarios"])
        self.assertEqual(an["scaling"]["sizes"], [8, 16])
        md = report.render_markdown(an)
        self.assertIn("## Scaling curve", md)
        self.assertIn("| messages_per_agent |", md)
        self.assertIn("units_per_usd", md)
        self.assertIn("adding agents stopped helping", md)
        self.assertRegex(md, r"\| Metric \| N=8 \| N=16 \| Ratio 16 over 8 \|")
        self.assertNotIn("## Scaling curve", report.render_markdown(report.analyse([make_record("trivial", "B0", "r1")])))

    def test_invalid_runs_are_not_in_the_scaling_medians(self):
        rs = self.records()
        bad = self.rec("n8", "B1", 8, 0, 99.0, 1, "rx", 999, 99999)
        bad["valid"] = False
        row = {r["metric"]: r for r in scale.scaling(rs + [bad])["arms"]["B1"]["rows"]}
        self.assertEqual(row["messages_per_agent"]["values"]["8"], 5.0)
        self.assertEqual(row["messages_per_agent"]["n"]["8"], 3)


if __name__ == "__main__":
    unittest.main()
