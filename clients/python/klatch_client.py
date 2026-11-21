"""A thin klatch client over the wire protocol (PROTOCOL.md, version 1): it speaks newline-delimited JSON to
`klatch serve --stdio`, over the Unix socket of the klatch-server daemon, or over HTTP and server-sent events to
`klatch-server http`, standard library only.

    c = Client("/path/to/klatch", env={"KLATCH_NAME": "alice", ...})
    c = Client.connect_socket("/path/to/daemon.sock", name="alice")   # or member="w-1", token="..."
    c = Client.connect_http("http://127.0.0.1:7777", member="w-1", token="...")   # loopback only
    c.post("lobby", "hello")
    c.subscribe("lobby", from_seq=0); c.collect(until_caught_up=True)

Over stdio, who you are is decided by the transport (the environment of the process), never by a request parameter.
Over the daemon's socket the operating system reports the peer, and the first request, `identify`, says more (#102):
a typed name (unattributed), or a member and its credential token (verified), or nothing (`unmapped-process`). A
frame this version does not know, and a field it does not know, are ignored, as the protocol requires.
"""
import http.client
import json
import queue
import socket
import subprocess
import threading
import time
import urllib.parse

PROTOCOL_VERSION = 1


class ProtocolError(Exception):
    """A frame that is not version 1 of the protocol."""


class ApiError(Exception):
    """A failed response. `refusals` are the refusal frames that came with it (a policy refused the call)."""

    def __init__(self, code, message, refusals=None):
        super().__init__(f"{code}: {message}")
        self.code, self.message, self.refusals = code, message, refusals or []


def decode_server_frame(line):
    """One line from the gateway as a dict, or None for a blank line. An unknown type or field is not an error; a
    line that is not a JSON object with `"v":1` and a `type` is."""
    if not line.strip():
        return None
    try:
        frame = json.loads(line)
    except ValueError as e:
        raise ProtocolError(f"not JSON: {e}") from None
    if not isinstance(frame, dict):
        raise ProtocolError("a frame is a JSON object")
    v = frame.get("v")
    if isinstance(v, bool) or not isinstance(v, int) or v != PROTOCOL_VERSION:
        raise ProtocolError(f"protocol version {v!r} is not supported; this client speaks {PROTOCOL_VERSION}")
    if not isinstance(frame.get("type"), str):
        raise ProtocolError("a frame has a type")
    return frame


def encode_line(frame):
    return json.dumps({"v": PROTOCOL_VERSION, **frame}, separators=(",", ":")) + "\n"


class _HttpTransport:
    """`klatch-server http` as a reader and a writer for `Client._start` (#1114). The reader opens `GET /v1/stream`
    with the credential, reads the `X-Klatch-Session` header, and yields the `data:` of each server-sent event, which is
    one frame line exactly. The writer POSTs each line to `/v1/frames` with that header; the answer comes on the
    stream, as it does on the daemon's connection. When the stream ends without a `close()` (the server went away, a
    reset), the reader opens a new one, drops the `hello` of the new session and resubscribes every room that was
    subscribed from the last `seq` seen in it, so the caller sees no gap and no repeat."""

    def __init__(self, host, port, member, token, connect_timeout, reconnect_attempts, reconnect_delay):
        self.host, self.port, self.member, self.token = host, port, member, token
        self.connect_timeout = connect_timeout
        self.reconnect_attempts, self.reconnect_delay = reconnect_attempts, reconnect_delay
        self.closed = False
        self.session = None
        self._conn = None
        self._stream = None
        self._lock = threading.Lock()
        self.hello_wanted = True
        self._fail = None  # an error to raise on the next read (a test hook: the platform's reset is not portable)
        self._subs = {}  # room -> the from_seq to resume from
        self._rid = 0
        self.reader = _HttpReader(self)
        self.writer = _HttpWriter(self)

    def _headers(self):
        return {"Authorization": f"Bearer {self.token}", "X-Klatch-Member": self.member}

    def open_stream(self):
        """Open a session. Raises ApiError (`identity.refused`) for a 401, OSError when nothing listens."""
        conn = http.client.HTTPConnection(self.host, self.port, timeout=self.connect_timeout)
        try:
            conn.request("GET", "/v1/stream", headers={**self._headers(), "Accept": "text/event-stream"})
            sock = conn.sock  # the response closes the connection's reference to it; close() needs the socket itself
            resp = conn.getresponse()
            if resp.status == 401:
                raise ApiError("identity.refused", "the server refused the credential")
            if resp.status != 200:
                raise OSError(f"the server answered {resp.status} to the stream")
            session = resp.getheader("X-Klatch-Session")
            if not session:
                raise ProtocolError("the stream has no X-Klatch-Session header")
            sock.settimeout(None)  # heartbeats keep a healthy stream busy; close() unblocks the read
        except BaseException:
            conn.close()
            raise
        with self._lock:
            self._conn, self._stream, self.session = (conn, sock), resp, session

    def fail_next_read(self, error):
        self._fail = error

    def readline(self):
        error, self._fail = self._fail, None
        if error is not None:
            raise error
        stream = self._stream
        return stream.readline() if stream is not None else b""

    def post(self, text):
        body = text.encode("utf-8")
        for attempt in range(40):
            conn = http.client.HTTPConnection(self.host, self.port, timeout=self.connect_timeout)
            try:
                conn.request(
                    "POST",
                    "/v1/frames",
                    body=body,
                    headers={**self._headers(), "X-Klatch-Session": self.session or "", "Content-Type": "application/x-ndjson"},
                )
                resp = conn.getresponse()
                resp.read()
                status = resp.status
            finally:
                conn.close()
            if status == 202:
                return
            if status == 429 and attempt < 39 and not self.closed:
                time.sleep(0.05)  # the session's inbox is full: it drains, so try again
                continue
            raise OSError(f"the server answered {status} to a post")

    def note_sent(self, line):
        """Remember what the session subscribes to, to resubscribe after a reconnect."""
        try:
            frame = json.loads(line)
        except ValueError:
            return
        if not isinstance(frame, dict) or frame.get("type") != "request":
            return
        params = frame.get("params") or {}
        room = params.get("room")
        if frame.get("method") == "subscribe" and isinstance(room, str):
            self._subs[room] = params.get("from_seq", 0)
        elif frame.get("method") == "unsubscribe":
            self._subs.pop(room, None)

    def note_seen(self, frame):
        if frame.get("type") == "message" and frame.get("room") in self._subs and isinstance(frame.get("seq"), int):
            self._subs[frame["room"]] = frame["seq"]

    def reconnect(self):
        """A new session in place of one that ended; True when it is open and its rooms are resubscribed."""
        self._release()
        for attempt in range(self.reconnect_attempts):
            if self.closed:
                return False
            try:
                self.open_stream()
                break
            except (OSError, ProtocolError, ApiError):
                time.sleep(self.reconnect_delay * (attempt + 1))
        else:
            return False
        return True

    def resubscribe(self):
        for room, seq in list(self._subs.items()):
            self._rid += 1
            line = encode_line({"type": "request", "id": f"r{self._rid}", "method": "subscribe", "params": {"room": room, "from_seq": seq}})
            try:
                self.post(line)
            except OSError:
                return

    def _release(self):
        with self._lock:
            conn, self._conn, self._stream = self._conn, None, None
        if conn is not None:
            conn, sock = conn
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.close()

    def shut(self):
        self.closed = True
        self._release()


class _HttpWriter:
    def __init__(self, transport):
        self._t = transport

    def write(self, text):
        self._t.note_sent(text)
        self._t.post(text)  # an OSError here is the caller's ProtocolError (`Client._send`)

    def flush(self):
        pass

    def close(self):
        pass


class _HttpReader:
    """Iterates the frame lines of the stream across reconnects."""

    def __init__(self, transport):
        self._t = transport

    def __iter__(self):
        t = self._t
        data = []
        first = True  # the first frame of a session is its hello; only the first session's is wanted
        while not t.closed:
            try:
                raw = t.readline()
            except (OSError, ValueError, http.client.HTTPException):
                raw = b""
            if raw:
                line = raw.decode("utf-8", "replace").rstrip("\r\n")
                if line == "":
                    if data:
                        frame_line, data = "\n".join(data), []
                        hello = first and self._is_hello(frame_line)
                        first = False
                        if hello and not t.hello_wanted:
                            continue  # a reconnect: the caller has its hello
                        self._note(frame_line)
                        yield frame_line
                elif line.startswith("data:"):
                    data.append(line[6:] if line[5:6] == " " else line[5:])
                continue
            # the stream ended
            if t.closed or not t.reconnect():
                return
            data, first = [], True
            t.hello_wanted = False
            t.resubscribe()

    @staticmethod
    def _is_hello(line):
        try:
            return json.loads(line).get("type") == "hello"
        except (ValueError, AttributeError):
            return False

    def _note(self, frame_line):
        try:
            frame = json.loads(frame_line)
        except ValueError:
            return
        if isinstance(frame, dict):
            self._t.note_seen(frame)

    def close(self):
        pass


class Client:
    """One session with the gateway. Not thread safe: use one client per thread."""

    def __init__(self, binary, env=None, args=("serve", "--stdio")):
        proc = subprocess.Popen(
            [binary, *args],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self._start(proc.stdout, proc.stdin, proc=proc)

    @classmethod
    def connect_socket(cls, path, name=None, member=None, token=None):
        """Connect to a `klatch-server daemon --socket PATH` and say who you are, as the daemon's `identify` first
        request: a typed `name` (unattributed), or a `member` and its credential `token` (verified; the token only,
        never the key bundle file), or neither (`unmapped-process`). A refused identity raises `ApiError` with code
        `identity.refused` (its message never repeats the name or the token); a path nobody listens on raises
        `OSError`. `close()` shuts the socket and waits for the daemon to end the session; it returns 0."""
        if name is not None and (member is not None or token is not None):
            raise ValueError("give a name, or a member and a token, not both")
        if (member is None) != (token is None):
            raise ValueError("a credential is a member and a token")
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.connect(path)
        except OSError:
            sock.close()
            raise
        self = cls.__new__(cls)
        self._sock = sock
        reader = sock.makefile("r", encoding="utf-8", newline="\n")
        writer = sock.makefile("w", encoding="utf-8", newline="\n")
        self._start(reader, writer, proc=None)
        try:
            params = {}
            if name is not None:
                params["name"] = name
            elif member is not None:
                params.update(member=member, token=token)
            if params:
                answer = self.request("identify", **params)
                self.member = answer.get("member", self.member)
                self.attribution = answer.get("attribution", self.attribution)
        except BaseException:
            self.close()
            raise
        return self

    @classmethod
    def connect_http(cls, url, member=None, token=None, connect_timeout=10, reconnect_attempts=5, reconnect_delay=0.2):
        """Connect to a `klatch-server http --listen ADDR` at `url` (`http://127.0.0.1:PORT`, loopback only) as `member`
        with its credential `token`, the only way to be known over HTTP (a worker credential the human issued). The
        stream is `GET /v1/stream` and each request is a `POST /v1/frames` carrying `X-Klatch-Session`. A refused
        credential raises `ApiError` with code `identity.refused` (its message never repeats the token); a server
        nobody listens on raises `OSError`; a url that is not plain `http` to a loopback address raises `ValueError`.
        If the stream ends without a `close()`, it is reopened up to `reconnect_attempts` times and every subscribed
        room is resubscribed from the last `seq` seen, so messages resume without a gap or a repeat. `close()` ends
        the session and returns 0."""
        if member is None or token is None:
            raise ValueError("a credential is a member and a token")
        parts = urllib.parse.urlsplit(url)
        if parts.scheme != "http" or parts.hostname not in ("127.0.0.1", "::1", "localhost") or parts.port is None:
            raise ValueError("the url is http://127.0.0.1:PORT (plain http, loopback only)")
        transport = _HttpTransport(parts.hostname, parts.port, member, token, connect_timeout, reconnect_attempts, reconnect_delay)
        transport.open_stream()
        self = cls.__new__(cls)
        self._sock = None
        self._http = transport
        try:
            self._start(transport.reader, transport.writer, proc=None)
        except BaseException:
            self.close()
            raise
        return self

    def _start(self, reader, writer, proc):
        self._proc = proc
        self._sock = getattr(self, "_sock", None)
        self._http = getattr(self, "_http", None)
        self._reader, self._writer = reader, writer
        self._frames = queue.Queue()
        self._next_id = 0
        self.refusals = []
        self._pending_messages = []
        self._caught_up = {}
        self._pumper = threading.Thread(target=self._pump, daemon=True)
        self._pumper.start()
        hello = self._frame(timeout=10)
        if hello is None or hello.get("type") != "hello":
            raise ProtocolError("the first frame is hello")
        self.member = hello["member"]
        self.attribution = hello["attribution"]
        self.clearance = hello["clearance"]

    # ---- reading ----

    def _pump(self):
        try:
            self._pump_lines()
        except (OSError, ValueError):
            pass  # the transport was closed under the reader
        self._frames.put(None)

    def _pump_lines(self):
        for line in self._reader:
            try:
                frame = decode_server_frame(line)
            except ProtocolError as e:
                frame = {"type": "_error", "error": str(e)}
            if frame is not None:
                self._frames.put(frame)

    def _frame(self, timeout):
        try:
            return self._frames.get(timeout=timeout)
        except queue.Empty:
            return None

    def _note(self, frame):
        """Keep what a response is not: messages, caught-up marks, refusals. Heartbeats and unknown types go."""
        kind = frame.get("type")
        if kind == "message":
            self._pending_messages.append(frame)
        elif kind == "caught_up":
            self._caught_up[frame["room"]] = frame
        elif kind == "refusal":
            self.refusals.append(frame)

    # ---- requests ----

    def request(self, method, **params):
        """Send a request and return its result, or raise ApiError. Frames that arrive meanwhile are kept."""
        self._next_id += 1
        rid = f"c{self._next_id}"
        self.refusals = []
        self._send({"type": "request", "id": rid, "method": method, "params": params})
        while True:
            frame = self._frame(timeout=30)
            if frame is None:
                raise ProtocolError("the gateway closed the session or did not answer")
            if frame.get("type") == "response" and frame.get("id") == rid:
                if frame.get("ok"):
                    return frame.get("result") or {}
                err = frame.get("error") or {}
                raise ApiError(err.get("code", "backend"), err.get("message", ""), list(self.refusals))
            self._note(frame)

    def _send(self, frame):
        try:
            self._writer.write(encode_line(frame))
            self._writer.flush()
        except (OSError, ValueError) as e:
            # a dead peer or a closed stream: one error type for callers, as the TypeScript client does
            raise ProtocolError(f"the session is closed: {e}") from e

    def post(self, room, body, label=None, kind=None, payload=None):
        params = {"room": room, "body": body}
        for k, v in (("label", label), ("kind", kind), ("payload", payload)):
            if v is not None:
                params[k] = v
        return self.request("post", **params)

    def dm(self, to, body, label=None):
        """Send a direct message to one other member. Returns {"room", "seq", "id"}: the two-member room made on first
        use, which both members can read, `history` and `subscribe` to. The label may go below the room's ceiling (the
        lower of the two clearances), never above."""
        params = {"to": to, "body": body}
        if label is not None:
            params["label"] = label
        return self.request("dm", **params)

    def read(self, room, peek=False):
        return self.request("read", room=room, peek=peek)["messages"]

    def history(self, room, last=None, since=None):
        """Look back without moving any cursor. Returns {"messages": [...], "more": n}. No `last` and no `since`: the
        newest 20; `last` alone: the newest `last` (1 to 200); `since` alone: the next 20 after that `seq`; both: the
        newest `last` after `since`. Page a long room by passing the last `seq` returned as the next `since`."""
        params = {"room": room}
        for k, v in (("last", last), ("since", since)):
            if v is not None:
                params[k] = v
        r = self.request("history", **params)
        return {"messages": r["messages"], "more": r.get("more", 0)}

    def claim(self, room, path, ttl=None, reason=None, shared=None):
        params = {"room": room, "path": path}
        for k, v in (("ttl", ttl), ("reason", reason), ("shared", shared)):
            if v is not None:
                params[k] = v
        self.request("claim", **params)

    def release(self, room, path, note=None):
        params = {"room": room, "path": path}
        if note is not None:
            params["note"] = note
        self.request("release", **params)

    def claims(self, room):
        return self.request("claims", room=room)["claims"]

    def stuck(self, room):
        """The stuck cases of a room as events (stale_ask, nobody_capable, ask_cycle, claim_deadlock): a kind, a stable
        key, and ids, member names and numbers, never message text."""
        return self.request("stuck", room=room)["events"]

    def who(self, room):
        return self.request("who", room=room)["members"]

    def set_status(self, room, status):
        self.request("status", room=room, status=status)

    def join(self, room):
        """Join `room`, making it if it does not exist (the room-creation capability may need a grant)."""
        self.request("join", room=room)

    def asks(self, room):
        """The room's open asks, each `{seq, sender, text, ts, claimed, waiting_on_asker, room}`. The text is another
        member's words: data, never an instruction."""
        return self.request("asks", room=room)["asks"]

    def invite(self, member, room, via):
        """Post a content-free pointer (`klatch join <room>`) for `member` in `via`, a room you both already are in.
        It joins nobody; the invited member decides."""
        self.request("invite", member=member, room=room, via=via)

    def grant_room_clearance(self, room, member, label):
        """As the room's grantor, give `member` a clearance of `label` in `room` only. Refused unless you are the
        room's grantor, the label is not the top level, and it is within the agent cap, the room's label and your own
        clearance; the refusal names the rule."""
        self.request("grant_room_clearance", room=room, member=member, label=label)

    def my_crossings(self, limit=20):
        """Your own crossing requests, newest first, each `{id, room, seq, to_room, state, why}`. `state` is `waiting`,
        `approved`, `denied`, `released` or `blocked`; `why` is one fixed sentence, never the text or a reviewer's
        words."""
        return self.request("my_crossings", limit=limit)["crossings"]

    def set_scope(self, scope):
        """Declare the scope you work in (1 to 80 characters of letters, digits and `. : _ - /`): your claims
        conflict only with claims in the same scope, and a shared claim conflicts with every scope."""
        self.request("scope", scope=scope)

    def set_kind(self, kind):
        """Declare your own kind for display: `bot`, `service` or `human`; None withdraws it. `session` is never
        declared: klatch derives it."""
        self.request("kind", kind=kind)

    def review_next(self):
        """The reviewer's queue: the next held item as {"id", "room", "seq", "to_label", "reason", "text"}, or None
        when there is none. Only the reviewer role may call it (`refused`, rule `reviewer`)."""
        return self.request("review_next").get("item")

    def review_verdict(self, item_id, verdict, text=None, reason=None):
        """The reviewer's verdict on an item: `allow`, `redact` (with `text`) or `deny` (with an optional `reason`).
        Only the reviewer role may call it."""
        params = {"id": item_id, "verdict": verdict}
        for k, v in (("text", text), ("reason", reason)):
            if v is not None:
                params[k] = v
        self.request("review_verdict", **params)

    def role_act(self, act, **params):
        """An act of the admin agent (`close`, `reopen`, `remove`, `allow`, `clear`, `label`, `audit`, `refusals`,
        `run-create`, `run-end`, `anchor-done`) with what it needs (`room`, `member`, `label`, `limit`, `run`,
        `anchor`). A member without the role is refused (`ApiError` code `refused`, rule `admin-scope`). Returns the
        result object."""
        return self.request("role_act", act=act, **params)

    def prune(self):
        """Housekeeping every member may run: remove the members not seen for the fixed retention. Returns how many."""
        return self.request("prune").get("removed", 0)

    def mute(self, room, muted=True):
        """Mute (or, with muted=False, unmute) a room for yourself: your digest then shows only messages that mention
        you. `read` and the stream still carry everything."""
        self.request("mute", room=room, muted=bool(muted))

    def subscribe(self, room, from_seq=0):
        """Start (or resume) a stream; returns the room's head. Messages arrive through `collect` and `next_message`."""
        self._caught_up.pop(room, None)
        return self.request("subscribe", room=room, from_seq=from_seq)["head"]

    def unsubscribe(self, room):
        self.request("unsubscribe", room=room)

    def ack(self, room, seq):
        self._send({"type": "ack", "room": room, "seq": seq})

    # ---- the stream ----

    def collect(self, until_caught_up=False, timeout=10):
        """Messages received so far; with `until_caught_up`, wait for the replay to finish first."""
        import time

        end = time.monotonic() + timeout
        while until_caught_up and not self._caught_up and time.monotonic() < end:
            frame = self._frame(timeout=max(0.0, end - time.monotonic()))
            if frame is None:
                break
            self._note(frame)
        messages, self._pending_messages = self._pending_messages, []
        caught_up = next(iter(self._caught_up.values()), None)
        return {"messages": messages, "caught_up": caught_up}

    def next_message(self, timeout=10):
        """The next message of a subscription, waiting for it; None on timeout."""
        import time

        end = time.monotonic() + timeout
        while True:
            if self._pending_messages:
                return self._pending_messages.pop(0)
            frame = self._frame(timeout=max(0.0, end - time.monotonic()))
            if frame is None:
                return None
            self._note(frame)

    def close(self):
        """End the session: close the input, which ends the gateway's side. Returns its exit code (over a socket or HTTP, 0)."""
        if self._http is not None:
            return self._close_http()
        if self._proc is None:
            return self._close_socket()
        if self._proc.poll() is None:
            try:
                self._proc.stdin.close()
            except OSError:
                pass
            try:
                self._proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait()
        for stream in (self._proc.stdin, self._proc.stdout):
            try:
                stream.close()
            except OSError:
                pass
        return self._proc.returncode

    def _close_http(self):
        """End the stream, which ends the session on the server, and let the reader thread see it."""
        self._http.shut()
        self._pumper.join(timeout=10)
        return 0

    def _close_socket(self):
        """Shut the socket's write side, wait (up to 10 seconds) for the daemon to end its side, then release it."""
        sock, self._sock = self._sock, None
        if sock is not None:
            try:
                self._writer.close()
            except OSError:
                pass
            try:
                sock.shutdown(socket.SHUT_WR)
            except OSError:
                pass
            self._pumper.join(timeout=10)
            for stream in (self._reader, sock):
                try:
                    stream.close()
                except OSError:
                    pass
        return 0
