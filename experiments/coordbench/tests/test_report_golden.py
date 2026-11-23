"""Characterisation of report.py (#1455): the analysis (as JSON) and the Markdown report over a fixed set of records,
compared with files made by the code as it was before the code-health refactor. Green on the old code and on the new;
a pure refactor has no red test. Regenerate the files only for a deliberate change of the report: python3 test_report_golden.py"""
import json
import os
import re
import sys
import unittest

from helpers import make_record
import report

DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
GOLDEN_MD = os.path.join(DATA, "report-golden.md")
GOLDEN_JSON = os.path.join(DATA, "report-golden.json")


SCENARIOS = (("trivial", ("B0", "B1", "K1", "K2")), ("onefile", ("B0", "K2")))


def arm_records(scenario, ai, arm, shift, suite):
    n = 7 if scenario == "trivial" else (3 if arm == "K2" else 6)
    return [make_record(
        scenario=scenario, arm=arm, run="r%02d" % (i + 1), suite=suite, units=(2, 2 if (i + ai) % 3 else 1),
        cost=round(0.3 + ai * 0.05 + i * 0.02 + shift, 4), turns=10 + ai * 3 + i, wall=60.0 + ai * 7 + i * 5 + shift * 10,
        outcome="timeout" if (i == 5 and ai == 1) else "ok", valid=not (i == 6 and ai == 2),
        messages=ai * 2 + i, tokens=1000 + 100 * i) for i in range(n)]


def records(shift=0.0, suite="s"):
    """Two scenarios, four arms, varied values; one invalid run, one timeout, one incomplete run, small and large arms."""
    return [r for sc, arms in SCENARIOS for ai, arm in enumerate(arms) for r in arm_records(sc, ai, arm, shift, suite)]


def render():
    new = report.analyse(records(), resamples=400)
    old = report.analyse(records(shift=0.05, suite="o"), resamples=400)
    new["comparison"] = report.compare(new, old, resamples=400)
    md = report.render_markdown(new, new["comparison"], "OLD", "NEW")
    md = re.sub(r"^Generated .*? from NEW", "Generated TIME from NEW", md, flags=re.M)
    only = report.render_markdown(new, new["comparison"], "OLD", "NEW", comparison_only=True)
    only = re.sub(r"^Generated .*? from NEW", "Generated TIME from NEW", only, flags=re.M)
    return md + "\n=====COMPARISON-ONLY=====\n" + only, json.dumps(new, indent=1, sort_keys=True) + "\n"


class TestReportGolden(unittest.TestCase):
    def test_markdown_and_analysis_are_unchanged(self):
        md, js = render()
        with open(GOLDEN_MD) as f:
            self.assertEqual(md, f.read())
        with open(GOLDEN_JSON) as f:
            self.assertEqual(js, f.read())


if __name__ == "__main__":
    md, js = render()
    with open(GOLDEN_MD, "w") as f:
        f.write(md)
    with open(GOLDEN_JSON, "w") as f:
        f.write(js)
    print("wrote", GOLDEN_MD, GOLDEN_JSON)
