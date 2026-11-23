"""Scenario S4 (#1369): the scenario file, and fixture F4 `specialists` through the whole harness with fake agents that ask and
get answers, never ask (and guess), ask late after the helper has stopped, repeat a seeded ask, duplicate an answer and
answer with a stale value. Asserts the checker's units and the ask metrics in scenario_metrics. No model, no spend. The B1
cases need no binary; the K-arm seeding test needs KLATCH_BIN."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import scenario
import schema
from test_run_fake import Base, NEEDS_BIN, read_json

FIX = os.path.join(BENCH, "fixtures", "specialists")
REF = os.path.join(FIX, "reference", "linkkit")
with open(os.path.join(BENCH, "checks", "specialists", "facts.json")) as _f:
    FACTS = json.load(_f)["facts"]
V = {k: v["value"] for k, v in FACTS.items()}


def ask(to, text):
    return {"op": "send", "to": to, "text": "seeking: " + text}


def answer(to, key):
    return {"op": "send", "to": to, "text": "answer: %s is %s" % (key, V[key])}


def ref(mod, replace=None):
    st = {"op": "copy_from", "from": os.path.join(REF, mod + ".py"), "path": "linkkit/%s.py" % mod}
    if replace:
        st["replace"] = replace
    return st


def sync(agent):
    return {"op": "sync", "message": "linkkit %s" % agent, "resolve": "union"}


def done(agent):
    return {"op": "final", "text": "DONE " + agent}


def good(late="avoid", c_repeat=False, dup=False, stale=False, guess=False):
    """Fake scripts for a, b and c on a timeline: asks at the start, answers at 0.5 s, c done early, b a little later, a's
    second phase after b has published. late: avoid (a needs no ask), ask (a asks the finished c, nobody answers, retry.py is
    not written), none."""
    wrong = lambda mod, key, bad: [[("%d" % V[key]) if isinstance(V[key], int) else repr(V[key]), bad]]
    a = [ask("b", "TOPIC_PREFIX in the routing spec?"), ask("c", "CHECKSUM_SEED in the ledger spec?"), {"op": "sleep", "s": 0.4},
         answer("b", "FRAME_MAGIC"), {"op": "sleep", "s": 0.3}]
    a += [ref("framing", wrong("framing", "TOPIC_PREFIX", "'topic'") + wrong("framing", "CHECKSUM_SEED", "5381")) if guess else ref("framing")]
    a += [sync("a"), {"op": "wait_origin_has", "path": "linkkit/routing.py", "contains": "frame_tag", "timeout": 20}]
    if late == "ask":
        a += [{"op": "sync", "message": "pull", "resolve": "union"}, ask("c", "LEDGER_MODULUS again, c? (late)"), {"op": "sleep", "s": 0.5}]
    elif late == "avoid":
        a += [ref("retry"), sync("a")]
    a += [done("a")]
    b = [ask("c", "LEDGER_MODULUS in the ledger spec?"), ask("a", "FRAME_MAGIC in the framing spec?"), {"op": "sleep", "s": 0.5},
         answer("a", "TOPIC_PREFIX"), answer("c", "PARTITIONS"), {"op": "sleep", "s": 1.2}, ref("routing"), sync("b"), done("b")]
    c = [ask("b", "PARTITIONS in the routing spec?")]
    if c_repeat:
        c += [ask("a", "MAX_PAYLOAD in the framing spec?")]
    c += [{"op": "sleep", "s": 0.5}, answer("b", "LEDGER_MODULUS"), answer("a", "CHECKSUM_SEED")]
    if dup:
        c += [answer("a", "CHECKSUM_SEED")]
    if stale:
        c += [{"op": "send", "to": "a", "text": "answer: CHECKSUM_SEED was %d" % FACTS["CHECKSUM_SEED"]["stale"]}]
    c += [ref("ledger"), sync("c"), done("c")]
    return {"a": a, "b": b, "c": c}


class TestScenarioFile(unittest.TestCase):
    def setUp(self):
        self.scn = scenario.load("s4")

    def test_arms_n_caps_and_metrics_come_from_the_spec(self):
        s = self.scn
        self.assertEqual(s["fixture"], "specialists")
        self.assertEqual(s["arms"], ["B1", "K1", "K2"])
        self.assertEqual(s["arms_at_m0"], ["B1", "K1", "K2"])
        self.assertIn("B0", s["arm_notes"])
        self.assertEqual((s["n_per_arm"]["m0"], s["n_per_arm"]["quick"]), (10, 3))
        self.assertEqual(s["caps"]["per_run_usd"], 1.8)
        self.assertEqual(s["caps"]["suite_usd_m0_haiku"], 22.5)
        for k in ("asks_made", "asks_answered", "asks_lost", "time_to_first_answer_s", "duplicate_answers", "repeat_asks",
                  "repeat_asks_avoided", "wrong_guesses", "need_info_events"):
            self.assertIn(k, s["metrics"]["secondary"])
            self.assertIn(k, s["metrics"]["definitions"])
        self.assertIn("asks_from_store", s["metrics"]["definitions"])

    def test_slots_facts_and_holders_agree_with_the_fixture_and_the_checker(self):
        s = self.scn
        self.assertEqual(sorted(s["facts"]), sorted(FACTS))
        for k, f in s["facts"].items():
            self.assertEqual(f["holder"], FACTS[k]["holder"])
        for slot, sp in s["slots"].items():
            self.assertNotEqual(s["facts"][sp["fact"]]["holder"], sp["agent"], "every need is a foreign fact: " + slot)
        self.assertEqual(sorted(sp["kind"] for sp in s["slots"].values() if "kind" in sp), ["late", "seeded-repeat"])
        for u in s["units"].values():
            self.assertTrue(os.path.exists(os.path.join(REF, os.path.basename(u["path"]))))
        with open(os.path.join(FIX, "fixture.json")) as f:
            fx = json.load(f)
        self.assertEqual(fx["scenario"], "s4")
        self.assertEqual([a["name"] for a in fx["agents"]], sorted(s["agents"]))
        self.assertEqual(s["seeded_answers"], ["MAX_PAYLOAD"])

    def test_every_agent_task_names_its_holders_and_every_needed_fact(self):
        for agent in "abc":
            text = open(os.path.join(FIX, "task-%s.md" % agent)).read()
            need = {sp["fact"] for sp in self.scn["slots"].values() if sp["agent"] == agent}
            for fact in need:
                self.assertIn(fact, text)
            for fact in V:
                if fact not in need:
                    self.assertNotIn(fact, text, "a task must not name a fact the agent does not need: %s %s" % (agent, fact))


class S4Base(Base):
    def argv(self, arms="B1", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = "specialists"
        a[a.index("--wall-s") + 1] = "120"
        return a

    def records(self, arm=None):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1", "specialists")):
            out += [read_json(os.path.join(d, n)) for n in names if n.endswith(".json") and (arm is None or os.path.basename(d) == arm)]
        return sorted(out, key=lambda r: r["run_id"])

    def one(self, agents, arm="B1"):
        path = os.path.join(self.tmp, "fake-s4-%d.json" % len(os.listdir(self.tmp)))
        with open(path, "w") as f:
            json.dump({"agents": agents}, f)
        code, out, err = self.go(self.argv(arm, 1, fake_script=path))
        recs = self.records(arm)
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["notes"])
        return r, r["scenario_metrics"]["asks"]


class TestAsksThroughTheHarness(S4Base):
    def test_agents_that_ask_and_get_answers_score_seven_of_seven_and_avoid_both_repeats(self):
        r, m = self.one(good())
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (7, 7))
        self.assertEqual((m["asks_made"], m["asks_answered"], m["asks_lost"]), (5, 5, 0))
        self.assertEqual((m["wrong_guesses"], m["unanswered_dependencies"], m["duplicate_answers"]), (0, 0, 0))
        self.assertEqual((m["repeat_opportunities"], m["repeat_asks_avoided"], m["repeat_asks"]), (2, 2, 0))
        self.assertGreater(m["time_to_first_answer_s"], 0.2)
        self.assertEqual(m["late_asks"], 0)
        self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")
        self.assertEqual(r["metrics"]["duplicate_work"], 0)

    def test_agents_that_never_ask_and_guess_are_counted_as_wrong_without_an_ask(self):
        agents = good(late="none", guess=True)
        agents["a"] = [s for s in agents["a"] if s["op"] != "send"]
        r, m = self.one(agents)
        self.assertEqual(m["asks_made"], 3)  # only b (two asks) and c ask
        self.assertEqual(r["scenario_metrics"]["slots"]["a.topic_name"]["state"], "wrong")
        self.assertEqual(r["scenario_metrics"]["slots"]["a.topic_name"]["value_used"], "topic")
        self.assertEqual(r["scenario_metrics"]["slots"]["a.payload_digest"]["value_used"], 5381)
        self.assertEqual((m["wrong_guesses"], m["wrong_without_ask"]), (2, 2))
        self.assertEqual(r["scenario_metrics"]["plausible_default_guesses"], 2)
        self.assertEqual(r["metrics"]["units_completed"], 4, "two guessed, retry.py not written")
        self.assertEqual(m["unanswered_dependencies"], 1)

    def test_a_late_ask_to_a_helper_that_has_stopped_is_lost_and_costs_a_unit(self):
        r, m = self.one(good(late="ask"))
        self.assertEqual((m["asks_made"], m["asks_answered"], m["asks_lost"]), (6, 5, 1))
        self.assertEqual((m["late_asks"], m["late_asks_lost"], m["late_asks_answered"]), (1, 1, 0))
        self.assertEqual(r["metrics"]["units_completed"], 6)
        self.assertEqual(r["scenario_metrics"]["slots"]["a.retry_slot"]["state"], "unanswered")
        self.assertEqual(m["repeat_asks"], 1, "the late ask repeats b's earlier answered ask")
        self.assertEqual(m["repeat_asks_avoided"], 1, "only the seeded need was met without an ask")

    def test_a_repeat_of_the_seeded_ask_is_counted_and_not_avoided(self):
        r, m = self.one(good(c_repeat=True))
        self.assertEqual(m["repeat_asks"], 1)
        self.assertEqual(m["repeat_asks_avoided"], 1)  # only the late need
        self.assertEqual(m["asks_made"], 6)
        self.assertEqual(m["asks_lost"], 1, "nobody answered c's MAX_PAYLOAD ask in this script")

    def test_a_duplicate_answer_and_a_stale_answer_are_counted(self):
        r, m = self.one(good(dup=True, stale=True))
        self.assertEqual(m["duplicate_answers"], 2, "a's CHECKSUM_SEED ask got the same answer twice and a stale one")
        self.assertEqual(m["answers_stale"], 1)

    def test_a_fixture_agent_cannot_see_another_agents_private_file_or_commit_its_own(self):
        specs = {}
        agents = {n: [{"op": "dump_specs", "path": "specs-%s.json" % n}] + good()[n] for n in "abc"}
        self.one(agents)
        for n in "abc":
            p = next(p for d, _, fs in os.walk(self.logs) for f in fs for p in [os.path.join(d, f)] if f == "specs-%s.json" % n)
            specs[n] = read_json(p)
        self.assertEqual(specs["a"]["files"], ["SPEC-FRAMING.md"])
        self.assertEqual(specs["b"]["files"], ["SPEC-ROUTING.md"])
        self.assertEqual(specs["c"]["files"], ["SPEC-LEDGER.md"])
        for n in "abc":
            self.assertEqual(specs[n]["tracked"], [], "a private file is untracked and excluded from git")
            self.assertEqual(specs[n]["status"], [], "and git status does not show it")

    def test_b1_board_carries_the_seeded_answer_and_it_is_not_counted_as_an_ask(self):
        agents = {n: [{"op": "dump_board", "path": "board-%s.txt" % n}] + good()[n] for n in "abc"}
        r, m = self.one(agents)
        board = next(open(os.path.join(d, f)).read() for d, _, fs in os.walk(self.logs) for f in fs if f == "board-c.txt")
        self.assertIn("MAX_PAYLOAD is %d bytes" % V["MAX_PAYLOAD"], board)
        self.assertEqual(m["asks_made"], 5)

    def test_the_run_is_priced_from_the_fixture_estimate(self):
        code, out, err = self.go(self.argv("B1,K1,K2", 10) + ["--dry-run"])
        self.assertEqual(code, 0, err)
        self.assertIn("3 agents per run", out)
        self.assertIn("30 runs", out)


@NEEDS_BIN
class TestSeededRoom(S4Base):
    def test_k_arms_find_the_seeded_exchange_in_the_room_before_any_agent_acts(self):
        read = {"op": "klatch", "args": ["read", "{{ROOM}}"], "allow_fail": True}
        agents = {n: [{"op": "klatch", "args": ["join", "{{ROOM}}"]}, read, {"op": "final", "text": "DONE " + n}] for n in "abc"}
        r, m = self.one(agents, arm="K1")
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual(m["asks_made"], 0, "the seeded ask is history, not an ask of this run")
        self.assertEqual(m["repeat_opportunities"], 1)
        seen = [open(os.path.join(d, f)).read() for d, _, fs in os.walk(self.logs) for f in fs if f == "c.stream.jsonl"]
        self.assertIn("MAX_PAYLOAD is %d bytes" % V["MAX_PAYLOAD"], seen[0], "the room history has the seeded answer")
