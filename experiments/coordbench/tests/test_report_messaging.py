"""A B1 result that cannot be read as a clean B1 result unless its messaging was verified (#1520 review, item 2b): the report
labels any B1 or B1-roles group that holds a run whose messaging_verified is not `verified` (unknown, or missing from an older
record) 'messaging unverified', in the arm tables, the tests, the comparison and a note at the top. Verified groups and every
other arm are unchanged."""
import os
import shutil
import tempfile
import unittest

from helpers import make_record
import report
from test_report import write

LABEL = "messaging unverified"


def rec(arm, run, status="verified", scenario="trivial", suite="s", drop=False):
    r = make_record(scenario=scenario, arm=arm, run=run, suite=suite)
    if drop:
        r.pop("messaging_verified")
    elif r["messaging_verified"] is not None:
        r["messaging_verified"]["status"] = status
    return r


class Labels(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="coordbench-rm-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def md(self, records, **kw):
        for r in records:
            write(self.tmp, r, r["milestone"])
        recs, _ = report.load_results(self.tmp)
        return report.render_markdown(report.analyse(recs, resamples=200), **kw)

    def team(self, b1_status=("verified",) * 3, drop=False):
        out = [rec("B1", "r%02d" % i, st, drop=drop) for i, st in enumerate(b1_status, 1)]
        out += [rec("K2", "r%02d" % i) for i in (1, 2, 3)] + [rec("B0", "r%02d" % i) for i in (1, 2, 3)]
        return out

    def test_a_b1_group_with_an_unknown_run_is_labelled_in_every_table_and_the_header(self):
        md = self.md(self.team(("verified", "unknown", "verified")))
        self.assertIn("B1 (%s)" % LABEL, md)
        self.assertIn("vs B1 (%s)" % LABEL, md)  # the tests table
        top = md.split("## Scenario")[0]
        self.assertIn(LABEL, top)
        self.assertIn("only on verified runs", top)

    def test_a_verified_group_has_no_label_and_no_note(self):
        md = self.md(self.team())
        self.assertNotIn(LABEL, md)

    def test_other_arms_are_never_labelled(self):
        md = self.md(self.team(("unknown",) * 3))
        for arm in ("K2", "B0"):
            self.assertNotIn("%s (%s)" % (arm, LABEL), md)

    def test_a_record_without_the_field_is_unverified_too(self):
        self.assertIn("B1 (%s)" % LABEL, self.md(self.team(drop=True)))

    def test_the_comparison_text_labels_the_arm(self):
        new = self.team(("unknown",) * 3)
        for r in new:
            write(self.tmp, r, "new")
        old = [rec("B1", "r%02d" % i, suite="old") for i in (1, 2, 3)]
        for r in old:
            write(self.tmp, r, "old")
        a = report.analyse(report.load_results(os.path.join(self.tmp, "new"))[0], resamples=200)
        b = report.analyse(report.load_results(os.path.join(self.tmp, "old"))[0], resamples=200)
        cmp_ = report.compare(a, b)
        md = report.render_markdown(a, cmp_, "old")
        self.assertIn("B1 (%s)" % LABEL, md.split("## Comparison")[1].split("## Scenario")[0])


if __name__ == "__main__":
    unittest.main()
