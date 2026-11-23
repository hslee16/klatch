"""Scenario S2 (#1367): the scenario file, and the metrics of fixture F2 `onefile` through the whole harness with fake agents:
agents that cooperate in sequence, that collide and resolve, that collide and leave it unresolved, that silently overwrite a
colleague with a stale copy, and that give up. No model, no spend. The harness only has per-agent clones, so these runs are
topology T-clone; T-shared is a documented gap (see the README and design/items/1367-s2-onefile.md)."""
import json
import os
import re
import unittest

from helpers import BENCH  # noqa: F401
import scenario
import schema
from test_run_fake import Base, read_json

FIXTURE = os.path.join(BENCH, "fixtures", "onefile")
EDITS = read_json(os.path.join(FIXTURE, "reference", "edits.json"))
MARK = {"a": "def parse_timeout", "b": "def backoff_delays", "c": "== settings ("}
CORE = "onefile/core.py"
PULL = {"op": "git", "args": ["pull", "--no-rebase", "--no-edit", "origin", "main"]}


def wait(agent):
    return {"op": "wait_origin_has", "path": CORE, "contains": MARK[agent], "timeout": 30}


def steps(agent, resolve="union", wait_for=None, pull_first=False):
    """Fake-agent steps for one ticket. wait_for: the agent whose push to wait for, before syncing (a forced collision) or,
    with pull_first, before pulling and then editing (a cooperative sequence with no collision)."""
    out = []
    if pull_first and wait_for:
        out += [wait(wait_for), PULL]
    out += [dict(s) for s in EDITS[agent]]
    if wait_for and not pull_first:
        out.append(wait(wait_for))
    out.append({"op": "sync", "message": "ticket " + agent.upper(), "resolve": resolve})
    out.append({"op": "final", "text": "ticket %s is published. DONE %s" % (agent.upper(), agent)})
    return out


GIVE_UP = [{"op": "final", "text": "gave up"}]


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s2")

    def test_arms_n_caps_and_metrics_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["id"], "S2")
        self.assertEqual(s["fixture"], "onefile")
        self.assertEqual(s["arms"], ["B0", "B1", "K1", "K2", "K3", "B1-roles"])
        self.assertEqual(s["arms_at_m0"], ["B0", "B1", "K1", "K2"])
        self.assertEqual(s["n_per_arm"]["m0"], 20)
        self.assertEqual(s["caps"]["per_run_usd"], 1.8)
        self.assertEqual(s["caps"]["suite_usd_m0_haiku"], 60.0)
        self.assertEqual(s["metrics"]["primary"], "units_completed")
        for k in ("units_completed", "overwrites", "silent_overwrites", "conflicts_resolved", "git_conflicts",
                  "claim_conflicts", "time_to_recover_s", "merge_state", "lost_edits"):
            self.assertIn(k, s["metrics"]["definitions"])

    def test_topologies_state_what_is_built_and_what_is_a_gap(self):
        t = self.scn["topologies"]
        self.assertEqual(t["T-clone"]["status"], "implemented")
        self.assertEqual(t["T-shared"]["status"], "gap")
        self.assertIn("harness", t["T-shared"]["note"])

    def test_not_computed_metrics_are_named_null_with_a_reason(self):
        nc = self.scn["not_computed"]
        self.assertNotIn("claim_conflicts", nc)  # the K arms' store export fills it (#1433); store_metrics says where
        self.assertEqual(self.scn["store_metrics"]["fields"]["claim_conflicts"].split(":")[0], "claims.history.conflicts_handled")
        self.assertIn("time_to_recover_s", nc)

    def test_the_agents_match_the_fixture_and_the_scenario_is_found_from_it(self):
        fx = read_json(os.path.join(FIXTURE, "fixture.json"))
        self.assertEqual(fx["scenario"], "s2")
        self.assertEqual(fx["topology"], "T-clone")
        self.assertEqual([a["name"] for a in fx["agents"]], sorted(self.scn["agents"]))
        self.assertEqual({k: v["ticket"] for k, v in self.scn["agents"].items()}, {"a": "A", "b": "B", "c": "C"})

    def test_the_table_entry_pattern_matches_an_added_row_and_nothing_else(self):
        pat = self.scn["shared_files"][CORE]["entry"]
        self.assertTrue(re.match(pat, '    "timeout": (30, "seconds"),'))
        self.assertFalse(re.match(pat, "def parse_timeout(text):"))
        self.assertFalse(re.match(pat, "    return str(value)"))

    def test_an_unknown_scenario_is_an_error(self):
        with self.assertRaises(scenario.ScenarioError):
            scenario.load("nope")


class TestFixtureScripts(unittest.TestCase):
    def test_the_fixtures_fake_script_carries_the_reference_edits(self):
        fake = read_json(os.path.join(FIXTURE, "fake.json"))["agents"]
        for a in "abc":
            self.assertEqual([s for s in fake[a] if s["op"] in ("replace", "append")], EDITS[a])

    def test_the_overwrite_split_separates_silent_from_after_a_conflict(self):
        split = scenario.overwrite_split
        one = [{"ticket": "A", "author": "a"}]
        self.assertEqual(split(one, 0, {}), {"silent_overwrites": 1, "overwrites_after_conflict": 0, "conflicts_resolved": 0})
        self.assertEqual(split(one, 2, {}), {"silent_overwrites": 0, "overwrites_after_conflict": 1, "conflicts_resolved": 0})
        self.assertEqual(split([], 3, {"correct": True})["conflicts_resolved"], 3)
        self.assertEqual(split([], 3, {"correct": False})["conflicts_resolved"], 0)
        self.assertEqual(split([], None, {"correct": True}), {"silent_overwrites": 0, "overwrites_after_conflict": 0,
                                                              "conflicts_resolved": 0})


class S2Base(Base):
    def argv(self, arms="B0", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "onefile"
        return a

    def records(self):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "onefile")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return sorted(out, key=lambda r: r["run_id"])

    def one(self, agents=None):
        extra = {}
        if agents:
            path = os.path.join(self.tmp, "fake-s2-%d.json" % len(os.listdir(self.tmp)))
            with open(path, "w") as f:
                json.dump({"agents": agents}, f)
            extra["fake_script"] = path
        code, out, err = self.go(self.argv("B0", 1, **extra))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r, {c["id"]: c["pass"] for c in r["outcome_checks"]}


class TestFakeAgentsOnS2(S2Base):
    def test_the_fixtures_own_fake_script_scores_three_of_three(self):
        r, ids = self.one()  # all three at once, resolving whatever collides
        self.assertEqual(r["outcome"], "ok")
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (3, 3))
        self.assertTrue(all(ids.values()), ids)
        self.assertEqual(r["metrics"]["overwrites"], 0)
        self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")
        self.assertTrue(r["scenario_metrics"]["correct"])
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b", "c"])

    def test_correct_sequential_cooperation_has_no_conflict_and_loses_nothing(self):
        r, ids = self.one({"a": steps("a"), "b": steps("b", wait_for="a", pull_first=True),
                           "c": steps("c", wait_for="b", pull_first=True)})
        self.assertEqual(r["metrics"]["units_completed"], 3)
        self.assertTrue(all(ids.values()), ids)
        self.assertEqual(r["metrics"]["git_conflicts"], 0)
        self.assertEqual(r["metrics"]["overwrites"], 0)
        sm = r["scenario_metrics"]
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"], sm["conflicts_resolved"]), (0, 0, 0))
        self.assertEqual(sm["lost_edits"], [])
        self.assertEqual(sorted(sm["per_agent"]), ["a", "b", "c"])
        self.assertTrue(all(v["surviving_lines"] > 0 for v in sm["per_agent"].values()))

    def test_a_collision_that_is_resolved_keeps_every_edit(self):
        r, ids = self.one({"a": steps("a"), "b": steps("b", wait_for="a"), "c": steps("c", wait_for="b")})
        self.assertEqual(r["metrics"]["units_completed"], 3)
        self.assertTrue(all(ids.values()), ids)
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 2, "b and c each pull a conflicted core.py")
        self.assertEqual(r["metrics"]["overwrites"], 0)
        sm = r["scenario_metrics"]
        self.assertEqual(sm["merge_state"], "clean")
        self.assertEqual(sm["conflicts_resolved"], r["metrics"]["git_conflicts"])
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"]), (0, 0))
        self.assertEqual(sm["lost_lines"], [])

    def test_a_collision_left_unresolved_loses_that_agents_edit_but_not_silently(self):
        r, ids = self.one({"a": steps("a"), "b": steps("b", wait_for="a"), "c": steps("c", None, wait_for="b")})
        self.assertEqual(r["outcome"], "crash")
        self.assertEqual(r["metrics"]["units_completed"], 2)
        self.assertFalse(ids["header"])
        self.assertTrue(ids["timeout"] and ids["retries"])
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 1)
        sm = r["scenario_metrics"]
        self.assertEqual(sm["lost_edits"], ["C"])
        self.assertEqual(r["metrics"]["overwrites"], 1, "c committed the change in its clone and main never got it")
        self.assertEqual([o["ticket"] for o in sm["overwritten"]], ["C"])
        self.assertEqual(sm["overwritten"][0]["author"], "c")
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"]), (0, 1))
        self.assertEqual(sm["conflicts_resolved"], 0)
        self.assertEqual(sm["merge_state"], "clean")

    def test_a_stale_copy_pushed_over_main_is_a_silent_overwrite(self):
        """a publishes; b, who edited a copy of the initial file, then force pushes it over main; c gives up."""
        b = [dict(s) for s in EDITS["b"]] + [
            {"op": "git", "args": ["add", "-A"]}, {"op": "git", "args": ["commit", "-q", "-m", "ticket B"]}, wait("a"),
            {"op": "git", "args": ["push", "--force", "origin", "HEAD:main"]},
            {"op": "final", "text": "ticket B is published. DONE b"}]
        r, ids = self.one({"a": steps("a"), "b": b, "c": GIVE_UP})
        self.assertEqual(r["metrics"]["units_completed"], 1)
        self.assertTrue(ids["retries"])
        self.assertFalse(ids["timeout"])
        self.assertEqual(r["metrics"]["git_conflicts"], 0, "nobody saw a conflict")
        self.assertEqual(r["metrics"]["overwrites"], 1, "a's change was committed in a's clone and is absent from main")
        sm = r["scenario_metrics"]
        self.assertEqual([o["ticket"] for o in sm["overwritten"]], ["A"])
        self.assertEqual(sm["overwritten"][0]["author"], "a")
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"], sm["conflicts_resolved"]), (1, 0, 0))
        self.assertEqual(sm["lost_edits"], ["A", "C"])
        self.assertEqual(sm["merge_state"], "clean")
        self.assertFalse(ids["table"])
        lost = [l["line"].strip() for l in sm["lost_lines"]]
        self.assertEqual(lost, ['"timeout": (30, "seconds"),'])

    def test_a_stale_copy_resolved_by_keeping_ours_is_a_destructive_resolution_not_a_silent_one(self):
        r, ids = self.one({"a": steps("a"), "b": steps("b", "ours", wait_for="a"), "c": steps("c", wait_for="b")})
        self.assertEqual(r["metrics"]["units_completed"], 2, "b's ours resolution drops a's lines; c brings its own")
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 1)
        sm = r["scenario_metrics"]
        self.assertEqual(sm["lost_edits"], ["A"])
        self.assertEqual(r["metrics"]["overwrites"], 1)
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"]), (0, 1))
        self.assertEqual(sm["conflicts_resolved"], 0)

    def test_agents_that_give_up_complete_nothing(self):
        r, ids = self.one({"a": GIVE_UP, "b": GIVE_UP, "c": GIVE_UP})
        self.assertEqual(r["outcome"], "ok")
        self.assertEqual(r["metrics"]["units_completed"], 0)
        self.assertEqual(r["metrics"]["overwrites"], 0)
        self.assertEqual(r["scenario_metrics"]["lost_edits"], ["A", "B", "C"])
        self.assertEqual(r["scenario_metrics"]["per_agent"], {})
        self.assertFalse(any(ids[k] for k in ("timeout", "retries", "header")))

    def test_time_to_recover_is_null_and_a_b_arm_claim_conflicts_is_null_not_zero(self):
        r, _ = self.one()
        self.assertIsNone(r["metrics"]["claim_conflicts"])
        self.assertIsNone(r["scenario_metrics"]["time_to_recover_s"])

    def test_a_replace_step_whose_text_is_absent_fails_the_agent(self):
        bad = [{"op": "replace", "path": CORE, "old": "this text is not in the file", "new": "x"}]
        r, _ = self.one({"a": steps("a"), "b": steps("b", wait_for="a"), "c": bad})
        self.assertEqual(r["outcome"], "crash")


class TestDryRun(S2Base):
    def test_dry_run_prices_the_s2_plan_from_the_fixture(self):
        code, out, err = self.go(self.argv("B0,B1,K1,K2", 20) + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("3 agents per run", out)
        self.assertIn("80 runs", out)
        self.assertIn("estimated cost", out)


if __name__ == "__main__":
    unittest.main()
