"""The S4 ask and answer metrics (#1369) from message events, with synthetic events: asks made and answered, never asking,
a late ask that a finished helper never answers, a repeat ask, duplicate answers, stale answers, need-info."""
import json
import unittest

from helpers import BENCH  # noqa: F401
import asks
import scenario

SCN = scenario.load("s4")
with open(__import__("os").path.join(BENCH, "checks", "specialists", "facts.json")) as _f:
    FACTS = json.load(_f)["facts"]
V = {k: v["value"] for k, v in FACTS.items()}
ST = {k: v["stale"] for k, v in FACTS.items()}


def ev(agent, t, text, verb="post", kind="klatch"):
    return {"agent": agent, "t": t, "kind": kind, "verb": verb, "text": text}


def slots(**states):
    base = {s: {"state": "correct"} for s in SCN["slots"]}
    for k, v in states.items():
        base[k.replace("__", ".")] = {"state": v}
    return base


def run(events, done=None, **st):
    return asks.compute(events, done or {}, SCN, FACTS, slots(**st))


class TestExtract(unittest.TestCase):
    def test_send_message_and_klatch_commands_become_events(self):
        out = asks.extract("SendMessage", {"to": "c", "message": "seeking: what is LEDGER_MODULUS?"}, "/x/klatch")
        self.assertEqual(out, [{"verb": "send", "text": "seeking: what is LEDGER_MODULUS?"}])
        out = asks.extract("Bash", {"command": 'cd /w && /x/klatch post lobby "answer to #2: LEDGER_MODULUS is 5" && git status'}, "/x/klatch")
        self.assertEqual(out, [{"verb": "post", "text": "answer to #2: LEDGER_MODULUS is 5"}])
        out = asks.extract("Bash", {"command": "/x/klatch ask lobby 'what is PARTITIONS'"}, "/x/klatch")
        self.assertEqual(out, [{"verb": "ask", "text": "what is PARTITIONS"}])
        self.assertEqual(asks.extract("Bash", {"command": "/x/klatch read lobby"}, "/x/klatch"), [])
        self.assertEqual(asks.extract("Bash", {"command": "git push"}, None), [])

    def test_classify(self):
        self.assertEqual(asks.classify("Seeking: x", "post"), "ask")
        self.assertEqual(asks.classify("x", "ask"), "ask")
        self.assertEqual(asks.classify("answer to #3: y", "post"), "answer")
        self.assertEqual(asks.classify("answer: y", "send"), "answer")
        self.assertEqual(asks.classify("need-info #1 (2 of 4): which unit?", "post"), "need_info")
        self.assertEqual(asks.classify("working: ledger", "post"), "other")


class TestMetrics(unittest.TestCase):
    def test_asks_that_get_answers_score_everything_and_lose_nothing(self):
        m = run([ev("a", 1.0, "seeking: what is TOPIC_PREFIX in the routing spec?"),
                 ev("b", 1.5, "answer to #1: TOPIC_PREFIX is %s" % V["TOPIC_PREFIX"]),
                 ev("a", 2.0, "seeking: CHECKSUM_SEED please"),
                 ev("c", 2.4, "answer: CHECKSUM_SEED = %d" % V["CHECKSUM_SEED"])])
        self.assertEqual((m["asks_made"], m["asks_answered"], m["asks_lost"]), (2, 2, 0))
        self.assertAlmostEqual(m["time_to_first_answer_s"], 0.5)
        self.assertEqual(m["answer_delays_s"], [0.5, 0.4])
        self.assertEqual((m["duplicate_answers"], m["repeat_asks"], m["wrong_guesses"], m["answers_stale"]), (0, 0, 0, 0))

    def test_an_agent_that_never_asks_guesses_and_the_guess_is_counted_without_an_ask(self):
        m = run([], a__topic_name="wrong", c__chunk_count="wrong", b__frame_tag="stale")
        self.assertEqual((m["asks_made"], m["asks_answered"], m["asks_lost"]), (0, 0, 0))
        self.assertIsNone(m["time_to_first_answer_s"])
        self.assertEqual((m["wrong_guesses"], m["wrong_without_ask"]), (3, 3))
        self.assertEqual(m["unanswered_dependencies"], 0)

    def test_a_wrong_slot_after_an_ask_is_not_a_wrong_without_ask(self):
        m = run([ev("c", 1.0, "seeking: PARTITIONS?"), ev("b", 1.2, "answer: PARTITIONS is %d" % ST["PARTITIONS"])],
                c__shard_for="stale")
        self.assertEqual((m["wrong_guesses"], m["wrong_without_ask"], m["answers_stale"]), (1, 0, 1))

    def test_a_late_ask_to_a_finished_helper_is_lost(self):
        m = run([ev("a", 5.0, "seeking: LEDGER_MODULUS, c?")], done={"c": 2.0}, a__retry_slot="unanswered")
        self.assertEqual((m["asks_made"], m["asks_answered"], m["asks_lost"]), (1, 0, 1))
        self.assertEqual((m["late_asks"], m["late_asks_lost"], m["late_asks_answered"]), (1, 1, 0))
        self.assertEqual(m["unanswered_dependencies"], 1)

    def test_a_late_ask_answered_from_history_by_a_live_agent_is_answered_not_lost(self):
        m = run([ev("a", 5.0, "seeking: LEDGER_MODULUS?"), ev("b", 5.3, "answer: LEDGER_MODULUS is %d" % V["LEDGER_MODULUS"])],
                done={"c": 2.0})
        self.assertEqual((m["late_asks"], m["late_asks_answered"], m["late_asks_lost"]), (1, 1, 0))

    def test_a_repeat_of_the_seeded_exchange_is_counted_and_not_avoided(self):
        m = run([ev("c", 1.0, "seeking: MAX_PAYLOAD again?"), ev("a", 1.4, "answer: MAX_PAYLOAD is %d" % V["MAX_PAYLOAD"])])
        self.assertEqual(m["repeat_asks"], 1)
        self.assertEqual(m["repeat_asks_avoided"], 0)

    def test_the_seeded_need_met_without_an_ask_is_a_repeat_avoided(self):
        m = run([])
        self.assertEqual(m["repeat_asks_avoided"], 1)  # c.chunk_count: seeded answer existed, no ask, correct
        self.assertEqual(m["repeat_opportunities"], 1)  # the late one had no earlier answer in this run

    def test_a_late_need_met_from_an_earlier_answer_without_an_ask_is_avoided(self):
        m = run([ev("b", 1.0, "seeking: LEDGER_MODULUS?"), ev("c", 1.2, "answer: LEDGER_MODULUS is %d" % V["LEDGER_MODULUS"])],
                done={"c": 2.0})
        self.assertEqual(m["repeat_opportunities"], 2)
        self.assertEqual(m["repeat_asks_avoided"], 2)

    def test_a_correct_slot_with_no_ask_and_no_earlier_answer_is_not_an_opportunity(self):
        m = run([ev("a", 1.0, "seeking: TOPIC_PREFIX"), ev("b", 1.2, "answer: TOPIC_PREFIX %s" % V["TOPIC_PREFIX"])])
        self.assertEqual(m["repeat_opportunities"], 1)

    def test_a_second_ask_for_an_answered_fact_is_a_repeat_ask(self):
        m = run([ev("b", 1.0, "seeking: LEDGER_MODULUS?"), ev("c", 1.2, "answer: LEDGER_MODULUS is %d" % V["LEDGER_MODULUS"]),
                 ev("a", 3.0, "seeking: LEDGER_MODULUS?"), ev("c", 3.2, "answer: LEDGER_MODULUS is %d" % V["LEDGER_MODULUS"])])
        self.assertEqual((m["asks_made"], m["asks_answered"], m["repeat_asks"]), (2, 2, 1))

    def test_two_answers_to_one_ask_are_a_duplicate_answer(self):
        m = run([ev("a", 1.0, "seeking: FRAME_MAGIC?"), ev("c", 1.2, "answer: FRAME_MAGIC is %d" % V["FRAME_MAGIC"]),
                 ev("b", 1.3, "answer to #1: FRAME_MAGIC %d" % V["FRAME_MAGIC"])])
        self.assertEqual((m["asks_answered"], m["duplicate_answers"]), (1, 1))

    def test_an_answer_is_matched_by_value_when_it_names_no_key(self):
        m = run([ev("a", 1.0, "seeking: the prefix for topics?"), ev("b", 1.2, "answer to #1: %s" % V["TOPIC_PREFIX"])])
        self.assertEqual(m["asks_unclassified"], 1)
        self.assertEqual(m["asks_made"], 1)

    def test_an_ask_naming_a_fact_by_words_is_attributed(self):
        m = run([ev("a", 1.0, "seeking: what is the topic prefix?"), ev("b", 1.2, "answer: topic_prefix is %s" % V["TOPIC_PREFIX"])])
        self.assertEqual((m["asks_unclassified"], m["asks_answered"]), (0, 1))

    def test_need_info_events_are_counted(self):
        m = run([ev("a", 1.0, "seeking: CHECKSUM_SEED"), ev("c", 1.2, "need-info #1 (1 of 2): which revision?"),
                 ev("a", 1.4, "answer to need-info: revision 3"), ev("c", 1.8, "answer to #1: CHECKSUM_SEED %d" % V["CHECKSUM_SEED"])])
        self.assertEqual(m["need_info_events"], 1)
        self.assertEqual(m["asks_answered"], 1)

    def test_events_without_times_give_no_times_but_still_match_in_order(self):
        m = run([ev("a", None, "seeking: PARTITIONS"), ev("b", None, "answer: PARTITIONS %d" % V["PARTITIONS"])])
        self.assertEqual(m["asks_answered"], 1)
        self.assertIsNone(m["time_to_first_answer_s"])


if __name__ == "__main__":
    unittest.main()
