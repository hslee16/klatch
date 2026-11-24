"""Scenario S3 (#1368): the scenario file, the fixture's seeded facts, and the metrics of fixture F3 `chain` through the whole
harness with fake agents: a perfect sequence, a blocked agent that polls many times, a handoff with the quirk passed, a
handoff where the quirk is lost, and partial work dropped. No model, no spend."""
import json
import os
import re
import subprocess
import sys
import unittest

from helpers import BENCH  # noqa: F401
import scenario
import schema
from test_run_fake import Base, KLATCH_BIN, NEEDS_BIN, read_json

FIXTURE = os.path.join(BENCH, "fixtures", "chain")
REF = os.path.join(FIXTURE, "reference", "chainkit")
EXPECTED = read_json(os.path.join(BENCH, "checks", "chain", "expected.json"))
Q = EXPECTED["quirk"]


def cp(name, to):
    return {"op": "copy_from", "from": os.path.join(REF, name), "path": "chainkit/" + to}


PULL = {"op": "git", "args": ["pull", "--no-rebase", "--no-edit", "origin", "main"]}


def wait(path, contains=""):
    return {"op": "wait_origin_has", "path": path, "contains": contains, "timeout": 30}


def handoff_note(with_quirk=True):
    text = "# Handoff from a\nschema.json is published. parse_bin is done. canonical_bin is unfinished.\n"
    if with_quirk:
        text += "Quirk: bins with prefix %s are legacy: canonical prefix %s, number plus %d.\n" % (Q["legacy"], Q["new"], Q["offset"])
    return text


def a_steps(with_quirk=True):
    return [cp("schema.json", "schema.json"), cp("bins_partial.py", "bins.py"),
            {"op": "write", "path": "HANDOFF.md", "content": handoff_note(with_quirk)},
            {"op": "sync", "message": "step 1 and the start of step 2", "resolve": "union"},
            {"op": "final", "text": "handed over to c. DONE a"}]


def b_steps(polls=0):
    s = [{"op": "poll", "n": polls, "command": "sleep 5"}] if polls else []
    return s + [wait("chainkit/schema.json"), PULL, cp("loader.py", "loader.py"),
                {"op": "sync", "message": "step 3", "resolve": "union"}, {"op": "final", "text": "loader published. DONE b"}]


def c_steps(bins):
    """bins: a fake-agent step that writes chainkit/bins.py after the pull of a's work."""
    return [wait("HANDOFF.md"), PULL, bins, {"op": "sync", "message": "step 2", "resolve": "union"},
            wait("chainkit/loader.py", "def load"), PULL, cp("report.py", "report.py"),
            {"op": "sync", "message": "step 4", "resolve": "union"}, {"op": "final", "text": "report published. DONE c"}]


def dropped_bins_text():
    with open(os.path.join(REF, "bins.py")) as f:
        final = f.read()
    start, end = final.index("def parse_bin"), final.index("LEGACY =")
    return final[:start] + "def _p(raw):\n    import re\n    m = re.match(r'([A-Z]{2})[-/]?(\\d+)$', raw)\n" \
                           "    return Bin(m.group(1), int(m.group(2)))\n\n\n" + final[end:].replace("parse_bin(", "_p(")


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s3")

    def test_arms_n_caps_and_metrics_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["id"], "S3")
        self.assertEqual(s["fixture"], "chain")
        self.assertEqual(s["arms"], ["B0", "B1", "K1", "K2"])
        self.assertEqual(s["n_per_arm"]["m0"], 10)
        self.assertEqual(s["n_per_arm"]["quick"], 3)
        self.assertEqual(s["caps"]["per_run_usd"], 2.4)
        self.assertEqual(s["caps"]["suite_usd_m0_haiku"], 40.0)
        self.assertEqual(s["metrics"]["primary"], "steps_correct")
        for k in ("steps_correct", "quirk_honoured", "lost_at_handoff", "idle_turns", "wasted_turns", "blocked_turns",
                  "guess_build_turns", "handoff_claims", "time_to_completion_s", "order"):
            self.assertIn(k, s["metrics"]["definitions"], k)

    def test_the_idle_definition_is_precise(self):
        d = self.scn["metrics"]["definitions"]["idle_turns"]
        for word in ("sleep", "klatch wait", "git fetch", "text-only", "before"):
            self.assertIn(word, d)

    def test_the_agents_match_the_fixture_and_the_scenario_is_found_from_it(self):
        fx = read_json(os.path.join(FIXTURE, "fixture.json"))
        self.assertEqual(fx["scenario"], "s3")
        self.assertEqual([a["name"] for a in fx["agents"]], sorted(self.scn["agents"]))
        self.assertEqual(self.scn["handoff"], {"from": "a", "to": "c"})
        self.assertEqual(sorted(self.scn["blocked"]), ["b", "c"])

    def test_the_hook_that_reads_the_streams_is_named(self):
        self.assertEqual(self.scn["stream_metrics"], "chain")

    def test_whether_a_claim_moves_with_a_handoff_is_recorded_not_graded_and_the_store_gap_is_stated(self):
        self.assertIn("transferred_by_klatch", self.scn["not_computed"])
        self.assertIn("#539", self.scn["metrics"]["definitions"]["handoff_claims"])
        self.assertIn("not graded", self.scn["metrics"]["definitions"]["handoff_claims"])


class TestFixtureFacts(unittest.TestCase):
    def test_the_generator_reproduces_every_committed_file(self):
        p = subprocess.run([sys.executable, "-I", os.path.join(FIXTURE, "gen.py"), "--check"], stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)

    def test_the_quirk_is_seeded_and_not_guessable(self):
        self.assertRegex(Q["legacy"], r"^[A-Z]{2}$")
        self.assertRegex(Q["new"], r"^[A-Z]{2}$")
        self.assertNotEqual(Q["legacy"], Q["new"])
        self.assertTrue(100 < Q["offset"] < 900)
        self.assertEqual(EXPECTED["seed"], 1368)

    def test_the_quirk_appears_in_a_s_task_and_in_no_agent_visible_file_but_a_s(self):
        with open(os.path.join(FIXTURE, "task-a.md")) as f:
            a = f.read()
        self.assertIn(Q["legacy"], a)
        self.assertIn(Q["new"], a)
        self.assertIn(str(Q["offset"]), a)
        for name in ("task-b.md", "task-c.md"):
            with open(os.path.join(FIXTURE, name)) as f:
                t = f.read()
            self.assertNotIn(Q["new"], t, name)
            self.assertNotIn(str(Q["offset"]), t, name)
        for d, _, names in os.walk(os.path.join(FIXTURE, "template")):
            for n in names:
                with open(os.path.join(d, n)) as f:
                    t = f.read()
                self.assertNotIn(Q["new"], t, os.path.join(d, n))
                self.assertNotIn("offset", t.lower(), os.path.join(d, n))

    def test_the_data_has_legacy_records_and_the_totals_depend_on_the_quirk(self):
        recs = EXPECTED["records"]
        legacy = [r for r in recs if r["bin"].startswith(Q["legacy"])]
        self.assertGreaterEqual(len(legacy), 5)
        self.assertGreaterEqual(len(recs), 40)
        plain = {}
        for r in recs:
            m = re.fullmatch(r"([A-Z]{2})[-/]?(\d+)", r["bin"])
            code = "%s-%05d" % (m.group(1), int(m.group(2)))
            plain[code] = plain.get(code, 0) + r["qty"]
        self.assertNotEqual(plain, EXPECTED["totals"])

    def test_setup_copies_the_template_and_never_the_reference_or_a_checker_file(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            dest = os.path.join(d, "w")
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "7"], check=True)
            seen = set()
            for root, _, names in os.walk(dest):
                for n in names:
                    seen.add(os.path.relpath(os.path.join(root, n), dest))
        self.assertIn("data/inventory.dat", seen)
        self.assertIn("tools/make_schema.py", seen)
        self.assertNotIn("chainkit/schema.json", seen, "the intermediate file is generated by agent a")
        self.assertFalse([s for s in seen if s.startswith("reference") or "expected" in s or s.endswith("loader.py")], seen)

    def test_the_schema_script_makes_the_expected_file_from_the_ruler_only_a_was_given(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            dest = os.path.join(d, "w")
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "7"], check=True)
            subprocess.run([sys.executable, "-I", "tools/make_schema.py", EXPECTED["ruler"]], cwd=dest, check=True,
                           stdout=subprocess.DEVNULL)
            got = read_json(os.path.join(dest, "chainkit", "schema.json"))
        self.assertEqual(got, EXPECTED["schema"])
        self.assertEqual(got, read_json(os.path.join(REF, "schema.json")))

    def test_b_cannot_make_the_schema_itself_the_ruler_is_in_a_task_text_only(self):
        """#1525 (I5): the ruler used to be data/ruler.txt in the checkout, so b ran the script and skipped the wait."""
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            dest = os.path.join(d, "w")
            subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "7"], check=True)
            p = subprocess.run([sys.executable, "-I", "tools/make_schema.py"], cwd=dest, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
            self.assertNotEqual(p.returncode, 0, "the script needs the ruler as its argument")
            self.assertFalse(os.path.exists(os.path.join(dest, "chainkit", "schema.json")))
            self.assertFalse(os.path.exists(os.path.join(dest, "data", "ruler.txt")))
            blob = ""
            for root, _, names in os.walk(dest):
                for n in names:
                    with open(os.path.join(root, n), errors="replace") as f:
                        blob += f.read()
        self.assertNotIn(EXPECTED["ruler"], blob, "no file of the checkout states the column widths")

    def test_only_agent_a_task_text_has_the_ruler(self):
        texts = {n: open(os.path.join(FIXTURE, "task-%s.md" % n)).read() for n in "abc"}
        self.assertIn(EXPECTED["ruler"], texts["a"])
        self.assertNotIn(EXPECTED["ruler"], texts["b"] + texts["c"])
        self.assertIn("tools/make_schema.py", texts["a"])


class S3Base(Base):
    def argv(self, arms="B0", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "chain"
        return a

    def records(self):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "chain")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return sorted(out, key=lambda r: r["run_id"])

    def one(self, agents=None, arm="B0"):
        extra = {}
        if agents:
            path = os.path.join(self.tmp, "fake-s3-%d.json" % len(os.listdir(self.tmp)))
            with open(path, "w") as f:
                json.dump({"agents": agents}, f)
            extra["fake_script"] = path
        code, out, err = self.go(self.argv(arm, 1, **extra))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r, {c["id"]: c["pass"] for c in r["outcome_checks"]}


class TestFakeAgentsOnS3(S3Base):
    def test_the_fixtures_own_fake_script_scores_full_marks(self):
        r, ids = self.one()
        self.assertEqual(r["outcome"], "ok")
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (4, 4))
        sm = r["scenario_metrics"]
        self.assertTrue(sm["quirk_honoured"])
        self.assertEqual(sm["lost_at_handoff"], [])
        self.assertTrue(sm["correct"])
        self.assertEqual(sm["steps_correct"], 4)
        self.assertEqual(sorted(sm["blocked"]), ["b", "c"])
        self.assertEqual(r["metrics"]["overwrites"], 0)

    def test_a_perfect_sequence_has_no_idle_turns_for_the_blocked_agent(self):
        r, ids = self.one({"a": a_steps(), "b": b_steps(), "c": c_steps(cp("bins.py", "bins.py"))})
        self.assertEqual(r["metrics"]["units_completed"], 4)
        sm = r["scenario_metrics"]
        b = sm["blocked"]["b"]
        self.assertEqual((b["idle_turns"], b["guess_build_turns"], b["wasted_turns"], b["blocked_turns"]), (0, 0, 0, 0))
        self.assertTrue(b["dependency_seen"])
        self.assertEqual(sm["order"], {"loader_after_schema": True, "report_after_loader": True})
        self.assertIsNone(sm["handoff_claims"], "B0 has no claims to follow")

    def test_a_blocked_agent_that_polls_many_times_is_idle_for_each_poll(self):
        r, ids = self.one({"a": a_steps(), "b": b_steps(polls=9), "c": c_steps(cp("bins.py", "bins.py"))})
        self.assertEqual(r["metrics"]["units_completed"], 4)
        b = r["scenario_metrics"]["blocked"]["b"]
        self.assertEqual(b["idle_turns"], 9)
        self.assertEqual(b["wasted_turns"], 9)
        self.assertEqual(b["blocked_turns"], 9)
        self.assertEqual(b["dependency_seen_turn"], 10)
        self.assertGreater(r["tool_calls"]["by_tool"]["Bash"], 9)

    def test_a_handoff_with_the_quirk_passed_makes_the_quirk_records_right(self):
        r, ids = self.one({"a": a_steps(True), "b": b_steps(), "c": c_steps(cp("bins.py", "bins.py"))})
        sm = r["scenario_metrics"]
        self.assertTrue(sm["quirk_honoured"])
        self.assertTrue(ids["quirk"])
        self.assertTrue(ids["integrated"])
        self.assertEqual(sm["lost_at_handoff"], [])

    def test_a_handoff_where_the_quirk_is_lost_leaves_the_plain_cases_right_and_the_quirk_wrong(self):
        r, ids = self.one({"a": a_steps(False), "b": b_steps(), "c": c_steps(cp("bins_partial.py", "bins.py"))})
        sm = r["scenario_metrics"]
        self.assertFalse(sm["quirk_honoured"])
        self.assertFalse(ids["quirk"])
        self.assertTrue(ids["bins"])
        self.assertEqual(r["metrics"]["units_completed"], 4)
        self.assertFalse(sm["correct"])
        self.assertEqual(sm["lost_at_handoff"], [])
        self.assertFalse(ids["integrated"])

    def test_partial_work_dropped_at_the_handoff_is_lost_work_and_a_lost_line(self):
        bins = {"op": "write", "path": "chainkit/bins.py", "content": dropped_bins_text()}
        r, ids = self.one({"a": a_steps(), "b": b_steps(), "c": c_steps(bins)})
        sm = r["scenario_metrics"]
        self.assertEqual([x["piece"] for x in sm["lost_at_handoff"]], ["parse_bin"])
        self.assertTrue(sm["quirk_honoured"], "c knew the quirk and still dropped a's function")
        self.assertEqual(r["metrics"]["units_completed"], 3)
        self.assertFalse(ids["bins"])
        self.assertEqual(r["metrics"]["overwrites"], 1, "the line a added that defines parse_bin is not in main")
        self.assertFalse(sm["correct"])

    def test_an_agent_that_builds_on_a_guess_before_the_dependency_is_a_guess_build_and_out_of_order(self):
        guess = [cp("loader.py", "loader.py"), {"op": "sync", "message": "loader on a guess", "resolve": "union"}]
        b = guess + [wait("chainkit/schema.json"), PULL, {"op": "final", "text": "DONE b"}]
        r, ids = self.one({"a": a_steps(), "b": b, "c": c_steps(cp("bins.py", "bins.py"))})
        sm = r["scenario_metrics"]
        self.assertFalse(sm["order"]["loader_after_schema"])
        self.assertFalse(ids["order-loader-after-schema"])
        self.assertEqual(sm["blocked"]["b"]["guess_build_turns"], 1)
        self.assertFalse(sm["correct"])
        self.assertEqual(r["metrics"]["units_completed"], 4, "the final tree is right; the order is its own finding")

    def test_the_run_record_is_built_from_the_streams_not_the_final_text(self):
        r, _ = self.one()
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b", "c"])
        self.assertIn("blocked", r["scenario_metrics"])
        self.assertIn("handoff_claims", r["scenario_metrics"])


class TestKArmFollowsTheClaim(S3Base):
    @NEEDS_BIN
    def test_a_k_run_records_what_a_did_with_its_claim_at_the_handoff(self):
        r, ids = self.one(arm="K2")
        h = r["scenario_metrics"]["handoff_claims"]
        self.assertIsNotNone(h)
        self.assertEqual(h["a_claimed"], ["chainkit/bins.py"])
        self.assertIn("chainkit/bins.py", h["c_claimed"])
        self.assertTrue(h["handoff_message"])
        self.assertTrue(h["checkpoint_message"])
        self.assertIsNone(h["transferred_by_klatch"])
        self.assertEqual(r["metrics"]["units_completed"], 4)


class TestDryRun(S3Base):
    def test_dry_run_prices_the_s3_plan_from_the_fixture(self):
        code, out, err = self.go(self.argv("B0,B1,K1,K2", 10) + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("3 agents per run", out)
        self.assertIn("40 runs", out)
        self.assertIn("estimated cost", out)


if __name__ == "__main__":
    unittest.main()
