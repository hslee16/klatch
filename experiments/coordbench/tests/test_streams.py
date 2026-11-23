"""The stream-json parser, against output written by hand in the shape `claude -p --output-format stream-json
--verbose --include-hook-events` prints (as read by experiments/dm-adoption/metric.py). Nothing here runs a model."""
import json
import unittest

from helpers import BENCH  # noqa: F401
import streams

KB = "/opt/klatch/klatch"


def ev(**kw):
    return json.dumps(kw)


def asst(mid, blocks, usage=None, model="claude-haiku-4-5-20251001"):
    return ev(type="assistant", message={"id": mid, "model": model, "content": blocks,
                                         "usage": usage or {"input_tokens": 100, "output_tokens": 10,
                                                            "cache_read_input_tokens": 50,
                                                            "cache_creation_input_tokens": 20}})


def tool(tid, name, **inp):
    return {"type": "tool_use", "id": tid, "name": name, "input": inp}


def res(tid, text):
    return ev(type="user", message={"content": [{"type": "tool_result", "tool_use_id": tid, "content": text}]})


SAMPLE = [
    ev(type="system", subtype="init", model="claude-haiku-4-5-20251001", session_id="s1"),
    asst("m1", [tool("t1", "Bash", command=KB + " post lobby 'working: alpha'")]),
    res("t1", "posted"),
    ev(type="system", subtype="hook_response", hook_event="PostToolUse", output="[klatch] 2 unread in lobby"),
    asst("m2", [tool("t2", "Bash", command=KB + " wait lobby --timeout 60")]),
    res("t2", "nothing new"),
    asst("m3", [tool("t3", "Edit", file_path="alpha.py", old_string="x", new_string="y"),
                tool("t4", "Bash", command="git pull --no-rebase origin main")]),
    res("t3", "ok"),
    res("t4", "CONFLICT (content): Merge conflict in registry.py\nAutomatic merge failed"),
    asst("m4", [{"type": "text", "text": "I will ask the human for help. DONE a"}]),
    ev(type="result", subtype="success", is_error=False, num_turns=4, total_cost_usd=0.0123, duration_ms=5000,
       usage={"input_tokens": 400, "output_tokens": 40, "cache_read_input_tokens": 200, "cache_creation_input_tokens": 80},
       result="I will ask the human for help. DONE a"),
]


class Parse(unittest.TestCase):
    def setUp(self):
        self.s = streams.parse_stream(SAMPLE, klatch_bin=KB)

    def test_result_message_is_the_source_of_turns_tokens_and_cost(self):
        self.assertEqual(self.s["turns"], 4)
        self.assertEqual(self.s["tokens"], {"input": 400, "output": 40, "cache_read": 200, "cache_creation": 80})
        self.assertAlmostEqual(self.s["cost_usd"], 0.0123)
        self.assertFalse(self.s["cost_estimated"])
        self.assertTrue(self.s["result_seen"])

    def test_tool_calls_by_tool(self):
        self.assertEqual(self.s["tool_calls"], {"total": 4, "by_tool": {"Bash": 3, "Edit": 1}})

    def test_models_are_the_resolved_ids(self):
        self.assertEqual(self.s["models"], ["claude-haiku-4-5-20251001"])

    def test_messages_are_counted_from_klatch_posts(self):
        self.assertEqual(self.s["messages"], 1)

    def test_git_conflicts_come_from_bash_results(self):
        self.assertEqual(self.s["git_conflicts"], 1)

    def test_idle_turn_is_a_turn_of_only_waits_and_polls(self):
        self.assertEqual(self.s["idle_turns"], 1)  # m2: a klatch wait; m1 posted, m3 edited

    def test_human_mentions_in_the_final_message(self):
        self.assertEqual(self.s["human_mentions"], 1)

    def test_final_text_and_done_line(self):
        self.assertIn("DONE a", self.s["final_text"])

    def test_coordination_context_counts_hook_output_calls_and_results(self):
        chars = self.s["coord_context_chars"]
        floor = len("[klatch] 2 unread in lobby") + len("posted") + len("nothing new")
        self.assertGreater(chars, floor)
        self.assertLess(chars, floor + 400)  # the Edit and the git pull are not coordination


class Fallbacks(unittest.TestCase):
    def test_killed_run_has_no_result_so_turns_and_cost_are_estimated_from_messages(self):
        lines = [x for x in SAMPLE if json.loads(x).get("type") != "result"]
        s = streams.parse_stream(lines, klatch_bin=KB)
        self.assertFalse(s["result_seen"])
        self.assertEqual(s["turns"], 4)  # four distinct assistant message ids
        self.assertTrue(s["cost_estimated"])
        # four messages of 100 in, 10 out, 50 cache read, 20 cache creation at the haiku rows of prices.json
        p = streams.load_prices()["per_mtok"]["haiku"]
        one = (100 * p["input"] + 10 * p["output"] + 50 * p["cache_read"] + 20 * p["cache_creation"]) / 1e6
        self.assertAlmostEqual(s["cost_usd"], 4 * one, places=9)
        self.assertEqual(s["tokens"]["input"], 400)

    def test_repeated_events_of_one_message_count_once(self):
        a = asst("m1", [{"type": "text", "text": "hi"}])
        b = asst("m1", [tool("t1", "Bash", command="ls")])
        s = streams.parse_stream([a, b])
        self.assertEqual(s["turns"], 1)
        self.assertEqual(s["tokens"]["input"], 100)

    def test_garbage_and_blank_lines_are_skipped(self):
        s = streams.parse_stream(["", "not json", "[1,2]", asst("m1", [{"type": "text", "text": "x"}])])
        self.assertEqual(s["turns"], 1)

    def test_unknown_model_is_priced_at_the_dearest_row(self):
        prices = streams.load_prices()["per_mtok"]
        self.assertEqual(streams.price_row("claude-mystery-9", prices), max(
            (prices[k] for k in ("haiku", "sonnet", "opus")), key=lambda r: r["output"]))

    def test_error_result_marks_is_error(self):
        s = streams.parse_stream([ev(type="result", subtype="error_max_turns", is_error=True, num_turns=9,
                                     total_cost_usd=0.5)])
        self.assertTrue(s["is_error"])
        self.assertEqual(s["turns"], 9)

    def test_sendmessage_counts_as_a_message(self):
        s = streams.parse_stream([asst("m1", [tool("t1", "SendMessage", to="b", message="hi")])])
        self.assertEqual(s["messages"], 1)


class Tracker(unittest.TestCase):
    def test_running_cost_grows_line_by_line_and_prefers_the_result(self):
        st = streams.StreamState()
        st.feed(asst("m1", [{"type": "text", "text": "x"}]))
        first = st.cost_usd
        st.feed(asst("m2", [{"type": "text", "text": "x"}]))
        self.assertGreater(st.cost_usd, first)
        st.feed(ev(type="result", num_turns=2, total_cost_usd=9.0))
        self.assertEqual(st.cost_usd, 9.0)

    def test_fake_cost_event_is_taken_as_cumulative(self):
        st = streams.StreamState()
        st.feed(ev(type="system", subtype="fake_cost", usd=0.4))
        st.feed(ev(type="system", subtype="fake_cost", usd=0.9))
        self.assertEqual(st.cost_usd, 0.9)


if __name__ == "__main__":
    unittest.main()
