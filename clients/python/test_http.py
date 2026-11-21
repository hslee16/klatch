"""The HTTP and server-sent-events transport of the Python client (#1114) for `klatch-server http` (#1059).

The success path runs against a small in-process server that speaks the same wire (`GET /v1/stream` as server-sent
events with `X-Klatch-Session`, `POST /v1/frames` with that header), because it can drop a stream
on demand. The whole conformance scenario over the real `klatch-server http`, with a credential issued per member in a
throwaway store, is `ConformanceOverHttp` in test_conformance.py (#1123). The refusal path runs against the real server here.
Every wait has a deadline, no test reads a stream to its end, and every error is injected, not provoked from the
platform (a peer that dies is ECONNRESET on Linux and a clean end on macOS).
"""
import json
import os
import queue
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import klatch_client as kc

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER_BIN = os.environ.get("KLATCH_SERVER_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch-server")
TOKEN = "tok-secret-1"
WAIT = 5


def event(frame):
    kind = frame["type"]
    head = f"event: {kind}\n"
    if kind == "message":
        head += f"id: {frame['room']}:{frame['seq']}\n"
    return (head + "data: " + json.dumps(frame, separators=(",", ":")) + "\n\n").encode()


class FakeServer:
    """The wire of `klatch-server http`, with one member: records what it was sent, keeps rooms in memory."""

    def __init__(self, member="w-1", token=TOKEN):
        self.member, self.token = member, token
        self.lock = threading.Lock()
        self.rooms = {}  # room -> list of message frames
        self.sessions = {}  # id -> {"out": Queue, "subs": set}
        self.requests = []  # (method, path, headers dict, body)
        self.posts_refused = 0  # answer this many posts with 429 before accepting
        self.opened = 0
        outer = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *a):
                pass

            def authorized(self):
                return (
                    self.headers.get("Authorization") == f"Bearer {outer.token}"
                    and self.headers.get("X-Klatch-Member") == outer.member
                )

            def reply(self, status, body=b""):
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                outer.requests.append(("GET", self.path, dict(self.headers), b""))
                if self.path != "/v1/stream" or not self.authorized():
                    return self.reply(401, b"unauthorized")
                with outer.lock:
                    outer.opened += 1
                    sid = f"s{outer.opened}"
                    out = queue.Queue()
                    outer.sessions[sid] = {"out": out, "subs": set()}
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("X-Klatch-Session", sid)
                self.end_headers()
                self.wfile.write(event({"v": 1, "type": "hello", "member": outer.member, "attribution": "verified", "clearance": "public"}))
                self.wfile.write(b": a comment line the client ignores\n\n")
                self.wfile.flush()
                try:
                    while True:
                        item = out.get(timeout=30)
                        if item is None:
                            return
                        self.wfile.write(item)
                        self.wfile.flush()
                except (OSError, queue.Empty):
                    pass
                finally:
                    with outer.lock:
                        outer.sessions.pop(sid, None)

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                outer.requests.append(("POST", self.path, dict(self.headers), body))
                if self.path != "/v1/frames" or not self.authorized():
                    return self.reply(401, b"unauthorized")
                session = outer.sessions.get(self.headers.get("X-Klatch-Session"))
                if session is None:
                    return self.reply(404, b"no such session")
                if outer.posts_refused > 0:
                    outer.posts_refused -= 1
                    return self.reply(429, b"the session is busy; retry")
                for line in body.decode().splitlines():
                    outer.handle(session, json.loads(line))
                self.reply(202)

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.httpd.daemon_threads = True
        self.port = self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.port}"

    def handle(self, session, frame):
        if frame["type"] != "request":
            return
        method, params, rid = frame["method"], frame["params"], frame["id"]
        result = {}
        replay = []
        with self.lock:
            if method == "post":
                msgs = self.rooms.setdefault(params["room"], [])
                msgs.append(self.message(params["room"], len(msgs) + 1, params["body"]))
                result = {"seq": len(msgs), "id": f"m{len(msgs)}"}
                for other in self.sessions.values():
                    if params["room"] in other["subs"]:
                        other["out"].put(event(msgs[-1]))
            elif method == "subscribe":
                room = params["room"]
                session["subs"].add(room)
                msgs = self.rooms.setdefault(room, [])
                result = {"head": len(msgs)}
                replay = [m for m in msgs if m["seq"] > params["from_seq"]]
            elif method == "unsubscribe":
                session["subs"].discard(params["room"])
        session["out"].put(event({"v": 1, "type": "response", "id": rid, "ok": True, "result": result}))
        if method == "subscribe":
            for m in replay:
                session["out"].put(event(m))
            session["out"].put(event({"v": 1, "type": "caught_up", "room": params["room"], "head": result["head"]}))

    @staticmethod
    def message(room, seq, body):
        return {"v": 1, "type": "message", "room": room, "seq": seq, "id": f"m{seq}", "sender": "other", "attribution": "verified", "label": "public", "body": body, "ts": seq}

    def say(self, room, body):
        """A message from someone else, delivered to the subscribed sessions."""
        with self.lock:
            msgs = self.rooms.setdefault(room, [])
            msgs.append(self.message(room, len(msgs) + 1, body))
            for s in self.sessions.values():
                if room in s["subs"]:
                    s["out"].put(event(msgs[-1]))

    def drop_streams(self):
        """End every open stream the way a peer that goes away does (a clean end of the body)."""
        with self.lock:
            for s in self.sessions.values():
                s["out"].put(None)

    def subscribes(self):
        out = []
        for method, path, _h, body in self.requests:
            if method == "POST":
                for line in body.decode().splitlines():
                    f = json.loads(line)
                    if f.get("method") == "subscribe":
                        out.append(f["params"])
        return out

    def close(self):
        self.drop_streams()
        self.httpd.shutdown()
        self.httpd.server_close()


class HttpClient(unittest.TestCase):
    def setUp(self):
        self.server = FakeServer()
        self.addCleanup(self.server.close)
        self.clients = []
        self.addCleanup(lambda: [c.close() for c in self.clients])

    def connect(self, **kw):
        c = kc.Client.connect_http(self.server.url, member="w-1", token=TOKEN, **kw)
        self.clients.append(c)
        return c

    def test_the_hello_is_read_first_and_who_you_are_comes_from_the_credential(self):
        c = self.connect()
        self.assertEqual((c.member, c.attribution, c.clearance), ("w-1", "verified", "public"))
        get = self.server.requests[0]
        self.assertEqual((get[0], get[1]), ("GET", "/v1/stream"))
        self.assertEqual(get[2]["Authorization"], f"Bearer {TOKEN}")
        self.assertEqual(get[2]["X-Klatch-Member"], "w-1")

    def test_a_request_is_a_post_with_the_session_header_and_its_response_comes_on_the_stream(self):
        c = self.connect()
        self.assertEqual(c.post("lobby", "hello")["seq"], 1)
        post = [r for r in self.server.requests if r[0] == "POST"][0]
        self.assertEqual(post[1], "/v1/frames")
        self.assertEqual(post[2]["X-Klatch-Session"], "s1")
        self.assertEqual(post[2]["Authorization"], f"Bearer {TOKEN}")
        frame = json.loads(post[3])
        self.assertEqual((frame["v"], frame["type"], frame["method"]), (1, "request", "post"))

    def test_history_then_live_messages_arrive_with_the_servers_sequence_numbers(self):
        self.server.say("lobby", "one")
        self.server.say("lobby", "two")
        c = self.connect()
        c.subscribe("lobby", from_seq=0)
        got = c.collect(until_caught_up=True, timeout=WAIT)
        self.assertEqual([m["seq"] for m in got["messages"]], [1, 2])
        self.assertIsNotNone(got["caught_up"])
        self.server.say("lobby", "three")
        self.assertEqual(c.next_message(timeout=WAIT)["seq"], 3)

    def test_a_comment_line_and_unknown_fields_are_ignored(self):
        c = self.connect()
        self.assertEqual(c.post("lobby", "x")["seq"], 1)  # the comment line after hello did not break the stream

    def test_a_dropped_stream_reconnects_and_resubscribes_from_the_last_seq_without_a_repeat(self):
        c = self.connect()
        c.subscribe("lobby", from_seq=0)
        c.post("lobby", "one")
        c.post("lobby", "two")
        self.assertEqual([c.next_message(timeout=WAIT)["seq"] for _ in range(2)], [1, 2])
        self.server.drop_streams()
        deadline = time.monotonic() + WAIT
        while len(self.server.subscribes()) < 2 and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.server.subscribes()[1], {"room": "lobby", "from_seq": 2})
        self.assertEqual(self.server.opened, 2)
        self.server.say("lobby", "three")
        self.assertEqual(c.next_message(timeout=WAIT)["seq"], 3)
        self.assertIsNone(c.next_message(timeout=0.3))  # nothing replayed twice
        self.assertEqual(c.post("lobby", "four")["seq"], 4)  # requests use the new session

    def test_an_unsubscribed_room_is_not_resubscribed(self):
        c = self.connect()
        c.subscribe("lobby", from_seq=0)
        c.unsubscribe("lobby")
        self.server.drop_streams()
        deadline = time.monotonic() + WAIT
        while self.server.opened < 2 and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(self.server.opened, 2)
        self.assertEqual(len(self.server.subscribes()), 1)

    def test_an_error_injected_into_the_reader_is_a_dropped_stream_not_a_crash(self):
        raised = []
        old = threading.excepthook
        threading.excepthook = lambda args: raised.append(args)
        self.addCleanup(setattr, threading, "excepthook", old)
        c = self.connect()
        c.subscribe("lobby", from_seq=0)
        c.post("lobby", "one")
        self.assertEqual(c.next_message(timeout=WAIT)["seq"], 1)

        # the reader is blocked in the real stream; the next line it gets is followed by an injected reset
        c._http.fail_next_read(ConnectionResetError(104, "Connection reset by peer"))
        self.server.say("lobby", "two")  # half of this event is read, then the reset: it is replayed from seq 1
        self.assertEqual(c.next_message(timeout=WAIT)["seq"], 2)
        self.assertEqual(self.server.opened, 2)
        self.assertEqual(self.server.subscribes()[1], {"room": "lobby", "from_seq": 1})
        self.assertIsNone(c.next_message(timeout=0.3))
        self.assertEqual(raised, [])

    def test_a_server_that_stays_gone_ends_the_session_with_a_protocol_error(self):
        c = self.connect(reconnect_attempts=2, reconnect_delay=0.01)
        self.server.close()
        with self.assertRaises(kc.ProtocolError):
            c.request("post", room="lobby", body="x")

    def test_a_busy_session_is_retried_and_an_ended_one_is_a_protocol_error(self):
        c = self.connect()
        self.server.posts_refused = 2
        self.assertEqual(c.post("lobby", "x")["seq"], 1)
        with self.server.lock:
            self.server.sessions.clear()  # the session has ended on the server
        with self.assertRaises(kc.ProtocolError):
            c._send({"type": "ack", "room": "lobby", "seq": 1})

    def test_a_refused_credential_is_identity_refused_and_never_repeats_the_token(self):
        with self.assertRaises(kc.ApiError) as ctx:
            kc.Client.connect_http(self.server.url, member="w-1", token="wrong-token-value")
        self.assertEqual(ctx.exception.code, "identity.refused")
        self.assertNotIn("wrong-token-value", str(ctx.exception))
        with self.assertRaises(kc.ApiError) as ctx:
            kc.Client.connect_http(self.server.url, member="someone-else", token=TOKEN)
        self.assertEqual(ctx.exception.code, "identity.refused")
        self.assertNotIn(TOKEN, str(ctx.exception))

    def test_a_member_without_a_token_or_the_reverse_is_refused_before_anything_is_sent(self):
        for kw in ({"member": "w-1"}, {"token": "t"}, {}):
            with self.assertRaises(ValueError):
                kc.Client.connect_http(self.server.url, **kw)
        self.assertEqual(self.server.requests, [])

    def test_only_a_loopback_http_url_is_accepted(self):
        for url in ("https://127.0.0.1:1", "http://example.com:80", "ftp://127.0.0.1:1", "127.0.0.1:1"):
            with self.assertRaises(ValueError, msg=url):
                kc.Client.connect_http(url, member="w-1", token=TOKEN)
        self.assertEqual(self.server.requests, [])

    def test_a_port_nobody_listens_on_is_an_os_error(self):
        port = self.server.port
        self.server.close()
        with self.assertRaises(OSError):
            kc.Client.connect_http(f"http://127.0.0.1:{port}", member="w-1", token=TOKEN, connect_timeout=2)

    def test_close_ends_the_session_and_answers_zero(self):
        c = self.connect()
        self.assertEqual(c.close(), 0)
        deadline = time.monotonic() + WAIT
        while self.server.sessions and time.monotonic() < deadline:
            self.server.drop_streams()
            time.sleep(0.01)
        self.assertEqual(self.server.sessions, {})
        self.assertEqual(self.server.opened, 1)  # a close is not a reconnect
        self.assertEqual(c.close(), 0)


class RealServer(unittest.TestCase):
    """The refusal path against the real `klatch-server http`: every failure is one 401 and the library says so."""

    @unittest.skipUnless(os.path.exists(SERVER_BIN), "klatch-server is not built")
    def test_a_credential_the_store_does_not_know_is_identity_refused(self):
        home = tempfile.mkdtemp(prefix="klatch-http-")
        log = tempfile.TemporaryFile("w+")
        env = {"PATH": "/usr/bin:/bin", "HOME": home, "KLATCH_HOME": home, "KLATCH_KEY_DIR": os.path.join(home, "keys")}
        proc = subprocess.Popen([SERVER_BIN, "http", "--listen", "127.0.0.1:0"], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log)
        self.addCleanup(shutil.rmtree, home, True)
        self.addCleanup(proc.wait)
        self.addCleanup(proc.send_signal, signal.SIGKILL)
        port = None
        until = time.monotonic() + 30
        while port is None and time.monotonic() < until:
            log.seek(0)
            m = re.search(r"http listening on 127\.0\.0\.1:(\d+)", log.read())
            if m:
                port = int(m.group(1))
            else:
                time.sleep(0.02)
        self.assertIsNotNone(port, "the server never listened")
        with self.assertRaises(kc.ApiError) as ctx:
            kc.Client.connect_http(f"http://127.0.0.1:{port}", member="w-1", token="not-a-live-token", connect_timeout=10)
        self.assertEqual(ctx.exception.code, "identity.refused")
        self.assertNotIn("not-a-live-token", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
