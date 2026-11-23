"""Tests for arms/check_parity.py: the real arm files pass, and each kind of unfairness is caught on a copy."""
import contextlib
import io
import os
import shutil
import sys
import tempfile
import unittest

from helpers import BENCH
ARMS = os.path.join(BENCH, "arms")
if ARMS not in sys.path:
    sys.path.insert(0, ARMS)
import check_parity as cp

ALL = list(cp.BASE_ARMS) + list(cp.ROLE_ARMS) + ["common"]


class Parity(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="parity-test-")
        for n in ALL:
            shutil.copy(os.path.join(ARMS, n + ".md"), self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir)

    def edit(self, name, fn):
        p = os.path.join(self.dir, name + ".md")
        with open(p) as f:
            s = f.read()
        with open(p, "w") as f:
            f.write(fn(s))

    def problems(self, tol=0.15):
        return cp.run_all(self.dir, tol)[0]

    def test_the_real_arms_pass(self):
        self.assertEqual(cp.run_all(ARMS, 0.15)[0], [])

    def test_cli_exit_codes(self):
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cp.main(["--arms-dir", ARMS]), 0)
            self.edit("B0", lambda s: s + " filler" * 400)
            self.assertEqual(cp.main(["--arms-dir", self.dir]), 1)

    def test_comments_do_not_count_as_length(self):
        before = cp.words(open(os.path.join(ARMS, "B0.md")).read())
        self.edit("B0", lambda s: s + "\n<!-- " + "long " * 500 + "-->\n")
        self.assertEqual(cp.words(open(os.path.join(self.dir, "B0.md")).read()), before)
        self.assertEqual(self.problems(), [])

    def test_a_short_arm_is_caught(self):
        self.edit("K1", lambda s: s[: len(s) // 2])
        self.assertTrue(any("K1" in p and "mean" in p for p in self.problems()))

    def test_a_padded_arm_is_caught(self):
        self.edit("B1", lambda s: s + " more" * 200)
        self.assertTrue(any(p.startswith("B1 is") for p in self.problems()))

    def test_the_tolerance_is_a_parameter(self):
        self.assertEqual(self.problems(0.15), [])
        self.assertTrue(self.problems(0.02))

    def test_a_missing_topic_is_caught(self):
        self.edit("B0", lambda s: s.replace("Handing work over:", "Passing it on:"))
        self.assertTrue(any("Handing work over" in p for p in self.problems()))

    def test_topics_out_of_order_are_caught(self):
        self.edit("K2", lambda s: s.replace("Waiting:", "Finishing:", 1).replace("Finishing: release", "Waiting: release"))
        self.assertTrue(any("different order" in p for p in self.problems()))

    def test_klatch_words_in_a_b_arm_are_caught(self):
        for word in ("klatch", "the room", "KLATCH_NAME"):
            self.edit("B1", lambda s: s + " " + word)
            self.assertTrue(any("B arm B1" in p for p in self.problems()), word)
            shutil.copy(os.path.join(ARMS, "B1.md"), self.dir)

    def test_klatch_placeholder_in_a_b_arm_is_caught(self):
        self.edit("B0", lambda s: s + " {{KLATCH}}")
        self.assertTrue(any("{{KLATCH}}" in p for p in self.problems()))

    def test_an_arm_naming_another_arms_tool_is_caught(self):
        self.edit("B0", lambda s: s + " Use SendMessage.")
        self.assertTrue(any("another arm" in p for p in self.problems()))
        shutil.copy(os.path.join(ARMS, "B0.md"), self.dir)
        self.edit("K1", lambda s: s + " Read TASKS.md.")
        self.assertTrue(any("another arm" in p for p in self.problems()))

    def test_common_with_two_slots_or_a_klatch_word_is_caught(self):
        self.edit("common", lambda s: s + "\n{{COORDINATION_SECTION}}\n")
        self.assertTrue(any("exactly once" in p for p in self.problems()))
        shutil.copy(os.path.join(ARMS, "common.md"), self.dir)
        self.edit("common", lambda s: s + "\nUse klatch.\n")
        self.assertTrue(any("names klatch" in p for p in self.problems()))

    def test_unknown_placeholder_is_caught(self):
        self.edit("K1", lambda s: s + " {{SECRET}}")
        self.assertTrue(any("SECRET" in p for p in self.problems()))

    def test_role_sections_are_compared_pairwise(self):
        self.edit("B1-roles", lambda s: s.replace("Never edit the code\nyourself.", "Never edit the code yourself. " + "pad " * 80, 1))
        self.assertTrue(any("B1-roles" in p and "section" in p for p in self.problems()))

    def test_a_missing_role_section_is_caught(self):
        self.edit("K3", lambda s: s.replace("## Supervisor prompt (draft)", "## Overseer"))
        self.assertTrue(any("supervisor" in p for p in self.problems()))

    def test_b1_arms_use_the_shared_prefixes_and_b0_has_none(self):
        for arm in ("B1", "B1-roles"):
            text = cp.strip_comments(open(os.path.join(ARMS, arm + ".md")).read())
            for prefix in cp.SHARED_PREFIXES:
                self.assertIn(prefix, text, arm)
        self.assertEqual(cp.check_prefixes(ARMS), [])
        self.assertIn("seeking:", cp.SHARED_PREFIXES)
        # the K arms say the same words
        for arm in ("K1", "K2"):
            text = cp.strip_comments(open(os.path.join(ARMS, arm + ".md")).read())
            for prefix in cp.SHARED_PREFIXES:
                self.assertIn(prefix.replace("answer:", "answer"), text, arm)

    def test_a_b1_arm_missing_a_prefix_is_caught(self):
        self.edit("B1", lambda s: s.replace("seeking:", "asking:"))
        self.assertTrue(any("B1" in p and "seeking:" in p for p in self.problems()))
        shutil.copy(os.path.join(ARMS, "B1.md"), self.dir)
        self.edit("B1-roles", lambda s: s.replace("ready:", "ready"))
        self.assertTrue(any("B1-roles" in p and "ready:" in p for p in self.problems()))

    def test_b0_with_a_prefix_is_caught(self):
        self.edit("B0", lambda s: s + " Start a message with `handoff:`.")
        self.assertTrue(any("B0" in p and "prefix" in p for p in self.problems()))

    def test_plain_prefix_words_are_not_klatch_words(self):
        self.edit("B1", lambda s: s + " question: answer: handoff: decision: review: overlap: interface:")
        self.assertFalse(any("klatch word" in p for p in self.problems()))

    def test_split_role_sections_keys(self):
        secs = cp.split_role_sections("## Worker section\nA b\n## Integrator prompt (draft)\nC\n")
        self.assertEqual(secs, {"worker": "A b", "integrator": "C"})


if __name__ == "__main__":
    unittest.main()
