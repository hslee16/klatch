"""The primary metric of each scenario comes from its scenario file (SPEC section 6, #1670); --primary overrides it."""
import unittest

from helpers import make_record
import report
import scenario


def runs(scn, n=6):
    out = []
    for arm, base in (("B0", 1.0), ("K2", 0.5)):
        for i in range(n):
            out.append(make_record(scenario=scn, arm=arm, run="r%02d" % i, cost=base + i / 10, turns=int(base * 10) + i,
                                   wall=60.0 + i))
    return out


def primaries(an, scn):
    return sorted({t["metric"] for t in an["scenarios"][scn]["tests"] if t["primary"] and t["metric"] != "completion"})


NAMED = {"alpha": "cost_usd", "beta": "turns", "gamma": "integration_ok"}.get


class PerScenario(unittest.TestCase):
    def analyse(self, **kw):
        return report.analyse(runs("alpha") + runs("beta") + runs("gamma") + runs("delta"), resamples=50,
                              primary_of=NAMED, **kw)

    def test_each_scenario_uses_the_primary_its_file_names(self):
        an = self.analyse()
        self.assertEqual(primaries(an, "alpha"), ["cost_usd"])
        self.assertEqual(primaries(an, "beta"), ["turns"])
        self.assertEqual(an["scenarios"]["alpha"]["primary_metric"], "cost_usd")

    def test_a_primary_that_is_not_a_tested_continuous_metric_leaves_only_the_completion_test_primary(self):
        an = self.analyse()
        self.assertEqual(primaries(an, "gamma"), [])
        self.assertTrue(any(t["metric"] == "completion" and t["primary"] for t in an["scenarios"]["gamma"]["tests"]))
        self.assertEqual(an["scenarios"]["gamma"]["primary_metric"], "integration_ok")

    def test_a_scenario_without_a_named_primary_falls_back_to_units_completed(self):
        self.assertEqual(primaries(self.analyse(), "delta"), ["units_completed"])

    def test_an_explicit_primary_overrides_every_scenario(self):
        an = self.analyse(primary="wall_s")
        for s in ("alpha", "beta", "gamma", "delta"):
            self.assertEqual(primaries(an, s), ["wall_s"], s)
        self.assertEqual(an["primary_metric"], "wall_s")

    def test_the_holm_family_is_the_per_scenario_primaries(self):
        an = self.analyse()
        # per scenario: 2 arms give one pair; its completion test is always primary, plus one primary metric when tested
        self.assertEqual(an["primary_tests_made"], 4 + 3)

    def test_the_tests_line_says_where_the_primary_comes_from(self):
        self.assertIn("file names", report.render_markdown(self.analyse()))
        self.assertIn("`wall_s`", report.render_markdown(self.analyse(primary="wall_s")))


class FromTheFiles(unittest.TestCase):
    def test_the_shipped_scenario_files_name_the_primaries(self):
        self.assertEqual(scenario.primary_metric("onefile"), "units_completed")
        self.assertEqual(scenario.primary_metric("tallykit"), "integration_ok")
        self.assertEqual(scenario.primary_metric("chain"), "steps_correct")

    def test_a_variant_and_a_metric_version_label_find_their_fixture(self):
        self.assertEqual(scenario.primary_metric("onefile-shared"), "units_completed")
        self.assertEqual(scenario.primary_metric("tallykit [metric v1]"), "integration_ok")

    def test_a_fixture_without_a_scenario_file_has_none(self):
        self.assertIsNone(scenario.primary_metric("trivial"))
        self.assertIsNone(scenario.primary_metric("no-such-fixture"))


if __name__ == "__main__":
    unittest.main()
