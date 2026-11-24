"""The hidden checker of fixture F9 `history` (#1374) against the reference solution and the failing variants: a wrong value,
a contradicted (superseded) decision, work duplicating what the history shows done, a rewritten finished module."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
from test_history_fixture import FACTS, FIXTURE, GENV, build, read

CHECK = os.path.join(BENCH, "checks", "history", "check.sh")
F = FACTS["facts"]


def overlay(dest, src_root):
    for d, _, names in os.walk(src_root):
        for n in names:
            rel = os.path.relpath(os.path.join(d, n), src_root)
            os.makedirs(os.path.dirname(os.path.join(dest, rel)), exist_ok=True)
            shutil.copyfile(os.path.join(d, n), os.path.join(dest, rel))


def check(dest, *flags):
    p = subprocess.run(["bash", CHECK] + list(flags) + [dest], env=GENV, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       text=True, timeout=120)
    assert p.returncode == 0, p.stdout + p.stderr
    return json.loads(p.stdout.strip().splitlines()[-1])


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-hchk-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def tree(self, *overlays):
        dest = build(self.tmp, "B0", "co%d" % len(os.listdir(self.tmp)))
        for o in overlays:
            overlay(dest, o)
        return dest

    def variant(self, name):
        return os.path.join(FIXTURE, "variants", name)

    def ids(self, r):
        return {c["id"]: c["pass"] for c in r["checks"]}


class TestReference(Base):
    def test_the_reference_solution_passes_everything(self):
        r = check(self.tree(os.path.join(FIXTURE, "reference")))
        self.assertTrue(r["correct"], r)
        self.assertEqual((r["units_total"], r["units_completed"]), (4, 4))
        self.assertEqual(r["facts"], {"TAG": "ok", "PAGE_ROWS": "ok", "TARE_G": "ok"})
        self.assertEqual(r["errors"], {"contradicted_decisions": 0, "wrong_values": 0, "duplicated_work": 0,
                                       "stale_understanding": 0})
        self.assertEqual(r["duplicated_helpers"], [])
        self.assertEqual(r["rewritten_modules"], [])
        self.assertEqual(r["merge_state"], "clean")

    def test_the_output_is_one_json_object_on_the_last_line_with_the_checks_schema(self):
        r = check(self.tree(os.path.join(FIXTURE, "reference")))
        self.assertEqual([c["id"] for c in r["checks"] if c.get("unit") is not False],
                         ["ticket-7-works", "fact-file-name", "fact-page-size", "fact-tare"])
        for c in r["checks"]:
            self.assertIn("pass", c)

    def test_the_tree_mode_prints_the_per_fact_result_without_the_slow_checks(self):
        r = check(self.tree(os.path.join(FIXTURE, "reference")), "--tree")
        self.assertEqual(set(r), {"task", "facts", "correct"})
        self.assertTrue(r["correct"])


class TestAlternativeSolutions(Base):
    """The independent review's variants (#1461), kept as regression tests (#1525): structurally different right solutions are
    accepted in full; the stale values are not."""

    def manifest(self, edit):
        ref = read(os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py"))
        text = edit(ref)
        self.assertNotEqual(text, ref, "the variant must change the reference")
        d = self.tree(os.path.join(FIXTURE, "reference"))
        with open(os.path.join(d, "parcelkit", "manifest.py"), "w") as f:
            f.write(text)
        return check(d)

    def slicing(self, tail):
        return lambda t: t.replace("paginate(rows, PAGE_ROWS)", "[rows[i:i + PAGE_ROWS] for i in range(0, len(rows), PAGE_ROWS)]" + tail)

    def test_v2_own_slicing_with_one_empty_page_for_empty_input_is_accepted(self):
        """M4: the task text does not say what pages is for no parcels; [[]] (one empty page) is as right as []."""
        r = self.manifest(self.slicing(" or [[]]"))
        self.assertTrue(self.ids(r)["ticket-7-works"], r)
        self.assertTrue(r["correct"], r)

    def test_v3_own_slicing_with_no_page_for_empty_input_is_accepted(self):
        r = self.manifest(self.slicing(""))
        self.assertTrue(r["correct"], r)

    def test_v4_the_date_written_through_isoformat(self):
        r = self.manifest(lambda t: t.replace('date.strftime("%Y%m%d")', 'date.isoformat().replace("-", "")'))
        self.assertTrue(r["correct"], r)

    def test_v5_the_total_by_sum(self):
        r = self.manifest(lambda t: t.replace('total_cents([r["cents"] for r in rows])', 'sum(r["cents"] for r in rows)'))
        self.assertTrue(r["correct"], r)

    def test_v6_the_stale_values_are_stale_not_accepted(self):
        r = self.manifest(lambda t: t.replace("'%s'" % F["TAG"]["value"], "'%s'" % F["TAG"]["stale"]))
        self.assertEqual(r["facts"]["TAG"], "stale")
        self.assertFalse(r["correct"])

    def test_an_empty_input_that_still_counts_a_parcel_is_rejected(self):
        r = self.manifest(lambda t: self.slicing(" or [[]]")(t).replace('"count": len(rows)', '"count": len(rows) + 1'))
        self.assertFalse(self.ids(r)["ticket-7-works"])


class TestFailingVariants(Base):
    def test_no_manifest_at_all_is_missing_not_stale(self):
        r = check(self.tree())
        self.assertFalse(r["correct"])
        self.assertEqual(r["facts"], {"TAG": "missing", "PAGE_ROWS": "missing", "TARE_G": "missing"})
        self.assertEqual(r["units_completed"], 0)
        self.assertEqual(r["errors"]["stale_understanding"], 0)

    def test_a_wrong_value_is_a_guess_for_each_fact(self):
        r = check(self.tree(self.variant("wrong_value")))
        self.assertFalse(r["correct"])
        self.assertEqual(r["facts"], {"TAG": "guess", "PAGE_ROWS": "guess", "TARE_G": "guess"})
        self.assertEqual(r["errors"]["wrong_values"], 3)
        self.assertEqual(r["errors"]["contradicted_decisions"], 0)
        ids = self.ids(r)
        self.assertTrue(ids["ticket-7-works"])  # the task itself works; only the decided values are wrong
        self.assertFalse(ids["fact-file-name"] or ids["fact-page-size"] or ids["fact-tare"])

    def test_a_superseded_decision_is_a_contradicted_decision(self):
        r = check(self.tree(self.variant("stale_decision")))
        self.assertEqual(r["facts"], {"TAG": "stale", "PAGE_ROWS": "stale", "TARE_G": "ok"})
        self.assertEqual(r["errors"]["contradicted_decisions"], 2)
        self.assertEqual(r["errors"]["wrong_values"], 0)
        self.assertEqual(r["errors"]["stale_understanding"], 2)
        self.assertFalse(r["correct"])
        self.assertEqual(r["units_completed"], 2)

    def test_work_duplicating_finished_helpers_is_found_by_behaviour(self):
        r = check(self.tree(self.variant("duplicated_work")))
        self.assertEqual(r["facts"], {"TAG": "ok", "PAGE_ROWS": "ok", "TARE_G": "ok"})
        self.assertEqual(sorted(h["helper"] for h in r["duplicated_helpers"]), ["paginate", "zone_for"])
        self.assertEqual(r["errors"]["duplicated_work"], 2)
        self.assertEqual(r["errors"]["stale_understanding"], 2)
        self.assertEqual(r["units_completed"], 4)  # the facts are honoured; the duplication is an error, not a lost unit
        self.assertTrue(r["correct"] is False, "duplicated work makes the run not correct")

    def test_a_rewritten_finished_module_counts_as_redone_work(self):
        r = check(self.tree(self.variant("rewrote_finished")))
        self.assertEqual(r["rewritten_modules"], ["parcelkit/ledger.py"])
        self.assertEqual(r["errors"]["duplicated_work"], 1)
        self.assertFalse(r["correct"])

    def test_a_plausible_default_is_a_guess_but_another_number_is_wrong(self):
        ref = read(os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py"))
        odd = ref.replace("PAGE_ROWS = %d" % F["PAGE_ROWS"]["value"], "PAGE_ROWS = 3333").replace(
            'TAG = %r' % F["TAG"]["value"], "TAG = 'zz9z'")
        d = self.tree(os.path.join(FIXTURE, "reference"))
        with open(os.path.join(d, "parcelkit", "manifest.py"), "w") as f:
            f.write(odd)
        r = check(d)
        self.assertEqual(r["facts"], {"TAG": "wrong", "PAGE_ROWS": "wrong", "TARE_G": "ok"})

    def test_a_missing_floor_at_zero_fails_the_tare_fact(self):
        ref = read(os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py"))
        d = self.tree(os.path.join(FIXTURE, "reference"))
        with open(os.path.join(d, "parcelkit", "manifest.py"), "w") as f:
            f.write(ref.replace("max(0, p[\"weight_g\"] - TARE_G)", "p[\"weight_g\"] - TARE_G"))
        r = check(d)
        self.assertEqual(r["facts"]["TARE_G"], "wrong")
        self.assertEqual(r["facts"]["TAG"], "ok")

    def test_the_wrong_filename_shape_is_wrong_even_with_the_right_tag(self):
        ref = read(os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py"))
        d = self.tree(os.path.join(FIXTURE, "reference"))
        with open(os.path.join(d, "parcelkit", "manifest.py"), "w") as f:
            f.write(ref.replace("%03d", "%d"))
        self.assertEqual(check(d)["facts"]["TAG"], "wrong")

    def test_a_manifest_that_crashes_on_import_is_missing(self):
        d = self.tree()
        with open(os.path.join(d, "parcelkit", "manifest.py"), "w") as f:
            f.write("raise RuntimeError('boom')\n")
        r = check(d)
        self.assertEqual(r["facts"], {"TAG": "missing", "PAGE_ROWS": "missing", "TARE_G": "missing"})

    def test_conflict_markers_are_reported_as_the_merge_state(self):
        d = self.tree(os.path.join(FIXTURE, "reference"))
        with open(os.path.join(d, "README.md"), "a") as f:
            f.write("<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\n")
        subprocess.run(["git", "-C", d, "add", "-A"], env=GENV, check=True)
        subprocess.run(["git", "-C", d, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"], env=GENV,
                       check=True)
        self.assertEqual(check(d)["merge_state"], "conflicts-left")

    def test_the_checker_does_not_need_the_seed_to_be_told_to_it(self):
        # a different but valid reference with the same values written differently still passes
        d = self.tree(os.path.join(FIXTURE, "reference"))
        p = os.path.join(d, "parcelkit", "manifest.py")
        text = read(p).replace("TAG = %r" % F["TAG"]["value"], "TAG = \"%s\"" % F["TAG"]["value"])
        with open(p, "w") as f:
            f.write(text)
        self.assertTrue(check(d)["correct"])


class TestReferenceMatchesTheFinishedCode(Base):
    def test_the_checkers_oracle_agrees_with_the_modules_in_the_checkout(self):
        sys.path.insert(0, os.path.join(BENCH, "checks", "history"))
        try:
            import check as chk
        finally:
            sys.path.pop(0)
        d = self.tree()
        sys.path.insert(0, d)
        try:
            from parcelkit import rates, zones, parcels
            for pc in ("01234", "29999", "30000", "55555", "60000", "88888", "90000"):
                self.assertEqual(chk.oracle_zone(pc), zones.zone_for(pc))
            for w in (0, 1, 99, 100, 101, 2500):
                for z in (1, 2, 3, 4):
                    self.assertEqual(chk.oracle_rate(w, z), rates.rate_cents(w, z))
            self.assertEqual(chk.oracle_parse("A1 , 5,12345"), parcels.parse_line("A1 , 5,12345"))
        finally:
            sys.path.pop(0)
            for m in [m for m in sys.modules if m.startswith("parcelkit")]:
                del sys.modules[m]


if __name__ == "__main__":
    unittest.main()
