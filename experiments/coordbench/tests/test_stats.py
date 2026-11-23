"""Statistical functions against values known from the literature or computed by hand from closed forms (#1364)."""
import math
import unittest

from helpers import BENCH  # noqa: F401  (puts the bench directory on sys.path)
import stats


class Quantiles(unittest.TestCase):
    def test_type7_quartiles(self):
        self.assertAlmostEqual(stats.quantile([1, 2, 3, 4], 0.25), 1.75)
        self.assertAlmostEqual(stats.quantile([1, 2, 3, 4], 0.75), 3.25)
        self.assertEqual(stats.quantile([5], 0.5), 5)

    def test_summary(self):
        s = stats.summary([4, 1, 3, 2])
        self.assertEqual((s["n"], s["min"], s["max"]), (4, 1, 4))
        self.assertAlmostEqual(s["median"], 2.5)
        self.assertAlmostEqual(s["mean"], 2.5)
        self.assertAlmostEqual(s["sd"], math.sqrt(5 / 3))

    def test_summary_of_nothing_is_none(self):
        self.assertIsNone(stats.summary([]))


class Wilson(unittest.TestCase):
    def test_five_of_ten_literature(self):
        lo, hi = stats.wilson(5, 10)
        self.assertAlmostEqual(lo, 0.2366, places=3)
        self.assertAlmostEqual(hi, 0.7634, places=3)

    def test_zero_of_ten_closed_form(self):
        z2 = 1.959964 ** 2
        lo, hi = stats.wilson(0, 10)
        self.assertAlmostEqual(lo, 0.0, places=9)
        self.assertAlmostEqual(hi, z2 / (10 + z2), places=5)

    def test_n_zero_has_no_interval(self):
        self.assertEqual(stats.wilson(0, 0), (None, None))


class Newcombe(unittest.TestCase):
    def test_newcombe_1998_example(self):
        # Newcombe 1998 (Statistics in Medicine 17:873), method 10: 56/70 against 48/80, difference 0.2000,
        # 95 percent interval 0.0524 to 0.3339
        d, lo, hi = stats.newcombe_diff(56, 70, 48, 80)
        self.assertAlmostEqual(d, 0.2, places=4)
        self.assertAlmostEqual(lo, 0.0524, places=3)
        self.assertAlmostEqual(hi, 0.3339, places=3)

    def test_risk_ratio(self):
        self.assertAlmostEqual(stats.risk_ratio(8, 10, 4, 10), 2.0)
        self.assertIsNone(stats.risk_ratio(1, 10, 0, 10))


class Fisher(unittest.TestCase):
    def test_lady_tasting_tea(self):
        # Fisher 1935: 3 of 4 right each way. one-sided 17/70, two-sided 34/70
        self.assertAlmostEqual(stats.fisher_exact(3, 1, 1, 3, "greater"), 17 / 70, places=12)
        self.assertAlmostEqual(stats.fisher_exact(3, 1, 1, 3), 34 / 70, places=12)

    def test_wikipedia_dieting_table_from_closed_form(self):
        # 1 and 9 against 11 and 3: the one-sided tail is C(10,1)C(14,11)/C(24,12) plus the more extreme table (0,10)
        tail = (math.comb(10, 1) * math.comb(14, 11) + math.comb(10, 0) * math.comb(14, 12)) / math.comb(24, 12)
        self.assertAlmostEqual(stats.fisher_exact(1, 9, 11, 3, "less"), tail, places=12)
        self.assertAlmostEqual(math.comb(10, 1) * math.comb(14, 11) / math.comb(24, 12), 0.001346, places=6)  # the table itself
        self.assertAlmostEqual(tail, 0.0013797, places=6)
        two = stats.fisher_exact(1, 9, 11, 3)
        self.assertAlmostEqual(two, 0.002759, places=5)  # the value R's fisher.test gives

    def test_extreme_table_is_two_over_choose(self):
        # 5 of 5 against 0 of 5: p = 2 / C(10,5)
        self.assertAlmostEqual(stats.fisher_exact(5, 0, 0, 5), 2 / math.comb(10, 5), places=12)

    def test_identical_rows_give_one(self):
        self.assertAlmostEqual(stats.fisher_exact(5, 5, 5, 5), 1.0, places=12)

    def test_empty_margin_gives_one(self):
        self.assertEqual(stats.fisher_exact(0, 0, 3, 4), 1.0)

    def test_symmetric_under_swapping_groups(self):
        self.assertAlmostEqual(stats.fisher_exact(7, 3, 2, 8), stats.fisher_exact(2, 8, 7, 3), places=12)

    def test_rejects_negative_counts(self):
        with self.assertRaises(ValueError):
            stats.fisher_exact(-1, 2, 3, 4)


class MannWhitney(unittest.TestCase):
    def test_complete_separation_exact(self):
        r = stats.mann_whitney([1, 2, 3], [4, 5, 6])
        self.assertEqual(r["method"], "exact")
        self.assertEqual(r["u"], 0)
        self.assertAlmostEqual(r["p"], 2 / math.comb(6, 3), places=12)  # 0.1

    def test_interleaved_exact_distribution(self):
        # n = m = 3: the counts of U from 0 to 9 are 1 1 2 3 3 3 3 2 1 1 (out of 20); U = 3 gives P(U<=3) = 7/20
        r = stats.mann_whitney([1, 3, 5], [2, 4, 6])
        self.assertEqual(r["u"], 3)
        self.assertAlmostEqual(r["p"], 0.7, places=12)

    def test_ties_use_normal_with_tie_correction_by_hand(self):
        # x = 1 2 2 3, y = 2 3 4 4. Average ranks: 1 -> 1, three 2s -> 3, two 3s -> 5.5, two 4s -> 7.5.
        # R(x) = 1 + 3 + 3 + 5.5 = 12.5, U(x) = 12.5 - 10 = 2.5, mean 8. Tie term (27-3) + (8-2) + (8-2) = 36.
        # var = 16/12 * (9 - 36/56). z with the 0.5 continuity correction = (|2.5 - 8| - 0.5) / sd.
        var = 16 / 12 * (9 - 36 / 56)
        z = (5.5 - 0.5) / math.sqrt(var)
        want = math.erfc(z / math.sqrt(2))
        r = stats.mann_whitney([1, 2, 2, 3], [2, 3, 4, 4])
        self.assertEqual(r["method"], "normal-ties")
        self.assertAlmostEqual(r["u1"], 2.5)
        self.assertAlmostEqual(r["p"], want, places=9)

    def test_all_values_equal_gives_p_one(self):
        r = stats.mann_whitney([4, 4, 4], [4, 4, 4])
        self.assertEqual(r["p"], 1.0)

    def test_large_samples_without_ties_use_normal(self):
        x = list(range(0, 60, 2))
        y = list(range(1, 61, 2))
        self.assertEqual(stats.mann_whitney(x, y)["method"], "normal")

    def test_empty_group_has_no_test(self):
        self.assertIsNone(stats.mann_whitney([], [1, 2]))

    def test_exact_p_matches_enumeration_of_all_assignments(self):
        # brute force over every split of the ranks of 5 values into groups of 2 and 3
        from itertools import combinations
        vals = [3.1, 0.2, 5.5, 4.4, 1.0]
        x, y = [3.1, 5.5], [0.2, 4.4, 1.0]
        def u_of(group):
            return sum(1 for a in group for b in vals if b not in group and a > b)
        obs = u_of(x)
        us = [u_of(c) for c in combinations(vals, 2)]
        centre = 2 * 3 / 2
        p = sum(1 for u in us if abs(u - centre) >= abs(obs - centre)) / len(us)
        self.assertAlmostEqual(stats.mann_whitney(x, y)["p"], p, places=12)


class ShiftAndBootstrap(unittest.TestCase):
    def test_hodges_lehmann_by_hand(self):
        # differences y - x for x = 1 2 3, y = 4 5 6 are 3 2 1 4 3 2 5 4 3, sorted 1 2 2 3 3 3 4 4 5, median 3
        self.assertEqual(stats.hodges_lehmann([1, 2, 3], [4, 5, 6]), 3)

    def test_bootstrap_is_deterministic_for_a_seed(self):
        a, b = [1, 2, 3, 4, 5, 9], [4, 6, 7, 8, 9, 12]
        one = stats.bootstrap_median_diff_ci(a, b, resamples=500, seed=17)
        two = stats.bootstrap_median_diff_ci(a, b, resamples=500, seed=17)
        three = stats.bootstrap_median_diff_ci(a, b, resamples=500, seed=18)
        self.assertEqual(one, two)
        self.assertNotEqual(one, three)

    def test_bootstrap_of_constant_samples_is_a_point(self):
        self.assertEqual(stats.bootstrap_median_diff_ci([1, 1, 1], [4, 4, 4], resamples=200, seed=1), (3, 3))

    def test_bootstrap_interval_is_ordered_and_covers_the_observed_difference(self):
        a, b = [10, 11, 12, 13, 14, 15, 16], [14, 15, 16, 17, 18, 19, 20]
        lo, hi = stats.bootstrap_median_diff_ci(a, b, resamples=2000, seed=5)
        self.assertLessEqual(lo, 4)
        self.assertGreaterEqual(hi, 4)
        self.assertLessEqual(lo, hi)


class Holm(unittest.TestCase):
    def test_holm_by_hand(self):
        # sorted .005 .01 .03 .04 times 4 3 2 1 = .02 .03 .06 .04, made monotone: .02 .03 .06 .06
        got = stats.holm([0.01, 0.04, 0.03, 0.005])
        for g, w in zip(got, [0.03, 0.06, 0.06, 0.02]):
            self.assertAlmostEqual(g, w)

    def test_holm_is_capped_at_one(self):
        self.assertEqual(stats.holm([0.9, 0.8]), [1.0, 1.0])

    def test_holm_empty(self):
        self.assertEqual(stats.holm([]), [])


if __name__ == "__main__":
    unittest.main()
