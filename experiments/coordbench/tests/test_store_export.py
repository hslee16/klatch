"""The run store export (#1433): the counts of the K arms' temporary store, from the bench tool `klatch-bench store-stats`
(no admin passphrase), written to artifacts.store_export, and the scenario metrics that were null filled from it, with the
ask counts cross-checked against what the streams show (asks.py). No model, no spend.

The real-export tests need KLATCH_BIN (cargo build -p klatch --locked) and the bench tool beside it
(cargo build -p klatch-bench --locked): with KLATCH_BIN set and no tool they fail with that instruction, they do not skip."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import asks
import schema
import storeexport
from test_run_fake import Base, KLATCH_BIN, NEEDS_BIN, read_json

# a store export as `klatch admin stats --json` prints it (the fields storeexport reads)
EXPORT = {
    "schema": 1, "asks": {"created": 3, "answered": 2, "closed": 1, "first_answer_median_secs": 14, "first_answer_p90_secs": 30},
    "claims": {"history": {
        "granted": [{"name": "file", "count": 4}, {"name": "ticket", "count": 1}],
        "refused": [{"name": "same-path", "count": 2}, {"name": "overlap", "count": 1}],
        "released": [{"name": "file", "count": 3}], "expired": [{"name": "file", "count": 1}],
        "takeovers": 0, "conflicts_handled": 2}},
    "members": {"joined": 3, "posted": 2, "posted_never_read": 1},
    "hook_bytes": [{"name": "PostToolUse", "count": 700}, {"name": "SessionStart", "count": 300}],
    "machine_checkpoints": {"written": 2, "bytes": 90},
}


class TestMetrics(unittest.TestCase):
    def test_the_null_metrics_come_from_the_export_fields(self):
        m = storeexport.metrics(EXPORT)
        self.assertEqual(m["claim_conflicts"], 2)
        self.assertEqual(m["claims_granted"], 5)
        self.assertEqual(m["claims_refused_by_rule"], 3)
        self.assertEqual(m["claims_released"], 3)
        self.assertEqual(m["claims_expired"], 1)
        self.assertEqual(m["asks_created"], 3)
        self.assertEqual(m["asks_answered"], 2)
        self.assertEqual(m["asks_closed"], 1)
        self.assertEqual(m["time_to_first_answer_s"], 14)
        self.assertEqual(m["members_posted_never_read"], 1)
        self.assertEqual(m["hook_bytes_injected"], 1000)

    def test_a_missing_field_is_null_not_zero(self):
        m = storeexport.metrics({"schema": 1})
        self.assertIsNone(m["claim_conflicts"])
        self.assertIsNone(m["hook_bytes_injected"])
        self.assertIsNone(m["asks_created"])

    def test_the_bench_tool_is_found_beside_the_binary_or_by_the_environment(self):
        import tempfile
        d = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(d, ignore_errors=True))
        k = os.path.join(d, "klatch")
        self.assertIsNone(storeexport.find_tool(k, {}))
        b = os.path.join(d, "klatch-bench")
        open(b, "w").close()
        self.assertEqual(storeexport.find_tool(k, {}), b)
        self.assertEqual(storeexport.find_tool(None, {"KLATCH_BENCH_BIN": "/x/y"}), "/x/y")
        self.assertIsNone(storeexport.find_tool(None, {}))


class TestAsksFromStreams(unittest.TestCase):
    def stream(self, *cmds):
        lines, n = [], 0
        for cmd, out in cmds:
            n += 1
            lines.append(json.dumps({"type": "assistant", "message": {"id": "m%d" % n, "content": [
                {"type": "tool_use", "id": "t%d" % n, "name": "Bash", "input": {"command": cmd}}]}}))
            lines.append(json.dumps({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t%d" % n, "content": out}]}}))
        return lines

    def test_asks_and_answers_are_counted_and_a_lost_race_is_not_an_answer(self):
        a = self.stream(("/k/klatch ask lobby which port", "asked #3"), ("/k/klatch ask lobby another", "asked #4"))
        b = self.stream(("/k/klatch answer lobby 3 8080", "answered #3"),
                        ("/k/klatch answer lobby 4 nope", "not answered: #4 is claimed by carol"))
        got = asks.from_streams({"a": a, "b": b}, "/k/klatch")
        self.assertEqual(got["created"], 2)
        self.assertEqual(got["answered"], 1)

    def test_agreement_and_mismatch_are_flagged_and_both_sides_recorded(self):
        ok = asks.cross_check({"created": 2, "answered": 1}, {"created": 2, "answered": 1})
        self.assertTrue(ok["match"])
        self.assertEqual(ok["differences"], [])
        bad = asks.cross_check({"created": 2, "answered": 1}, {"created": 3, "answered": 1})
        self.assertFalse(bad["match"])
        self.assertEqual(bad["store"], {"created": 3, "answered": 1})
        self.assertEqual(bad["streams"], {"created": 2, "answered": 1})
        self.assertEqual(bad["differences"], ["created: streams 2, store 3"])


class TestReport(unittest.TestCase):
    """The report generator shows the store metrics for the arms that have them and skips the arms that do not."""

    def test_k_arm_store_metrics_are_in_the_per_metric_table_and_b_arms_are_left_out(self):
        import report
        from helpers import make_record
        recs = []
        for arm, conflicts in (("B0", None), ("K2", 2), ("K2", 4)):
            r = make_record(scenario="trivial", arm=arm, run="r%d" % len(recs))
            r["metrics"]["claim_conflicts"] = conflicts
            r["scenario_metrics"] = {"store": {"claim_conflicts": conflicts, "claims_granted": 5, "asks_created": 3,
                                              "asks_answered": 2, "time_to_first_answer_s": 14, "hook_bytes_injected": 900,
                                              "members_posted_never_read": 1}} if conflicts is not None else \
                {"store": {"reason": "no klatch store in a B0 arm"}}
            recs.append(r)
        a = report.analyse(recs, resamples=200)
        text = report.render_markdown(a)
        cell = a["scenarios"]["trivial"]["arms"]
        self.assertEqual(cell["K2"]["metrics"]["claim_conflicts"]["median"], 3)
        self.assertIsNone(cell["B0"]["metrics"]["claim_conflicts"])
        for name in ("claim_conflicts", "claims_granted", "asks_created", "time_to_first_answer_s", "hook_bytes_injected"):
            self.assertIn("| K2 | %s |" % name, text)
            self.assertNotIn("| B0 | %s |" % name, text)


def wait(path):
    return {"op": "wait_origin_has", "path": path, "contains": "", "timeout": 40}


PULL = {"op": "git", "args": ["pull", "--no-rebase", "--no-edit", "origin", "main"]}
K = ["K1", "K2"]
# a and b really claim, collide, ask and answer through the binary; git files order the two agents
A = [{"op": "klatch", "args": ["join", "{{ROOM}}"]},
     {"op": "klatch", "args": ["claim", "{{ROOM}}", "shared.txt"]},
     {"op": "klatch", "args": ["ask", "{{ROOM}}", "which", "port", "SENTINELASKWORDS"], "capture": "q"},
     {"op": "write", "path": "ask.txt", "content": "{{var:q}}"},
     {"op": "sync", "message": "asked"}, wait("b-done.txt"), PULL,
     {"op": "klatch", "args": ["release", "{{ROOM}}", "shared.txt"]},
     {"op": "write", "path": "a-released.txt", "content": "x"}, {"op": "sync", "message": "released"},
     {"op": "write", "path": "alpha.py", "content": "def alpha():\n    return 'alpha-done'\n"},
     {"op": "sync", "message": "alpha"}, {"op": "final", "text": "DONE a"}]
B = [{"op": "klatch", "args": ["join", "{{ROOM}}"]}, wait("ask.txt"), PULL,
     {"op": "set_var_from_file", "key": "q", "path": "ask.txt"},
     {"op": "klatch", "args": ["claim", "{{ROOM}}", "shared.txt"], "allow_fail": True},
     {"op": "klatch", "args": ["answer", "{{ROOM}}", "{{var:q}}", "8080"]},
     {"op": "write", "path": "b-done.txt", "content": "x"}, {"op": "sync", "message": "b done"},
     wait("a-released.txt"), PULL,
     {"op": "klatch", "args": ["claim", "{{ROOM}}", "shared.txt"]},
     {"op": "write", "path": "beta.py", "content": "def beta():\n    return 'beta-done'\n"},
     {"op": "sync", "message": "beta"}, {"op": "final", "text": "DONE b"}]


@NEEDS_BIN
class RealExport(Base):
    def run_k(self, arm="K2", **kw):
        s = self.script(a=A, b=B)
        code, out, err = self.go(self.argv(arm, runs=1, fake_script=s, **kw))
        self.assertEqual(code, 0, out + err)
        (r,) = self.records(arm)
        return r

    def test_a_k_run_writes_the_store_export_and_fills_the_null_metrics(self):
        r = self.run_k()
        self.assertEqual(schema.validate_record(r), [])
        self.assertEqual(r["outcome"], "ok", r["notes"])
        ex = r["artifacts"]["store_export"]
        self.assertIsInstance(ex, dict, r["notes"])
        self.assertEqual(ex["asks"]["created"], 1)
        self.assertEqual(ex["asks"]["answered"], 1)
        sm = r["scenario_metrics"]["store"]
        self.assertEqual(r["metrics"]["claim_conflicts"], 1)
        self.assertEqual(sm["claim_conflicts"], 1)
        # two file claims (a, then b after a released) and the claim an answer takes on its ask and releases
        self.assertEqual(sm["claims_granted"], 3)
        self.assertEqual(sm["claims_released"], 2)
        self.assertEqual(sm["claims_refused_by_rule"], 1)
        self.assertEqual(sm["members_posted_never_read"], 2)  # the fake agents post and never read
        self.assertEqual(sm["asks_created"], 1)
        self.assertEqual(sm["asks_answered"], 1)
        self.assertIsInstance(sm["time_to_first_answer_s"], (int, float))
        self.assertIn("hook_bytes_injected", sm)
        self.assertIn("members_posted_never_read", sm)

    def test_the_ask_counts_are_cross_checked_against_the_streams(self):
        r = self.run_k()
        x = r["scenario_metrics"]["store"]["asks_cross_check"]
        self.assertTrue(x["match"], x)
        self.assertEqual(x["streams"], {"created": 1, "answered": 1})
        self.assertEqual(x["store"], {"created": 1, "answered": 1})

    def test_the_export_holds_no_text_the_agents_wrote(self):
        r = self.run_k()
        blob = json.dumps(r["artifacts"]["store_export"]) + json.dumps(r["scenario_metrics"]["store"])
        for s in ("SENTINELASKWORDS", "which", "8080", "shared.txt"):
            self.assertNotIn(s, blob)

    def test_b_arms_keep_the_metrics_null_with_a_reason(self):
        s = self.script()
        code, out, err = self.go(self.argv("B0", runs=1, fake_script=s))
        self.assertEqual(code, 0, out + err)
        (r,) = self.records("B0")
        self.assertIsNone(r["artifacts"]["store_export"])
        self.assertIsNone(r["metrics"]["claim_conflicts"])
        self.assertIn("no klatch store", r["scenario_metrics"]["store"]["reason"])

    def test_a_missing_tool_keeps_the_nulls_and_a_note_and_the_run_stays_valid(self):
        r = self.run_k(arm="K1", bench_bin="/nonexistent/klatch-bench")
        self.assertTrue(r["valid"])
        self.assertEqual(r["outcome"], "ok")
        self.assertIsNone(r["artifacts"]["store_export"])
        self.assertIsNone(r["metrics"]["claim_conflicts"])
        self.assertIn("store export", r["notes"])


if __name__ == "__main__":
    unittest.main()
