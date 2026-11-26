"""Characterisation tests for the socket and stdio paths of the Python client (#1443).

They run against a small in-process server on a Unix socket that speaks the wire of the klatch-server daemon (a hello,
then one answer per request line), and against a tiny fake gateway process for stdio. They pin what the client does
today: the request framing, how a request skips the frames that are not its answer, what `identify` changes, and what
`close` does, so the internals can be reorganised without a change in behaviour. The real daemon is exercised by
`ConformanceOverSocket` in test_conformance.py. Every wait has a deadline.
"""
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import unittest

import klatch_client as kc

HELLO = {"type": "hello", "member": "m-1", "attribution": "typed", "clearance": "public"}


def frame_line(frame):
    return (json.dumps({"v": 1, **frame}, separators=(",", ":")) + "\n").encode()


def ok(rid, result=None):
    return {"type": "response", "id": rid, "ok": True, **({} if result is None else {"result": result})}


def default_answer(frame):
    return [ok(frame["id"], {})]


def read_lines(conn):
    """The newline-terminated lines the peer sends, until it ends its side."""
    buf = b""
    while True:
        data = conn.recv(4096)
        if not data:
            return
        buf += data
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            yield line.decode()


class FakeDaemon:
    """A daemon on a Unix socket: sends `hello`, records every line it is sent, answers each request with `answer`
    (a function of the request frame returning the list of frames, or raw byte lines, to send back)."""

    def __init__(self, answer=default_answer, hello=HELLO):
        self.dir = tempfile.mkdtemp(prefix="kc-", dir="/tmp")
        self.path = os.path.join(self.dir, "d.sock")
        self.answer, self.hello = answer, hello
        self.received = []  # raw lines, as sent by the client
        self.eof = threading.Event()
        self.conns = []
        self.srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.srv.bind(self.path)
        self.srv.listen(4)
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()

    def _serve(self):
        while True:
            try:
                conn, _ = self.srv.accept()
            except OSError:
                return
            self.conns.append(conn)
            threading.Thread(target=self._session, args=(conn,), daemon=True).start()

    def _session(self, conn):
        if self.hello is not None:
            conn.sendall(frame_line(self.hello))
        try:
            for line in read_lines(conn):
                self.received.append(line)
                self._answer_line(conn, line)
        except OSError:
            pass
        self.eof.set()
        try:
            conn.close()
        except OSError:
            pass

    def _answer_line(self, conn, line):
        frame = json.loads(line)
        if frame.get("type") != "request":
            return
        for out in self.answer(frame):
            conn.sendall(out if isinstance(out, bytes) else frame_line(out))

    def frames(self):
        return [json.loads(line) for line in self.received]

    def drop_clients(self):
        for conn in self.conns:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def stop(self):
        self.srv.close()
        for conn in self.conns:
            conn.close()
        shutil.rmtree(self.dir, ignore_errors=True)


class SocketCase(unittest.TestCase):
    def serve(self, **kw):
        d = FakeDaemon(**kw)
        self.addCleanup(d.stop)
        return d

    def connect(self, d, **kw):
        c = kc.Client.connect_socket(d.path, **kw)
        self.addCleanup(c.close)
        return c


class ConnectSocket(SocketCase):
    def test_no_identity_sends_no_identify_and_takes_the_hello(self):
        d = self.serve()
        c = self.connect(d)
        self.assertEqual((c.member, c.attribution, c.clearance), ("m-1", "typed", "public"))
        self.assertEqual(d.received, [])

    def test_a_name_is_identified_first_and_the_answer_replaces_member_and_attribution(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {"member": "alice", "attribution": "unattributed"})])
        c = self.connect(d, name="alice")
        self.assertEqual((c.member, c.attribution, c.clearance), ("alice", "unattributed", "public"))
        self.assertEqual(
            d.received, ['{"v":1,"type":"request","id":"c1","method":"identify","params":{"name":"alice"}}']
        )

    def test_a_member_and_token_are_identified_as_such(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {"member": "w-1", "attribution": "verified"})])
        c = self.connect(d, member="w-1", token="tok")
        self.assertEqual((c.member, c.attribution), ("w-1", "verified"))
        self.assertEqual(d.frames()[0]["params"], {"member": "w-1", "token": "tok"})

    def test_an_answer_without_member_or_attribution_keeps_the_hello_values(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {})])
        c = self.connect(d, name="alice")
        self.assertEqual((c.member, c.attribution), ("m-1", "typed"))

    def test_bad_identity_arguments_raise_value_error_before_any_connect(self):
        nobody = os.path.join(tempfile.gettempdir(), "kc-no-such.sock")
        for kw, text in (
            ({"name": "a", "member": "w"}, "not both"),
            ({"name": "a", "token": "t"}, "not both"),
            ({"member": "w"}, "a member and a token"),
            ({"token": "t"}, "a member and a token"),
        ):
            with self.subTest(kw=kw):
                with self.assertRaises(ValueError) as cm:
                    kc.Client.connect_socket(nobody, **kw)
                self.assertIn(text, str(cm.exception))

    def test_a_path_nobody_listens_on_is_an_os_error(self):
        d = self.serve()
        with self.assertRaises(OSError):
            kc.Client.connect_socket(os.path.join(d.dir, "nobody.sock"))

    def test_a_refused_identity_raises_api_error_and_closes_the_socket(self):
        def refuse(f):
            return [{"type": "response", "id": f["id"], "ok": False, "error": {"code": "identity.refused", "message": "no"}}]

        d = self.serve(answer=refuse)
        with self.assertRaises(kc.ApiError) as cm:
            kc.Client.connect_socket(d.path, name="mallory")
        self.assertEqual((cm.exception.code, cm.exception.message), ("identity.refused", "no"))
        self.assertTrue(d.eof.wait(5), "the client closed its socket")

    def test_a_first_frame_that_is_not_hello_is_a_protocol_error(self):
        d = self.serve(hello={"type": "heartbeat"})
        with self.assertRaises(kc.ProtocolError):
            kc.Client.connect_socket(d.path)
        self.assertTrue(d.eof.wait(5), "the client closed its socket when the hello was not a hello (#1469)")

    def test_a_gateway_that_never_says_hello_is_not_waited_on_forever_when_it_closes(self):
        d = self.serve(hello=None)
        threading.Timer(0.3, d.drop_clients).start()
        with self.assertRaises(kc.ProtocolError):
            kc.Client.connect_socket(d.path)


class Request(SocketCase):
    def test_framing_is_compact_json_with_version_type_id_method_params_and_ids_count_up(self):
        d = self.serve()
        c = self.connect(d)
        self.assertEqual(c.request("who", room="lobby"), {})
        self.assertEqual(c.request("join", room="r2"), {})
        self.assertEqual(
            d.received,
            [
                '{"v":1,"type":"request","id":"c1","method":"who","params":{"room":"lobby"}}',
                '{"v":1,"type":"request","id":"c2","method":"join","params":{"room":"r2"}}',
            ],
        )

    def test_a_response_without_a_result_is_an_empty_dict(self):
        c = self.connect(self.serve(answer=lambda f: [ok(f["id"])]))
        self.assertEqual(c.request("prune"), {})

    def test_a_result_is_returned_as_is(self):
        c = self.connect(self.serve(answer=lambda f: [ok(f["id"], {"head": 7})]))
        self.assertEqual(c.request("subscribe", room="r"), {"head": 7})

    def test_an_error_response_is_an_api_error_with_code_and_message(self):
        def fail(f):
            return [{"type": "response", "id": f["id"], "ok": False, "error": {"code": "refused", "message": "rule x"}}]

        c = self.connect(self.serve(answer=fail))
        with self.assertRaises(kc.ApiError) as cm:
            c.request("post", room="r", body="b")
        self.assertEqual((cm.exception.code, cm.exception.message, str(cm.exception)), ("refused", "rule x", "refused: rule x"))
        self.assertEqual(cm.exception.refusals, [])

    def test_an_error_response_without_a_code_is_backend(self):
        c = self.connect(self.serve(answer=lambda f: [{"type": "response", "id": f["id"], "ok": False}]))
        with self.assertRaises(kc.ApiError) as cm:
            c.request("x")
        self.assertEqual((cm.exception.code, cm.exception.message), ("backend", ""))

    def test_refusal_frames_before_the_answer_ride_on_the_api_error_and_are_reset_per_request(self):
        refusal = {"type": "refusal", "rule": "scope", "why": "w"}

        def answer(f):
            if f["method"] == "bad":
                return [refusal, {"type": "response", "id": f["id"], "ok": False, "error": {"code": "refused", "message": "m"}}]
            return [ok(f["id"], {})]

        c = self.connect(self.serve(answer=answer))
        with self.assertRaises(kc.ApiError) as cm:
            c.request("bad")
        self.assertEqual([r["rule"] for r in cm.exception.refusals], ["scope"])
        self.assertEqual([r["rule"] for r in c.refusals], ["scope"])
        c.request("fine")
        self.assertEqual(c.refusals, [])

    def test_frames_that_arrive_meanwhile_are_kept_and_noise_is_dropped(self):
        msg = {"type": "message", "room": "r", "seq": 1, "body": "hi"}

        def answer(f):
            return [
                b"\n",
                b"this is not json\n",
                {"type": "heartbeat"},
                {"type": "never-heard-of-it", "extra": 1},
                {"type": "response", "id": "someone-else", "ok": True, "result": {"wrong": True}},
                msg,
                {"type": "caught_up", "room": "r", "head": 1},
                ok(f["id"], {"head": 1}),
            ]

        c = self.connect(self.serve(answer=answer))
        self.assertEqual(c.request("subscribe", room="r", from_seq=0), {"head": 1})
        got = c.collect()
        self.assertEqual(got["messages"], [{"v": 1, **msg}])
        self.assertEqual(got["caught_up"], {"v": 1, "type": "caught_up", "room": "r", "head": 1})
        self.assertEqual(c.collect()["messages"], [])

    def test_a_gateway_that_closes_without_answering_is_a_protocol_error(self):
        d = self.serve(answer=lambda f: [])
        c = self.connect(d)
        threading.Timer(0.3, d.drop_clients).start()
        with self.assertRaises(kc.ProtocolError) as cm:
            c.request("who", room="r")
        self.assertEqual(str(cm.exception), "the gateway closed the session or did not answer")

    def test_a_version_two_frame_surfaces_nothing_to_the_caller_and_does_not_break_the_stream(self):
        def answer(f):
            return [b'{"v":2,"type":"message"}\n', ok(f["id"], {"fine": True})]

        c = self.connect(self.serve(answer=answer))
        self.assertEqual(c.request("who", room="r"), {"fine": True})

    def test_a_request_after_close_is_a_protocol_error_and_nothing_is_sent(self):
        d = self.serve()
        c = self.connect(d)
        c.close()
        with self.assertRaises(kc.ProtocolError) as cm:
            c.request("who", room="r")
        self.assertEqual(str(cm.exception), "the session is closed")
        self.assertEqual(d.received, [])


class Methods(SocketCase):
    """What each method puts on the wire: an omitted optional is left out, a given one is kept."""

    def wire(self, call, result=None):
        d = self.serve(answer=lambda f: [ok(f["id"], result if result is not None else {})])
        c = self.connect(d)
        call(c)
        return d.frames()[-1]

    def test_post_keeps_only_the_given_optionals_in_a_fixed_order(self):
        f = self.wire(lambda c: c.post("lobby", "seeking: x", label="L", scope="s", refs=[{"kind": "pr", "value": "1"}]))
        self.assertEqual(f["method"], "post")
        self.assertEqual(list(f["params"]), ["room", "body", "label", "scope", "refs"])
        f = self.wire(lambda c: c.post("lobby", "b"))
        self.assertEqual(f["params"], {"room": "lobby", "body": "b"})
        f = self.wire(lambda c: c.post("lobby", "b", kind="k", payload="p"))
        self.assertEqual(list(f["params"]), ["room", "body", "kind", "payload"])

    def test_post_keeps_a_falsey_optional_that_is_not_none(self):
        f = self.wire(lambda c: c.post("lobby", "b", label="", refs=[]))
        self.assertEqual(f["params"], {"room": "lobby", "body": "b", "label": "", "refs": []})

    def test_dm_history_claim_release_and_review_verdict_leave_out_none(self):
        self.assertEqual(self.wire(lambda c: c.dm("bob", "hi"))["params"], {"to": "bob", "body": "hi"})
        self.assertEqual(self.wire(lambda c: c.dm("bob", "hi", label="L"))["params"], {"to": "bob", "body": "hi", "label": "L"})
        h = self.wire(lambda c: c.history("r"), {"messages": []})
        self.assertEqual(h["params"], {"room": "r"})
        h = self.wire(lambda c: c.history("r", last=5, since=2), {"messages": []})
        self.assertEqual(list(h["params"]), ["room", "last", "since"])
        self.assertEqual(self.wire(lambda c: c.claim("r", "a/b"))["params"], {"room": "r", "path": "a/b"})
        f = self.wire(lambda c: c.claim("r", "a/b", ttl=0, reason="x", shared=False))
        self.assertEqual(f["params"], {"room": "r", "path": "a/b", "ttl": 0, "reason": "x", "shared": False})
        self.assertEqual(self.wire(lambda c: c.release("r", "p"))["params"], {"room": "r", "path": "p"})
        self.assertEqual(self.wire(lambda c: c.release("r", "p", note="n"))["params"], {"room": "r", "path": "p", "note": "n"})
        f = self.wire(lambda c: c.review_verdict(3, "deny", reason="r"))
        self.assertEqual(f["params"], {"id": 3, "verdict": "deny", "reason": "r"})

    def test_history_defaults_more_and_read_peek_and_mute_and_role_act(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {"messages": [1]})])
        c = self.connect(d)
        self.assertEqual(c.history("r"), {"messages": [1], "more": 0})
        self.assertEqual(c.read("r"), [1])
        self.assertEqual(d.frames()[-1]["params"], {"room": "r", "peek": False})
        c.mute("r", muted=0)
        self.assertEqual(d.frames()[-1]["params"], {"room": "r", "muted": False})
        c.role_act("close", room="r")
        self.assertEqual(d.frames()[-1]["params"], {"act": "close", "room": "r"})

    def test_read_sends_max_messages_and_max_bytes_only_when_given(self):
        f = self.wire(lambda c: c.read("r"), {"messages": []})
        self.assertEqual(f["params"], {"room": "r", "peek": False})
        f = self.wire(lambda c: c.read("r", True, max_messages=3, max_bytes=2000), {"messages": []})
        self.assertEqual(list(f["params"]), ["room", "peek", "max_messages", "max_bytes"])
        self.assertEqual(f["params"], {"room": "r", "peek": True, "max_messages": 3, "max_bytes": 2000})
        f = self.wire(lambda c: c.read("r", max_messages=0), {"messages": []})
        self.assertEqual(f["params"], {"room": "r", "peek": False, "max_messages": 0})

    def test_read_bounded_answers_the_messages_omitted_and_by_importance(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {"messages": [1], "omitted": 4, "by_importance": True})])
        c = self.connect(d)
        self.assertEqual(c.read("r", max_messages=1), [1])
        self.assertEqual(
            c.read_bounded("r", max_messages=1), {"messages": [1], "omitted": 4, "by_importance": True}
        )
        self.assertEqual(d.frames()[-1]["params"], {"room": "r", "peek": False, "max_messages": 1})

    def test_read_bounded_without_a_bound_has_no_omitted_and_no_by_importance(self):
        d = self.serve(answer=lambda f: [ok(f["id"], {"messages": [1]})])
        c = self.connect(d)
        self.assertEqual(c.read_bounded("r", peek=True), {"messages": [1], "omitted": None, "by_importance": False})
        self.assertEqual(d.frames()[-1]["params"], {"room": "r", "peek": True})

    def test_a_bound_the_server_refuses_is_an_api_error_and_was_sent_as_given(self):
        def fail(f):
            return [{"type": "response", "id": f["id"], "ok": False, "error": {"code": "bad_params", "message": "max_messages"}}]

        d = self.serve(answer=fail)
        c = self.connect(d)
        with self.assertRaises(kc.ApiError) as cm:
            c.read("r", max_messages=-1)
        self.assertEqual(cm.exception.code, "bad_params")
        self.assertEqual(d.frames()[-1]["params"]["max_messages"], -1)

    def test_ack_is_a_frame_not_a_request_and_waits_for_no_answer(self):
        d = self.serve()
        c = self.connect(d)
        c.ack("r", 4)
        c.request("who", room="r")
        self.assertEqual(d.received[0], '{"v":1,"type":"ack","room":"r","seq":4}')


class Stream(SocketCase):
    def test_subscribe_returns_the_head_and_collect_waits_for_caught_up(self):
        m1 = {"type": "message", "room": "r", "seq": 1, "body": "a"}
        m2 = {"type": "message", "room": "r", "seq": 2, "body": "b"}

        def answer(f):
            if f["method"] == "subscribe":
                return [ok(f["id"], {"head": 2}), m1, m2, {"type": "caught_up", "room": "r", "head": 2}]
            return [ok(f["id"], {})]

        c = self.connect(self.serve(answer=answer))
        self.assertEqual(c.subscribe("r"), 2)
        got = c.collect(until_caught_up=True, timeout=5)
        self.assertEqual([m["seq"] for m in got["messages"]], [1, 2])
        self.assertEqual(got["caught_up"]["head"], 2)

    def test_collect_without_waiting_returns_what_is_there_and_none_for_caught_up(self):
        c = self.connect(self.serve())
        self.assertEqual(c.collect(), {"messages": [], "caught_up": None})

    def test_collect_until_caught_up_gives_up_at_the_deadline(self):
        c = self.connect(self.serve())
        self.assertEqual(c.collect(until_caught_up=True, timeout=0.2), {"messages": [], "caught_up": None})

    def test_subscribe_forgets_an_earlier_caught_up_of_that_room(self):
        def answer(f):
            if f["method"] == "subscribe":
                return [ok(f["id"], {"head": 0}), {"type": "caught_up", "room": "r", "head": 0}]
            return [ok(f["id"], {})]

        c = self.connect(self.serve(answer=answer))
        c.subscribe("r")
        self.assertIsNotNone(c.collect(until_caught_up=True, timeout=5)["caught_up"])
        c._caught_up.clear()
        c.subscribe("r")
        self.assertIsNotNone(c.collect(until_caught_up=True, timeout=5)["caught_up"])

    def test_next_message_pops_in_order_and_is_none_on_timeout(self):
        def answer(f):
            msgs = [{"type": "message", "room": "r", "seq": n, "body": str(n)} for n in (1, 2)]
            return [ok(f["id"], {"head": 2}), *msgs]

        c = self.connect(self.serve(answer=answer))
        c.subscribe("r")
        self.assertEqual(c.next_message(timeout=5)["seq"], 1)
        self.assertEqual(c.next_message(timeout=5)["seq"], 2)
        self.assertIsNone(c.next_message(timeout=0.2))

    def test_next_message_is_none_when_the_gateway_has_gone(self):
        d = self.serve()
        c = self.connect(d)
        d.drop_clients()
        self.assertIsNone(c.next_message(timeout=5))


class CloseSocket(SocketCase):
    def test_close_ends_the_write_side_so_the_daemon_sees_eof_and_answers_zero(self):
        d = self.serve()
        c = kc.Client.connect_socket(d.path)
        self.assertEqual(c.close(), 0)
        self.assertTrue(d.eof.wait(5))

    def test_close_twice_is_harmless_and_answers_zero_again(self):
        c = kc.Client.connect_socket(self.serve().path)
        self.assertEqual(c.close(), 0)
        self.assertEqual(c.close(), 0)

    def test_close_returns_once_the_daemon_ends_its_side(self):
        d = self.serve()
        c = kc.Client.connect_socket(d.path)
        c.request("who", room="r")
        self.assertEqual(c.close(), 0)
        self.assertFalse(c._pumper.is_alive())

    def test_close_after_the_daemon_dropped_the_session_still_answers_zero(self):
        d = self.serve()
        c = kc.Client.connect_socket(d.path)
        d.drop_clients()
        self.assertEqual(c.close(), 0)


GATEWAY = r"""
import json, sys
def out(f):
    sys.stdout.write(json.dumps({"v": 1, **f}, separators=(",", ":")) + "\n"); sys.stdout.flush()
out({"type": "hello", "member": "p-1", "attribution": "session", "clearance": "public"})
for line in sys.stdin:
    f = json.loads(line)
    out({"type": "response", "id": f["id"], "ok": True, "result": {"method": f["method"], "args": sys.argv[1:]}})
sys.exit(3)
"""


class Stdio(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="kc-gw-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.script = os.path.join(self.dir, "gateway.py")
        with open(self.script, "w") as f:
            f.write(GATEWAY)

    def client(self):
        return kc.Client(sys.executable, args=(self.script, "a", "b"))

    def test_the_process_hello_request_and_close_return_the_exit_code(self):
        c = self.client()
        self.assertEqual((c.member, c.attribution, c.clearance), ("p-1", "session", "public"))
        self.assertEqual(c.request("who", room="r"), {"method": "who", "args": ["a", "b"]})
        self.assertEqual(c.close(), 3)

    def test_close_twice_answers_the_same_exit_code(self):
        c = self.client()
        self.assertEqual(c.close(), 3)
        self.assertEqual(c.close(), 3)

    def test_a_process_that_exits_at_once_fails_the_hello(self):
        with open(self.script, "w") as f:
            f.write("import sys\nsys.exit(0)\n")
        with self.assertRaises(kc.ProtocolError):
            self.client()


if __name__ == "__main__":
    unittest.main()
