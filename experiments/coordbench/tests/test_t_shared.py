"""Topology T-shared (#1427): several agent processes in ONE working directory, scenario S2 `onefile`, variant `shared`.

Everything here runs through run.py with fake agents: no model, no spend. The scenario is recorded as `onefile-shared` and
never pooled with the T-clone runs. Lost updates are made on purpose with the fake agents' barrier ops, so that two agents
read the file, both write, and the second silently overwrites the first.
"""
import json
import os
import re
import sys
import unittest

from helpers import BENCH  # noqa: F401
import drivers
import fixture as fixture_mod
import run
import schema
import shared
from test_run_fake import KLATCH_BIN, NEEDS_BIN, read_json
from test_s2_scenario import CORE, EDITS, S2Base

ARMS = os.path.join(BENCH, "arms")
if ARMS not in sys.path:
    sys.path.insert(0, ARMS)
import check_parity as cp


def pairs_and_append(agent):
    """The ticket's reference edits as ([old, new] pairs, appended text) for write_from_var."""
    pairs = [[s["old"], s["new"]] for s in EDITS[agent] if s["op"] == "replace"]
    return pairs, "".join(s["content"] for s in EDITS[agent] if s["op"] == "append")


def stale_write(agent, key="base"):
    pairs, app = pairs_and_append(agent)
    return {"op": "write_from_var", "path": CORE, "key": key, "replace": pairs, "append": app}


def commit(agent):
    return [{"op": "git", "args": ["add", CORE]}, {"op": "git", "args": ["commit", "-q", "-m", "ticket " + agent.upper()]}]


DONE = lambda n: {"op": "final", "text": "DONE " + n}  # noqa: E731
GIVE_UP = [{"op": "final", "text": "gave up"}]


def lost_update_scripts(hold="snapshot"):
    """a and b both read the initial file; a writes its ticket; b then writes its own on top of the stale copy, so a's
    edit is gone. hold: 'snapshot' a stays 0.6 s after its write (a watcher sees it), 'exit' a exits at once and b waits
    0.8 s (the exit snapshot sees it), 'none' a stays until b has written (no snapshot can see it)."""
    a = [{"op": "read_into", "path": CORE, "key": "base"}, {"op": "barrier", "name": "read", "parties": 2}, stale_write("a")]
    b = [{"op": "read_into", "path": CORE, "key": "base"}, {"op": "barrier", "name": "read", "parties": 2},
         {"op": "wait_signal", "name": "a_wrote"}]
    if hold == "snapshot":
        a += [{"op": "sleep", "s": 0.6}, {"op": "signal", "name": "a_wrote"}]
    elif hold == "exit":
        a += [{"op": "signal", "name": "a_wrote"}]
        b += [{"op": "sleep", "s": 0.8}]
    else:
        a += [{"op": "signal", "name": "a_wrote"}, {"op": "wait_signal", "name": "b_wrote"}]
    b += [stale_write("b")]
    if hold == "none":
        b += [{"op": "signal", "name": "b_wrote"}]
    # c gives up only after b has written (hold none), so that no exit snapshot of c falls between a's and b's writes
    c = [{"op": "wait_signal", "name": "b_wrote"}] + GIVE_UP if hold == "none" else GIVE_UP
    return {"a": a + [DONE("a")], "b": b + [DONE("b")], "c": c}


class SharedBase(S2Base):
    def argv(self, arms="B0", runs=1, **kw):
        kw.setdefault("snapshot_s", 0.05)
        a = super().argv(arms, runs, **kw)
        return a + ["--variant", "shared"]

    def records(self):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "onefile-shared")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json")]
        return sorted(out, key=lambda r: r["run_id"])

    def shared_run(self, agents=None, arms="B0", **kw):
        extra = dict(kw)
        if agents:
            path = os.path.join(self.tmp, "fake-sh-%d.json" % len(os.listdir(self.tmp)))
            with open(path, "w") as f:
                json.dump({"agents": agents}, f)
            extra["fake_script"] = path
        code, out, err = self.go(self.argv(arms, 1, **extra))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r, {c["id"]: c["pass"] for c in r["outcome_checks"]}

    def log_dir(self, r):
        return os.path.join(self.logs, r["run_id"])


class TestSharedTopology(SharedBase):
    def test_cooperating_agents_in_one_tree_score_three_of_three(self):
        r, ids = self.shared_run()  # the fixture's own shared fake script (variant shared)
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual((r["scenario"], r["topology"]), ("onefile-shared", "T-shared"))
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (3, 3))
        self.assertTrue(all(ids.values()), ids)
        self.assertEqual(r["metrics"]["overwrites"], 0)
        self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")
        self.assertTrue(r["scenario_metrics"]["correct"])
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b", "c"])

    def test_the_shared_script_carries_the_same_edits_as_the_clone_script(self):
        fake = read_json(os.path.join(BENCH, "fixtures", "onefile", "fake-shared.json"))["agents"]
        for a in "abc":
            self.assertEqual([s for s in fake[a] if s["op"] in ("replace", "append")], EDITS[a])

    def test_clone_runs_are_not_pooled_with_shared_runs(self):
        r, _ = self.shared_run()
        self.assertTrue(r["run_id"].startswith("t1-onefile-shared-"), r["run_id"])
        self.assertTrue(os.path.isdir(os.path.join(self.results, "t1", "onefile-shared", "B0")))
        self.assertFalse(os.path.exists(os.path.join(self.results, "t1", "onefile")))

    def test_every_agent_has_the_same_cwd_and_its_own_identity(self):
        dump = [{"op": "dump_env", "path": "env-%s.json"}]
        agents = {}
        for n in "abc":
            d = dict(dump[0], path="env-%s.json" % n)
            agents[n] = [d, DONE(n)]
        r, _ = self.shared_run(agents)
        facts = {n: read_json(os.path.join(self.log_dir(r), "env-%s.json" % n)) for n in "abc"}
        self.assertEqual(len({f["cwd"] for f in facts.values()}), 1, "one working directory")
        for n, f in facts.items():
            self.assertEqual(f["KLATCH_NAME"], n)
            self.assertEqual(f["identity"]["GIT_AUTHOR_NAME"], n)
            self.assertEqual(f["identity"]["GIT_COMMITTER_NAME"], n)
        self.assertEqual(len({f["HOME"] for f in facts.values()}), 3, "a home of its own per agent")
        self.assertEqual(len({f["identity"]["TMPDIR"] for f in facts.values()}), 3, "a tmp directory of its own per agent")
        cwd = facts["a"]["cwd"]
        for f in facts.values():
            self.assertFalse(f["HOME"].startswith(cwd + os.sep), "agent home is outside the shared tree")
            self.assertFalse(f["identity"]["TMPDIR"].startswith(cwd + os.sep))

    def test_harness_state_is_outside_the_shared_tree(self):
        agents = {n: [{"op": "dump_env", "path": "env-%s.json" % n}, DONE(n)] for n in "abc"}
        r, _ = self.shared_run(agents)
        for n in "abc":
            f = read_json(os.path.join(self.log_dir(r), "env-%s.json" % n))
            self.assertFalse(f["out_inside_cwd"], "the log directory is not in the tree")
            bad = [p for p in f["tree_files"] if re.search(r"stream|\.prompt|\.err$|sig-|check\.json|timeline|snapshot", p)]
            self.assertEqual(bad, [], "no harness file in the shared tree")
        # each agent's own stream, prompt and error log
        for n in "abc":
            for ext in ("stream.jsonl", "prompt.md", "err"):
                self.assertTrue(os.path.exists(os.path.join(self.log_dir(r), "%s.%s" % (n, ext))), (n, ext))

    def test_each_agents_commits_carry_its_own_name_in_the_shared_history(self):
        r, _ = self.shared_run()
        per = r["scenario_metrics"]["per_agent"]
        self.assertEqual(sorted(per), ["a", "b", "c"])
        for n in "abc":
            self.assertGreaterEqual(per[n]["commits"], 1, n)
            self.assertGreater(per[n]["surviving_lines"], 0, n)

    def test_the_shared_prompt_says_colleagues_edit_the_same_files_right_now(self):
        r, _ = self.shared_run()
        for n in "abc":
            with open(os.path.join(self.log_dir(r), "%s.prompt.md" % n)) as f:
                text = f.read()
            self.assertNotIn("your own clone", text)
            self.assertNotIn("git push", text)
            self.assertRegex(text, r"same directory")
            self.assertRegex(text, r"editing the same files")
            self.assertNotRegex(text, r"\{\{")

    def test_the_clone_prompt_is_unchanged(self):
        kw = dict(arms_dir=ARMS, arm="B0", name="a", team=["a", "b"], task="T", workdir="/w", test_command="t",
                  klatch_bin=None, room="r", max_turns=3, max_minutes=2)
        clone = run.build_prompt(**kw)
        self.assertEqual(clone, run.build_prompt(topology="T-clone", **kw))
        self.assertIn("your own clone", clone)
        shared_text = run.build_prompt(topology="T-shared", **kw)
        self.assertNotIn("your own clone", shared_text)
        self.assertIn("same directory", shared_text)

    def test_uncommitted_work_is_committed_by_the_harness_under_a_neutral_author(self):
        # a edits the file and never commits; b and c do nothing
        agents = {"a": [dict(s) for s in EDITS["a"]] + [DONE("a")], "b": GIVE_UP, "c": GIVE_UP}
        r, ids = self.shared_run(agents)
        sm = r["scenario_metrics"]
        self.assertEqual(sm["final_commit"]["made"], True)
        self.assertEqual(sm["final_commit"]["author"], shared.FINAL_AUTHOR)
        self.assertTrue(ids["timeout"], "a's uncommitted edit counts: the final state is the committed tree")
        self.assertIn(shared.FINAL_AUTHOR, sm["per_agent"])

    def test_nothing_to_commit_means_no_final_commit(self):
        r, _ = self.shared_run()
        self.assertEqual(r["scenario_metrics"]["final_commit"]["made"], False)

    def test_a_lost_update_is_found_from_snapshots_and_the_checker(self):
        r, ids = self.shared_run(lost_update_scripts("snapshot"))
        self.assertEqual(r["metrics"]["units_completed"], 1)
        self.assertTrue(ids["retries"])
        self.assertFalse(ids["timeout"])
        self.assertEqual(r["metrics"]["overwrites"], 1, r["notes"])
        sm = r["scenario_metrics"]
        self.assertEqual([(o["ticket"], o["author"], o["source"]) for o in sm["overwritten"]], [("A", "a", "snapshot")])
        self.assertEqual((sm["silent_overwrites"], sm["overwrites_after_conflict"]), (1, 0))
        self.assertEqual(sm["lost_edits"], ["A", "C"])
        self.assertGreaterEqual(sm["snapshots"]["distinct"], 3, "initial tree, a's write, b's write")
        self.assertEqual(sm["snapshots"]["interval_s"], 0.05)

    def test_the_exit_snapshot_finds_it_when_the_interval_is_long(self):
        r, _ = self.shared_run(lost_update_scripts("exit"), snapshot_s=3600)
        sm = r["scenario_metrics"]
        self.assertEqual([(o["ticket"], o["source"]) for o in sm["overwritten"]], [("A", "snapshot")], r["notes"])
        self.assertIn("exit:a", [s["reason"] for s in sm["snapshots"]["taken"]])

    def test_a_lost_update_between_two_snapshots_is_found_from_the_edit_events_only(self):
        r, ids = self.shared_run(lost_update_scripts("none"), snapshot_s=3600)
        sm = r["scenario_metrics"]
        self.assertFalse(ids["timeout"])
        # no snapshot ever held a's edit: the snapshot measure alone says nothing was lost
        self.assertEqual([o for o in sm["overwritten"] if o["source"] == "snapshot"], [])
        # the stream measure still sees a Write whose added lines are gone from the final file
        events = sm["lost_edit_events"]
        self.assertEqual([e["agent"] for e in events], ["a"])
        self.assertGreater(events[0]["lost_lines"], 0)
        self.assertEqual([(o["ticket"], o["author"], o["source"]) for o in sm["overwritten"]], [("A", "a", "stream")])
        self.assertEqual(r["metrics"]["overwrites"], 1)
        self.assertIn("resolution", sm["snapshots"]["limit"])

    def test_a_cooperating_run_has_no_lost_edit_events(self):
        r, _ = self.shared_run()
        self.assertEqual(r["scenario_metrics"]["lost_edit_events"], [])

    def test_a_self_revision_is_not_a_lost_colleague_edit_when_the_final_file_has_it(self):
        agents = {"a": [dict(s) for s in EDITS["a"]] + [
            {"op": "append", "path": CORE, "content": "\n# note\n"}, DONE("a")], "b": GIVE_UP, "c": GIVE_UP}
        r, _ = self.shared_run(agents)
        self.assertEqual(r["scenario_metrics"]["lost_edit_events"], [])

    def test_the_polls_see_the_uncommitted_shared_tree(self):
        """one agent makes all three edits, commits nothing and then lingers: the poll finds the pass long before the end"""
        edits = [dict(x) for t in "abc" for x in EDITS[t]]
        agents = {"a": edits + [{"op": "sleep", "s": 2.0}, DONE("a")], "b": GIVE_UP, "c": GIVE_UP}
        r, _ = self.shared_run(agents)
        self.assertEqual(r["metrics"]["units_completed"], 3)
        self.assertIsNotNone(r["time_to_first_pass_s"])
        self.assertLess(r["time_to_first_pass_s"], r["wall_s"] - 1.0)


@NEEDS_BIN
class TestSharedWithKlatch(SharedBase):
    def test_k2_members_in_one_tree_each_post_under_their_own_name(self):
        r, ids = self.shared_run(arms="K2")
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual(r["metrics"]["units_completed"], 3)
        self.assertGreaterEqual(r["metrics"]["messages"], 3)
        self.assertIsNotNone(r["artifacts"]["store_export"], r["notes"])


class TestFixtureAndScenario(unittest.TestCase):
    def test_the_variant_is_recorded_as_a_separate_scenario(self):
        fx = fixture_mod.Fixture("onefile", variant="shared")
        self.assertEqual((fx.name, fx.topology), ("onefile-shared", "T-shared"))
        self.assertEqual(os.path.basename(fx.fake_script), "fake-shared.json")
        plain = fixture_mod.Fixture("onefile")
        self.assertEqual((plain.name, plain.topology), ("onefile", "T-clone"))
        self.assertEqual(os.path.basename(plain.fake_script), "fake.json")

    def test_s2_json_says_t_shared_is_built_and_lists_the_variant(self):
        import scenario
        s = scenario.load("s2")
        self.assertEqual(s["topologies"]["T-shared"]["status"], "implemented")
        self.assertEqual(s["variants"]["shared"]["topology"], "T-shared")
        self.assertEqual(s["variants"]["shared"]["scenario_name"], "onefile-shared")
        self.assertEqual(s["shared_check"]["path"], "onefile/core.py")
        self.assertIn("resolution", s["shared_check"]["limit"])

    def test_a_shared_fixture_may_not_have_private_files(self):
        import tempfile
        import shutil
        d = tempfile.mkdtemp()
        try:
            os.makedirs(os.path.join(d, "x", "private", "a"))
            with open(os.path.join(d, "x", "private", "a", "f.txt"), "w") as f:
                f.write("secret")
            with open(os.path.join(d, "x", "t.md"), "w") as f:
                f.write("task")
            with open(os.path.join(d, "x", "fixture.json"), "w") as f:
                json.dump({"name": "x", "topology": "T-shared", "agents": [{"name": "a", "task": "t.md"}], "checker": "c",
                           "private": "private"}, f)
            with self.assertRaises(fixture_mod.FixtureError):
                fixture_mod.Fixture("x", d)
        finally:
            shutil.rmtree(d)


class TestSnapshots(unittest.TestCase):
    def test_edit_events_count_added_lines_missing_from_the_final_file(self):
        base = "x = 1\n"
        final = "x = 1\ny = 2\n"
        lines = [json.dumps({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Write", "input": {"file_path": "/t/onefile/core.py",
                                                            "content": "x = 1\nz = 3\n"}},
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "/t/onefile/core.py", "old_string": "x = 1",
                                                           "new_string": "x = 1\ny = 2"}},
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "/t/other.py", "old_string": "q",
                                                           "new_string": "r"}}]}})]
        ev = shared.lost_edit_events({"a": lines}, "onefile/core.py", base, final)
        self.assertEqual([(e["agent"], e["tool"], e["lost_lines"]) for e in ev], [("a", "Write", 1)])

    def test_the_snapshot_list_keeps_only_changes(self):
        s = shared.Snapshots()
        s.add(0.0, "start", "a")
        s.add(1.0, "tick", "a")
        s.add(2.0, "tick", "b")
        s.add(3.0, "tick", None)
        self.assertEqual([x["text"] for x in s.distinct], ["a", "b"])
        self.assertEqual(s.summary(5.0)["taken"][0]["reason"], "start")


if __name__ == "__main__":
    unittest.main()
