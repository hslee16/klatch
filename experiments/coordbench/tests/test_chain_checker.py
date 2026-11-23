"""The hidden checker of fixture F3 `chain` (#1368) against the reference solution and against broken variants.

The reference lives in fixtures/chain/reference (never in the agent-visible checkout). Each variant is a git repository built
from the fixture's setup.sh and the commits of the three agents in order (a: schema and the unfinished bins.py; b: the
loader; c: the finished bins.py, then the report), with one thing broken.
"""
import json
import os
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401

FIXTURE = os.path.join(BENCH, "fixtures", "chain")
CHECK = os.path.join(BENCH, "checks", "chain", "check.sh")
REF = os.path.join(FIXTURE, "reference", "chainkit")
EXPECTED = os.path.join(BENCH, "checks", "chain", "expected.json")
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull}


def git(cwd, *args, author="bench"):
    env = dict(GENV, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_COMMITTER_NAME=author,
               GIT_COMMITTER_EMAIL=author + "@bench.invalid")
    p = subprocess.run(["git"] + list(args), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), p.stdout))
    return p.stdout


def read(path):
    with open(path) as f:
        return f.read()


def ref(name):
    return read(os.path.join(REF, name))


def quirk():
    return json.loads(read(EXPECTED))["quirk"]


def reference_commits():
    """The commits of a correct run: (author, {path: text or None to delete})."""
    return [
        ("a", {"chainkit/schema.json": ref("schema.json"), "chainkit/bins.py": ref("bins_partial.py")}),
        ("b", {"chainkit/loader.py": ref("loader.py")}),
        ("c", {"chainkit/bins.py": ref("bins.py")}),
        ("c", {"chainkit/report.py": ref("report.py")}),
    ]


def build(dest, commits):
    subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, stdout=subprocess.DEVNULL,
                   env=GENV)
    git(dest, "init", "-q", "-b", "main")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "initial")
    for author, files in commits:
        for path, text in files.items():
            full = os.path.join(dest, path)
            if text is None:
                os.remove(full)
                continue
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w") as f:
                f.write(text)
        git(dest, "add", "-A", author=author)
        git(dest, "commit", "-q", "-m", "%s: step" % author, author=author)


def run_check(commits):
    with tempfile.TemporaryDirectory() as d:
        dest = os.path.join(d, "checkout")
        os.mkdir(dest)
        build(dest, commits)
        p = subprocess.run(["bash", CHECK, dest], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                           env=GENV, timeout=120)
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout.strip().splitlines()[-1])


def ids(res):
    return {c["id"]: c["pass"] for c in res["checks"]}


def with_change(index, path, fn):
    """The reference commits with one file's text changed by fn in commit index."""
    cs = [(a, dict(f)) for a, f in reference_commits()]
    cs[index][1][path] = fn(cs[index][1][path])
    return cs


class TestReference(unittest.TestCase):
    def test_the_reference_scores_full_marks(self):
        r = run_check(reference_commits())
        self.assertEqual(r["units_total"], 4)
        self.assertEqual(r["units_completed"], 4)
        self.assertTrue(all(ids(r).values()), ids(r))
        self.assertTrue(r["quirk_honoured"])
        self.assertEqual(r["lost_at_handoff"], [])
        self.assertEqual(r["merge_state"], "clean")
        self.assertTrue(r["correct"])
        self.assertEqual(r["order"], {"loader_after_schema": True, "report_after_loader": True})

    def test_the_four_steps_are_the_units_and_the_rest_is_structural(self):
        r = run_check(reference_commits())
        units = [c["id"] for c in r["checks"] if c.get("unit") is not False]
        self.assertEqual(units, ["schema", "bins", "loader", "report"])
        self.assertEqual({c["id"] for c in r["checks"] if c.get("unit") is False},
                         {"quirk", "handoff-state", "integrated", "order-loader-after-schema",
                          "order-report-after-loader", "no-conflict-markers", "visible-tests"})

    def test_the_initial_tree_alone_scores_nothing(self):
        r = run_check([])
        self.assertEqual(r["units_completed"], 0)
        self.assertFalse(r["quirk_honoured"])
        self.assertFalse(r["correct"])

    def test_the_checker_is_repeatable(self):
        self.assertEqual(run_check(reference_commits()), run_check(reference_commits()))


class TestQuirk(unittest.TestCase):
    def test_a_normaliser_that_ignores_the_quirk_passes_the_plain_cases_only(self):
        q = quirk()
        final = ref("bins.py")
        self.assertIn(q["legacy"], final)
        plain = ref("bins_partial.py").replace("# TODO(a): unfinished, see the handoff", "# done")  # the plain format
        cs = reference_commits()
        cs[2] = ("c", {"chainkit/bins.py": plain})
        r = run_check(cs)
        self.assertFalse(r["quirk_honoured"])
        self.assertFalse(ids(r)["quirk"])
        self.assertTrue(ids(r)["bins"], "the plain cases are right, only the quirk records are wrong")
        self.assertFalse(ids(r)["integrated"])
        self.assertTrue(ids(r)["report"], "the report is tested against the reference normaliser")
        self.assertEqual(r["units_completed"], 4)
        self.assertEqual(r["lost_at_handoff"], [])
        self.assertFalse(r["correct"])

    def test_a_guessed_offset_is_not_the_quirk(self):
        q = quirk()
        cs = with_change(2, "chainkit/bins.py", lambda t: t.replace(str(q["offset"]), str(q["offset"] + 1)))
        r = run_check(cs)
        self.assertFalse(r["quirk_honoured"])
        self.assertFalse(r["correct"])

    def test_a_wrong_new_prefix_is_not_the_quirk(self):
        q = quirk()
        cs = with_change(2, "chainkit/bins.py", lambda t: t.replace(q["new"], "ZZ"))
        r = run_check(cs)
        self.assertFalse(r["quirk_honoured"])


class TestLostWorkAtTheHandoff(unittest.TestCase):
    def test_parse_bin_dropped_is_lost_work(self):
        final = ref("bins.py")
        start = final.index("def parse_bin")
        end = final.index("LEGACY =")
        dropped = final[:start] + "def _p(raw):\n    import re\n    m = re.match(r'([A-Z]{2})[-/]?(\\d+)$', raw)\n" \
                                  "    return Bin(m.group(1), int(m.group(2)))\n\n\n" + final[end:].replace("parse_bin(", "_p(")
        cs = reference_commits()
        cs[2] = ("c", {"chainkit/bins.py": dropped})
        r = run_check(cs)
        self.assertEqual([x["piece"] for x in r["lost_at_handoff"]], ["parse_bin"])
        self.assertEqual(r["lost_at_handoff"][0]["how"], "dropped")
        self.assertFalse(ids(r)["bins"])
        self.assertFalse(ids(r)["handoff-state"])
        self.assertEqual(r["units_completed"], 3)
        self.assertFalse(r["correct"])

    def test_parse_bin_redone_differently_is_lost_work(self):
        cs = with_change(2, "chainkit/bins.py", lambda t: t.replace("return Bin(", "return tuple(["
                                                                                   "").replace("int(m.group(2)))", "int(m.group(2))])", 1))
        r = run_check(cs)
        self.assertEqual([x["piece"] for x in r["lost_at_handoff"]], ["parse_bin"], r["lost_at_handoff"])
        self.assertEqual(r["lost_at_handoff"][0]["how"], "redone-differently")
        self.assertFalse(ids(r)["bins"])

    def test_the_schema_published_by_a_replaced_by_a_different_one_is_lost_work(self):
        obj = json.loads(ref("schema.json"))
        obj["fields"][0]["end"] += 1
        r = run_check(reference_commits() + [("c", {"chainkit/schema.json": json.dumps(obj)})])
        self.assertEqual(r["lost_at_handoff"], [{"piece": "schema", "how": "redone-differently"}])
        self.assertFalse(ids(r)["schema"])
        self.assertTrue(ids(r)["loader"], "the loader is tested with the reference schema in place")

    def test_a_schema_that_was_wrong_from_the_start_is_a_failed_step_not_lost_work(self):
        obj = json.loads(ref("schema.json"))
        obj["fields"][0]["end"] += 1
        r = run_check(with_change(0, "chainkit/schema.json", lambda t: json.dumps(obj)))
        self.assertEqual(r["lost_at_handoff"], [])
        self.assertFalse(ids(r)["schema"])

    def test_the_schema_deleted_by_a_later_commit_is_lost_work(self):
        cs = reference_commits() + [("c", {"chainkit/schema.json": None})]
        r = run_check(cs)
        self.assertEqual(r["lost_at_handoff"][0], {"piece": "schema", "how": "dropped"})
        self.assertFalse(ids(r)["integrated"])


class TestOrderAndInterfaces(unittest.TestCase):
    def test_a_report_committed_before_the_loader_is_out_of_order(self):
        cs = reference_commits()
        cs[1], cs[3] = cs[3], cs[1]  # the report is committed before the loader
        r = run_check(cs)
        self.assertFalse(r["order"]["report_after_loader"])
        self.assertFalse(ids(r)["order-report-after-loader"])
        self.assertEqual(r["units_completed"], 4, "the final tree is fine; the order is its own finding")
        self.assertFalse(r["correct"])

    def test_a_loader_committed_before_the_schema_exists_is_out_of_order(self):
        cs = reference_commits()
        cs[0], cs[1] = ("a", {"chainkit/bins.py": ref("bins_partial.py")}), ("b", {"chainkit/loader.py": ref("loader.py")})
        cs.insert(2, ("a", {"chainkit/schema.json": ref("schema.json")}))
        r = run_check(cs)
        self.assertFalse(r["order"]["loader_after_schema"])
        self.assertTrue(r["order"]["report_after_loader"])
        self.assertEqual(r["units_completed"], 4)
        self.assertFalse(r["correct"])

    def test_a_loader_with_the_columns_written_in_does_not_follow_the_schema(self):
        text = ref("loader.py")
        hard = ("import os\n\n\ndef load(data_path=None, schema_path=None):\n"
                "    path = data_path or os.path.join(os.path.dirname(__file__), '..', 'data', 'inventory.dat')\n"
                "    spans = %s\n    out = []\n    with open(path) as f:\n        for line in f:\n"
                "            line = line.rstrip('\\n')\n            if not line:\n                continue\n"
                "            row = {k: line[a:b].strip() for k, a, b in spans}\n"
                "            row['qty'] = int(row['qty'])\n            out.append(row)\n    return out\n")
        real = json.loads(ref("schema.json"))
        spans = [(f["name"], f["start"], f["end"]) for f in real["fields"]]
        self.assertIn("def load", text)
        cs = reference_commits()
        cs[1] = ("b", {"chainkit/loader.py": hard % repr(spans)})
        r = run_check(cs)
        self.assertFalse(ids(r)["loader"], "right on the real data, wrong on another layout")
        self.assertTrue(ids(r)["integrated"])
        self.assertEqual(r["units_completed"], 3)

    def test_a_loader_that_returns_tuples_breaks_the_interface(self):
        cs = with_change(1, "chainkit/loader.py", lambda t: t.rstrip() + "\n\n\n_load = load\n\n\n"
                         "def load(*a, **k):\n    return [tuple(r.values()) for r in _load(*a, **k)]\n")
        r = run_check(cs)
        self.assertFalse(ids(r)["loader"])
        self.assertFalse(ids(r)["integrated"])

    def test_a_report_with_the_wrong_function_name_breaks_the_interface(self):
        cs = with_change(3, "chainkit/report.py", lambda t: t.replace("def bin_totals", "def totals_by_bin"))
        r = run_check(cs)
        self.assertFalse(ids(r)["report"])
        self.assertEqual(r["units_completed"], 3)

    def test_a_report_that_miscounts_fails_its_step(self):
        cs = with_change(3, "chainkit/report.py", lambda t: t.replace("+ row[\"qty\"]", "+ 1").replace("+= row[\"qty\"]", "+= 1"))
        r = run_check(cs)
        self.assertFalse(ids(r)["report"])


class TestMergeState(unittest.TestCase):
    def test_conflict_markers_are_a_conflicts_left_state(self):
        cs = with_change(2, "chainkit/bins.py", lambda t: t + "\n<<<<<<< HEAD\nx = 1\n=======\nx = 2\n>>>>>>> origin/main\n")
        r = run_check(cs)
        self.assertEqual(r["merge_state"], "conflicts-left")
        self.assertFalse(ids(r)["no-conflict-markers"])
        self.assertFalse(r["correct"])

    def test_a_failing_visible_test_is_a_tests_broken_state(self):
        cs = reference_commits() + [("c", {"tests/test_extra.py": "import unittest\n\n\nclass T(unittest.TestCase):\n"
                                           "    def test_x(self):\n        self.fail('no')\n"})]
        r = run_check(cs)
        self.assertEqual(r["merge_state"], "tests-broken")
        self.assertFalse(ids(r)["visible-tests"])
        self.assertEqual(r["units_completed"], 4)
        self.assertFalse(r["correct"])


if __name__ == "__main__":
    unittest.main()
