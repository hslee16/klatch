import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import run_tests  # noqa: E402


class TestRunTests(unittest.TestCase):
    def test_every_slow_name_is_a_real_module(self):
        self.assertEqual(sorted(run_tests.SLOW - set(run_tests.modules())), [])

    def test_the_fast_subset_leaves_out_exactly_the_slow_modules(self):
        self.assertEqual(set(run_tests.modules()) - set(run_tests.selected(False)), set(run_tests.SLOW))
        self.assertEqual(run_tests.selected(True), run_tests.modules())

    def test_the_fast_subset_is_most_of_the_modules(self):
        self.assertGreater(len(run_tests.selected(False)), len(run_tests.SLOW))
