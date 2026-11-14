"""The conformance scenario for a klatch client (#119): the same steps run in every language against the real gateway.

Run from this directory: `KLATCH_BIN=/path/to/klatch python3 -m unittest -v`. The binary defaults to the debug build of
the workspace. Each test starts its own throwaway store.
"""
import json
import os
import shutil
import tempfile
import unittest

import klatch_client as kc

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.environ.get("KLATCH_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch")
FIXTURES = os.path.join(HERE, "..", "..", "proto", "klatch-protocol", "fixtures")


class Store:
    """A throwaway KLATCH_HOME with its own key directory, and clients as members of it."""

    def __init__(self):
        self.home = tempfile.mkdtemp(prefix="klatch-client-")
        self.clients = []

    def client(self, name):
        env = dict(os.environ, KLATCH_HOME=self.home, KLATCH_KEY_DIR=os.path.join(self.home, "keys"), KLATCH_NAME=name)
        for k in ("KLATCH_IDENTITY_FILE", "KLATCH_DISCOVER"):
            env.pop(k, None)
        c = kc.Client(BIN, env=env)
        self.clients.append(c)
        return c

    def close(self):
        for c in self.clients:
            c.close()
        shutil.rmtree(self.home, ignore_errors=True)


class Conformance(unittest.TestCase):
    def setUp(self):
        self.store = Store()
        self.addCleanup(self.store.close)

    def test_the_protocol_version_is_one(self):
        self.assertEqual(kc.PROTOCOL_VERSION, 1)

    def test_hello_says_who_the_gateway_believes_you_are(self):
        a = self.store.client("alice")
        self.assertEqual((a.member, a.attribution, a.clearance), ("alice", "unattributed", "public"))

    def test_every_server_fixture_line_is_understood_and_unknown_things_are_ignored(self):
        with open(os.path.join(FIXTURES, "server_basic.ndjson")) as f:
            lines = [l for l in f.read().splitlines() if l.strip()]
        self.assertGreater(len(lines), 5)
        for line in lines:
            frame = kc.decode_server_frame(line)
            self.assertIn(frame["type"], {"hello", "message", "caught_up", "heartbeat", "refusal", "response"}, line)
        with open(os.path.join(FIXTURES, "forward_compat.ndjson")) as f:
            for line in f.read().splitlines():
                if line.strip():
                    kc.decode_server_frame(line)  # unknown types and extra fields must not raise
        self.assertEqual(kc.decode_server_frame('{"v":1,"type":"presence_changed","who":"bob"}')["type"], "presence_changed")
        for bad in ["not json", "[1]", '{"type":"message"}', '{"v":2,"type":"message"}', '{"v":"1","type":"message"}']:
            with self.assertRaises(kc.ProtocolError, msg=bad):
                kc.decode_server_frame(bad)
        self.assertIsNone(kc.decode_server_frame("   "))

    def test_a_post_is_read_by_another_member_and_a_handoff_keeps_its_kind_and_payload(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        a.post("r", "just words")
        a.post("r", "see the order", kind="work-order", payload='{"id":7}')
        messages = b.read("r", peek=True)
        self.assertEqual([m["body"] for m in messages], ["just words", "see the order"])
        self.assertNotIn("kind", messages[0])
        self.assertNotIn("payload", messages[0])
        self.assertEqual((messages[1]["kind"], messages[1]["payload"]), ("work-order", '{"id":7}'))
        self.assertEqual(messages[0]["attribution"], "unattributed")
        self.assertEqual(len(b.read("r")), 2)
        self.assertEqual(b.read("r"), [])

    def test_subscribe_replays_then_catches_up_then_pushes_and_resumes_after_a_reconnect(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        for i in range(3):
            a.post("r", f"m{i}")
        head = b.subscribe("r", from_seq=0)
        self.assertEqual(head, 3)
        seen = b.collect(until_caught_up=True)
        self.assertEqual([m["seq"] for m in seen["messages"]], [1, 2, 3])
        self.assertEqual(seen["caught_up"]["seq"], 3)
        a.post("r", "live")
        live = b.next_message(timeout=5)
        self.assertEqual((live["seq"], live["body"]), (4, "live"))
        b.ack("r", 4)
        b.close()
        a.post("r", "while away")
        b2 = self.store.client("bob")
        b2.subscribe("r", from_seq=4)
        resumed = b2.collect(until_caught_up=True)
        self.assertEqual([m["seq"] for m in resumed["messages"]], [5])

    def test_a_second_claim_conflicts_names_the_holder_and_a_release_frees_the_path(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        a.claim("r", "src/a.rs", ttl=60, reason="refactor")
        with self.assertRaises(kc.ApiError) as ctx:
            b.claim("r", "src/a.rs")
        self.assertEqual(ctx.exception.code, "conflict")
        self.assertIn("alice", ctx.exception.message)
        self.assertEqual([c["owner"] for c in b.claims("r")], ["alice"])
        a.release("r", "src/a.rs", note="done")
        b.claim("r", "src/a.rs")
        with self.assertRaises(kc.ApiError) as ctx:
            a.release("r", "never/claimed")
        self.assertEqual(ctx.exception.code, "not_found")

    def test_a_refusal_carries_its_rule_and_never_repeats_the_text(self):
        a = self.store.client("alice")
        secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789"
        with self.assertRaises(kc.ApiError) as ctx:
            a.post("r", f"token {secret}")
        self.assertEqual(ctx.exception.code, "refused")
        self.assertTrue(any(r["rule"] == "credentials" for r in ctx.exception.refusals), ctx.exception.refusals)
        self.assertNotIn(secret, json.dumps(ctx.exception.refusals) + ctx.exception.message)

    def test_bad_requests_get_a_failed_response_and_the_session_carries_on(self):
        a = self.store.client("alice")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("no_such_method")
        self.assertEqual(ctx.exception.code, "unknown_method")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("post", room="r")  # no body
        self.assertEqual(ctx.exception.code, "invalid_params")
        with self.assertRaises(kc.ApiError) as ctx:
            a.post("bad room name", "x")
        self.assertEqual(ctx.exception.code, "invalid")
        with self.assertRaises(kc.ApiError):
            a.post("r", "p", payload="needs a kind")
        self.assertEqual(a.post("r", "still working")["seq"], 1)

    def test_closing_the_input_ends_the_session_cleanly(self):
        a = self.store.client("alice")
        a.post("r", "x")
        self.assertEqual(a.close(), 0)


if __name__ == "__main__":
    unittest.main()
