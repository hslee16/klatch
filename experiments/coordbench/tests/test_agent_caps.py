"""An agent that stops on its own budget or turn cap makes the run `spend-cap` or `turn-cap`, not `crash` (#1553).

Streams are written by hand in the shape `claude -p` prints for the two stops (result subtype error_max_budget_usd with
terminal_reason budget_exhausted; error_max_turns). Nothing here runs a model, uses a key or the network."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import agentcap
import schema
import streams
from test_run_fake import Base


def ev(**kw):
    return json.dumps(kw)


def stream(subtype, reason=None, usd=0.33, turns=7):
    extra = {"terminal_reason": reason} if reason else {}
    return streams.parse_stream([
        ev(type="system", subtype="init", model="claude-haiku-4-5-20251001", session_id="s"),
        ev(type="result", subtype=subtype, is_error=True, num_turns=turns, total_cost_usd=usd, **extra)])


class Summary(unittest.TestCase):
    def test_summary_carries_the_result_subtype_and_terminal_reason(self):
        s = stream("error_max_budget_usd", "budget_exhausted")
        self.assertEqual(s["result_subtype"], "error_max_budget_usd")
        self.assertEqual(s["terminal_reason"], "budget_exhausted")

    def test_summary_fields_are_none_without_a_result(self):
        s = streams.parse_stream([ev(type="system", subtype="init", model="m", session_id="s")])
        self.assertIsNone(s["result_subtype"])
        self.assertIsNone(s["terminal_reason"])


class Classify(unittest.TestCase):
    def test_budget_stop_is_spend_cap_and_names_the_agent(self):
        sums = {"a": stream("success"), "b": stream("error_max_budget_usd", "budget_exhausted")}
        self.assertEqual(agentcap.agent_caps(sums), {"spend-cap": ["b"]})
        self.assertEqual(agentcap.cap_outcome(agentcap.agent_caps(sums)), "spend-cap")

    def test_turn_stop_is_turn_cap(self):
        caps = agentcap.agent_caps({"a": stream("error_max_turns"), "b": stream("success")})
        self.assertEqual(agentcap.cap_outcome(caps), "turn-cap")

    def test_budget_wins_over_turns_when_both_happened(self):
        caps = agentcap.agent_caps({"a": stream("error_max_turns"), "b": stream("error_max_budget_usd")})
        self.assertEqual(agentcap.cap_outcome(caps), "spend-cap")

    def test_terminal_reason_alone_counts(self):
        caps = agentcap.agent_caps({"a": stream("error_during_execution", "budget_exhausted")})
        self.assertEqual(agentcap.cap_outcome(caps), "spend-cap")

    def test_other_errors_and_success_are_no_cap(self):
        caps = agentcap.agent_caps({"a": stream("error_during_execution"), "b": stream("success")})
        self.assertEqual(caps, {})
        self.assertIsNone(agentcap.cap_outcome(caps))

    def test_turn_cap_is_a_schema_outcome(self):
        self.assertIn("turn-cap", schema.OUTCOMES)


class EndToEnd(Base):
    def test_agent_budget_stop_with_exit_1_records_spend_cap_not_crash(self):
        s = self.script(a=[{"op": "result_error", "subtype": "error_max_budget_usd", "terminal_reason": "budget_exhausted",
                            "usd": 0.2}])
        self.go(self.argv("B0", runs=1, fake_script=s))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "spend-cap")
        self.assertTrue(r["valid"])
        self.assertEqual(r["metrics"]["units_completed"], 1)  # b still finished
        self.assertIn("agent a stopped on its own spend cap (error_max_budget_usd", r["notes"])

    def test_agent_turn_stop_records_turn_cap(self):
        s = self.script(b=[{"op": "result_error", "subtype": "error_max_turns"}])
        self.go(self.argv("B0", runs=1, fake_script=s))
        r = self.records()[0]
        self.assertEqual(r["outcome"], "turn-cap")
        self.assertIn("agent b stopped on its own turn cap (error_max_turns", r["notes"])

    def test_a_plain_nonzero_exit_is_still_a_crash(self):
        s = self.script(a=[{"op": "exit", "code": 1}])
        self.go(self.argv("B0", runs=1, fake_script=s))
        self.assertEqual(self.records()[0]["outcome"], "crash")


if __name__ == "__main__":
    unittest.main()
