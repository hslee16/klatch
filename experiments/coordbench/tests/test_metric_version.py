"""#1521, #1522, #1524: metric_version in the record and the report, S1's primary as an integration outcome, the coordination
context record and the B1 board metrics through the whole harness with fake agents. No model, no spend."""
import copy
import unittest

from helpers import BENCH, make_record  # noqa: F401
import metricver
import report
import scenario
from test_run_fake import Base
from test_s1_scenario import S1Base, steps, TICKETS


def checks(**over):
    base = {"units": True, "tokenizer": True, "dates": True, "checksum": True, "slug": True, "roman": True, "rle": True,
            "matrix": True, "registry": True, "changelog": True, "no-conflict-markers": True, "visible-tests": True}
    base.update(over)
    return [{"id": k, "pass": v, **({} if k in ("units", "tokenizer", "dates", "checksum", "slug", "roman", "rle", "matrix")
                                      else {"unit": False})} for k, v in base.items()]


class IntegrationOk(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s1")

    def test_s1_names_integration_ok_as_its_primary_and_units_as_secondary(self):
        m = self.scn["metrics"]
        self.assertEqual(m["primary"], "integration_ok")
        self.assertIn("units_completed", m["secondary"])
        self.assertIn("integration_ok", m["definitions"])
        self.assertEqual(sorted(self.scn["integration_checks"]),
                         ["changelog", "no-conflict-markers", "registry", "visible-tests"])

    def test_all_four_structural_checks_passing_is_an_integration_outcome(self):
        self.assertIs(scenario.integration_ok(self.scn, checks()), True)

    def test_a_failed_module_does_not_fail_integration(self):
        c = checks(tokenizer=False)
        self.assertIs(scenario.integration_ok(self.scn, c), True)  # model ability goes to units_completed only

    def test_each_structural_failure_fails_integration(self):
        for k in ("registry", "changelog", "no-conflict-markers", "visible-tests"):
            self.assertIs(scenario.integration_ok(self.scn, checks(**{k: False})), False, k)

    def test_a_missing_check_is_not_computed_never_false(self):
        c = [x for x in checks() if x["id"] != "registry"]
        self.assertIsNone(scenario.integration_ok(self.scn, c))

    def test_a_scenario_without_integration_checks_has_no_such_metric(self):
        self.assertIsNone(scenario.integration_ok({"metrics": {}}, checks()))


def rec(version=None, integ=None, **kw):
    r = make_record(**kw)
    if version is not None:
        r["metric_version"] = version
    if integ is not None:
        r["metrics"]["integration_ok"] = integ
    return r


class ReportCompletion(unittest.TestCase):
    def test_a_record_with_integration_ok_is_complete_by_it_not_by_units(self):
        self.assertTrue(report.is_complete(rec(integ=True, units=(8, 5))))
        self.assertFalse(report.is_complete(rec(integ=False, units=(8, 8))))

    def test_a_record_without_it_is_complete_by_units_as_before(self):
        self.assertTrue(report.is_complete(rec(units=(2, 2))))
        self.assertFalse(report.is_complete(rec(units=(2, 1))))

    def test_the_cell_and_the_table_say_which_basis_the_completion_rate_has(self):
        rs = [rec(scenario="S1", arm="B0", run="r%d" % i, integ=(i % 2 == 0), units=(8, 6), version=2) for i in range(4)]
        a = report.analyse(rs, resamples=50)
        self.assertEqual(a["scenarios"]["S1"]["arms"]["B0"]["completion_basis"], "integration_ok")
        self.assertEqual(a["scenarios"]["S1"]["arms"]["B0"]["completed"], 2)
        self.assertIn("Completed (integration ok)", report.render_markdown(a))

    def test_without_integration_ok_the_cell_has_no_basis_key_and_the_old_header(self):
        a = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        self.assertNotIn("completion_basis", a["scenarios"]["trivial"]["arms"]["B0"])
        self.assertIn("Completed (all units)", report.render_markdown(a))


class MetricVersionInReport(unittest.TestCase):
    def test_records_without_a_version_are_version_one(self):
        self.assertEqual(metricver.version_of(rec()), 1)
        self.assertEqual(metricver.version_of(rec(version=2)), 2)

    def test_old_only_records_leave_the_analysis_unchanged(self):
        a = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        self.assertNotIn("metric_versions", a)

    def test_mixed_versions_are_never_pooled(self):
        rs = [rec(run="r1", version=1, turns=10), rec(run="r2", version=1, turns=12),
              rec(run="r3", version=2, turns=40), rec(run="r4", version=2, turns=44)]
        a = report.analyse(rs, resamples=50)
        self.assertEqual(a["metric_versions"], {"1": 2, "2": 2})
        self.assertEqual(sorted(a["scenarios"]), ["trivial", "trivial [metric v1]"])
        self.assertEqual(a["scenarios"]["trivial"]["arms"]["B0"]["n"], 2)
        self.assertEqual(a["scenarios"]["trivial [metric v1]"]["arms"]["B0"]["n"], 2)
        md = report.render_markdown(a)
        self.assertIn("metric versions", md.lower())
        self.assertIn("not pooled", md)

    def test_the_conditions_list_the_versions_present(self):
        a = report.analyse([rec(run="r1", version=2)], resamples=50)
        self.assertEqual(a["conditions"]["metric version"], [2])

    def test_comparing_two_analyses_of_different_versions_says_so(self):
        new = report.analyse([rec(run="r1", version=2), rec(run="r2", version=2)], resamples=50)
        old = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        c = report.compare(new, old, resamples=50)
        self.assertIn("metric_version_mismatch", c)
        md = report.render_markdown(new, c, "OLD", "NEW")
        self.assertIn("different metric versions", md)

    def test_comparing_the_same_version_adds_nothing(self):
        new = report.analyse([rec(run="r1"), rec(run="r2")], resamples=50)
        self.assertNotIn("metric_version_mismatch", report.compare(new, copy.deepcopy(new), resamples=50))


class ScaleParts(unittest.TestCase):
    """S7 reads the same sources as the record: inbound messages and tool schemas are per member figures too (#1521)."""

    def states(self):
        import streams
        from test_coord_context import INIT_B1, inbound, asst
        st = streams.StreamState(klatch_bin="/opt/klatch/klatch")
        for line in (INIT_B1, asst("m1", [{"type": "text", "text": "x"}]), inbound("y" * 400)):
            st.feed(line)
        return {"a": st, "b": streams.StreamState(klatch_bin="/opt/klatch/klatch")}

    def test_stream_parts_carry_inbound_and_schema_chars(self):
        import ctxcount
        import scale
        sp = scale.stream_parts(self.states(), 2, 0)
        self.assertEqual(sp["inbound_context_chars"], 400)
        self.assertEqual(sp["schema_context_chars"], ctxcount.SCHEMA_CHARS["SendMessage"] + ctxcount.SCHEMA_CHARS["ListAgents"])
        self.assertEqual(sp["context_method"], "chars/4")

    def test_derive_gives_per_member_inbound_and_schema_tokens(self):
        import ctxcount
        import scale
        sp = scale.stream_parts(self.states(), 2, 0)
        d = scale.derive({"scenario_metrics": {"scale": sp}, "metrics": {}, "turns": {}, "spend": {}})
        self.assertEqual(d["inbound_tokens_per_member"], 400 / 4 / 2)
        self.assertEqual(d["schema_tokens_per_member"], (ctxcount.SCHEMA_CHARS["SendMessage"] + ctxcount.SCHEMA_CHARS["ListAgents"]) / 4 / 2)
        self.assertIn("inbound_tokens_per_member", [m for m, _ in scale.METRICS])


class HarnessRecord(S1Base):
    def run_arm(self, arm, agents):
        argv = self.argv(arm, 1, fake_script=self.script_for(agents))
        code, out, err = self.go(argv)
        recs = self.records(arm)
        self.assertEqual(len(recs), 1, out + err)
        return recs[0]

    def test_a_new_record_carries_the_metric_version_and_the_context_method(self):
        r, ids = self.one()
        self.assertEqual(r["metric_version"], metricver.METRIC_VERSION)
        cc = r["coordination_context"]
        self.assertEqual(cc["method"], "chars/4")
        self.assertEqual(cc["chars_per_token"], 4)
        self.assertIn("limits", cc)
        self.assertEqual(cc["tokens"], r["metrics"]["coordination_context_tokens"])

    def test_integration_ok_is_in_the_record_and_units_stay_secondary(self):
        r, ids = self.one()
        self.assertIs(r["metrics"]["integration_ok"], True)
        self.assertEqual(r["metrics"]["units_completed"], 8)
        self.assertIs(r["scenario_metrics"]["integration_ok"], True)

    def test_overwritten_registry_lines_fail_integration_though_every_module_is_there(self):
        r, ids = self.one(self.chain("ours"))
        self.assertEqual(r["metrics"]["units_completed"], 8)
        self.assertIs(r["metrics"]["integration_ok"], False)

    def test_a_scenario_without_integration_checks_has_no_integration_metric(self):
        code, out, err = self.go(Base.argv(self, "B0", 1))
        r = Base.records(self)[0]
        self.assertNotIn("integration_ok", r["metrics"])
        self.assertEqual(r["metric_version"], metricver.METRIC_VERSION)

    def b1_chain(self, same_file):
        agents = {}
        for n in "abcd":
            letter, mods = TICKETS[n]
            f = "tallykit/units.py" if same_file else "tallykit/%s.py" % mods[0]
            s = [{"op": "append", "path": "TASKS.md", "content": "- %s | working | %s | ticket %s\n" % (n, f, letter)},
                 {"op": "sync", "message": "claim %s" % n, "resolve": "union"}]
            agents[n] = s + steps(n, "union")
        return agents

    def test_b1_claim_conflicts_come_from_the_board_and_claim_lines_are_not_messages(self):
        r = self.run_arm("B1", self.b1_chain(same_file=True))
        self.assertGreaterEqual(r["metrics"]["claim_conflicts"], 1)
        self.assertEqual(r["metrics"]["messages"], 0)  # a claim line is a board edit, not a message (#1520 review)
        self.assertIsNotNone(r["scenario_metrics"]["board"]["claim_conflict_detail"])

    def test_b1_with_distinct_files_has_zero_claim_conflicts(self):
        r = self.run_arm("B1", self.b1_chain(same_file=False))
        self.assertEqual(r["metrics"]["claim_conflicts"], 0)

    def test_b0_has_no_board_so_claim_conflicts_stay_null(self):
        r, _ = self.one()
        self.assertIsNone(r["metrics"]["claim_conflicts"])


if __name__ == "__main__":
    unittest.main()
