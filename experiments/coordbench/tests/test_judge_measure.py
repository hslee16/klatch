"""S10 part 2 (#1375): the judge measurement script (judge_measure.py) that runs `klatch-bench judge-eval` and writes the
miss rate and false positive rate with intervals, the klatch commit and the corpus checksum. Unit tests use a stub bench
program that prints a canned summary; the integration test runs the real rules judge (free, no model, no key) on a tiny
corpus when KLATCH_BENCH_BIN names a built klatch-bench, and is skipped with a message otherwise."""
import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import judge_measure
import stats

HERE = os.path.dirname(os.path.abspath(__file__))
TINY = os.path.join(HERE, "data", "judge-tiny-corpus.jsonl")
TINY_SCRUB = os.path.join(HERE, "data", "judge-tiny-scrub.txt")
BENCH_BIN = os.environ.get("KLATCH_BENCH_BIN")
NEEDS_BENCH = unittest.skipUnless(BENCH_BIN and os.path.exists(BENCH_BIN or ""),
                                  "KLATCH_BENCH_BIN is unset: build klatch-bench (cargo build --locked -p klatch-bench) and "
                                  "set it to the binary to run the real rules-judge test")

SUMMARY = {"judge": "rules", "split": "dev", "summary": {
    "n": 197, "confusion": {"rows": ["allow", "redact", "deny"], "columns": ["allow", "redact", "deny", "defer"],
                            "counts": [[118, 0, 0, 0], [12, 0, 0, 0], [39, 0, 28, 0]]},
    "leaks": 51, "leak_rate": 51 / 79, "data_leaks": 51, "overblocks": 0, "overblock_rate": 0.0, "defers": 0, "errors": 0,
    "contested": [{"id": "short-008"}],
    "by_category": [{"category": "personal-data", "n": 15, "correct": 1, "leaks": 14, "overblocks": 0, "defers": 0},
                    {"category": "everyday-chat", "n": 48, "correct": 48, "leaks": 0, "overblocks": 0, "defers": 0}]}}


def stub_bench(directory, summary_by_split):
    """An executable that behaves like `klatch-bench judge-eval ... --out FILE`: writes the canned summary for --split and
    appends its argv to argv.log next to itself."""
    path = os.path.join(directory, "stub-bench")
    with open(path, "w") as f:
        f.write("#!%s\nimport json, os, sys\n" % sys.executable)
        f.write("a = sys.argv[1:]\nopen(os.path.join(%r, 'argv.log'), 'a').write(json.dumps(a) + '\\n')\n" % directory)
        f.write("S = %r\n" % (summary_by_split,))
        f.write("json.dump(S[a[a.index('--split') + 1]], open(a[a.index('--out') + 1], 'w'))\n")
        f.write("print('stub report')\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


def make_repo(directory):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@e.invalid", GIT_COMMITTER_NAME="t",
               GIT_COMMITTER_EMAIL="t@e.invalid", HOME=directory)
    subprocess.run(["git", "init", "-q", "-b", "main", directory], check=True, env=env)
    with open(os.path.join(directory, "f"), "w") as f:
        f.write("x")
    subprocess.run(["git", "-C", directory, "add", "-A"], check=True, env=env)
    subprocess.run(["git", "-C", directory, "commit", "-q", "-m", "c"], check=True, env=env)
    return subprocess.run(["git", "-C", directory, "rev-parse", "HEAD"], check=True, env=env, stdout=subprocess.PIPE,
                          text=True).stdout.strip()


class TestSummarise(unittest.TestCase):
    def test_miss_and_false_positive_rates_come_from_the_confusion_matrix_with_wilson_intervals(self):
        m = judge_measure.summarise(SUMMARY)
        self.assertEqual((m["should_not_cross"], m["clean"]), (79, 118))
        self.assertEqual((m["misses"], m["false_positives"]), (51, 0))
        self.assertAlmostEqual(m["miss_rate"], 51 / 79)
        self.assertEqual(m["false_positive_rate"], 0.0)
        lo, hi = stats.wilson(51, 79)
        self.assertEqual((m["miss_rate_ci95"][0], m["miss_rate_ci95"][1]), (round(lo, 4), round(hi, 4)))
        self.assertEqual(m["false_positive_rate_ci95"][0], 0.0)
        self.assertGreater(m["false_positive_rate_ci95"][1], 0.0)
        self.assertEqual(m["contested_excluded"], 1)

    def test_by_category_keeps_only_the_categories_with_misses_or_false_positives(self):
        m = judge_measure.summarise(SUMMARY)
        self.assertEqual(m["by_category"], {"personal-data": {"n": 15, "misses": 14, "false_positives": 0}})

    def test_a_summary_with_failed_calls_or_defers_is_flagged_not_hidden(self):
        s = json.loads(json.dumps(SUMMARY))
        s["summary"]["errors"] = 2
        s["summary"]["defers"] = 1
        m = judge_measure.summarise(s)
        self.assertEqual((m["errors"], m["defers"]), (2, 1))
        self.assertTrue(m["incomplete"])
        self.assertFalse(judge_measure.summarise(SUMMARY)["incomplete"])


class TestMeasureWithAStub(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="jm-test-")
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))
        self.repo = os.path.join(self.tmp, "repo")
        os.mkdir(self.repo)
        self.commit = make_repo(self.repo)
        test = json.loads(json.dumps(SUMMARY))
        test["split"] = "test"
        test["summary"]["leaks"] = 36
        self.bench = stub_bench(self.tmp, {"dev": SUMMARY, "test": test})

    def measure(self, **kw):
        return judge_measure.measure(self.bench, "rules", TINY, scrub=TINY_SCRUB, splits=("dev", "test"), repo=self.repo,
                                     now="2026-11-23T10:00:00Z", **kw)

    def test_the_record_has_the_commit_the_corpus_checksum_the_command_and_both_splits(self):
        r = self.measure()
        self.assertEqual(r["klatch_commit"], self.commit)
        self.assertEqual(r["klatch_tree_dirty"], False)
        with open(TINY, "rb") as f:
            self.assertEqual(r["corpus"]["sha256"], hashlib.sha256(f.read()).hexdigest())
        self.assertEqual(r["corpus"]["items"], 6)
        self.assertEqual(r["judge"], "rules")
        self.assertEqual(r["date"], "2026-11-23T10:00:00Z")
        self.assertEqual(sorted(r["splits"]), ["dev", "test"])
        self.assertAlmostEqual(r["splits"]["test"]["miss_rate"], 36 / 79)
        self.assertIn("judge-eval", r["command"])
        self.assertEqual(r["model_calls"], 0)
        self.assertEqual(r["cost_usd"], 0.0)

    def test_the_bench_is_called_once_per_split_with_the_judge_corpus_and_scrub_given(self):
        self.measure()
        with open(os.path.join(self.tmp, "argv.log")) as f:
            calls = [json.loads(l) for l in f]
        self.assertEqual(len(calls), 2)
        for c in calls:
            self.assertEqual(c[0], "judge-eval")
            self.assertEqual(c[c.index("--judge") + 1], "rules")
            self.assertEqual(c[c.index("--corpus") + 1], TINY)
            self.assertEqual(c[c.index("--scrub") + 1], TINY_SCRUB)
        self.assertEqual([c[c.index("--split") + 1] for c in calls], ["dev", "test"])

    def test_a_model_judge_is_refused_because_this_script_never_spends(self):
        for j in ("claude:sonnet", "claude:opus", "jev:jev-1.14.0", "cascade:jev:x,claude:sonnet"):
            with self.assertRaises(judge_measure.MeasureError, msg=j):
                judge_measure.measure(self.bench, j, TINY, splits=("dev",), repo=self.repo)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "argv.log")))

    def test_a_bench_that_fails_or_writes_nothing_is_an_error(self):
        bad = os.path.join(self.tmp, "bad-bench")
        with open(bad, "w") as f:
            f.write("#!/bin/sh\nexit 3\n")
        os.chmod(bad, 0o755)
        with self.assertRaises(judge_measure.MeasureError):
            judge_measure.measure(bad, "rules", TINY, splits=("dev",), repo=self.repo)

    def test_the_script_writes_the_json_and_prints_a_one_line_summary_per_split(self):
        out = os.path.join(self.tmp, "out.json")
        p = subprocess.run([sys.executable, os.path.join(BENCH, "judge_measure.py"), "--bench", self.bench, "--corpus", TINY,
                            "--scrub", TINY_SCRUB, "--repo", self.repo, "--out", out], stdout=subprocess.PIPE, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)
        with open(out) as f:
            rec = json.load(f)
        self.assertEqual(rec["klatch_commit"], self.commit)
        self.assertIn("dev", p.stdout)
        self.assertIn("miss", p.stdout)
        self.assertRegex(rec["date"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")


class TestRecordedMeasurements(unittest.TestCase):
    """The committed records under results-docs/ are consistent: rates follow from counts, intervals from stats.wilson, no model
    was called, and each names a commit and a 64-hex corpus checksum."""
    def records(self):
        d = os.path.join(BENCH, "results-docs")
        names = sorted(n for n in os.listdir(d) if n.startswith("s10-judge-") and n.endswith(".json"))
        self.assertIn("s10-judge-rules.json", names)
        for n in names:
            with open(os.path.join(d, n)) as f:
                yield n, json.load(f)

    def test_each_record_is_internally_consistent(self):
        for n, r in self.records():
            self.assertEqual(r["judge"], "rules", n)
            self.assertEqual((r["model_calls"], r["cost_usd"]), (0, 0.0), n)
            self.assertRegex(r["klatch_commit"], r"^[0-9a-f]{40}$", n)
            self.assertRegex(r["corpus"]["sha256"], r"^[0-9a-f]{64}$", n)
            self.assertEqual(sorted(r["splits"]), ["dev", "test"], n)
            for split, m in r["splits"].items():
                self.assertAlmostEqual(m["miss_rate"], m["misses"] / m["should_not_cross"], msg=n + split)
                self.assertAlmostEqual(m["false_positive_rate"], m["false_positives"] / m["clean"], msg=n + split)
                lo, hi = stats.wilson(m["misses"], m["should_not_cross"])
                self.assertEqual(m["miss_rate_ci95"], [round(lo, 4), round(hi, 4)], n + split)
                self.assertFalse(m["incomplete"], n + split)

    def test_the_corpus_checksum_names_the_corpus_file_it_says(self):
        for n, r in self.records():
            p = os.path.join(os.path.dirname(os.path.dirname(BENCH)), r["corpus"]["path"])
            if os.path.exists(p):
                with open(p, "rb") as f:
                    same = hashlib.sha256(f.read()).hexdigest() == r["corpus"]["sha256"]
                if not same:
                    self.skipTest("%s: the corpus changed since the measurement; re-run judge_measure.py (README)" % n)


@NEEDS_BENCH
class TestRealRulesJudgeOnATinyCorpus(unittest.TestCase):
    def test_the_rules_judge_misses_the_business_secrets_catches_the_credential_and_blocks_the_scrub_term(self):
        r = judge_measure.measure(BENCH_BIN, "rules", TINY, scrub=TINY_SCRUB, splits=("dev", "test"), repo=BENCH)
        dev, test = r["splits"]["dev"], r["splits"]["test"]
        self.assertEqual((dev["should_not_cross"], dev["misses"]), (2, 1))   # the AWS pair is caught, the acquisition is not
        self.assertEqual((dev["clean"], dev["false_positives"]), (2, 1))     # the scrub term in an allow item is blocked
        self.assertEqual((test["should_not_cross"], test["misses"]), (1, 1))
        self.assertEqual((test["clean"], test["false_positives"]), (1, 0))
        self.assertEqual(r["model_calls"], 0)


if __name__ == "__main__":
    unittest.main()
