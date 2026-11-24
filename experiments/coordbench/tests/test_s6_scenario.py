"""Scenario S6 (#1371): the scenario file, the kill option and the recovery metrics of fixture F6 `recovery` through the whole
harness with fake agents. The victim c is killed mid-task (SIGKILL to its process group); the survivors notice and recover,
never notice, take over a live agent's module (a false alarm), or recover twice. No model, no spend."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import recovery
import scenario
import schema
from test_recovery_checker import FACTS, MSB_RANGES
from test_run_fake import Base, NEEDS_BIN, read_json

FIXTURE = os.path.join(BENCH, "fixtures", "recovery")
REF = os.path.join(FIXTURE, "reference", "recoverkit")


def cp(name):
    return {"op": "copy_from", "from": os.path.join(REF, name + ".py"), "path": "recoverkit/%s.py" % name}


SYNC = lambda msg: {"op": "sync", "message": msg, "resolve": "union"}  # noqa: E731
KILLED = {"op": "wait_file", "path": "kill.json", "timeout": 40}
PULL = {"op": "git", "args": ["pull", "--no-rebase", "--no-edit", "origin", "main"]}
HALF = {"op": "write", "path": "recoverkit/bits.py", "content": "def set_bit(mask, index):\n    return mask | (0x8000 >> index)\n"}
ASK = {"op": "hook", "text": "[klatch] a: seeking: what is the index map of a mask?", "arms": ["B1", "K2"]}
BOARD = [{"op": "append", "path": "TASKS.md", "content": "- c | working | recoverkit/bits.py | bits\n", "arms": ["B1"]},
         {"op": "sync", "message": "c: working", "arms": ["B1"]}]


def victim(*extra, pre=()):
    return list(pre) + BOARD + [ASK] + list(extra) + [HALF, {"op": "hang"}]


A_OK = [KILLED, PULL, cp("ranges"), SYNC("ranges"), {"op": "final", "text": "DONE a"}]
B_BASE = [cp("wrap"), cp("pairs"), SYNC("wrap and pairs")]
MAP = "position = (index * %d + %d) mod 16" % (FACTS["K"], FACTS["R"])
ANSWER = {"op": "send", "to": "a", "text": "answer: " + MAP}
B_RECOVER = B_BASE + [KILLED, cp("bits"), SYNC("bits, taken over"), ANSWER, {"op": "final", "text": "DONE b"}]
GUESS = {"op": "write", "path": "recoverkit/ranges.py", "content": MSB_RANGES}  # what a guesses, or reads in docs/FORMATS.md


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s6")

    def test_arms_n_and_caps_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["id"], "S6")
        self.assertEqual(s["fixture"], "recovery")
        self.assertEqual(s["arms"], ["B0", "B1", "K2", "K3", "B1-roles"])
        self.assertEqual(s["arms_at_m0"], ["B0", "B1", "K2"])
        self.assertEqual(s["n_per_arm"]["m0"], 10)
        self.assertEqual(s["n_per_arm"]["quick"], 3)
        self.assertEqual(s["caps"]["per_run_usd"], 2.4)
        self.assertIn("1421", s["arm_notes"]["K3"])

    def test_the_agents_match_the_fixture_and_the_scenario_is_found_from_it(self):
        fx = read_json(os.path.join(FIXTURE, "fixture.json"))
        self.assertEqual(fx["scenario"], "s6")
        self.assertEqual([a["name"] for a in fx["agents"]], sorted(self.scn["agents"]))
        self.assertEqual(self.scn["kill_agent"]["agent"], "c")
        self.assertTrue(self.scn["agents"]["c"]["victim"])
        self.assertEqual(self.scn["stream_metrics"], "recovery")

    def test_every_metric_is_defined_precisely(self):
        d = self.scn["metrics"]["definitions"]
        for k in ("recovered", "units_completed", "kill", "time_to_first_survivor_action_s", "first_notice", "how_noticed",
                  "time_to_module_recovery_s", "time_to_ask_answer_s", "false_alarms", "duplicated_recovery", "merge_state"):
            self.assertIn(k, d, k)
        for word in ("SIGKILL", "process group", "fallback_turns", "no_commit", "board_working", "ask_open", "claimed"):
            self.assertIn(word, d["kill"])
        for marker in ("dead_holder", "stale_ask", "claim_taken_after_lapse", "lapsed without an answer", "1182", "1386"):
            self.assertIn(marker, d["first_notice"])
        self.assertIn("never the agents' words", d["how_noticed"])

    def test_the_gaps_are_stated(self):
        g = self.scn["gaps"]
        self.assertIn("1421", g["k3_supervisor"])
        self.assertIn("1314", g["sigkill_and_hooks"])
        self.assertIn("store", g["store_events"])

    def test_the_kill_conditions_per_variant(self):
        default = recovery.kill_spec(self.scn)
        claim = recovery.kill_spec(self.scn, "claim")
        ask = recovery.kill_spec(self.scn, "ask")
        kinds = lambda spec: [c["kind"] for c in spec["after"]]  # noqa: E731
        self.assertEqual(kinds(default), ["edited", "no_commit"], "arm-neutral: nothing only some arms can do (#1525)")
        self.assertEqual(kinds(claim), ["edited", "no_commit"])
        self.assertEqual(kinds(ask), ["turns"])
        for spec in (default, claim, ask):
            self.assertFalse([c for c in spec["after"] if "arms" in c], spec)
        self.assertEqual(recovery.kill_spec({"id": "x"}), None)

    def test_the_reference_checker_names_the_victim_the_scenario_names(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("rc", os.path.join(BENCH, "checks", "recovery", "check.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        self.assertEqual(mod.VICTIM, self.scn["kill_agent"]["agent"])
        owner = {u: a for a, v in self.scn["agents"].items() for u in v["units"]}
        self.assertEqual(mod.OWNER, owner)


class S6Base(Base):
    def argv(self, arms="B0", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "recovery"
        return a

    def records(self):
        out = []
        for d, _, names in os.walk(self.results):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json") and n != "suite.json"]
        return sorted(out, key=lambda r: r["run_id"])

    def one(self, agents=None, arm="B0", variant=None):
        extra = {}
        if agents:
            path = os.path.join(self.tmp, "fake-s6-%d.json" % len(os.listdir(self.tmp)))
            with open(path, "w") as f:
                json.dump({"agents": agents}, f)
            extra["fake_script"] = path
        if variant:
            extra["variant"] = variant
        code, out, err = self.go(self.argv(arm, 1, **extra))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        return r, r["scenario_metrics"]

    def log_dir(self, rec):
        return os.path.join(self.logs, rec["run_id"])


class TestReferenceRecovery(S6Base):
    def test_b0_the_victim_is_killed_a_survivor_recovers_and_everything_is_scored(self):
        r, sm = self.one(arm="B0")
        self.assertTrue(r["valid"], r["notes"])
        self.assertEqual(r["outcome"], "ok", "a SIGKILLed victim is not a crash")
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (4, 4))
        self.assertTrue(sm["correct"] and sm["recovered"] and sm["ask_answered"])
        self.assertEqual(sm["recovered_by"], ["b"])
        k = sm["kill"]
        self.assertTrue(k["fired"])
        self.assertEqual((k["agent"], k["trigger"]), ("c", "conditions"))
        self.assertTrue(k["conditions"]["edited"] and k["conditions"]["no_commit"])
        self.assertNotIn("claimed", k["conditions"], "a K-only condition does not apply to B0")
        self.assertNotIn("board_working", k["conditions"])
        self.assertEqual(sm["victim"], "c")
        self.assertEqual(sm["victim_unit"], "bits")

    def test_b0_the_first_survivor_action_and_the_time_to_recovery(self):
        r, sm = self.one(arm="B0")
        fa = sm["first_survivor_action"]
        self.assertEqual((fa["agent"], fa["kind"]), ("b", "edit"))
        self.assertIsNotNone(sm["time_to_first_survivor_action_s"])
        self.assertGreaterEqual(sm["time_to_first_survivor_action_s"], 0)
        self.assertGreaterEqual(sm["survivor_turns_to_first_action"], 1)
        self.assertIsNotNone(sm["time_to_module_recovery_s"])
        self.assertGreaterEqual(sm["time_to_module_recovery_s"], 0)
        self.assertIsNone(sm["time_to_ask_answer_s"], "B0 has no way to answer")
        self.assertIsNone(sm["noticed_by"], "the markers are klatch's: K arms only")
        self.assertIsNone(sm["how_noticed"])

    def test_b0_no_false_alarm_no_duplicate_work(self):
        r, sm = self.one(arm="B0")
        self.assertEqual(sm["false_alarms"], 0)
        self.assertEqual(r["metrics"]["duplicate_work"], 0)
        self.assertEqual(sm["duplicated_recovery"], 0)
        self.assertFalse(sm["redone_after_victim_commit"])
        self.assertEqual(sm["victim_module_authors"], ["b"])
        self.assertEqual(sm["duplicates"], [])

    def test_the_kill_and_the_timeline_are_kept_with_the_streams(self):
        r, sm = self.one(arm="B0")
        d = self.log_dir(r)
        kill = read_json(os.path.join(d, "kill.json"))
        self.assertTrue(kill["fired"])
        self.assertEqual(kill["agent"], "c")
        self.assertEqual(set(kill["lines"]), {"a", "b", "c"})
        self.assertGreater(kill["lines"]["c"], 0)
        tl = read_json(os.path.join(d, "timeline.json"))
        for name in ("a", "b", "c"):
            with open(os.path.join(d, name + ".stream.jsonl")) as f:
                n = len(f.read().splitlines())
            self.assertEqual(len(tl[name]), n, name)
        self.assertEqual(tl["c"], sorted(tl["c"]))

    def test_the_victim_wrote_no_result_message_and_ran_no_hook(self):
        r, _ = self.one(arm="B0")
        with open(os.path.join(self.log_dir(r), "c.stream.jsonl")) as f:
            types = [json.loads(l).get("type") for l in f if l.strip()]
        self.assertNotIn("result", types, "a SIGKILLed process says nothing at the end")

    def test_b1_the_board_line_is_stale_and_the_message_answers_the_ask(self):
        r, sm = self.one(arm="B1")
        self.assertTrue(r["valid"], r["notes"])
        self.assertTrue(sm["correct"], sm)
        k = sm["kill"]
        self.assertEqual(k["trigger"], "conditions")
        self.assertEqual(sorted(k["conditions"]), ["edited", "no_commit"])
        for c in ("board_working", "ask_open"):
            self.assertTrue(k["observed"][c], c)
        self.assertFalse(k["observed"]["claim_held"])
        self.assertIsNotNone(sm["time_to_ask_answer_s"])
        self.assertEqual(sm["first_survivor_action"]["kind"] in ("edit", "answer"), True)
        self.assertEqual(sm["false_alarms"], 0)

    @NEEDS_BIN
    def test_k2_the_victim_holds_a_real_claim_when_it_is_killed(self):
        r, sm = self.one(arm="K2")
        self.assertTrue(r["valid"], r["notes"])
        k = sm["kill"]
        self.assertEqual(k["trigger"], "conditions")
        self.assertEqual(sorted(k["conditions"]), ["edited", "no_commit"])
        for c in ("claim_held", "ask_open"):
            self.assertTrue(k["observed"][c], c)
        self.assertTrue(sm["correct"] and sm["recovered"], sm)
        self.assertIsInstance(sm["noticed_by"], list)
        self.assertIn("claim_ops", dir(recovery))


class TestFailureModes(S6Base):
    def test_survivors_that_never_notice(self):
        a = [KILLED, GUESS, SYNC("ranges"), {"op": "final", "text": "DONE a"}]
        b = B_BASE + [KILLED, {"op": "final", "text": "DONE b"}]
        r, sm = self.one({"a": a, "b": b, "c": victim()})
        self.assertTrue(r["valid"], r["notes"])
        self.assertTrue(sm["kill"]["fired"])
        self.assertFalse(sm["recovered"])
        self.assertFalse(sm["ask_answered"])
        self.assertFalse(sm["correct"])
        self.assertEqual(r["metrics"]["units_completed"], 2, "wrap and pairs only")
        self.assertIsNone(sm["first_survivor_action"])
        self.assertIsNone(sm["time_to_first_survivor_action_s"])
        self.assertIsNone(sm["time_to_module_recovery_s"])
        self.assertIsNone(sm["time_to_ask_answer_s"])
        self.assertIsNone(sm["recovery_commit"])
        self.assertEqual(sm["false_alarms"], 0)

    def test_recovering_the_module_without_answering_leaves_the_ask_unanswered(self):
        # bits is recoverable from the repository alone; the map is not, so a's question stays orphaned
        b = B_BASE + [KILLED, cp("bits"), SYNC("bits, taken over"), {"op": "final", "text": "DONE b"}]
        a = [KILLED, PULL, GUESS, SYNC("ranges, guessed"), {"op": "final", "text": "DONE a"}]
        r, sm = self.one({"a": a, "b": b, "c": victim()})
        self.assertTrue(sm["recovered"], "the module needed only the repository")
        self.assertFalse(sm["ask_answered"], "the map was never answered, a guessed")
        self.assertFalse(sm["correct"])
        self.assertEqual(r["metrics"]["units_completed"], 3, "bits, wrap, pairs; ranges used the wrong map")
        self.assertIsNone(sm["time_to_ask_answer_s"])

    def test_a_victim_that_answered_before_the_kill_needs_no_recovery_of_the_ask(self):
        # variant claim: the trigger does not wait for the ask; c answers a first, then claims, edits and is killed
        c = BOARD + [ASK, {"op": "send", "to": "a", "text": "answer: " + MAP}, HALF, {"op": "hang"}]
        a = [KILLED, PULL, cp("ranges"), SYNC("ranges, with the map c sent"), {"op": "final", "text": "DONE a"}]
        b = B_BASE + [KILLED, cp("bits"), SYNC("bits, taken over"), {"op": "final", "text": "DONE b"}]
        r, sm = self.one({"a": a, "b": b, "c": c}, arm="B1", variant="claim")
        self.assertNotIn("ask_open", sm["kill"]["conditions"])
        self.assertFalse(sm["kill"]["observed"]["ask_open"], "c had answered before the kill")
        self.assertTrue(sm["recovered"])
        self.assertTrue(sm["ask_answered"])
        self.assertTrue(sm["correct"], sm)
        self.assertIsNone(sm["time_to_ask_answer_s"], "no survivor had to answer")

    def test_the_map_is_private_to_c_and_its_deputy_b_and_never_committed(self):
        sh = lambda cmd: {"op": "run", "argv": ["sh", "-c", cmd]}  # noqa: E731
        clean = sh('test -z "$(git status --porcelain)"')
        a = [sh("test ! -e SPEC-INDEXMAP.md && test ! -e SPEC-INDEXMAP-COPY.md"), KILLED, {"op": "final", "text": "DONE a"}]
        b = [{"op": "read", "path": "SPEC-INDEXMAP-COPY.md"}, sh("test ! -e SPEC-INDEXMAP.md"), clean, KILLED,
             {"op": "final", "text": "DONE b"}]
        c = [{"op": "read", "path": "SPEC-INDEXMAP.md"}, clean, sh("test ! -e SPEC-INDEXMAP-COPY.md"), HALF, {"op": "hang"}]
        r, sm = self.one({"a": a, "b": b, "c": c})
        self.assertTrue(sm["kill"]["fired"], r["notes"])
        self.assertNotEqual(r["outcome"], "crash", r["notes"])

    def test_a_survivor_that_takes_over_a_live_agents_module_is_a_false_alarm(self):
        b = [cp("ranges"), SYNC("ranges, which is a's")] + B_RECOVER
        r, sm = self.one({"a": A_OK, "b": b, "c": victim()})
        self.assertEqual(sm["false_alarms"], 1, sm["false_alarm_detail"])
        d = sm["false_alarm_detail"][0]
        self.assertEqual((d["agent"], d["unit"], d["owner"], d["victim_unit"]), ("b", "ranges", "a", False))
        self.assertIn("ranges", [x["module"] for x in sm["duplicates"]])
        self.assertTrue(sm["recovered"], "the recovery itself still happened")
        self.assertFalse(sm["correct"], "foreign work is conflicting work")

    def test_touching_the_victims_module_before_the_kill_is_a_false_alarm(self):
        early = {"op": "write", "path": "recoverkit/bits.py", "content": "x = 1\n"}
        b = [early] + B_RECOVER
        slow = victim(pre=[{"op": "sleep", "s": 1.5}])
        r, sm = self.one({"a": A_OK, "b": b, "c": slow})
        self.assertTrue(sm["kill"]["fired"])
        alarms = [d for d in sm["false_alarm_detail"] if d["victim_unit"]]
        self.assertEqual(len(alarms), 1, sm["false_alarm_detail"])
        self.assertEqual((alarms[0]["agent"], alarms[0]["unit"]), ("b", "bits"))

    def test_taking_over_the_victims_module_after_the_kill_is_not_a_false_alarm(self):
        r, sm = self.one({"a": A_OK, "b": B_RECOVER, "c": victim()})
        self.assertEqual(sm["false_alarms"], 0)

    def test_duplicated_recovery(self):
        a = [KILLED, PULL, cp("bits"), {"op": "append", "path": "recoverkit/bits.py", "content": "# a too\n"}, SYNC("bits as well"),
             cp("ranges"), SYNC("ranges"), {"op": "final", "text": "DONE a"}]
        r, sm = self.one({"a": a, "b": B_RECOVER, "c": victim()})
        self.assertEqual(sm["duplicated_recovery"], 1)
        self.assertEqual(sorted(sm["victim_module_authors"]), ["a", "b"])
        self.assertGreaterEqual(r["metrics"]["duplicate_work"], 1)
        self.assertFalse(sm["correct"])
        self.assertIn("bits", [x["module"] for x in sm["duplicates"]])

    def test_the_victim_exits_before_the_trigger_so_no_kill_fires(self):
        quiet = [{"op": "exit", "code": 0}]
        a = [{"op": "final", "text": "DONE a"}]
        b = [{"op": "final", "text": "DONE b"}]
        r, sm = self.one({"a": a, "b": b, "c": quiet})
        self.assertFalse(sm["kill"]["fired"])
        self.assertTrue(r["valid"], "SPEC 6.4: a run is never invalid because of what the agents did (#1525)")
        self.assertIn("kill", r["notes"])
        self.assertEqual(r["outcome"], "trigger-not-fired")

    def test_the_fallback_kills_a_victim_that_never_meets_the_conditions(self):
        c = [{"op": "poll", "n": 30, "command": "ls"}, {"op": "hang"}]
        a = [KILLED, {"op": "final", "text": "DONE a"}]
        b = [KILLED, {"op": "final", "text": "DONE b"}]
        r, sm = self.one({"a": a, "b": b, "c": c})
        self.assertTrue(sm["kill"]["fired"])
        self.assertEqual(sm["kill"]["trigger"], "fallback")
        self.assertGreaterEqual(sm["victim_turns_at_kill"], 14)
        self.assertTrue(r["valid"])


class TestVariants(S6Base):
    def test_variant_claim_has_the_arm_neutral_trigger_and_is_never_pooled(self):
        r, sm = self.one(arm="B1", variant="claim")
        self.assertEqual(r["scenario"], "recovery-claim")
        self.assertEqual(sorted(sm["kill"]["conditions"]), ["edited", "no_commit"])
        self.assertEqual(sm["kill"]["trigger"], "conditions")
        self.assertTrue(sm["kill"]["observed"]["board_working"])

    def test_variant_ask_kills_at_the_same_turn_count_in_every_arm_and_records_whether_the_question_arrived(self):
        c = [ASK, {"op": "poll", "n": 8, "command": "ls"}, {"op": "hang"}]
        r, sm = self.one({"a": [KILLED, {"op": "final", "text": "DONE a"}], "b": [KILLED, {"op": "final", "text": "DONE b"}],
                          "c": c}, arm="B1", variant="ask")
        self.assertEqual(r["scenario"], "recovery-ask")
        self.assertEqual(sm["kill"]["trigger"], "conditions")
        self.assertEqual(list(sm["kill"]["conditions"]), ["turns"])
        self.assertTrue(sm["kill"]["observed"]["ask_open"])

    def test_variant_ask_on_b0_is_the_same_trigger(self):
        c = [{"op": "poll", "n": 8, "command": "ls"}, {"op": "hang"}]
        r, sm = self.one({"a": [KILLED, {"op": "final", "text": "DONE a"}], "b": [KILLED, {"op": "final", "text": "DONE b"}],
                          "c": c}, arm="B0", variant="ask")
        self.assertEqual(sm["kill"]["trigger"], "conditions")
        self.assertEqual(list(sm["kill"]["conditions"]), ["turns"])
        self.assertFalse(sm["kill"]["observed"]["ask_open"], "B0 cannot be asked, recorded not required")


if __name__ == "__main__":
    unittest.main()
