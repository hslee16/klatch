"""The conformance scenario for a klatch client (#119): the same steps run in every language against the real gateway.

Run from this directory: `KLATCH_BIN=/path/to/klatch python3 -m unittest -v`. The binary defaults to the debug build of
the workspace. Each test starts its own throwaway store.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

import klatch_client as kc

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.environ.get("KLATCH_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch")
FIXTURES = os.path.join(HERE, "..", "..", "proto", "klatch", "fixtures", "protocol")
SERVER_BIN = os.environ.get("KLATCH_SERVER_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch-server")


class Store:
    """A throwaway KLATCH_HOME with its own key directory, and clients as members of it."""

    attribution = "unattributed"  # what the gateway says of a member of this store in the hello

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


class DaemonStore(Store):
    """The same throwaway store, with a `klatch-server daemon` over a short socket path, and clients that connect to
    it over the socket (#1095) by a typed name."""

    def __init__(self):
        super().__init__()
        self.sockdir = tempfile.mkdtemp(prefix="kc-", dir="/tmp")  # a socket path is limited to about 100 bytes
        self.sock = os.path.join(self.sockdir, "d.sock")
        self.log = open(os.path.join(self.sockdir, "err.log"), "w+")
        env = {"PATH": "/usr/bin:/bin", "HOME": self.home, "KLATCH_HOME": self.home, "KLATCH_KEY_DIR": os.path.join(self.home, "keys")}
        self.daemon = subprocess.Popen(
            [SERVER_BIN, "daemon", "--socket", self.sock], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=self.log
        )
        until = time.monotonic() + 30
        while time.monotonic() < until:
            self.log.seek(0)
            if "daemon listening on" in self.log.read():
                return
            time.sleep(0.02)
        raise RuntimeError("the daemon never listened")

    def client(self, name):
        c = kc.Client.connect_socket(self.sock, name=name)
        self.clients.append(c)
        return c

    def close(self):
        for c in self.clients:
            c.close()
        self.daemon.send_signal(signal.SIGKILL)
        self.daemon.wait()
        self.log.close()
        shutil.rmtree(self.sockdir, ignore_errors=True)
        super().close()


ISSUE_BIN = os.environ.get("KLATCH_ISSUE_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "examples", "issue_test_credential")


class HttpStore(Store):
    """The same throwaway store, with a real `klatch-server http` on a port the system chooses, and clients that connect
    to it over HTTP and server-sent events (#1114). Every connection needs a worker credential, so one is issued per
    member with the test-only helper (`cargo build -p klatch-server --example issue_test_credential`), which refuses
    any store that is not a throwaway one under a temp directory. The member is `verified`, proven by its credential."""

    attribution = "verified"

    def __init__(self):
        super().__init__()
        self.keys = os.path.join(self.home, "keys")
        self.tokens = {}
        self.log = tempfile.TemporaryFile("w+")
        env = {"PATH": "/usr/bin:/bin", "HOME": self.home, "KLATCH_HOME": self.home, "KLATCH_KEY_DIR": self.keys}
        self.server = subprocess.Popen(
            [SERVER_BIN, "http", "--listen", "127.0.0.1:0"], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=self.log
        )
        self.port = None
        until = time.monotonic() + 60
        text = ""
        while self.port is None and time.monotonic() < until:
            self.log.seek(0)
            text = self.log.read()
            # The line counts only once it is complete (its newline is there), never half an address.
            m = re.search(r"http listening on 127\.0\.0\.1:(\d+) [^\n]*\n", text)
            if m:
                self.port = int(m.group(1))
            else:
                time.sleep(0.02)
        if self.port is None:
            self.close()
            raise RuntimeError("the server never listened; its stderr was:\n" + text)

    def token(self, name):
        if name not in self.tokens:
            env = {"PATH": "/usr/bin:/bin", "HOME": self.home, "KLATCH_HOME": self.home, "KLATCH_KEY_DIR": self.keys}
            out = subprocess.run([ISSUE_BIN, name], env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
            if out.returncode != 0:
                raise RuntimeError("the credential helper failed: " + out.stderr.strip())
            self.tokens[name] = out.stdout.strip()
        return self.tokens[name]

    def client(self, name):
        c = kc.Client.connect_http(f"http://127.0.0.1:{self.port}", member=name, token=self.token(name), connect_timeout=10)
        self.clients.append(c)
        return c

    def close(self):
        for c in self.clients:
            c.close()
        self.clients = []
        self.server.send_signal(signal.SIGKILL)
        self.server.wait()
        self.log.close()
        super().close()


class Conformance(unittest.TestCase):
    make_store = Store

    def setUp(self):
        self.store = self.make_store()
        self.addCleanup(self.store.close)

    def test_the_protocol_version_is_one(self):
        self.assertEqual(kc.PROTOCOL_VERSION, 1)

    def test_hello_says_who_the_gateway_believes_you_are(self):
        a = self.store.client("alice")
        self.assertEqual((a.member, a.attribution, a.clearance), ("alice", self.store.attribution, "public"))

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
        self.assertEqual(messages[0]["attribution"], self.store.attribution)
        self.assertEqual(len(b.read("r")), 2)
        self.assertEqual(b.read("r"), [])

    def test_an_ask_scope_survives_post_read_history_and_asks_and_could_answer_names_a_mismatch(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        b.set_scope("git:bar")
        a.post("r", "seeking: plain")
        seq = a.post("r", "seeking: scoped", scope="git:foo")["seq"]
        messages = b.read("r", peek=True)
        self.assertNotIn("scope", messages[0])
        self.assertEqual(messages[1]["scope"], "git:foo")
        self.assertEqual(b.history("r")["messages"][1]["scope"], "git:foo")
        asks = b.asks("r")
        self.assertEqual(asks[1]["scope"], "git:foo")
        self.assertIsNone(asks[0].get("scope"))
        answer = b.could_answer("r", "bob", seq)
        self.assertFalse(answer["could"])
        self.assertEqual(answer["blockers"][0]["rule"], "scope")
        with self.assertRaises(kc.ApiError):
            a.post("r", "seeking: x", scope="has space")
        with self.assertRaises(kc.ApiError):
            a.post("r", "hello", scope="git:foo")

    def test_history_looks_back_without_moving_the_cursor_and_pages_with_since(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        for i in range(1, 6):
            a.post("r", f"m{i}")
        bodies = lambda h: [m["body"] for m in h["messages"]]
        full = b.history("r")
        self.assertEqual(bodies(full), ["m1", "m2", "m3", "m4", "m5"])
        self.assertEqual(full["more"], 0)
        self.assertEqual(bodies(b.history("r", last=2)), ["m4", "m5"])
        since = full["messages"][1]["seq"]
        self.assertEqual(bodies(b.history("r", last=2, since=since)), ["m4", "m5"])
        self.assertEqual(bodies(b.history("r", since=since)), ["m3", "m4", "m5"])
        self.assertEqual(bodies(a.history("r", last=1)), ["m5"])
        self.assertEqual(len(b.read("r")), 5)
        with self.assertRaises(kc.ApiError) as ctx:
            b.history("r", last=0)
        self.assertEqual(ctx.exception.code, "invalid")

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

    def test_a_member_without_the_admin_role_is_refused_every_act_and_may_prune(self):
        a = self.store.client("alice")
        for act, params in (
            ("run-create", {"run": "r1"}),
            ("run-end", {"run": "r1"}),
            ("anchor-done", {"anchor": "gh:pr:o/r/1"}),
            ("close", {"room": "r"}),
        ):
            with self.assertRaises(kc.ApiError, msg=act) as ctx:
                a.role_act(act, **params)
            self.assertEqual(ctx.exception.code, "refused", act)
            self.assertTrue(any(r["rule"] == "admin-scope" for r in ctx.exception.refusals), act)
        with self.assertRaises(kc.ApiError) as ctx:
            a.role_act("reap-anchors")
        self.assertEqual(ctx.exception.code, "unknown_act")
        with self.assertRaises(kc.ApiError) as ctx:
            a.role_act("run-create")
        self.assertEqual(ctx.exception.code, "invalid_params")
        self.assertEqual(a.prune(), 0)
        a.post("r", "still here")
        self.assertEqual(a.prune(), 0)
        self.assertEqual([m["name"] for m in a.who("r")], ["alice"])

    def test_a_direct_message_reaches_one_member_in_a_room_nobody_else_may_use(self):
        a, b, c = self.store.client("alice"), self.store.client("bob"), self.store.client("carol")
        sent = a.dm("bob", "hello bob")
        self.assertEqual((sent["room"], sent["seq"]), ("dm.alice.bob", 1))
        self.assertEqual(len(sent["id"]), 26)
        messages = b.read(sent["room"])
        self.assertEqual([(m["sender"], m["body"]) for m in messages], [("alice", "hello bob")])
        self.assertEqual(b.dm("alice", "hi back")["seq"], 2)
        with self.assertRaises(kc.ApiError) as ctx:
            c.read(sent["room"])
        self.assertEqual(ctx.exception.code, "refused")
        self.assertTrue(any(r["rule"] == "dm-members" for r in ctx.exception.refusals), ctx.exception.refusals)
        with self.assertRaises(kc.ApiError) as ctx:
            a.dm("alice", "to myself")
        self.assertEqual(ctx.exception.code, "invalid")
        with self.assertRaises(kc.ApiError) as ctx:
            a.dm("bob", "above the ceiling", label="secret")
        self.assertEqual(ctx.exception.code, "refused")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("dm", to="bob")
        self.assertEqual(ctx.exception.code, "invalid_params")

    def test_a_member_mutes_and_unmutes_a_room_and_read_still_shows_everything(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        b.post("r", "chatter")
        a.mute("r")
        self.assertEqual([m["body"] for m in a.read("r", peek=True)], ["chatter"])
        a.mute("r", muted=False)
        a.mute("r", muted=True)
        with self.assertRaises(kc.ApiError) as ctx:
            a.mute("bad room")
        self.assertEqual(ctx.exception.code, "invalid")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("mute", room="r")
        self.assertEqual(ctx.exception.code, "invalid_params")

    def test_an_invite_is_a_pointer_posted_where_the_member_already_is(self):
        a, b = self.store.client("alice"), self.store.client("bob")
        a.post("team", "hello")
        b.post("team", "hi")
        a.invite("bob", "design", "team")
        bodies = [m["body"] for m in b.read("team")]
        self.assertTrue(any("klatch join design" in x for x in bodies), bodies)
        with self.assertRaises(kc.ApiError) as ctx:
            a.invite("alice", "design", "team")
        self.assertEqual(ctx.exception.code, "invalid")
        with self.assertRaises(kc.ApiError) as ctx:
            a.invite("bob", "bad room", "team")
        self.assertEqual(ctx.exception.code, "invalid")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("invite", member="bob")
        self.assertEqual(ctx.exception.code, "invalid_params")

    def test_a_member_reads_its_own_role(self):
        a = self.store.client("alice")
        a.post("r", "here")
        self.assertIsNone(a.my_role())

    def test_a_member_reads_its_rooms_its_board_and_its_receipt(self):
        a = self.store.client("alice")
        b = self.store.client("bob")
        sent = a.post("r", "offering: review help")
        b.post("r", "hi")
        self.assertEqual(a.my_rooms(), ["r"])
        self.assertIn("review help", a.board("r"))
        rows = a.receipt("r", sent["seq"])["recipients"]
        self.assertEqual([r["name"] for r in rows], ["bob"])
        with self.assertRaises(kc.ApiError) as ctx:
            b.receipt("r", sent["seq"])
        self.assertEqual(ctx.exception.code, "refused")

    def test_a_member_declares_its_scope_its_kind_and_its_status(self):
        a = self.store.client("alice")
        a.post("r", "here")
        a.set_scope("git:demo-1")
        a.claim("r", "src/a.rs")
        self.assertEqual([c.get("scope") for c in a.claims("r")], ["git:demo-1"])
        with self.assertRaises(kc.ApiError) as ctx:
            a.set_scope("not a scope!")
        self.assertEqual(ctx.exception.code, "invalid")
        a.set_kind("bot")
        self.assertEqual([m.get("kind") for m in a.who("r")], ["bot"])
        a.set_kind(None)
        self.assertEqual([m.get("kind") for m in a.who("r")], [None])
        with self.assertRaises(kc.ApiError) as ctx:
            a.set_kind("session")
        self.assertEqual(ctx.exception.code, "invalid")
        a.set_status("r", "done")
        self.assertEqual([m.get("status") for m in a.who("r")], ["done"])
        with self.assertRaises(kc.ApiError) as ctx:
            a.set_status("r", "")
        self.assertEqual(ctx.exception.code, "invalid")

    def test_a_member_that_is_not_the_reviewer_is_refused_both_review_methods(self):
        a = self.store.client("alice")
        with self.assertRaises(kc.ApiError) as ctx:
            a.review_next()
        self.assertEqual(ctx.exception.code, "refused")
        self.assertTrue(any(r["rule"] == "reviewer" for r in ctx.exception.refusals), ctx.exception.refusals)
        with self.assertRaises(kc.ApiError) as ctx:
            a.review_verdict(1, "allow")
        self.assertEqual(ctx.exception.code, "refused")
        with self.assertRaises(kc.ApiError) as ctx:
            a.request("review_verdict", id=1)
        self.assertEqual(ctx.exception.code, "invalid_params")

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


@unittest.skipUnless(os.path.exists(SERVER_BIN), "klatch-server is not built (cargo build -p klatch-server)")
class ConformanceOverTheDaemon(Conformance):
    """Every step of the scenario above, unchanged, over the klatch-server daemon's Unix socket (#1095): the same
    events and sequence numbers are required of both transports."""

    make_store = DaemonStore


@unittest.skipUnless(
    os.path.exists(SERVER_BIN) and os.path.exists(ISSUE_BIN),
    "klatch-server or its test helper is not built (cargo build -p klatch-server --example issue_test_credential)",
)
class ConformanceOverHttp(Conformance):
    """Every step of the scenario above, unchanged, over `klatch-server http` (#1123): each member has a credential
    issued in the throwaway store, and the same events and sequence numbers are required of this transport too."""

    make_store = HttpStore


@unittest.skipUnless(os.path.exists(SERVER_BIN), "klatch-server is not built (cargo build -p klatch-server)")
class SocketConnect(unittest.TestCase):
    def setUp(self):
        self.store = DaemonStore()
        self.addCleanup(self.store.close)

    def test_a_typed_name_is_the_member_and_unattributed_after_the_handshake(self):
        a = self.store.client("alice")
        self.assertEqual((a.member, a.attribution, a.clearance), ("alice", "unattributed", "public"))
        a.post("r", "hi")
        self.assertEqual(a.history("r")["messages"][0]["sender"], "alice")

    def test_no_identity_is_the_unmapped_process_the_daemon_gives_every_local_process(self):
        c = kc.Client.connect_socket(self.store.sock)
        self.addCleanup(c.close)
        self.assertEqual((c.member, c.attribution), ("unmapped-process", "unattributed"))

    def test_a_refused_identity_raises_without_echoing_the_name(self):
        for bad in ("admin", "a b", "x/y"):
            with self.assertRaises(kc.ApiError, msg=bad) as ctx:
                kc.Client.connect_socket(self.store.sock, name=bad)
            self.assertEqual(ctx.exception.code, "identity.refused")
            self.assertNotIn(bad, ctx.exception.message)

    def test_a_name_together_with_a_credential_is_refused_before_anything_is_sent(self):
        with self.assertRaises(ValueError):
            kc.Client.connect_socket(self.store.sock, name="alice", member="w-1", token="t")
        with self.assertRaises(ValueError):
            kc.Client.connect_socket(self.store.sock, member="w-1")

    def test_a_credential_that_is_not_live_is_refused_and_the_token_is_never_echoed(self):
        token = "0" * 64
        with self.assertRaises(kc.ApiError) as ctx:
            kc.Client.connect_socket(self.store.sock, member="w-1", token=token)
        self.assertEqual(ctx.exception.code, "identity.refused")
        self.assertNotIn(token, str(ctx.exception))

    def test_a_socket_nobody_listens_on_is_an_error_and_close_answers_zero_once_the_daemon_ends_the_session(self):
        with self.assertRaises(OSError):
            kc.Client.connect_socket(os.path.join(self.store.sockdir, "nobody.sock"), name="alice")
        a = self.store.client("alice")
        self.assertEqual(a.close(), 0)
        self.assertEqual(a.close(), 0)  # closing again is harmless
        b = self.store.client("bob")
        self.assertEqual(b.post("r", "after")["seq"], 1)

    def test_the_daemon_ending_the_session_is_a_protocol_error_not_a_hang(self):
        a = self.store.client("alice")
        a.post("r", "x")
        self.store.daemon.send_signal(signal.SIGKILL)
        self.store.daemon.wait()
        with self.assertRaises(kc.ProtocolError):
            a.post("r", "y")


class ResetReader:
    """A transport that says hello and then fails as Linux reports a dead peer: ConnectionResetError on the read."""

    def __iter__(self):
        yield '{"v":1,"type":"hello","member":"u","attribution":"unattributed","clearance":"public"}\n'
        raise ConnectionResetError(104, "Connection reset by peer")

    def close(self):
        pass


class Sink:
    def write(self, text):
        raise BrokenPipeError(32, "Broken pipe")

    def flush(self):
        pass

    def close(self):
        pass


class SocketReset(unittest.TestCase):
    def test_a_reset_ends_the_session_and_never_raises_in_the_reader_thread(self):
        import threading

        raised = []
        old = threading.excepthook
        threading.excepthook = lambda args: raised.append(args)
        self.addCleanup(setattr, threading, "excepthook", old)
        c = kc.Client.__new__(kc.Client)
        c._sock = None
        c._start(ResetReader(), Sink(), proc=None)
        with self.assertRaises(kc.ProtocolError):
            c.request("post", room="r", body="x")
        time.sleep(0.2)
        self.assertEqual(raised, [])

    def test_a_send_failure_is_a_protocol_error_that_keeps_its_cause(self):
        c = kc.Client.__new__(kc.Client)
        c._sock = None
        c._next_id = 0
        c._writer = Sink()
        with self.assertRaises(kc.ProtocolError) as ctx:
            c.request("post", room="r", body="x")
        self.assertIsInstance(ctx.exception.__cause__, BrokenPipeError)


if __name__ == "__main__":
    unittest.main()
