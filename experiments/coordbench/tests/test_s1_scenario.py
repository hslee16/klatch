"""Scenario S1 (#1366): the scenario file, and the metrics of fixture F1 `tallykit` through the whole harness with fake
agents: agents that solve it correctly, that collide on the registry file, that overwrite each other's lines and that
implement a module twice. No model, no spend."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import scenario
import schema
from test_run_fake import Base, read_json

REFERENCE = os.path.join(BENCH, "fixtures", "tallykit", "reference")
MODULES = ["units", "tokenizer", "dates", "checksum", "slug", "roman", "rle", "matrix"]
FUNCS = {"units": "convert_length, convert_temperature", "tokenizer": "tokenize, token_kinds",
         "dates": "is_leap_year, days_in_month, days_between", "checksum": "adler32, luhn_valid",
         "slug": "slugify, unique_slug", "roman": "to_roman, from_roman", "rle": "rle_encode, rle_decode",
         "matrix": "transpose, multiply, identity"}
TICKETS = {"a": ("A", ["units", "tokenizer"]), "b": ("B", ["dates", "checksum"]), "c": ("C", ["slug", "roman"]),
           "d": ("D", ["rle", "matrix"])}


def steps(agent, resolve="union", wait_for=None, extra_modules=(), registry=True):
    """Fake-agent steps for one ticket: copy the reference modules, add the registry and changelog lines, sync."""
    letter, mods = TICKETS[agent]
    out = []
    for m in list(mods) + list(extra_modules):
        out.append({"op": "copy_from", "from": os.path.join(REFERENCE, "tallykit", m + ".py"), "path": "tallykit/%s.py" % m})
    if registry:
        for m in mods:
            out.append({"op": "append", "path": "tallykit/__init__.py", "content": "from tallykit.%s import %s\n" % (m, FUNCS[m])})
    out.append({"op": "append", "path": "CHANGELOG.md", "content": "- ticket %s: %s\n" % (letter, ", ".join(mods))})
    if wait_for:
        out.append({"op": "wait_origin_has", "path": "CHANGELOG.md", "contains": "- ticket %s:" % wait_for, "timeout": 30})
    out.append({"op": "sync", "message": "ticket %s" % letter, "resolve": resolve})
    out.append({"op": "final", "text": "ticket %s is published. DONE %s" % (letter, agent)})
    return out


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s1")

    def test_arms_n_caps_and_metrics_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["fixture"], "tallykit")
        self.assertEqual(s["arms"], ["B0", "B1", "K1", "K2", "K3", "B1-roles"])
        self.assertEqual(s["arms_at_m0"], ["B0", "B1", "K1", "K2"])
        self.assertEqual(s["n_per_arm"]["m0"], 10)
        self.assertEqual(s["n_per_arm"]["quick"], 3)
        self.assertEqual(s["caps"]["per_run_usd"], 6.0)
        self.assertEqual(s["caps"]["suite_usd_m0_haiku"], 100.0)
        for k in ("duplicate_work", "overwrites", "git_conflicts", "units_completed"):
            self.assertIn(k, s["metrics"]["definitions"])
        self.assertEqual(s["metrics"]["primary"], "units_completed")

    def test_the_unit_to_path_map_covers_every_module_and_every_ticket(self):
        s = self.scn
        self.assertEqual(sorted(s["units"]), sorted(MODULES))
        for m, u in s["units"].items():
            self.assertEqual(u["path"], "tallykit/%s.py" % m)
            self.assertTrue(os.path.exists(os.path.join(REFERENCE, u["path"])))
        owned = [m for a in s["agents"].values() for m in a["units"]]
        self.assertEqual(sorted(owned), sorted(MODULES))
        self.assertEqual({k: (v["ticket"], v["units"]) for k, v in s["agents"].items()}, TICKETS)

    def test_the_agents_match_the_fixture_and_the_scenario_is_found_from_it(self):
        with open(os.path.join(BENCH, "fixtures", "tallykit", "fixture.json")) as f:
            fx = json.load(f)
        self.assertEqual(fx["scenario"], "s1")
        self.assertEqual([a["name"] for a in fx["agents"]], sorted(self.scn["agents"]))

    def test_shared_files_have_entry_patterns_that_match_what_an_agent_adds(self):
        import re
        shared = self.scn["shared_files"]
        self.assertEqual(sorted(shared), ["CHANGELOG.md", "tallykit/__init__.py"])
        self.assertTrue(re.match(shared["tallykit/__init__.py"]["entry"], "from tallykit.units import convert_length"))
        self.assertTrue(re.match(shared["CHANGELOG.md"]["entry"], "- ticket A: units, tokenizer"))
        self.assertFalse(re.match(shared["tallykit/__init__.py"]["entry"], "from tallykit import something"))

    def test_an_unknown_scenario_is_an_error(self):
        with self.assertRaises(scenario.ScenarioError):
            scenario.load("nope")


class S1Base(Base):
    def argv(self, arms="B0", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "tallykit"
        return a

    def records(self, arm=None):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "tallykit")):
            for n in names:
                if n.endswith(".json") and (arm is None or os.path.basename(d) == arm):
                    out.append(read_json(os.path.join(d, n)))
        return sorted(out, key=lambda r: r["run_id"])

    def script_for(self, agents):
        path = os.path.join(self.tmp, "fake-s1-%d.json" % len(os.listdir(self.tmp)))
        with open(path, "w") as f:
            json.dump({"agents": agents}, f)
        return path

    def one(self, agents=None, **kw):
        extra = {"fake_script": self.script_for(agents)} if agents else {}
        code, out, err = self.go(self.argv("B0", 1, **extra, **kw))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r, {c["id"]: c["pass"] for c in r["outcome_checks"]}

    def chain(self, resolve="union", **special):
        agents, prev = {}, None
        for n in "abcd":
            agents[n] = steps(n, resolve, wait_for=prev, **special.get(n, {}))
            prev = TICKETS[n][0]
        return agents


class TestFakeAgentsOnS1(S1Base):
    def test_agents_that_solve_it_correctly_score_eight_of_eight(self):
        r, ids = self.one()  # the fixture's own fake script, all four at once
        self.assertEqual(r["outcome"], "ok")
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (8, 8))
        self.assertTrue(all(ids.values()), ids)
        self.assertEqual(r["metrics"]["duplicate_work"], 0)
        self.assertEqual(r["metrics"]["overwrites"], 0)
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b", "c", "d"])
        sm = r["scenario_metrics"]
        self.assertEqual(sm["merge_state"], "clean")
        self.assertEqual(sorted(sm["per_agent"]), ["a", "b", "c", "d"])
        self.assertEqual(sm["per_agent"]["c"]["modules"], ["roman", "slug"])
        self.assertEqual(sm["foreign_edits"], [])

    def test_agents_that_collide_on_the_registry_and_resolve_it_lose_nothing(self):
        r, ids = self.one(self.chain("union"))
        self.assertEqual(r["metrics"]["units_completed"], 8)
        self.assertTrue(all(ids.values()), ids)
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 3, "b, c and d each pull a conflicted registry")
        self.assertEqual(r["metrics"]["overwrites"], 0)
        self.assertEqual(r["metrics"]["duplicate_work"], 0)
        self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")

    def test_an_agent_that_does_not_resolve_its_conflict_loses_its_ticket(self):
        agents = self.chain("union")
        agents["c"] = steps("c", None, wait_for="B")  # no resolve: the pull stops on the conflict and the agent fails
        agents["d"] = steps("d", "union", wait_for="B")
        r, ids = self.one(agents)
        self.assertEqual(r["outcome"], "crash")
        self.assertEqual(r["metrics"]["units_completed"], 6)
        self.assertFalse(ids["slug"] or ids["roman"], ids)
        self.assertFalse(ids["changelog"], ids)
        self.assertGreaterEqual(r["metrics"]["git_conflicts"], 1)

    def test_agents_that_overwrite_each_others_lines_are_counted_from_history(self):
        r, ids = self.one(self.chain("ours"))
        self.assertEqual(r["metrics"]["units_completed"], 8, "every module is there; only the shared lines are lost")
        self.assertFalse(ids["registry"])
        self.assertFalse(ids["changelog"])
        self.assertTrue(ids["no-conflict-markers"])
        # a, b and c each lost two registry lines and one changelog line; d's survive
        self.assertEqual(r["metrics"]["overwrites"], 9)
        lost = r["scenario_metrics"]["lost_lines"]
        self.assertEqual(sorted({l["file"] for l in lost}), ["CHANGELOG.md", "tallykit/__init__.py"])
        self.assertEqual(sorted({l["author"] for l in lost}), ["a", "b", "c"])
        self.assertEqual(r["metrics"]["duplicate_work"], 0)

    def test_a_module_implemented_by_two_agents_is_duplicate_work_and_a_foreign_edit(self):
        r, ids = self.one(self.chain("union", b={"extra_modules": ["units"]}))
        self.assertEqual(r["metrics"]["units_completed"], 8)
        self.assertEqual(r["metrics"]["duplicate_work"], 1)
        self.assertEqual(r["scenario_metrics"]["duplicated_units"], ["units"])
        self.assertEqual(r["scenario_metrics"]["foreign_edits"], [{"unit": "units", "author": "b", "owner": "a"}])
        self.assertEqual(r["metrics"]["overwrites"], 0)

    def test_a_ticket_done_by_none_is_not_a_unit(self):
        agents = self.chain("union")
        agents["d"] = [{"op": "final", "text": "gave up"}]
        r, ids = self.one(agents)
        self.assertEqual(r["metrics"]["units_completed"], 6)
        self.assertFalse(ids["rle"] or ids["matrix"])
        self.assertFalse(ids["changelog"])

    def test_scenario_metrics_stay_null_for_a_fixture_without_a_scenario(self):
        code, out, err = self.go(Base.argv(self, "B0", 1))
        r = [x for x in self.records_trivial()][0]
        self.assertIsNone(r["metrics"]["duplicate_work"])
        # no scenario metrics, only the note that a B arm has no store (#1433)
        self.assertEqual(list(r["scenario_metrics"]), ["store"])
        self.assertIn("no klatch store", r["scenario_metrics"]["store"]["reason"])

    def records_trivial(self):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "trivial")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return out

    def test_each_agent_commits_under_its_own_name(self):
        r, _ = self.one()
        self.assertEqual(sorted(r["scenario_metrics"]["per_agent"]), ["a", "b", "c", "d"])
        self.assertNotIn("bench", r["scenario_metrics"]["per_agent"])


class TestDryRun(S1Base):
    def test_dry_run_prices_the_s1_plan_from_the_fixture(self):
        code, out, err = self.go(self.argv("B0,B1,K1,K2", 10) + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("4 agents per run", out)
        self.assertIn("40 runs", out)
        self.assertIn("estimated cost", out)


if __name__ == "__main__":
    unittest.main()
