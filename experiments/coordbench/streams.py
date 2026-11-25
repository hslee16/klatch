"""Parse the stream-json output of one agent (`claude -p --output-format stream-json --verbose --include-hook-events`).

The fake driver prints the same event shapes, so one parser serves both drivers. Metrics come from the stream, never
from what an agent says about itself (SPEC section 5), except the human-intervention count which reads the final message.
Standard library only; nothing here starts a process or reads the environment.
"""
import json
import os
import re

import asks
import cmdkinds
import boardmsgs
import ctxcount
import messaging
import streamshape

HERE = os.path.dirname(os.path.abspath(__file__))
USAGE_KEYS = (("input", "input_tokens"), ("output", "output_tokens"), ("cache_read", "cache_read_input_tokens"),
              ("cache_creation", "cache_creation_input_tokens"))
COORD_TOOLS = {"SendMessage", "ListAgents"}
INBOUND_SUBTYPES = ("inbound_message", "cross_session_message", "message_received")
HUMAN_WORDS = re.compile(r"\b(human|operator|user)\b", re.I)
CONFLICT = re.compile(r"CONFLICT \(|Automatic merge failed|unmerged paths", re.I)


def load_prices(path=None):
    with open(path or os.path.join(HERE, "prices.json")) as f:
        return json.load(f)


def price_row(model, per_mtok):
    """The price row for a model id by family; an unknown id gets the dearest row so a cap errs on the safe side."""
    m = (model or "").lower()
    for fam in ("haiku", "sonnet", "opus", "fake"):
        if fam in m:
            return per_mtok[fam]
    return max((per_mtok[k] for k in ("haiku", "sonnet", "opus")), key=lambda r: r["output"])


def usd_of(usage, row):
    return sum((usage.get(src, 0) or 0) * row[k] for k, src in USAGE_KEYS) / 1e6


_text_of = ctxcount.text_of


class StreamState:
    """Incremental parser: feed lines as they arrive, read the totals at any moment."""

    def __init__(self, klatch_bin=None, prices=None):
        self.prices = (prices or load_prices())["per_mtok"]
        binre = re.escape(klatch_bin) if klatch_bin else r"klatch"
        self.msg_re = re.compile(binre + r"\s+(?:-\S+\s+)*(?:post|dm|ask|answer)\b")
        self.cmd_re = re.compile(binre + r"\s+(?:-\S+\s+)*[a-z]")
        # the verbs that read the room back to the agent (S7: the cost of reading the room per member, #1372)
        self.read_re = re.compile(binre + r"\s+(?:-\S+\s+)*(?:read|wait|digest|board|history)\b")
        self.models = []
        self.msgs = {}      # message id -> {"usage", "model", "tools": [tool_use dicts], "text"}
        self.order = []
        self.tools = {}     # tool_use id -> {"name", "input", "coord"}
        self.result = None
        self.fake_usd = None
        self.git_conflicts = 0
        # what entered the agent's context because of the coordination layer, in characters, by source (ctxcount.py):
        # hook output (K2), inbound message events (B1), the coordination calls' inputs, their results, the init tool schemas
        self.parts = {"hook": 0, "inbound": 0, "tool_calls": 0, "tool_results": 0, "schema": 0}
        self.schema_source = ctxcount.SCHEMA_SOURCE_PINNED
        self.new_chars = 0         # the part of that which was new content (a board re-read counts only its new lines' worth)
        self.inbound_events = 0
        self.board_edits = 0       # tool calls that changed TASKS.md: B1's messages on the board (SPEC 5)
        self.last_read = {}        # read-only call kind -> its last normalised result, for "new content" (cmdkinds.is_new)
        self._init_seen = False
        self._assistant_seen = False
        self._last_mid = None
        self.gaps = {}             # assistant message id -> {"coord", "other"} characters that entered after it, before the next
        self.read_chars = 0        # results of tool calls that read the room (klatch read, wait, digest, board, history; TASKS.md)
        self.hook_chars = 0        # what hooks injected (hook_response output): the digest, nudges and wake text
        self.max_coord_result = 0  # the largest single coordination tool result, in characters
        self.tool_total = 0
        self.by_tool = {}
        self.messages = 0
        self._anon = 0
        self.last_text = ""
        self.msg_events = []  # messages sent: {"t", "verb", "text"} (S4 ask metrics, asks.py)
        self.done_t = None    # the harness clock when the result event was read
        self._t = None
        self.klatch_bin = klatch_bin
        # what the run needs to show that native messaging worked (messaging.py): the init tools, every ListAgents and SendMessage
        # result with the harness clock when it was read
        self.msg_tools = None
        self.sent_texts, self.board_texts = {}, {}  # normalised message text -> times, for SendMessage and for board lines
        self.msg_calls = {"listagents": [], "sends": []}

    @property
    def coord_chars(self):
        return sum(self.parts.values())

    def _note_model(self, m):
        if m and m not in self.models:
            self.models.append(m)

    def _is_coord(self, name, inp):
        if name in COORD_TOOLS:
            return True
        blob = json.dumps(inp)
        if name == "Bash":
            return bool(self.cmd_re.search(inp.get("command", ""))) or "TASKS.md" in blob
        return "TASKS.md" in blob

    def _is_read(self, name, inp):
        if cmdkinds.is_board_edit(name, inp):
            return False
        if name == "Bash":
            return bool(self.read_re.search(inp.get("command", ""))) or "TASKS.md" in inp.get("command", "")
        return "TASKS.md" in json.dumps(inp)

    def feed(self, line, t=None):
        self._t = t
        line = line.strip()
        if not line:
            return
        try:
            e = json.loads(line)
        except ValueError:
            return
        if not isinstance(e, dict):
            return
        handler = {"system": self._on_system, "assistant": self._on_assistant, "user": self._on_user,
                   "result": self._on_result}.get(e.get("type"))
        if handler is not None:
            handler(e)

    def _on_system(self, e):
        sub = e.get("subtype")
        if sub == "init":
            self._on_init(e)
        elif sub == "hook_response":
            text = e.get("output") or e.get("stdout") or ""
            if isinstance(text, str):
                self._enter("hook", len(text))
                self.hook_chars += len(text)
        elif sub in INBOUND_SUBTYPES:
            self._enter_inbound(_text_of(e.get("text") or e.get("message") or e.get("content")))
        elif sub == "fake_cost":
            self.fake_usd = float(e.get("usd", 0))

    def _on_init(self, e):
        self._note_model(streamshape.text(e.get("model")))
        if self.msg_tools is None:
            self.msg_tools = messaging.tool_names(e.get("tools"))
        if not self._init_seen:  # the schemas are carried once per agent, however many init events a stream repeats
            self._init_seen = True
            self.parts["schema"], self.schema_source = ctxcount.schema_chars(e.get("tools"))

    def _gap(self):
        return self.gaps.setdefault(self._last_mid, {"coord": 0, "other": 0})

    def _enter(self, part, n, new=True):
        """`n` characters of coordination context entered the agent's context from `part`."""
        self.parts[part] += n
        if new:
            self.new_chars += n
        if self._last_mid is not None:
            self._gap()["coord"] += n

    def _enter_inbound(self, text):
        self.inbound_events += 1
        self._enter("inbound", len(text))

    def _message_slot(self, msg):
        """The record of this assistant message (by id; an id-less one gets an anonymous id), created on first sight."""
        mid = streamshape.ident(msg.get("id"))
        if not mid:
            self._anon += 1
            mid = "anon-%d" % self._anon
        if mid not in self.msgs:
            self.msgs[mid] = {"usage": {}, "model": None, "tools": [], "text": ""}
            self.order.append(mid)
        self._last_mid = mid
        return self.msgs[mid]

    def _on_assistant(self, e):
        msg = streamshape.message(e)
        self._assistant_seen = True
        m = self._message_slot(msg)
        self._note_usage_and_model(m, msg)
        for b in streamshape.blocks(e):
            if b.get("type") == "tool_use":
                self._on_tool_use(m, b)
            elif b.get("type") == "text" and isinstance(b.get("text"), str):
                m["text"] = b["text"]
                self.last_text = b["text"]

    def _note_usage_and_model(self, m, msg):
        if isinstance(msg.get("usage"), dict):
            m["usage"] = streamshape.numbers(msg["usage"])
        if streamshape.text(msg.get("model")):
            m["model"] = msg["model"]
            self._note_model(msg["model"])

    def _is_message(self, name, cmd):
        return name == "SendMessage" or bool(cmd and self.msg_re.search(cmd))

    def _messages_of(self, name, inp, edit):
        """How many messages one tool call is: a klatch post, DM, ask or answer, a SendMessage, or the message-prefix lines a board
        edit adds (boardmsgs.py). A SendMessage and a board line with the same text within 30 s are one message."""
        if name == "SendMessage":
            text = boardmsgs.normalise(inp.get("message"))
            dup = boardmsgs.same_recently(self._t, self.board_texts.get(text, []))
            self.sent_texts.setdefault(text, []).append(self._t)
            return 0 if dup else 1
        if edit:
            n = 0
            for text in boardmsgs.message_texts(name, inp):
                dup = boardmsgs.same_recently(self._t, self.sent_texts.get(text, []))
                self.board_texts.setdefault(text, []).append(self._t)
                n += 0 if dup else 1
            return n
        return 1 if self._is_message(name, inp.get("command", "") if name == "Bash" else "") else 0

    def _on_tool_use(self, m, b):
        tu = streamshape.tool_use(b)
        name, raw = tu["name"], tu["input"]
        # a klatch call through a variable, alias or function defined in the same command is read as the binary (ctxcount)
        inp = dict(raw, command=cmdkinds.resolve_command(raw.get("command"), self.klatch_bin)) if name == "Bash" and "command" in raw else raw
        coord = self._is_coord(name, inp)
        self.tools[tu["id"]] = {"name": name, "input": inp, "coord": coord, "read": coord and self._is_read(name, inp),
                                "kinds": cmdkinds.read_kinds(name, inp, self.klatch_bin), "new": None}
        m["tools"].append(self.tools[tu["id"]])
        self.tool_total += 1
        self.by_tool[name] = self.by_tool.get(name, 0) + 1
        if coord:
            self._enter("tool_calls", len(json.dumps(raw)))
        edit = cmdkinds.is_board_edit(name, inp)
        self.board_edits += edit
        self.messages += self._messages_of(name, inp, edit)
        for ev in asks.extract(name, inp, self.klatch_bin):
            self.msg_events.append(dict(ev, t=self._t))

    def _on_user(self, e):
        content = streamshape.message(e).get("content")
        for b in streamshape.content(e):
            if isinstance(b, dict) and b.get("type") == "tool_result":
                self._on_tool_result(b)
        text = ctxcount.inbound_text(content, self._assistant_seen)
        if text:
            self._enter_inbound(text)

    def _note_new(self, tu, text):
        """Whether a read-only coordination call's result held new content (cmdkinds.is_new); True for any other call."""
        if not tu["kinds"]:
            return True
        key = ",".join(tu["kinds"])
        tu["new"] = cmdkinds.is_new(text, tu["kinds"], self.last_read.get(key))
        self.last_read[key] = cmdkinds.normalize(text)
        return tu["new"]

    def _note_messaging(self, name, text, error):
        """Keep the result of a ListAgents or SendMessage call with the clock reading (messaging.py)."""
        kind = {"ListAgents": "listagents", "SendMessage": "sends"}.get(name)
        if kind:
            self.msg_calls[kind].append({"t": self._t, "text": text, "error": error})

    def _on_tool_result(self, b):
        text = _text_of(b.get("content"))
        tu = self.tools.get(streamshape.ident(b.get("tool_use_id")))
        if tu is None:
            return
        new = self._note_new(tu, text)
        self._note_messaging(tu["name"], text, bool(b.get("is_error")))
        if tu["coord"]:
            self._enter("tool_results", len(text), new)
            self.max_coord_result = max(self.max_coord_result, len(text))
            if tu["read"]:
                self.read_chars += len(text)
        elif text and self._last_mid is not None:
            self._gap()["other"] += len(text)
        if tu["name"] == "Bash" and CONFLICT.search(text):
            self.git_conflicts += 1

    def _on_result(self, e):
        self.result = e
        self.done_t = self._t

    @property
    def estimated_usd(self):
        total = 0.0
        for mid in self.order:
            m = self.msgs[mid]
            total += usd_of(m["usage"], price_row(m["model"] or (self.models[0] if self.models else None), self.prices))
        return total

    @property
    def cost_usd(self):
        if self.result is not None and isinstance(self.result.get("total_cost_usd"), (int, float)):
            return float(self.result["total_cost_usd"])
        if self.fake_usd is not None:
            return self.fake_usd
        return self.estimated_usd

    def _tokens(self, r):
        if isinstance(r.get("usage"), dict):
            usage = streamshape.numbers(r["usage"])
            return {k: int(usage.get(src, 0)) for k, src in USAGE_KEYS}
        return {k: sum(int(self.msgs[i]["usage"].get(src, 0) or 0) for i in self.order) for k, src in USAGE_KEYS}

    def _idle_turns(self):
        """Turns whose every tool call is a read-only coordination call (cmdkinds.read_kinds) whose result had no new
        content (cmdkinds.is_new). A call whose result never arrived is not called idle."""
        return sum(1 for i in self.order if self.msgs[i]["tools"] and all(
            t["kinds"] and t["new"] is False for t in self.msgs[i]["tools"]))

    def summary(self):
        r = self.result or {}
        final = r.get("result") if isinstance(r.get("result"), str) else self.last_text
        return {
            "turns": r["num_turns"] if isinstance(r.get("num_turns"), int) else len(self.order),
            "tokens": self._tokens(r),
            "cost_usd": self.cost_usd,
            "cost_estimated": not isinstance(r.get("total_cost_usd"), (int, float)),
            "tool_calls": {"total": self.tool_total, "by_tool": dict(self.by_tool)},
            "models": list(self.models),
            "messages": self.messages,
            "git_conflicts": self.git_conflicts,
            "idle_turns": self._idle_turns(),
            "board_edits": self.board_edits,
            "human_mentions": 1 if HUMAN_WORDS.search(final or "") else 0,
            "coord_context_chars": self.coord_chars,
            "coord_context_parts": dict(self.parts),
            "coord_context_new_chars": self.new_chars,
            "coord_context_method": ctxcount.method_record(),
            "inbound_events": self.inbound_events,
            "schema_source": self.schema_source,
            "usage_exact": ctxcount.usage_exact([self.msgs[i]["usage"] for i in self.order],
                                                [self.gaps.get(i) for i in self.order]),
            "read_context_chars": self.read_chars,
            "hook_context_chars": self.hook_chars,
            "max_coord_result_chars": self.max_coord_result,
            "messaging": dict(self.msg_calls, tools=self.msg_tools),
            "final_text": final or "",
            "is_error": bool(r.get("is_error")),
            "result_seen": self.result is not None,
            "result_subtype": r.get("subtype") if isinstance(r.get("subtype"), str) else None,
            "terminal_reason": r.get("terminal_reason") if isinstance(r.get("terminal_reason"), str) else None,
        }


def parse_stream(lines, klatch_bin=None, prices=None):
    st = StreamState(klatch_bin=klatch_bin, prices=prices)
    for line in lines:
        st.feed(line)
    return st.summary()


def parse_stream_file(path, klatch_bin=None, prices=None):
    try:
        with open(path, errors="replace") as f:
            return parse_stream(f, klatch_bin=klatch_bin, prices=prices)
    except OSError:
        return parse_stream([], klatch_bin=klatch_bin, prices=prices)
