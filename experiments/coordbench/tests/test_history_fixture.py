"""Fixture F9 `history` (#1374): the prebuilt git history, the board files of arm B1, the prebuilt room history of the K arms,
the seeded facts, and the parity of the information across arms. No model, no spend. The room history is posted into a
temporary store with a temporary key directory (never ~/.klatch); those tests need KLATCH_BIN like every K-arm test."""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import fixture as fixture_mod
import history
import run
from test_run_fake import NEEDS_BIN, KLATCH_BIN

FIXTURE = os.path.join(BENCH, "fixtures", "history")
def jload(path):
    with open(path) as f:
        return json.load(f)


FACTS = jload(os.path.join(BENCH, "checks", "history", "facts.json"))
F = FACTS["facts"]
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull, "GIT_TERMINAL_PROMPT": "0"}
CURRENT = [F["TAG"]["value"], str(F["PAGE_ROWS"]["value"]), str(F["TARE_G"]["value"])]
STALE = [F["TAG"]["stale"], str(F["PAGE_ROWS"]["stale"])]


def read(path):
    with open(path) as f:
        return f.read()


def has(text, v):
    """v as a whole token (85 is not in 485)."""
    return re.search(r"(?<![0-9A-Za-z])%s(?![0-9A-Za-z])" % re.escape(v), text) is not None


def build(tmp, arm, name="co"):
    dest = os.path.join(tmp, name)
    os.makedirs(dest)
    subprocess.run(["bash", os.path.join(FIXTURE, "setup.sh"), dest, "1"], check=True, env=dict(GENV, COORDBENCH_ARM=arm),
                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return dest


def git(cwd, *args):
    p = subprocess.run(["git", "-C", cwd] + list(args), env=GENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    assert p.returncode == 0, p.stdout
    return p.stdout


class Tmp(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-hist-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))


class TestGenerator(unittest.TestCase):
    def test_committed_files_equal_what_the_seed_generates(self):
        p = subprocess.run([sys.executable, os.path.join(FIXTURE, "gen_facts.py"), "--check"], stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)

    def test_the_facts_are_not_conventional_defaults(self):
        for key, f in F.items():
            self.assertNotIn(f["value"], f["plausible"], key)
            if f["stale"] is not None:
                self.assertNotIn(f["stale"], f["plausible"], key)
                self.assertNotEqual(f["stale"], f["value"], key)
        self.assertIsNone(F["TARE_G"]["stale"])


class TestPrebuiltRepository(Tmp):
    def test_25_commits_by_three_earlier_agents_with_meaningful_messages(self):
        co = build(self.tmp, "B0")
        out = git(co, "log", "--format=%an%x01%s")
        rows = [l.split("\x01") for l in out.splitlines()]
        self.assertEqual(len(rows), 25)
        self.assertEqual(sorted(set(a for a, _ in rows)), ["ada", "bo", "cy"])
        for _, subject in rows:
            self.assertGreater(len(subject.split()), 3, subject)
        self.assertEqual(git(co, "status", "--porcelain"), "")

    def test_b0_has_no_board_and_no_decision_file(self):
        co = build(self.tmp, "B0")
        self.assertFalse(os.path.exists(os.path.join(co, "TASKS.md")))
        self.assertFalse(os.path.exists(os.path.join(co, "DECISIONS.md")))

    def test_k_arms_have_the_same_repository_as_b0(self):
        shas = {arm: git(build(self.tmp, arm, "co-" + arm), "rev-parse", "HEAD") for arm in ("B0", "K1", "K2")}
        self.assertEqual(len(set(shas.values())), 1, shas)

    def test_the_history_is_deterministic(self):
        a = git(build(self.tmp, "B1", "x1"), "rev-parse", "HEAD")
        b = git(build(self.tmp, "B1", "x2"), "rev-parse", "HEAD")
        self.assertEqual(a, b)

    def test_b1_has_the_board_and_decisions_with_the_same_commit_messages(self):
        b0 = build(self.tmp, "B0", "b0")
        b1 = build(self.tmp, "B1", "b1")
        self.assertEqual(git(b0, "log", "--format=%B"), git(b1, "log", "--format=%B"))
        tasks = read(os.path.join(b1, "TASKS.md"))
        self.assertTrue(tasks.startswith(run.TASK_BOARD), tasks[:200])
        self.assertIn("handoff", tasks)
        dec = read(os.path.join(b1, "DECISIONS.md"))
        for v in CURRENT + STALE:
            self.assertIn(v, dec)
        self.assertIn("supersedes D2", dec)
        self.assertIn("supersedes D3", dec)

    def test_the_visible_tests_pass_at_the_head(self):
        co = build(self.tmp, "B0")
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=co, env=GENV,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)

    def test_ticket_7_is_not_in_the_checkout_and_nothing_hidden_is(self):
        co = build(self.tmp, "B1")
        self.assertFalse(os.path.exists(os.path.join(co, "parcelkit", "manifest.py")))
        for d, _, names in os.walk(co):
            if ".git" in d.split(os.sep):
                continue
            for n in names:
                self.assertNotIn(n, ("facts.json", "check.py", "check.sh", "gen_facts.py"))
        self.assertFalse(os.path.exists(os.path.join(co, "reference")))

    def test_no_seeded_value_is_in_any_file_of_the_head_or_in_its_code(self):
        for arm in ("B0", "B1"):
            co = build(self.tmp, arm, "v-" + arm)
            for d, dirs, names in os.walk(co):
                dirs[:] = [x for x in dirs if x != ".git"]
                for n in names:
                    if arm == "B1" and n in ("TASKS.md", "DECISIONS.md"):
                        continue  # the arm's medium is where the record is meant to be
                    text = read(os.path.join(d, n))
                    for v in CURRENT + STALE:
                        self.assertFalse(has(text, v), "%s holds %s in arm %s" % (n, v, arm))

    def test_every_arm_can_recover_every_fact_from_the_git_log_alone(self):
        for arm in ("B0", "B1", "K1"):
            log = git(build(self.tmp, arm, "p-" + arm), "log", "--format=%B")
            for v in CURRENT + STALE:
                self.assertIn(v, log, "%s is not in the git log of arm %s" % (v, arm))
            self.assertIn("supersedes D2", log)
            self.assertIn("supersedes D3", log)

    def test_the_old_values_precede_the_revisions_in_the_log(self):
        log = git(build(self.tmp, "B0"), "log", "--reverse", "--format=%B")
        self.assertLess(log.index(str(F["PAGE_ROWS"]["stale"])), log.index("PAGE_ROWS is now %d" % F["PAGE_ROWS"]["value"]))
        self.assertLess(log.index("TAG = %s" % F["TAG"]["stale"]), log.index("TAG = %s, not" % F["TAG"]["value"]))

    def test_the_harness_board_header_is_the_one_in_the_history_file(self):
        h = jload(os.path.join(FIXTURE, "history.json"))
        self.assertEqual(h["board_header"], run.TASK_BOARD)


class TestRoomScript(unittest.TestCase):
    def setUp(self):
        self.steps = jload(os.path.join(FIXTURE, "room.json"))["steps"]
        self.msgs = [s for s in self.steps if s["argv"][0] in ("post", "ask", "answer")]

    def test_about_sixty_messages_by_three_members(self):
        self.assertGreaterEqual(len(self.msgs), 55)
        self.assertLessEqual(len(self.msgs), 70)
        self.assertEqual(sorted(set(s["as"] for s in self.steps)), ["ada", "bo", "cy"])

    def test_it_has_claims_an_ask_and_its_answer_decisions_a_handoff_and_a_checkpoint(self):
        texts = [s["argv"][2] if s["argv"][0] != "answer" else s["argv"][3] for s in self.msgs]
        self.assertTrue(any(s["argv"][0] == "claim" for s in self.steps))
        self.assertTrue(any(s["argv"][0] == "ask" for s in self.steps))
        self.assertTrue(any(s["argv"][0] == "answer" for s in self.steps))
        self.assertGreaterEqual(sum(t.startswith("decision:") for t in texts), 5)
        self.assertTrue(any(t.startswith("handoff:") for t in texts))
        self.assertTrue(any(t.startswith("checkpoint:") for t in texts))

    def test_every_fact_is_in_the_room_and_the_revisions_name_what_they_supersede(self):
        blob = json.dumps(self.steps)
        for v in CURRENT + STALE:
            self.assertIn(v, blob)
        self.assertGreaterEqual(blob.count("supersedes #{{ask:"), 3)

    def test_every_ask_key_is_defined_before_it_is_used(self):
        defined = set()
        for s in self.steps:
            for key in re.findall(r"\{\{ask:(\w+)\}\}", json.dumps(s["argv"])):
                self.assertIn(key, defined, s)
            if s["argv"][0] == "ask":
                defined.add(s["key"])


@NEEDS_BIN
class TestRoomThroughTheBinary(Tmp):
    def test_the_room_history_is_posted_into_a_temp_store_and_is_readable(self):
        home, keys = os.path.join(self.tmp, "kh"), os.path.join(self.tmp, "kk")
        os.makedirs(home, mode=0o700)
        os.makedirs(keys, mode=0o700)
        env = dict(GENV, KLATCH_HOME=home, KLATCH_KEY_DIR=keys, HOME=self.tmp)
        steps = jload(os.path.join(FIXTURE, "room.json"))["steps"]
        n = history.post_room(KLATCH_BIN, "lobby", steps, env, self.tmp)
        self.assertGreaterEqual(n, 55)
        out = subprocess.run([KLATCH_BIN, "history", "lobby", "--last", "200"], env=dict(env, KLATCH_NAME="dana"),
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True).stdout
        for v in CURRENT:
            self.assertIn(v, out)
        self.assertGreaterEqual(out.count("\n"), 55)
        self.assertNotIn("{{ask:", out)
        asks = subprocess.run([KLATCH_BIN, "asks", "lobby"], env=dict(env, KLATCH_NAME="dana"), stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True).stdout
        self.assertNotIn("seeking", asks)  # every ask was answered and closed

    def test_an_unanswerable_step_raises(self):
        home, keys = os.path.join(self.tmp, "kh"), os.path.join(self.tmp, "kk")
        os.makedirs(home, mode=0o700)
        os.makedirs(keys, mode=0o700)
        env = dict(GENV, KLATCH_HOME=home, KLATCH_KEY_DIR=keys, HOME=self.tmp)
        with self.assertRaises(RuntimeError):
            history.post_room(KLATCH_BIN, "lobby", [{"as": "ada", "argv": ["answer", "lobby", "{{ask:nope}}", "x"]}], env, self.tmp)


class TestFixtureDescription(unittest.TestCase):
    def test_variants_agents_and_prebuilt_flags(self):
        fx = fixture_mod.Fixture("history")
        self.assertTrue(fx.prebuilt_history)
        self.assertEqual(fx.names, ["dana"])
        self.assertIsNone(fx.resume)
        self.assertEqual(len(fx.room_script), 78)
        rx = fixture_mod.Fixture("history", variant="resume")
        self.assertEqual(rx.name, "history-resume")
        self.assertEqual(rx.names, ["dana"])
        self.assertEqual(rx.resume["agent"], "dana")
        self.assertGreater(rx.resume["after_turns"], 0)
        self.assertIn("context", rx.resume["note"])
        self.assertEqual(rx.agents[0]["task"], fx.agents[0]["task"])

    def test_an_unknown_variant_is_refused(self):
        with self.assertRaises(fixture_mod.FixtureError):
            fixture_mod.Fixture("history", variant="nope")

    def test_the_job_text_does_not_contain_a_seeded_value(self):
        fx = fixture_mod.Fixture("history")
        for v in CURRENT + STALE:
            self.assertNotIn(v, fx.agents[0]["task"])


if __name__ == "__main__":
    unittest.main()
