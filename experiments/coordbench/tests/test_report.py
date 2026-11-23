"""The report generator and the release comparison, on fixture results written by the tests (no harness run needed,
no binary, no model). One test at the end feeds it a real fake-agent suite."""
import contextlib
import io
import json
import os
import shutil
import tempfile
import unittest

from helpers import BENCH, make_record
import report
import stats


def write(root, rec, suite="s"):
    d = os.path.join(root, suite, rec["scenario"], rec["arm"])
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, rec["run_id"] + ".json"), "w") as f:
        json.dump(rec, f)


def fill(root, arm, n, units=(2, 2), suite="s", scenario="trivial", start=1, **kw):
    for i in range(start, start + n):
        write(root, make_record(scenario=scenario, arm=arm, run="r%02d" % i, suite=suite, units=units, **kw), suite)


def many(root, arm, specs, suite="s", scenario="trivial"):
    """specs: a list of (units_completed, cost, turns, wall, outcome)."""
    for i, (done, cost, turns, wall, outcome) in enumerate(specs, 1):
        write(root, make_record(scenario=scenario, arm=arm, run="r%02d" % i, suite=suite, units=(2, done), cost=cost,
                                turns=turns, wall=wall, outcome=outcome), suite)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="coordbench-report-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.new = os.path.join(self.tmp, "new")
        self.old = os.path.join(self.tmp, "old")

    def analyse(self, root=None, **kw):
        recs, _ = report.load_results(root or self.new)
        return report.analyse(recs, resamples=300, **kw)


class Tables(Base):
    def setUp(self):
        super().setUp()
        # B0: 3 of 10 complete (all units); K2: 9 of 10
        many(self.new, "B0", [(2, 0.5 + i / 100, 20 + i, 100 + i, "ok") if i < 3 else (1, 0.5 + i / 100, 20 + i, 100 + i, "ok")
                              for i in range(10)])
        many(self.new, "K2", [(2, 0.4 + i / 100, 12 + i, 80 + i, "ok") if i < 9 else (1, 0.4, 12, 80, "ok")
                              for i in range(10)])

    def test_cell_has_n_completion_and_wilson_interval(self):
        a = self.analyse()
        b0 = a["scenarios"]["trivial"]["arms"]["B0"]
        self.assertEqual(b0["n"], 10)
        self.assertEqual(b0["completed"], 3)
        self.assertAlmostEqual(b0["completion_rate"], 0.3)
        lo, hi = stats.wilson(3, 10)
        self.assertAlmostEqual(b0["wilson"][0], lo)
        self.assertAlmostEqual(b0["wilson"][1], hi)

    def test_metric_summaries(self):
        a = self.analyse()
        cost = a["scenarios"]["trivial"]["arms"]["K2"]["metrics"]["cost_usd"]
        self.assertEqual(cost["n"], 10)
        self.assertAlmostEqual(cost["median"], stats.median([0.4 + i / 100 if i < 9 else 0.4 for i in range(10)]))

    def test_fisher_between_arms_matches_the_stats_module(self):
        a = self.analyse()
        t = next(t for t in a["scenarios"]["trivial"]["tests"] if t["arm"] == "K2" and t["against"] == "B0"
                 and t["metric"] == "completion")
        self.assertEqual(t["method"], "Fisher exact")
        self.assertAlmostEqual(t["p"], stats.fisher_exact(9, 1, 3, 7))
        self.assertAlmostEqual(t["diff"], 0.6)
        self.assertIsNotNone(t["rr"])
        self.assertEqual(len(t["diff_ci"]), 2)

    def test_mann_whitney_with_shift_and_a_seeded_bootstrap(self):
        a = self.analyse()
        t = next(t for t in a["scenarios"]["trivial"]["tests"] if t["arm"] == "K2" and t["against"] == "B0"
                 and t["metric"] == "turns")
        self.assertTrue(t["method"].startswith("Mann-Whitney"))
        self.assertIn(t["u_method"], ("exact", "normal", "normal-ties"))
        self.assertLess(t["shift"], 0)  # K2 used fewer turns
        self.assertEqual(len(t["shift_ci"]), 2)
        self.assertEqual(self.analyse()["scenarios"]["trivial"]["tests"], a["scenarios"]["trivial"]["tests"])
        self.assertIn("bootstrap_seed", a)

    def test_only_k_against_b_pairs_are_tested(self):
        fill(self.new, "K1", 10, units=(2, 2), suite="s")
        a = self.analyse()
        pairs = {(t["arm"], t["against"]) for t in a["scenarios"]["trivial"]["tests"]}
        self.assertEqual(pairs, {("K1", "B0"), ("K2", "B0")})

    def test_holm_is_applied_to_the_primary_tests_and_the_count_is_stated(self):
        a = self.analyse()
        primary = [t for s in a["scenarios"].values() for t in s["tests"] if t["primary"]]
        self.assertTrue(primary)
        for t in primary:
            self.assertGreaterEqual(t["p_holm"], t["p"] - 1e-12)
        self.assertGreaterEqual(a["tests_made"], len(primary))
        for s in a["scenarios"].values():
            for t in s["tests"]:
                if not t["primary"]:
                    self.assertIsNone(t["p_holm"])

    def test_markdown_has_tables_with_n_and_no_em_dash(self):
        md = report.render_markdown(self.analyse())
        self.assertIn("| Arm | N |", md)
        self.assertIn("Fisher exact", md)
        self.assertIn("Mann-Whitney", md)
        self.assertIn("Tests made", md)
        self.assertNotIn("\u2014", md)

    def test_minimum_detectable_effect_is_stated_next_to_the_tests(self):
        md = report.render_markdown(self.analyse())
        self.assertIn("no significant difference", md.lower())
        self.assertIn("not the same as no difference", md.lower())

    def test_not_exploratory_at_n_ten(self):
        a = self.analyse()
        self.assertFalse(a["scenarios"]["trivial"]["exploratory"])
        self.assertNotIn("EXPLORATORY", report.render_markdown(a))


class Exploratory(Base):
    def test_n_under_five_is_labelled_exploratory_and_claims_nothing(self):
        fill(self.new, "B0", 4, units=(2, 0))
        fill(self.new, "K2", 3, units=(2, 2))
        a = self.analyse()
        self.assertTrue(a["scenarios"]["trivial"]["exploratory"])
        md = report.render_markdown(a)
        self.assertIn("EXPLORATORY", md)
        for t in a["scenarios"]["trivial"]["tests"]:
            self.assertTrue(t["exploratory"])
            self.assertFalse(t["significant"])

    def test_exactly_five_is_not_exploratory(self):
        fill(self.new, "B0", 5, units=(2, 0))
        fill(self.new, "K2", 5, units=(2, 2))
        self.assertFalse(self.analyse()["scenarios"]["trivial"]["exploratory"])

    def test_one_small_arm_makes_the_scenario_exploratory(self):
        fill(self.new, "B0", 10, units=(2, 0))
        fill(self.new, "K2", 4, units=(2, 2))
        self.assertTrue(self.analyse()["scenarios"]["trivial"]["exploratory"])


class InvalidAndAborted(Base):
    def test_failed_and_aborted_runs_stay_in_the_denominator(self):
        many(self.new, "B0", [(2, 0.1, 5, 50, "ok"), (0, 0.2, 9, 100, "timeout"), (0, 0.3, 9, 60, "spend-cap"),
                              (0, 0.0, 1, 5, "crash"), (2, 0.1, 5, 50, "ok")])
        c = self.analyse()["scenarios"]["trivial"]["arms"]["B0"]
        self.assertEqual(c["n"], 5)
        self.assertEqual(c["completed"], 2)
        self.assertEqual(c["outcomes"], {"ok": 2, "timeout": 1, "spend-cap": 1, "crash": 1})

    def test_censored_wall_times_are_counted_and_a_sensitivity_line_is_kept(self):
        many(self.new, "B0", [(2, 0.1, 5, 50 + i, "ok") for i in range(5)] + [(0, 0.1, 5, 100, "timeout")] * 2)
        many(self.new, "K2", [(2, 0.1, 5, 30 + i, "ok") for i in range(7)])
        a = self.analyse()
        self.assertEqual(a["scenarios"]["trivial"]["arms"]["B0"]["censored_wall"], 2)
        t = next(t for t in a["scenarios"]["trivial"]["tests"] if t["metric"] == "wall_s" and t["arm"] == "K2")
        self.assertIn("p_without_censored", t)
        self.assertIsNotNone(t["p_without_censored"])

    def test_invalid_runs_are_counted_not_analysed_and_flagged_over_twenty_percent(self):
        for i in range(1, 9):
            write(self.new, make_record(arm="B0", run="r%02d" % i))
        for i in range(9, 12):
            write(self.new, make_record(arm="B0", run="r%02d" % i, valid=False, outcome="error"))
        c = self.analyse()["scenarios"]["trivial"]["arms"]["B0"]
        self.assertEqual((c["n"], c["n_invalid"]), (8, 3))
        self.assertTrue(c["invalid_flag"])
        md = report.render_markdown(self.analyse())
        self.assertIn("invalid", md)

    def test_ten_percent_invalid_is_not_flagged(self):
        for i in range(1, 10):
            write(self.new, make_record(arm="B0", run="r%02d" % i))
        write(self.new, make_record(arm="B0", run="r10", valid=False, outcome="error"))
        self.assertFalse(self.analyse()["scenarios"]["trivial"]["arms"]["B0"]["invalid_flag"])


class Comparison(Base):
    def both(self, old_specs, new_specs, arm="K2"):
        many(self.old, arm, old_specs, suite="release-0.2.0")
        many(self.new, arm, new_specs, suite="release-0.3.0")

    def compare(self):
        old, _ = report.load_results(self.old)
        new, _ = report.load_results(self.new)
        return report.compare(report.analyse(new, resamples=200), report.analyse(old, resamples=200))

    def flags(self):
        return {(r["arm"], r["metric"]): r["flag"] for r in self.compare()["rows"]}

    def test_a_clear_regression_is_flagged_and_confirmed(self):
        self.both([(2, 0.5 + i / 100, 10, 60, "ok") for i in range(10)],
                  [(0, 0.5 + i / 100, 10, 60, "ok") for i in range(10)])
        f = self.flags()
        self.assertEqual(f[("K2", "units_completed")], "REGRESSION")
        self.assertEqual(f[("K2", "completion")], "REGRESSION")
        self.assertEqual(f[("K2", "turns")], "")

    def test_a_cost_increase_beyond_the_old_spread_is_a_regression(self):
        self.both([(2, 0.50 + i / 1000, 10, 60, "ok") for i in range(10)],
                  [(2, 1.50 + i / 1000, 10, 60, "ok") for i in range(10)])
        self.assertEqual(self.flags()[("K2", "cost_usd")], "REGRESSION")

    def test_small_n_shift_is_flagged_but_not_confirmed(self):
        self.both([(2, 0.5, 10, 60, "ok")] * 3, [(2, 0.9, 10, 60, "ok")] * 3)
        self.assertEqual(self.flags()[("K2", "cost_usd")], "possible regression (not confirmed)")

    def test_no_change_and_improvement_are_not_flagged(self):
        self.both([(2, 0.5 + i / 100, 10 + i, 60, "ok") for i in range(10)],
                  [(2, 0.5 + i / 100, 10 + i, 60, "ok") for i in range(10)])
        self.assertTrue(all(v == "" for v in self.flags().values()))
        shutil.rmtree(self.new)
        many(self.new, "K2", [(2, 0.1 + i / 100, 5 + i, 30, "ok") for i in range(10)], suite="release-0.3.0")
        self.assertTrue(all(v == "" for v in self.flags().values()))

    def test_difference_within_the_old_interquartile_range_is_not_flagged(self):
        self.both([(2, 0.1 * (i + 1), 10, 60, "ok") for i in range(10)],  # IQR about 0.45
                  [(2, 0.1 * (i + 1) + 0.2, 10, 60, "ok") for i in range(10)])
        self.assertEqual(self.flags()[("K2", "cost_usd")], "")

    def test_only_cells_in_both_directories_are_compared(self):
        self.both([(2, 0.5, 10, 60, "ok")] * 6, [(2, 0.5, 10, 60, "ok")] * 6)
        fill(self.new, "B1", 6, suite="release-0.3.0")
        arms = {r["arm"] for r in self.compare()["rows"]}
        self.assertEqual(arms, {"K2"})

    def test_markdown_lists_regressions_first_and_the_compare_cli_works(self):
        self.both([(2, 0.5, 10, 60, "ok")] * 10, [(0, 0.5, 10, 60, "ok")] * 10)
        out = os.path.join(self.tmp, "cmp.md")
        with contextlib.redirect_stdout(io.StringIO()):
            code = report.main([self.new, "--baseline", self.old, "--out", out, "--resamples", "200"])
        self.assertEqual(code, 0)
        with open(out) as f:
            md = f.read()
        self.assertIn("Comparison with", md)
        self.assertIn("REGRESSION", md)
        self.assertLess(md.index("REGRESSION"), md.index("## Scenario"))
        import compare
        with contextlib.redirect_stdout(io.StringIO()) as buf:
            self.assertEqual(compare.main([self.new, self.old, "--resamples", "200"]), 0)
        self.assertIn("REGRESSION", buf.getvalue())
        self.assertNotIn("## Scenario", buf.getvalue())


class LoadingAndCli(Base):
    def test_loader_skips_suite_json_and_reads_nested_suites(self):
        fill(self.new, "B0", 2, suite="m0")
        fill(self.new, "B1", 2, suite="m0")
        with open(os.path.join(self.new, "m0", "suite.json"), "w") as f:
            json.dump({"suite_id": "m0"}, f)
        recs, problems = report.load_results(self.new)
        self.assertEqual(len(recs), 4)
        self.assertEqual(problems, [])

    def test_a_malformed_record_is_reported_not_silently_dropped(self):
        fill(self.new, "B0", 1)
        bad = os.path.join(self.new, "s", "trivial", "B0", "broken.json")
        with open(bad, "w") as f:
            f.write("{not json")
        recs, problems = report.load_results(self.new)
        self.assertEqual(len(recs), 1)
        self.assertEqual(len(problems), 1)
        self.assertIn("broken.json", problems[0])

    def test_an_empty_directory_gives_a_report_that_says_so(self):
        os.makedirs(self.new)
        md = report.render_markdown(self.analyse())
        self.assertIn("No runs", md)

    def test_json_output_and_default_report_path(self):
        fill(self.new, "B0", 3)
        fill(self.new, "K2", 3)
        js = os.path.join(self.tmp, "r.json")
        with contextlib.redirect_stdout(io.StringIO()):
            code = report.main([self.new, "--json", js, "--resamples", "100"])
        self.assertEqual(code, 0)
        with open(js) as f:
            data = json.load(f)
        self.assertEqual(data["records"], 6)
        self.assertEqual(data["bootstrap_seed"], report.DEFAULT_SEED)
        self.assertTrue(os.path.exists(os.path.join(self.new, "REPORT.md")))

    def test_conditions_table_lists_models_versions_and_commits(self):
        fill(self.new, "B0", 2)
        md = report.render_markdown(self.analyse())
        self.assertIn("klatch commit", md)
        self.assertIn("arm text version", md)
        self.assertIn("draft-1", md)

    def test_ledger_check_finds_a_missing_line_and_a_missing_record(self):
        fill(self.new, "B0", 3)
        ledger = os.path.join(self.new, "LEDGER.md")
        with open(ledger, "w") as f:
            f.write("# header\n")
            for i in (1, 2):
                f.write("s-trivial-b0-r%02d | trivial | B0 | ok | true | 0.1000\n" % i)
        ok, info = report.check_ledger(self.new)
        self.assertFalse(ok)
        self.assertEqual(info["records"], 3)
        self.assertEqual(info["ledger_lines"], 2)
        self.assertEqual(info["missing_from_ledger"], ["s-trivial-b0-r03"])
        with open(ledger, "a") as f:
            f.write("s-trivial-b0-r03 | trivial | B0 | ok | true | 0.1000\n")
        self.assertTrue(report.check_ledger(self.new)[0])
        with open(ledger, "a") as f:
            f.write("s-trivial-b0-r09 | trivial | B0 | ok | true | 0.1000\n")
        ok, info = report.check_ledger(self.new)
        self.assertFalse(ok)
        self.assertEqual(info["missing_records"], ["s-trivial-b0-r09"])

    def test_check_ledger_cli_exit_code(self):
        fill(self.new, "B0", 1)
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(report.main([self.new, "--check-ledger"]), 1)


class FromARealFakeSuite(Base):
    def test_the_report_reads_what_the_harness_wrote(self):
        import run
        res, logs, scratch = (os.path.join(self.tmp, d) for d in ("res", "logs", "scratch"))
        os.mkdir(scratch)
        argv = ["--fixture", "trivial", "--arm", "B0,B1", "--runs", "2", "--driver", "fake", "--suite-id", "e2e",
                "--results-dir", res, "--logs-dir", logs, "--tmp-root", scratch, "--poll-s", "0.2"]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(run.main(argv), 0)
            self.assertEqual(report.main([res, "--resamples", "100"]), 0)
        self.assertTrue(report.check_ledger(res)[0])
        with open(os.path.join(res, "REPORT.md")) as f:
            md = f.read()
        self.assertIn("trivial", md)
        self.assertIn("EXPLORATORY", md)  # N of 2 per arm


if __name__ == "__main__":
    unittest.main()
