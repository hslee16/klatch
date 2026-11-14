"""A thin klatch client over the wire protocol (PROTOCOL.md, version 1): it speaks newline-delimited JSON to
`klatch serve --stdio`, standard library only.

    c = Client("/path/to/klatch", env={"KLATCH_NAME": "alice", ...})
    c.post("lobby", "hello")
    c.subscribe("lobby", from_seq=0); c.collect(until_caught_up=True)

Who you are is decided by the transport (the environment of the process), never by a request parameter. A frame this
version does not know, and a field it does not know, are ignored, as the protocol requires.
"""
import json
import queue
import subprocess
import threading

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


class Client:
    """One session with the gateway. Not thread safe: use one client per thread."""

    def __init__(self, binary, env=None, args=("serve", "--stdio")):
        self._proc = subprocess.Popen(
            [binary, *args],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self._frames = queue.Queue()
        self._next_id = 0
        self.refusals = []
        self._pending_messages = []
        self._caught_up = {}
        threading.Thread(target=self._pump, daemon=True).start()
        hello = self._frame(timeout=10)
        if hello is None or hello.get("type") != "hello":
            raise ProtocolError("the first frame is hello")
        self.member = hello["member"]
        self.attribution = hello["attribution"]
        self.clearance = hello["clearance"]

    # ---- reading ----

    def _pump(self):
        for line in self._proc.stdout:
            try:
                frame = decode_server_frame(line)
            except ProtocolError as e:
                frame = {"type": "_error", "error": str(e)}
            if frame is not None:
                self._frames.put(frame)
        self._frames.put(None)

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
        self._proc.stdin.write(encode_line(frame))
        self._proc.stdin.flush()

    def post(self, room, body, label=None, kind=None, payload=None):
        params = {"room": room, "body": body}
        for k, v in (("label", label), ("kind", kind), ("payload", payload)):
            if v is not None:
                params[k] = v
        return self.request("post", **params)

    def read(self, room, peek=False):
        return self.request("read", room=room, peek=peek)["messages"]

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

    def who(self, room):
        return self.request("who", room=room)["members"]

    def set_status(self, room, status):
        self.request("status", room=room, status=status)

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
        """End the session: close the input, which ends the gateway's side. Returns its exit code."""
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
