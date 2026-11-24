"""B1 and B1-roles messaging must be shown to work in the run itself (#1520). The rule, on recorded-style streams (the result
texts are the shapes of the two-session probe of the pilot plan: "Peer sessions (1): b [eab75d] ... busy" and "No reachable
agents"), and end to end through the harness with fake agents that print those streams. No model, no spend."""
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import drivers
import fixture as fixture_mod
import messaging
import run
import runcli
import schema
import streams
from test_streams import asst, ev, res, tool

TOOLS = ["Bash", "Edit", "SendMessage", "ListAgents"]
PEERS = "Peer sessions (1): b [eab75d] /scratch/b/clone busy"
NONE = "No reachable agents"


def init(tools=TOOLS):
    return ev(type="system", subtype="init", model="claude-haiku-4-5-20251001", tools=tools)


def session(agent, lines):
    """The summary of a stream; lines is [(time, line)]."""
    st = streams.StreamState()
    for t, ln in lines:
        st.feed(ln, t)
    s = st.summary()
    s["agent"] = agent
    return s


def lister(text, t, n=1, tools=TOOLS):
    return [(0.0, init(tools)), (t - 0.1, asst("l%d" % n, [tool("L%d" % n, "ListAgents")])), (t, res("L%d" % n, text))]


def sender(text, t, n=1):
    return [(t - 0.1, asst("s%d" % n, [tool("S%d" % n, "SendMessage", to="b", message="PING")])), (t, res("S%d" % n, text))]


def judge(sess, alive, arm="B1"):
    return messaging.judge(arm, {k: v for k, v in sess.items()}, alive)


class Parse(unittest.TestCase):
    def test_the_init_tools_and_the_results_of_the_two_tools_are_kept_with_their_times(self):
        s = session("a", lister(PEERS, 20.0) + sender("Delivered to b.", 21.0))
        m = s["messaging"]
        self.assertEqual(m["tools"], TOOLS)
        self.assertEqual(m["listagents"], [{"t": 20.0, "text": PEERS, "error": False}])
        self.assertEqual(m["sends"], [{"t": 21.0, "text": "Delivered to b.", "error": False}])

    def test_a_stream_whose_init_event_carries_no_tools_list_says_so(self):
        s = session("a", [(0.0, ev(type="system", subtype="init", model="m"))])
        self.assertIsNone(s["messaging"]["tools"])

    def test_tool_names_are_read_from_objects_too(self):
        s = session("a", [(0.0, init([{"name": "SendMessage", "description": "x"}, {"name": "ListAgents"}]))])
        self.assertEqual(s["messaging"]["tools"], ["SendMessage", "ListAgents"])

    def test_an_error_flag_on_the_result_is_kept(self):
        bad = ev(type="user", message={"content": [{"type": "tool_result", "tool_use_id": "S1", "is_error": True,
                                                    "content": "No agent named b is reachable"}]})
        s = session("a", [(0.0, init()), (1.0, asst("s1", [tool("S1", "SendMessage", to="b", message="x")])), (2.0, bad)])
        self.assertTrue(s["messaging"]["sends"][0]["error"])


class Rule(unittest.TestCase):
    def two(self, a_lines, b_lines, alive=((0.0, 100.0), (0.0, 100.0))):
        sess = {"a": session("a", a_lines), "b": session("b", b_lines)}
        return judge(sess, {"a": alive[0], "b": alive[1]})

    def test_a_listagents_result_that_names_the_colleague_verifies_messaging(self):
        r = self.two(lister(PEERS, 20.0) + sender("Delivered to b.", 21.0), [(0.0, init())])
        self.assertEqual(r["status"], "verified")
        self.assertEqual({k: r[k] for k in ("tools_present", "listagents_named_peer", "messages_sent", "messages_delivered")},
                         {"tools_present": True, "listagents_named_peer": True, "messages_sent": 1, "messages_delivered": 1})

    def test_an_init_event_without_listagents_or_without_sendmessage_fails_the_run(self):
        for missing in ("ListAgents", "SendMessage"):
            tools = [t for t in TOOLS if t != missing]
            r = self.two(lister(PEERS, 20.0), [(0.0, init(tools))])
            self.assertEqual(r["status"], "failed", missing)
            self.assertIs(r["tools_present"], False)
            self.assertIn("b", r["reason"])
            self.assertIn(missing, r["reason"])

    def test_no_peer_ever_named_while_a_colleague_was_alive_for_a_while_fails_the_run(self):
        r = self.two(lister(NONE, 30.0), [(0.0, init())])
        self.assertEqual(r["status"], "failed")
        self.assertIs(r["listagents_named_peer"], False)
        self.assertIn("No reachable agents", r["reason"])  # the reason says what ListAgents showed
        self.assertIn("ListAgents", r["reason"])

    def test_a_colleague_that_had_only_just_started_is_no_evidence(self):
        r = self.two(lister(NONE, 3.0), [(0.0, init())], alive=((0.0, 100.0), (0.0, 100.0)))
        self.assertEqual(r["status"], "unknown")

    def test_agents_that_never_overlapped_are_not_evidence_of_failure(self):
        r = self.two(lister(NONE, 8.0), [(0.0, init())], alive=((0.0, 10.0), (12.0, 40.0)))
        self.assertEqual(r["status"], "unknown")

    def test_a_colleague_that_had_already_finished_is_no_evidence(self):
        r = self.two(lister(NONE, 60.0), [(0.0, init())], alive=((0.0, 100.0), (0.0, 30.0)))
        self.assertEqual(r["status"], "unknown")

    def test_no_listagents_call_at_all_is_unknown_not_invalid(self):
        r = self.two([(0.0, init())], [(0.0, init())])
        self.assertEqual(r["status"], "unknown")
        self.assertIs(r["tools_present"], True)
        self.assertIs(r["listagents_named_peer"], False)

    def test_one_agent_cannot_verify_anything(self):
        r = judge({"a": session("a", lister(NONE, 30.0))}, {"a": (0.0, 100.0)})
        self.assertEqual(r["status"], "unknown")

    def test_a_later_result_that_names_the_colleague_outweighs_an_earlier_empty_one(self):
        lines = lister(NONE, 20.0, 1) + lister(PEERS, 50.0, 2)[1:]
        r = self.two(lines, [(0.0, init())])
        self.assertEqual(r["status"], "verified")

    def test_a_streams_without_a_tools_list_does_not_fail_the_tools_check(self):
        r = self.two(lister(PEERS, 20.0, tools=None), [(0.0, ev(type="system", subtype="init", model="m"))])
        self.assertIsNone(r["tools_present"])
        self.assertEqual(r["status"], "verified")

    def test_the_agent_itself_and_a_name_that_only_contains_a_teammates_name_do_not_count(self):
        own = "Peer sessions (1): a [111111] /x busy"
        longer = "Peer sessions (1): bob [222222] /x busy"
        for text in (own, longer):
            r = self.two(lister(text, 30.0), [(0.0, init())])
            self.assertIs(r["listagents_named_peer"], False, text)
            self.assertEqual(r["status"], "failed", text)

    def test_the_names_of_several_peers_are_found_on_any_line(self):
        text = "Peer sessions (2): c [aaaaaa] /x idle\nb [eab75d] /y busy"
        r = self.two(lister(text, 30.0), [(0.0, init())])
        self.assertIs(r["listagents_named_peer"], True)

    def test_a_send_that_was_not_delivered_is_counted_as_sent_only(self):
        lines = lister(PEERS, 20.0) + sender("No agent named b is reachable", 21.0, 1) + sender("Delivered to b.", 22.0, 2)
        r = self.two(lines, [(0.0, init())])
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (2, 1))
        r = self.two(lister(PEERS, 20.0) + sender("Not sent: b is busy", 21.0), [(0.0, init())])
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (1, 0))

    def test_messages_of_every_agent_are_added_up(self):
        r = self.two(lister(PEERS, 20.0) + sender("Delivered to b.", 21.0), lister(PEERS.replace("b [", "a ["), 22.0) + sender("Delivered to a.", 23.0))
        self.assertEqual((r["messages_sent"], r["messages_delivered"]), (2, 2))

    def test_a_restarted_session_of_an_agent_is_not_a_colleague_of_itself(self):
        sess = {"a": session("a", lister(NONE, 30.0)), "a.r2": session("a", [(0.0, init())])}
        r = judge(sess, {"a": (0.0, 100.0), "a.r2": (0.0, 100.0)})
        self.assertEqual(r["status"], "unknown")  # one agent only

    def test_role_members_count_as_agents(self):
        sess = {"a": session("a", lister(NONE, 30.0)), "supervisor": session("supervisor", [(0.0, init())])}
        r = judge(sess, {"a": (0.0, 100.0), "supervisor": (0.0, 100.0)}, arm="B1-roles")
        self.assertEqual(r["status"], "failed")

    def test_streams_without_times_cannot_fail_a_run(self):
        st = streams.StreamState()
        for _, ln in lister(NONE, 30.0):
            st.feed(ln)
        r = judge({"a": dict(st.summary(), agent="a"), "b": session("b", [(0.0, init())])}, {"a": (0.0, 100.0), "b": (0.0, 100.0)})
        self.assertEqual(r["status"], "unknown")

    def test_only_the_messaging_arms_are_judged(self):
        for arm in ("B0", "K1", "K2", "K3"):
            self.assertIsNone(messaging.judge(arm, {}, {}))


class RecordShape(unittest.TestCase):
    def test_the_field_is_optional_null_or_an_object_with_its_four_numbers(self):
        rec = __import__("helpers").make_record()
        self.assertEqual(schema.validate_record(rec), [])
        rec["messaging_verified"] = None
        self.assertEqual(schema.validate_record(rec), [])
        rec["messaging_verified"] = {"tools_present": True, "listagents_named_peer": True, "messages_sent": 1, "messages_delivered": 1}
        self.assertEqual(schema.validate_record(rec), [])
        rec["messaging_verified"] = {"tools_present": True}
        self.assertTrue([e for e in schema.validate_record(rec) if "messaging_verified" in e])
        rec["messaging_verified"] = "yes"
        self.assertTrue([e for e in schema.validate_record(rec) if "messaging_verified" in e])


class ProbeDriver(drivers.FakeDriver):
    """Agents that print a fixed stream (init with a tools list, a ListAgents call with a canned result) and stay alive a moment
    so that the two overlap. tools_of maps an agent name to its init tools list; peers is the ListAgents result text."""
    lists_logs = True

    def __init__(self, tools_of=None, peers=PEERS, stay=1.5):
        super().__init__(os.path.join(BENCH, "fixtures", "trivial", "fake.json"))
        self.tools_of, self.peers, self.stay = tools_of or {}, peers, stay
        self.configs = {}
        self.isos = {}

    def extra_env_for(self, scratch):
        return {"CLAUDE_CONFIG_DIR": os.path.join(scratch, "claude-config")}

    def command(self, name, arm, prompt, cwd, out_dir, **kw):
        self.isos[name] = kw.get("isolation")
        code = ("import json,os,sys,time\n"
                "name,tools,peers,stay,out=sys.argv[1],json.loads(sys.argv[2]),sys.argv[3],float(sys.argv[4]),sys.argv[5]\n"
                "emit=lambda e:print(json.dumps(e),flush=True)\n"
                "emit({'type':'system','subtype':'init','model':'claude-fake','tools':tools})\n"
                "open(os.path.join(out,name+'.env.json'),'w').write(json.dumps({'cfg':os.environ.get('CLAUDE_CONFIG_DIR'),"
                "'home':os.environ.get('HOME'),'tmp':os.environ.get('TMPDIR'),'cwd':os.getcwd(),"
                "'attrs':open('.gitattributes').read() if os.path.exists('.gitattributes') else None}))\n"
                "cfg=os.environ.get('CLAUDE_CONFIG_DIR')\n"
                "if cfg:\n"
                "    os.makedirs(os.path.join(cfg,'sessions'),exist_ok=True)\n"
                "    open(os.path.join(cfg,'sessions',name+'.json'),'w').write('{}')\n"
                "emit({'type':'assistant','message':{'id':'m1','content':[{'type':'tool_use','id':'L1','name':'ListAgents','input':{}}]}})\n"
                "time.sleep(stay)\n"
                "emit({'type':'user','message':{'content':[{'type':'tool_result','tool_use_id':'L1','content':peers}]}})\n"
                "time.sleep(stay)\n"
                "emit({'type':'result','subtype':'success','is_error':False,'num_turns':1,'result':'DONE '+name,'total_cost_usd':0.0})\n")
        tools = self.tools_of.get(name, TOOLS)
        return [sys.executable, "-c", code, name, json.dumps(tools), self.peers, str(self.stay), out_dir], {}


class Harness(unittest.TestCase):
    """A whole harness run with a driver of the test's choosing (no tests of its own)."""

    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-msg-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.home = os.path.join(self.tmp, "home")
        os.mkdir(self.home)
        p = mock.patch.dict(os.environ, {"HOME": self.home})
        p.start()
        self.addCleanup(p.stop)
        g = mock.patch.object(messaging, "ALIVE_GRACE_S", 0.2)
        g.start()
        self.addCleanup(g.stop)
        self.results, self.logs, self.scratch = (os.path.join(self.tmp, d) for d in ("results", "logs", "scratch"))
        os.mkdir(self.scratch)

    def go(self, driver, arm="B1"):
        argv = ["--fixture", "trivial", "--arm", arm, "--runs", "1", "--driver", "fake", "--suite-id", "t1", "--results-dir",
                self.results, "--logs-dir", self.logs, "--tmp-root", self.scratch, "--seed", "100", "--poll-s", "0.2",
                "--wall-s", "60"]
        a = runcli.parse_args(argv)
        a.arm_list = [arm]
        a.klatch_bin = None
        a.tmp_root = os.path.realpath(a.tmp_root)
        fx = fixture_mod.Fixture("trivial", a.fixtures_dir, a.checks_dir)
        with contextlib.redirect_stdout(io.StringIO()):
            run.Suite(a, fx, driver, run.plan_runs([arm], 1, a.seed)).go()
        with open(os.path.join(self.results, "t1", "trivial", arm, "t1-trivial-%s-r01.json" % arm.lower())) as f:
            return json.load(f)


class EndToEnd(Harness):
    def test_a_b1_run_whose_agents_name_each_other_is_valid_and_records_the_evidence(self):
        rec = self.go(ProbeDriver())
        mv = rec["messaging_verified"]
        self.assertEqual(mv["status"], "verified", mv)
        self.assertIs(mv["tools_present"], True)
        self.assertIs(mv["listagents_named_peer"], True)
        self.assertEqual(schema.validate_record(rec), [])
        self.assertTrue(rec["valid"], rec["notes"])

    def test_the_record_lists_what_the_run_left_in_the_shared_config_directory(self):
        rec = self.go(ProbeDriver())
        left = rec["messaging_verified"]["config_dir_entries"]
        self.assertEqual(left, {"count": 2, "files": [{"path": "sessions/a.json", "bytes": 2}, {"path": "sessions/b.json", "bytes": 2}]})

    def test_a_b1_run_with_an_agent_that_lacks_a_messaging_tool_is_invalid_with_the_reason(self):
        rec = self.go(ProbeDriver(tools_of={"b": ["Bash", "Edit", "SendMessage"]}))
        self.assertFalse(rec["valid"])
        self.assertIs(rec["messaging_verified"]["tools_present"], False)
        self.assertIn("messaging", rec["notes"])
        self.assertIn("ListAgents", rec["notes"])

    def test_a_b1_run_in_which_listagents_never_names_a_colleague_is_invalid(self):
        rec = self.go(ProbeDriver(peers=NONE))
        self.assertFalse(rec["valid"])
        self.assertEqual(rec["messaging_verified"]["status"], "failed")
        self.assertIn("No reachable agents", rec["notes"])

    def test_another_arm_has_no_messaging_field_content(self):
        rec = self.go(ProbeDriver(), arm="B0")
        self.assertIsNone(rec["messaging_verified"])
        self.assertTrue(rec["valid"], rec["notes"])


if __name__ == "__main__":
    unittest.main()
