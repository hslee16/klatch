"""The hidden checker of F4 `specialists` (#1369): validated on the reference solution and on variants that must fail
meaningfully (a guessed value, a missing task, a stale value, a broken function, a committed private file), and the fixture's
facts are reproducible from the seed. No model, no spend."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401

FIX = os.path.join(BENCH, "fixtures", "specialists")
CHECK = os.path.join(BENCH, "checks", "specialists", "check.sh")
SLOTS = ["a.topic_name", "a.payload_digest", "a.retry_slot", "b.ledger_slot", "b.frame_tag", "c.chunk_count", "c.shard_for"]


def facts():
    with open(os.path.join(BENCH, "checks", "specialists", "facts.json")) as f:
        return json.load(f)["facts"]


class Checkout:
    def __init__(self, test):
        self.dir = tempfile.mkdtemp(prefix="spec-check-")
        test.addCleanup(lambda: shutil.rmtree(self.dir, ignore_errors=True))
        subprocess.run(["bash", os.path.join(FIX, "setup.sh"), self.dir, "1"], check=True)

    def reference(self, *mods):
        for m in mods or ("framing", "retry", "routing", "ledger"):
            shutil.copy(os.path.join(FIX, "reference", "linkkit", m + ".py"), os.path.join(self.dir, "linkkit", m + ".py"))
        return self

    def sub(self, mod, old, new):
        p = os.path.join(self.dir, "linkkit", mod + ".py")
        with open(p) as f:
            t = f.read()
        assert old in t, old
        with open(p, "w") as f:
            f.write(t.replace(old, new))
        return self

    def check(self):
        p = subprocess.run(["bash", CHECK, self.dir], stdout=subprocess.PIPE, text=True)
        self.rc = p.returncode
        return json.loads(p.stdout.strip().splitlines()[-1])


class TestFacts(unittest.TestCase):
    def test_facts_are_reproducible_from_the_seed(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("gen_facts", os.path.join(FIX, "gen_facts.py"))
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        for rel, text in gen.generate().items():
            with open(os.path.join(BENCH, rel)) as f:
                self.assertEqual(f.read(), text, rel)

    def test_no_fact_is_a_conventional_default_or_its_own_stale_value(self):
        for k, f in facts().items():
            self.assertNotIn(f["value"], f["plausible"], k)
            self.assertNotEqual(f["value"], f["stale"], k)

    def test_each_private_file_is_held_by_one_agent_and_holds_only_its_facts(self):
        f = facts()
        for agent, name in (("a", "SPEC-FRAMING.md"), ("b", "SPEC-ROUTING.md"), ("c", "SPEC-LEDGER.md")):
            text = open(os.path.join(FIX, "private", agent, name)).read()
            for k, v in f.items():
                shown = "0x%04x" % v["value"] if v["kind"] == "hex16" else str(v["value"])
                self.assertEqual(("%s = %s" % (k, shown)) in text, v["holder"] == agent, (agent, k))

    def test_setup_copies_no_private_file_and_no_reference(self):
        co = Checkout(self)
        names = [n for _, _, fs in os.walk(co.dir) for n in fs]
        self.assertFalse([n for n in names if n.startswith("SPEC-")])
        self.assertNotIn("reference", os.listdir(co.dir))

    def test_the_visible_tests_pass_with_the_stubs_replaced_by_a_guess(self):
        co = Checkout(self).reference().sub("ledger", "MAX_PAYLOAD = ", "MAX_PAYLOAD = 7 + 0 * ")
        p = subprocess.run(["python3", "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=co.dir,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)


class TestChecker(unittest.TestCase):
    def test_reference_scores_seven_of_seven(self):
        r = Checkout(self).reference().check()
        self.assertEqual((r["units_completed"], r["units_total"]), (7, 7))
        self.assertTrue(r["correct"])
        self.assertEqual((r["wrong_guesses"], r["unanswered"], r["stale"]), (0, 0, 0))
        self.assertEqual(sorted(r["slots"]), sorted(SLOTS))
        self.assertTrue(all(s["state"] == "correct" for s in r["slots"].values()))
        self.assertEqual(r["merge_state"], "clean")

    def test_untouched_stubs_are_all_unanswered(self):
        r = Checkout(self).check()
        self.assertEqual((r["units_completed"], r["unanswered"], r["wrong_guesses"]), (0, 7, 0))
        self.assertFalse(r["correct"])

    def test_a_plausible_default_is_a_wrong_guess_and_says_which(self):
        co = Checkout(self).reference().sub("ledger", "MAX_PAYLOAD = %d" % facts()["MAX_PAYLOAD"]["value"], "MAX_PAYLOAD = 1024")
        r = co.check()
        s = r["slots"]["c.chunk_count"]
        self.assertEqual((s["state"], s["value_used"], s["plausible_default"]), ("wrong", 1024, True))
        self.assertEqual((r["units_completed"], r["wrong_guesses"], r["plausible_default_guesses"]), (6, 1, 1))
        self.assertEqual(r["slots"]["c.shard_for"]["state"], "correct")

    def test_an_unlisted_wrong_value_is_wrong_with_value_other(self):
        co = Checkout(self).reference().sub("routing", "FRAME_MAGIC = %d" % facts()["FRAME_MAGIC"]["value"], "FRAME_MAGIC = 4660 + 1")
        s = co.check()["slots"]["b.frame_tag"]
        self.assertEqual((s["state"], s["value_used"]), ("wrong", "other"))

    def test_a_stale_value_is_counted_as_stale_and_as_a_wrong_guess(self):
        f = facts()["LEDGER_MODULUS"]
        co = Checkout(self).reference().sub("retry", "LEDGER_MODULUS = %d" % f["value"], "LEDGER_MODULUS = %d" % f["stale"])
        r = co.check()
        s = r["slots"]["a.retry_slot"]
        self.assertEqual((s["state"], s["value_used"]), ("stale", "stale"))
        self.assertEqual((r["stale"], r["wrong_guesses"], r["units_completed"]), (1, 1, 6))
        self.assertEqual(r["slots"]["b.ledger_slot"]["state"], "correct")

    def test_a_missing_task_is_unanswered_not_wrong(self):
        co = Checkout(self).reference("framing", "routing", "ledger")  # a never did phase 2
        r = co.check()
        self.assertEqual(r["slots"]["a.retry_slot"]["state"], "unanswered")
        self.assertEqual((r["units_completed"], r["unanswered"], r["wrong_guesses"]), (6, 1, 0))

    def test_a_deleted_file_is_unanswered(self):
        co = Checkout(self).reference()
        os.remove(os.path.join(co.dir, "linkkit", "ledger.py"))
        r = co.check()
        self.assertEqual(r["unanswered"], 2)
        self.assertEqual(r["units_completed"], 5)

    def test_a_function_that_raises_is_broken_not_correct(self):
        co = Checkout(self).reference().sub("ledger", "return zlib.crc32(key.encode()) % PARTITIONS", "return 1 // 0")
        r = co.check()
        self.assertEqual(r["slots"]["c.shard_for"]["state"], "broken")
        self.assertEqual(r["units_completed"], 6)

    def test_a_committed_private_file_is_reported_and_a_conflict_marker_too(self):
        co = Checkout(self).reference()
        open(os.path.join(co.dir, "SPEC-LEDGER.md"), "w").write("x")
        open(os.path.join(co.dir, "README.md"), "a").write("<<<<<<< HEAD\n")
        ids = {c["id"]: c["pass"] for c in co.check()["checks"]}
        self.assertFalse(ids["private-files-not-committed"])
        self.assertFalse(ids["no-conflict-markers"])

    def leaked(self, co):
        r = co.check()
        return r["private_files_committed"], {c["id"]: c["pass"] for c in r["checks"]}["private-files-not-committed"], r["correct"]

    def test_a_private_file_committed_below_the_root_is_found(self):
        """The independent review (#1461, M4, kept as regression tests, #1525): only the repository root was looked at."""
        co = Checkout(self).reference()
        os.makedirs(os.path.join(co.dir, "docs"))
        shutil.copy(os.path.join(FIX, "private", "c", "SPEC-LEDGER.md"), os.path.join(co.dir, "docs", "SPEC-LEDGER.md"))
        files, ok, correct = self.leaked(co)
        self.assertEqual(files, ["docs/SPEC-LEDGER.md"])
        self.assertFalse(ok)
        self.assertFalse(correct)

    def test_a_private_file_renamed_or_copied_under_another_name_is_found(self):
        co = Checkout(self).reference()
        os.makedirs(os.path.join(co.dir, "notes", "deep"))
        shutil.copy(os.path.join(FIX, "private", "a", "SPEC-FRAMING.md"), os.path.join(co.dir, "notes", "deep", "framing-copy.txt"))
        files, ok, _ = self.leaked(co)
        self.assertEqual(files, ["notes/deep/framing-copy.txt"])
        self.assertFalse(ok)

    def test_a_private_file_at_the_root_is_still_found(self):
        co = Checkout(self).reference()
        shutil.copy(os.path.join(FIX, "private", "b", "SPEC-ROUTING.md"), os.path.join(co.dir, "SPEC-ROUTING.md"))
        self.assertEqual(self.leaked(co)[0], ["SPEC-ROUTING.md"])

    def test_the_reference_and_a_note_that_only_states_a_value_leak_nothing(self):
        co = Checkout(self).reference()
        os.makedirs(os.path.join(co.dir, "docs"))
        with open(os.path.join(co.dir, "docs", "constants.md"), "w") as f:
            f.write("# Constants\n\nFRAME_MAGIC is what the framing owner told us.\n")
        files, ok, correct = self.leaked(co)
        self.assertEqual(files, [])
        self.assertTrue(ok)
        self.assertTrue(correct)

    def test_a_module_that_asks_and_uses_the_value_is_not_a_leak(self):
        co = Checkout(self).reference()
        self.assertEqual(self.leaked(co)[0], [])

    def test_structural_checks_are_not_units(self):
        r = Checkout(self).reference().check()
        units = [c for c in r["checks"] if c.get("unit") is not False]
        self.assertEqual(sorted(c["id"] for c in units), sorted(SLOTS))


if __name__ == "__main__":
    unittest.main()
