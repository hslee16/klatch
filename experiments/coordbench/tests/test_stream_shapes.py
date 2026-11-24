"""Every reader of a stream tolerates any event shape (#1519, found by the first real run of isolation_probe.py): a "message"
that is a plain string (an error or retry event), a missing or string "content", blocks that are not objects, lines that are
not JSON objects. A parser that crashes after a paid run loses the run, so none may."""
import json
import os
import sys
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import chain
import drivers
import history
import injmetrics
import isolation
import isolation_probe
import recovery
import schema
import shared
import streams
from test_isolation_run import Base

GOOD_USE = {"type": "assistant", "message": {"id": "m1", "content": [{"type": "tool_use", "id": "t1", "name": "Bash",
                                                                      "input": {"command": "git status"}}]}}
GOOD_RESULT = {"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "clean"}]}}
ODD = [
    {"type": "system", "subtype": "api_retry", "message": "retrying after a 529", "error": "overloaded", "attempt": 1},
    {"type": "system", "subtype": "init", "message": "sandbox unavailable", "tools": ["Bash"]},
    {"type": "error", "message": "boom"},
    {"type": "assistant", "message": "just a string"},
    {"type": "user", "message": "just a string"},
    {"type": "assistant", "message": {"content": "a string"}},
    {"type": "assistant", "message": {"content": ["str", 3, None, [], {"type": "tool_use"}, {"type": "text", "text": 5}]}},
    {"type": "assistant", "message": None},
    {"type": "assistant"},
    {"type": "user", "message": {"content": "x"}},
    {"type": "user", "message": {"content": ["x", None, {"type": "tool_result", "tool_use_id": "t1", "content": [None, "a", {"text": 3}]}]}},
    {"type": "result", "subtype": "error_during_execution", "is_error": True, "result": "Sandbox is required but failed to initialize"},
    {"type": "result", "is_error": True, "result": None, "total_cost_usd": "n/a"},
    "a string line", 5, None, [], [1, 2],
]


def lines(*events):
    return [json.dumps(e) for e in events] + ["", "not json {", "{"]


PLAN = {"forms": [], "words": [], "token": "x"}
ALL = lines(*ODD[:3], GOOD_USE, *ODD[3:], GOOD_RESULT)


class Readers(unittest.TestCase):
    def test_the_stream_summary_of_the_harness(self):
        s = streams.parse_stream(ALL, klatch_bin="/b/klatch")
        self.assertEqual(s["tool_calls"]["by_tool"].get("Bash"), 1)

    def test_the_isolation_scan(self):
        n, v = isolation.scan_lines(ALL, isolation.Policy("a", ("/t/c", "/t/h", "/t/m"), guard=["/t"]))
        self.assertEqual(v, [])
        self.assertGreaterEqual(n, 1)

    def test_the_chain_parser(self):
        self.assertTrue(chain.parse_stream(ALL))

    def test_the_history_timeline(self):
        history.timeline_from_sessions({"a": ALL})

    def test_the_recovery_reader(self):
        recovery.read_events(ALL)

    def test_the_injection_reader(self):
        injmetrics.from_stream("\n".join(ALL), PLAN)

    def test_the_lost_edit_reader(self):
        shared.lost_edit_events({"a": ALL}, "x.py", "a", "b")

    def test_every_odd_event_alone_never_raises_in_any_reader(self):
        for ev in ODD:
            one = lines(ev)
            streams.parse_stream(one, klatch_bin=None)
            isolation.scan_lines(one, isolation.Policy("a", ("/t/c", "/t/h", "/t/m"), guard=["/t"]))
            chain.parse_stream(one)
            history.timeline_from_sessions({"a": one})
            recovery.read_events(one)
            shared.lost_edit_events({"a": one}, "x.py", "a", "b")
            injmetrics.from_stream("\n".join(one), PLAN)
            isolation_probe.tool_results("\n".join(one))
            isolation_probe.event_summary("\n".join(one))


class OddDriver(drivers.FakeDriver):
    """An agent that prints the odd stream and exits: the whole harness must still produce a record."""
    def command(self, name, arm, prompt, cwd, out_dir, **kw):
        code = "import sys\nfor l in %r:\n    print(l)\n" % (ALL,)
        return [sys.executable, "-c", code], {}


class WholeRun(Base):
    def test_a_run_whose_streams_carry_odd_events_still_gets_its_record(self):
        self.run_with(OddDriver("x"))
        rec = self.record()
        self.assertEqual(rec["outcome"], "ok", rec["notes"])
        self.assertEqual(rec["isolation"]["violations"], [])
        self.assertEqual(schema.validate_record(rec), [])


class Probe(unittest.TestCase):
    STREAM = "\n".join(lines(
        {"type": "system", "subtype": "init", "model": "m", "tools": ["Bash"]},
        {"type": "system", "subtype": "api_retry", "message": "x" * 300, "error": "overloaded"},
        GOOD_USE, GOOD_RESULT, ODD[3], ODD[5],
        {"type": "result", "subtype": "error_during_execution", "is_error": True, "result": "Sandbox is required but failed to initialize: no bwrap"}))

    def test_tool_results_tolerates_every_shape_and_keeps_the_real_ones(self):
        self.assertEqual(isolation_probe.tool_results(self.STREAM), ["clean"])

    def test_the_event_summary_shows_startup_refusals_and_errors_truncated(self):
        text = isolation_probe.event_summary(self.STREAM)
        self.assertIn("system/init", text)
        self.assertIn("system/api_retry", text)
        self.assertIn("overloaded", text)
        self.assertIn("result/error_during_execution", text)
        self.assertIn("Sandbox is required but failed to initialize", text)
        self.assertNotIn("x" * 201, text)
        self.assertNotIn("assistant", text)  # the tool traffic is the table's business

    def test_the_raw_output_is_written_before_anything_is_parsed(self):
        root = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(root, ignore_errors=True))
        out, err = isolation_probe.save_raw(root, "STDOUT TEXT", "STDERR TEXT")
        self.assertEqual((open(out).read(), open(err).read()), ("STDOUT TEXT", "STDERR TEXT"))
        self.assertTrue(out.startswith(root))

    def test_a_parser_bug_cannot_lose_the_run_and_the_root_is_kept(self):
        root = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(root, ignore_errors=True))

        def boom(_):
            raise AttributeError("'str' object has no attribute 'get'")
        printed = []
        keep = isolation_probe.report(root, ("RAW", "ERR", 0), [], boom, printed.append)
        self.assertTrue(keep)
        self.assertTrue(any("RAW" in open(os.path.join(root, n)).read() for n in os.listdir(root) if n.endswith(".out")))
        self.assertTrue(any("raw output" in p for p in printed))
        self.assertTrue(any("AttributeError" in p for p in printed))

    def test_a_nonzero_exit_keeps_the_root_and_a_clean_run_does_not(self):
        root = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(root, ignore_errors=True))
        quiet = lambda s: None
        self.assertTrue(isolation_probe.report(root, ("", "auth failed", 1), [], lambda o: [], quiet))
        self.assertFalse(isolation_probe.report(root, (self.STREAM, "", 0), [], lambda o: [], quiet))


if __name__ == "__main__":
    unittest.main()
