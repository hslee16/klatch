"""Scenario S9 (#1374): the scenario file, and the metrics of fixture F9 `history` through the whole harness with fake agents:
a late joiner that reads the record and does it right, one that never reads and guesses, one that reads half and contradicts a
decision, one that duplicates finished work, and a resumed agent (its context cleared mid-task) that recovers from a short
note. No model, no spend. K-arm runs post the room history through KLATCH_BIN into the run's own temporary store."""
import json
import os
import re
import shutil
import unittest

from helpers import BENCH  # noqa: F401
import drivers
import fixture as fixture_mod
import run
import scenario
import schema
from test_history_fixture import CURRENT, jload, read
from test_run_fake import Base, KLATCH_BIN, NEEDS_BIN, read_json

FIXTURE = os.path.join(BENCH, "fixtures", "history")
REF = "reference/parcelkit/manifest.py"
V = lambda name: "variants/%s/parcelkit/manifest.py" % name


def commit_steps(manifest, message="ticket 7 manifest"):
    return [{"op": "copy_from", "from": manifest, "path": "parcelkit/manifest.py"},
            {"op": "copy_from", "from": "reference/tests/test_manifest.py", "path": "tests/test_manifest.py"},
            {"op": "git", "args": ["add", "-A"]}, {"op": "git", "args": ["commit", "-m", message]},
            {"op": "git", "args": ["push", "origin", "HEAD:main"]}]


def reads(arm_kinds):
    """The reading steps of one arm: B arms git log (and B1 the board files), K arms the room."""
    return [{"op": "git", "args": ["log", "--format=%B"], "arms": ["B0", "B1"]},
            {"op": "read", "path": "DECISIONS.md", "arms": ["B1"]},
            {"op": "klatch", "args": ["join", "{{ROOM}}"], "arms": ["K1"]},
            {"op": "klatch", "args": ["history", "{{ROOM}}", "--last", "100"], "arms": ["K1", "K2"]}]


FINAL = [{"op": "final", "text": "DONE dana"}]


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s9")

    def test_arms_n_caps_and_metrics_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["id"], "S9")
        self.assertEqual(s["fixture"], "history")
        self.assertEqual(s["arms"], ["B0", "B1", "K1", "K2"])
        self.assertEqual(s["n_per_arm"]["m0"], 10)
        self.assertEqual(s["n_per_arm"]["quick"], 3)
        self.assertEqual(s["caps"]["per_run_usd"], 0.9)
        self.assertEqual(s["caps"]["max_turns_per_agent"], 40)
        self.assertEqual(s["metrics"]["primary"], "correct")
        for k in ("correct", "facts", "errors", "first_correct", "history_reads", "briefing_seen",
                  "history_read_before_first_edit", "room_history_used_unprompted"):
            self.assertIn(k, s["metrics"]["definitions"])

    def test_the_arm_notes_say_what_is_where(self):
        n = self.scn["arm_notes"]
        self.assertIn("commit messages", n["B0"])
        self.assertIn("TASKS.md", n["B1"])
        self.assertIn("DECISIONS.md", n["B1"])
        self.assertIn("room history", n["K1"])
        self.assertIn("briefing", n["K2"])

    def test_gaps_are_stated(self):
        self.assertIn("briefing_header_after_compact", self.scn["gaps"])
        self.assertIn("compaction_summary", self.scn["gaps"])

    def test_the_scenario_is_found_from_the_fixture(self):
        fx = read_json(os.path.join(FIXTURE, "fixture.json"))
        self.assertEqual(fx["scenario"], "s9")
        self.assertEqual(self.scn["history_metrics"]["agent"], fx["agents"][0]["name"])
        self.assertEqual(sorted(self.scn["variants"]), sorted(fx["variants"]))

    def test_the_case_plan_matches_the_spec_arms(self):
        # SPEC S9: arms B0, B1, K1, K2 (the K3 column of the matrix is empty for S9)
        self.assertNotIn("K3", self.scn["arms"])


class S9Base(Base):
    def argv(self, arms="B0", runs=1, variant=None, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "history"
        if variant:
            a += ["--variant", variant]
        return a

    def records(self, scenario_dir="history"):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", scenario_dir)):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return sorted(out, key=lambda r: r["run_id"])

    def one(self, arm="B0", agents=None, variant=None):
        extra = {}
        if agents is not None:
            path = os.path.join(FIXTURE, "fake-test-%d.json" % os.getpid())
            self.addCleanup(lambda: os.path.exists(path) and os.remove(path))
            with open(path, "w") as f:
                json.dump({"agents": agents}, f)
            extra["fake_script"] = path
        code, out, err = self.go(self.argv(arm, 1, variant=variant, **extra))
        recs = self.records("history-resume" if variant == "resume" else "history")
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r


class TestLateJoinerThatReadsAndDoesItRight(S9Base):
    def test_the_fixtures_own_fake_script_in_b0(self):
        r = self.one("B0")
        self.assertEqual(r["outcome"], "ok")
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (4, 4))
        sm = r["scenario_metrics"]
        self.assertTrue(sm["correct"])
        self.assertEqual(sm["facts"], {"TAG": "ok", "PAGE_ROWS": "ok", "TARE_G": "ok"})
        self.assertEqual(sm["errors"]["stale_understanding"], 0)
        h = sm["history"]
        self.assertEqual(h["history_reads_by_kind"], {"git_log": 1})
        self.assertTrue(h["history_read_before_first_edit"])
        self.assertIsNone(h["room_history_used_unprompted"])
        fc = h["first_correct"]
        self.assertEqual((fc["commit_index"], fc["located_by"], fc["history_reads_before"]), (1, "sha", 1))
        self.assertEqual(fc["turn"], 5)  # git log, 2 copies, add, commit
        self.assertEqual(h["turns"], 7)  # ... then the push and the final message
        self.assertEqual(fc["tokens"]["total"], 5 * 15)
        self.assertEqual(r["scenario"], "history")
        self.assertEqual(r["turns"]["per_agent"], {"dana": 7})

    def test_the_repository_the_agent_gets_is_the_prebuilt_one_for_its_arm(self):
        # no first commit of the harness's own: the agent's clone has the 25 scripted commits and nothing else
        self.check_clone("B0", False)
        self.check_clone("B1", True)

    def check_clone(self, arm, board):
        shutil.rmtree(self.results, ignore_errors=True)
        r = self.one(arm, {"dana": [{"op": "run", "argv": ["git", "log", "--format=%an"]}, {"op": "run", "argv": ["ls"]}] + FINAL})
        events = [json.loads(l) for l in read(os.path.join(self.logs, r["run_id"], "dana.stream.jsonl")).splitlines()]
        results = [b["content"] for e in events if e.get("type") == "user" for b in e["message"]["content"]]
        authors = results[0].split()
        self.assertEqual(len(authors), 25, arm)
        self.assertEqual(sorted(set(authors)), ["ada", "bo", "cy"], arm)
        self.assertEqual("DECISIONS.md" in results[1].split(), board, arm)
        self.assertEqual("TASKS.md" in results[1].split(), board, arm)

    def test_b1_reads_the_board_files_and_they_count_by_kind(self):
        r = self.one("B1")
        h = r["scenario_metrics"]["history"]
        self.assertEqual(h["history_reads_by_kind"], {"board_file": 2, "git_log": 1})
        self.assertTrue(r["scenario_metrics"]["correct"])

    @NEEDS_BIN
    def test_k1_reads_the_room_history_posted_before_it_started(self):
        r = self.one("K1")
        h = r["scenario_metrics"]["history"]
        self.assertEqual(h["history_reads_by_kind"], {"klatch_history": 1})
        self.assertTrue(h["room_history_used_unprompted"])
        self.assertFalse(h["briefing_seen"])
        self.assertGreater(h["history_read_chars"], 3000)  # the history of ~60 messages came back
        blob = read(os.path.join(self.logs, r["run_id"], "dana.stream.jsonl"))
        for v in CURRENT:
            self.assertIn(v, blob)  # the real binary returned the seeded record to the agent

    @NEEDS_BIN
    def test_k2_sees_the_briefing_and_reads_the_room(self):
        r = self.one("K2")
        h = r["scenario_metrics"]["history"]
        self.assertTrue(h["briefing_seen"])
        self.assertTrue(h["room_history_used_unprompted"])
        self.assertEqual(h["history_reads_by_kind"], {"klatch_history": 1})


class TestLateJoinerThatNeverReads(S9Base):
    def test_a_guess_is_a_wrong_value_with_no_history_reads(self):
        r = self.one("B0", {"dana": commit_steps(V("wrong_value"), "ticket 7 manifest, guessed") + FINAL})
        sm = r["scenario_metrics"]
        self.assertFalse(sm["correct"])
        self.assertEqual(sm["facts"], {"TAG": "guess", "PAGE_ROWS": "guess", "TARE_G": "guess"})
        self.assertEqual(sm["errors"], {"contradicted_decisions": 0, "wrong_values": 3, "duplicated_work": 0,
                                        "stale_understanding": 3})
        h = sm["history"]
        self.assertEqual(h["history_reads"], 0)
        self.assertFalse(h["history_read_before_first_edit"])
        self.assertIsNone(h["first_correct"])
        self.assertEqual((h["agent_commits"], h["correct_commits"]), (1, 0))
        self.assertEqual(r["metrics"]["units_completed"], 1)  # the ticket works; none of the three facts

    @NEEDS_BIN
    def test_a_k_arm_agent_that_never_reads_the_room_did_not_use_it(self):
        r = self.one("K2", {"dana": commit_steps(V("wrong_value")) + FINAL})
        h = r["scenario_metrics"]["history"]
        self.assertFalse(h["room_history_used_unprompted"])
        self.assertFalse(h["briefing_seen"])


class TestLateJoinerThatReadsHalf(S9Base):
    def test_it_reads_the_old_decisions_and_contradicts_the_later_ones(self):
        steps = [{"op": "git", "args": ["log", "--reverse", "-n", "12", "--format=%B"]}] + commit_steps(V("stale_decision")) + FINAL
        r = self.one("B0", {"dana": steps})
        sm = r["scenario_metrics"]
        self.assertEqual(sm["facts"], {"TAG": "stale", "PAGE_ROWS": "stale", "TARE_G": "ok"})
        self.assertEqual(sm["errors"]["contradicted_decisions"], 2)
        self.assertEqual(sm["errors"]["stale_understanding"], 2)
        self.assertEqual(r["metrics"]["units_completed"], 2)
        self.assertEqual(sm["history"]["history_reads"], 1)
        self.assertIsNone(sm["history"]["first_correct"])

    def test_it_corrects_itself_with_a_second_commit_and_the_first_correct_is_the_second(self):
        steps = [{"op": "git", "args": ["log", "--reverse", "-n", "12", "--format=%B"]}] + commit_steps(
            V("stale_decision"), "first cut") + [{"op": "git", "args": ["log", "--format=%B"]}] + commit_steps(REF, "fixed") + FINAL
        r = self.one("B0", {"dana": steps})
        h = r["scenario_metrics"]["history"]
        self.assertEqual((h["agent_commits"], h["correct_commits"]), (2, 1))
        fc = h["first_correct"]
        self.assertEqual((fc["commit_index"], fc["located_by"]), (2, "sha"))
        self.assertEqual(fc["history_reads_before"], 2)
        self.assertEqual(fc["turn"], 1 + 5 + 1 + 5 - 1)  # log, five, log, the commit call of the second round (fifth step: push after)
        self.assertTrue(r["scenario_metrics"]["correct"])  # the final main is right


class TestLateJoinerThatDuplicatesWork(S9Base):
    def test_a_second_paginate_and_zone_rule_is_duplicated_work(self):
        r = self.one("B0", {"dana": [{"op": "git", "args": ["log", "--format=%B"]}] + commit_steps(V("duplicated_work")) + FINAL})
        sm = r["scenario_metrics"]
        self.assertEqual(sm["facts"], {"TAG": "ok", "PAGE_ROWS": "ok", "TARE_G": "ok"})
        self.assertEqual(sm["errors"]["duplicated_work"], 2)
        self.assertFalse(sm["correct"])
        self.assertEqual(r["metrics"]["units_completed"], 4)
        self.assertEqual(sm["history"]["first_correct"]["commit_index"], 1)  # the first commit passes the per-fact checks


class TestResumeVariant(S9Base):
    def resume_agents(self, second):
        first = [{"op": "git", "args": ["log", "--format=%B"]}] + commit_steps(V("wrong_value"), "wip: manifest first cut") + FINAL
        return {"dana": first, "dana#2": second}

    def test_the_second_session_has_the_same_name_the_same_clone_and_a_short_note(self):
        second = [{"op": "git", "args": ["log", "-n", "2", "--format=%an %s"]},
                  {"op": "git", "args": ["log", "--format=%B"]}] + commit_steps(REF, "ticket 7 manifest, resumed") + FINAL
        r = self.one("B0", self.resume_agents(second), variant="resume")
        self.assertEqual(r["scenario"], "history-resume")
        self.assertTrue(r["run_id"].startswith("t1-history-resume-b0"), r["run_id"])
        self.assertEqual(r["outcome"], "ok")
        files = sorted(os.listdir(os.path.join(self.logs, r["run_id"])))
        for want in ("dana.stream.jsonl", "dana.r2.stream.jsonl", "dana.prompt.md", "dana.r2.prompt.md"):
            self.assertIn(want, files)
        p1 = read(os.path.join(self.logs, r["run_id"], "dana.prompt.md"))
        p2 = read(os.path.join(self.logs, r["run_id"], "dana.r2.prompt.md"))
        self.assertNotIn("Resume note", p1)
        self.assertIn("Resume note", p2)
        self.assertIn("You are dana", p2)
        job = fixture_mod.Fixture("history").agents[0]["task"]
        self.assertIn(job, p1)
        self.assertIn(job, p2)  # the job text survives; the memory does not
        self.assertEqual(len(p2) - len(p1), len(fixture_mod.Fixture("history", variant="resume").resume["note"]) + 2)
        self.assertEqual(list(r["turns"]["per_agent"]), ["dana"])  # one agent, two sessions

    def test_the_agent_recovers_and_the_cost_counts_both_sessions(self):
        second = [{"op": "git", "args": ["log", "--format=%B"]}] + commit_steps(REF, "ticket 7 manifest, resumed") + FINAL
        r = self.one("B0", self.resume_agents(second), variant="resume")
        h = r["scenario_metrics"]["history"]
        self.assertEqual(h["sessions"], 2)
        self.assertEqual(h["history_reads_by_session"], [1, 1])
        self.assertEqual((h["agent_commits"], h["correct_commits"]), (2, 1))
        fc = h["first_correct"]
        self.assertEqual((fc["session"], fc["commit_index"]), (2, 2))
        self.assertGreater(fc["turn"], 6)  # session 1's turns count: the cost of recovering includes the first session
        self.assertEqual(h["turns"], r["turns"]["total"])
        self.assertTrue(r["scenario_metrics"]["correct"])
        self.assertEqual(r["metrics"]["units_completed"], 4)

    def test_an_agent_that_resumes_without_reading_keeps_its_stale_work(self):
        second = [{"op": "git", "args": ["status", "--short"]}] + FINAL
        r = self.one("B0", self.resume_agents(second), variant="resume")
        sm = r["scenario_metrics"]
        self.assertFalse(sm["correct"])
        self.assertEqual(sm["history"]["history_reads_by_session"], [1, 0])
        self.assertIsNone(sm["history"]["first_correct"])
        self.assertEqual(sm["facts"]["TAG"], "guess")

    def test_a_cut_session_that_exits_nonzero_is_not_a_crash(self):
        first = [{"op": "git", "args": ["log", "--format=%B"]}, {"op": "exit", "code": 1}]
        second = commit_steps(REF) + FINAL
        r = self.one("B0", {"dana": first, "dana#2": second}, variant="resume")
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertTrue(r["scenario_metrics"]["correct"])

    def test_the_join_variant_has_one_session_and_no_second_prompt(self):
        r = self.one("B0")
        self.assertFalse(os.path.exists(os.path.join(self.logs, r["run_id"], "dana.r2.prompt.md")))
        self.assertEqual(r["scenario_metrics"]["history"]["sessions"], 1)

    def test_an_explicit_join_variant_is_the_default(self):
        r = self.one("B0", variant="join")
        self.assertEqual(r["scenario"], "history")

    def test_the_real_driver_cuts_the_first_session_by_turns_and_not_the_second(self):
        fx = fixture_mod.Fixture("history", variant="resume")
        d = drivers.ClaudeDriver("claude", "claude-haiku-4-5", 40, 0.3)
        argv1, _ = d.command("dana", "B0", "p", "/w", "/o", session=1, max_turns=fx.resume["after_turns"])
        argv2, _ = d.command("dana", "B0", "p2", "/w", "/o", session=2)
        self.assertEqual(argv1[argv1.index("--max-turns") + 1], str(fx.resume["after_turns"]))
        self.assertEqual(argv2[argv2.index("--max-turns") + 1], "40")
        self.assertEqual(argv1[argv1.index("--name") + 1], argv2[argv2.index("--name") + 1])  # the same member name
        self.assertIn("--no-session-persistence", argv2)  # nothing of session 1 is resumed


class TestRoomHistoryIsolation(S9Base):
    @NEEDS_BIN
    def test_the_room_history_lives_in_the_runs_temp_store_only(self):
        live = os.path.join(os.path.expanduser("~"), ".klatch")
        before = os.path.getmtime(live) if os.path.exists(live) else None
        r = self.one("K1")
        self.assertEqual(r["outcome"], "ok")
        after = os.path.getmtime(live) if os.path.exists(live) else None
        self.assertEqual(before, after)

    def test_b_arms_post_no_room_history(self):
        # no binary is needed for a B arm: the run works without KLATCH_BIN in the arguments
        argv = [x for x in self.argv("B0") if x != KLATCH_BIN and x != "--klatch-bin"]
        code, out, err = self.go(argv)
        self.assertEqual(code, 0, out + err)


class TestDryRunAndEstimate(S9Base):
    def test_the_dry_run_names_the_variant_scenario_and_has_an_estimate(self):
        code, out, err = self.go(self.argv("B0,K2", 2, variant="resume") + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("fixture history-resume", out)
        self.assertIn("estimated cost", out)
        self.assertNotIn("no planning figure", out)


if __name__ == "__main__":
    unittest.main()
