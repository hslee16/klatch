"""The messaging rule against REAL streams (#1520 review): the three two-session probes of 2026-11-23 (claude 2.1.331, Haiku; the
home path stripped to <HOME>, the init event cut to its tools) and the real B1 pilot run on the trivial fixture. The probe streams
carry no times, so each line is stamped here with a fixed step, both agents started together as the harness starts them, and an
agent ends at its last line. Nothing runs a model."""
import json
import os
import unittest

from helpers import BENCH  # noqa: F401
import messaging
import streams

FIX = os.path.join(BENCH, "tests", "fixtures")
STEP = 2.0


def lines(path):
    with open(os.path.join(FIX, path)) as f:
        return [ln for ln in f.read().splitlines() if ln.strip()]


def replay(prefix, names=("a", "b"), step=STEP, fmt="msgprobe/%s-%s.jsonl", starts=None):
    """(sessions, alive) of a two-agent probe stamped line by line."""
    sess, alive = {}, {}
    for n in names:
        st = streams.StreamState()
        start = (starts or {}).get(n, 0.0)
        ls = lines(fmt % (prefix, n))
        for i, ln in enumerate(ls):
            st.feed(ln, start + i * step)
        sess[n] = dict(st.summary(), agent=n)
        alive[n] = (start, start + len(ls) * step)
    return sess, alive


class Probes(unittest.TestCase):
    def judge(self, prefix, **kw):
        sess, alive = replay(prefix, **kw)
        return messaging.judge("B1", sess, alive)

    def test_the_shared_config_probe_verifies_and_both_sends_were_delivered(self):
        r = self.judge("shared")
        self.assertEqual(r["status"], "verified")
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (2, 2))
        self.assertIs(r["tools_present"], True)
        self.assertIs(r["listagents_named_peer"], True)

    def test_the_separate_config_probe_delivered_nothing(self):
        r = self.judge("separate-config")
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (1, 0))  # is_error is not set on these
        self.assertIs(r["listagents_named_peer"], False)

    def test_the_separate_home_probe_delivered_one_of_three(self):
        r = self.judge("separate-home")
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (3, 1))  # a to b worked; b to a failed twice
        self.assertEqual(r["status"], "verified")

    def test_the_broken_probe_is_failed_when_the_agents_start_together_the_case_the_harness_runs(self):
        r = self.judge("separate-config")
        self.assertEqual(r["status"], "failed", r)
        self.assertIn("No reachable agents", r["reason"] + " " + json.dumps(r.get("first_listagents_after_all_started")))

    def test_the_first_listagents_result_of_a_real_agent_is_too_early_to_count(self):
        sess, alive = replay("separate-config", step=0.8)  # a's first ListAgents result at a few seconds
        a = sess["a"]["messaging"]["listagents"][0]
        self.assertLess(a["t"], messaging.SETTLE_S)
        first = messaging.judge("B1", {"a": sess["a"]}, {"a": alive["a"]})
        self.assertEqual(first["status"], "unknown")  # one agent, and nothing after the settle time

    def test_the_failed_rule_names_what_listagents_really_said(self):
        r = self.judge("separate-config")
        self.assertIn("No reachable agents", r["reason"])
        self.assertNotIn("This session is", r["reason"])  # the key sentence, not the first 80 characters of the preamble

    def test_a_real_send_result_is_json_and_is_read_by_its_success_field(self):
        sess, _ = replay("separate-config")
        send = sess["a"]["messaging"]["sends"][0]
        self.assertTrue(send["text"].startswith('{"success":false'))
        self.assertFalse(send["error"])
        self.assertFalse(messaging.delivered(send))
        ok = replay("shared")[0]["a"]["messaging"]["sends"][0]
        self.assertTrue(messaging.delivered(ok))

    def test_the_stale_socket_failure_is_not_delivered(self):
        sess, _ = replay("separate-home")
        texts = [s["text"] for s in sess["b"]["messaging"]["sends"]]
        self.assertTrue(any("Failed to send to uds:" in t for t in texts))
        self.assertEqual([messaging.delivered(s) for s in sess["b"]["messaging"]["sends"]], [False, False])

    def test_the_peer_line_of_a_real_listagents_result_names_the_teammate_and_the_preamble_does_not(self):
        sess, _ = replay("shared")
        ra = sess["a"]["messaging"]["listagents"][0]["text"]
        self.assertTrue(messaging.names_peer(ra, "a", ["a", "b"]))
        rb = sess["b"]["messaging"]["listagents"][0]["text"]  # "This session is b [..] ... No reachable agents"
        self.assertFalse(messaging.names_peer(rb, "b", ["a", "b"]))
        self.assertFalse(messaging.names_peer(rb, "c", ["a", "b"]))  # "b [id]" in the preamble is the asker, not a peer

    def test_the_init_events_of_the_probes_name_both_tools(self):
        for n in ("a", "b"):
            self.assertEqual(replay("shared")[0][n]["messaging"]["tools"].count("SendMessage"), 1)

    def test_the_clock_facts_are_recorded_so_an_unknown_run_can_be_explained(self):
        sess, alive = replay("shared", starts={"a": 0.0, "b": 3.0})
        r = messaging.judge("B1", sess, alive)
        self.assertEqual(r["clock"]["last_agent_start"], 3.0)
        self.assertEqual(r["clock"]["settle_s"], messaging.SETTLE_S)
        self.assertEqual(set(r["clock"]["alive"]), {"a", "b"})
        f = r["first_listagents_after_all_started"]
        self.assertTrue(f is None or f["t"] >= 3.0)


class PilotRun(unittest.TestCase):
    """The real B1 pilot on the trivial fixture: no ListAgents, no SendMessage, so unknown (never failed, never verified)."""

    def test_the_real_pilot_run_is_unknown_with_the_reason(self):
        sess, alive = replay("", fmt="pilot-b1/%s%s.stream.jsonl", step=1.0)
        r = messaging.judge("B1", sess, alive)
        self.assertEqual(r["status"], "unknown")
        self.assertEqual((r["messages_sent"], r["listagents_calls"]), (0, 0))
        self.assertIsNone(r["first_listagents_after_all_started"])
        self.assertIn("no ListAgents", r["reason"])


if __name__ == "__main__":
    unittest.main()
