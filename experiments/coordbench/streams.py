"""Parse the stream-json output of one agent (`claude -p --output-format stream-json --verbose --include-hook-events`).

The fake driver prints the same event shapes, so one parser serves both drivers. Metrics come from the stream, never
from what an agent says about itself (SPEC section 5), except the human-intervention count which reads the final message.
Standard library only; nothing here starts a process or reads the environment.
"""
import json
import os
import re

import asks

HERE = os.path.dirname(os.path.abspath(__file__))
USAGE_KEYS = (("input", "input_tokens"), ("output", "output_tokens"), ("cache_read", "cache_read_input_tokens"),
              ("cache_creation", "cache_creation_input_tokens"))
COORD_TOOLS = {"SendMessage", "ListAgents"}
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


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and isinstance(b.get("text"), str))
    return ""


class StreamState:
    """Incremental parser: feed lines as they arrive, read the totals at any moment."""

    def __init__(self, klatch_bin=None, prices=None):
        self.prices = (prices or load_prices())["per_mtok"]
        binre = re.escape(klatch_bin) if klatch_bin else r"klatch"
        self.msg_re = re.compile(binre + r"\s+(?:-\S+\s+)*(?:post|dm|ask|answer)\b")
        self.cmd_re = re.compile(binre + r"\s+(?:-\S+\s+)*[a-z]")
        # the verbs that read the room back to the agent (S7: the cost of reading the room per member, #1372)
        self.read_re = re.compile(binre + r"\s+(?:-\S+\s+)*(?:read|wait|digest|board|history)\b")
        self.idle_re = re.compile(binre + r"\s+(?:-\S+\s+)*(?:wait|read|history|board|who|claims|asks|stuck|hooks)\b"
                                  r"|^\s*git\s+(?:fetch|log)\b|^\s*sleep\b", re.M)
        self.models = []
        self.msgs = {}      # message id -> {"usage", "model", "tools": [tool_use dicts], "text"}
        self.order = []
        self.tools = {}     # tool_use id -> {"name", "input", "coord"}
        self.result = None
        self.fake_usd = None
        self.git_conflicts = 0
        self.coord_chars = 0
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
            self._note_model(e.get("model"))
        elif sub == "hook_response":
            text = e.get("output") or e.get("stdout") or ""
            if isinstance(text, str):
                self.coord_chars += len(text)
                self.hook_chars += len(text)
        elif sub == "fake_cost":
            self.fake_usd = float(e.get("usd", 0))

    def _message_slot(self, msg):
        """The record of this assistant message (by id; an id-less one gets an anonymous id), created on first sight."""
        mid = msg.get("id")
        if not mid:
            self._anon += 1
            mid = "anon-%d" % self._anon
        if mid not in self.msgs:
            self.msgs[mid] = {"usage": {}, "model": None, "tools": [], "text": ""}
            self.order.append(mid)
        return self.msgs[mid]

    def _on_assistant(self, e):
        msg = e.get("message") or {}
        m = self._message_slot(msg)
        self._note_usage_and_model(m, msg)
        for b in msg.get("content") or []:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_use":
                self._on_tool_use(m, b)
            elif b.get("type") == "text" and isinstance(b.get("text"), str):
                m["text"] = b["text"]
                self.last_text = b["text"]

    def _note_usage_and_model(self, m, msg):
        if isinstance(msg.get("usage"), dict):
            m["usage"] = msg["usage"]
        if msg.get("model"):
            m["model"] = msg["model"]
            self._note_model(msg["model"])

    def _is_message(self, name, cmd):
        return name == "SendMessage" or bool(cmd and self.msg_re.search(cmd))

    def _on_tool_use(self, m, b):
        name, inp = b.get("name", "?"), b.get("input") or {}
        coord = self._is_coord(name, inp)
        self.tools[b.get("id")] = {"name": name, "input": inp, "coord": coord, "read": coord and self._is_read(name, inp)}
        m["tools"].append(self.tools[b.get("id")])
        self.tool_total += 1
        self.by_tool[name] = self.by_tool.get(name, 0) + 1
        if coord:
            self.coord_chars += len(json.dumps(inp))
        if self._is_message(name, inp.get("command", "") if name == "Bash" else ""):
            self.messages += 1
        for ev in asks.extract(name, inp, self.klatch_bin):
            self.msg_events.append(dict(ev, t=self._t))

    def _on_user(self, e):
        for b in (e.get("message") or {}).get("content") or []:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                self._on_tool_result(b)

    def _on_tool_result(self, b):
        text = _text_of(b.get("content"))
        tu = self.tools.get(b.get("tool_use_id"))
        if tu is None:
            return
        if tu["coord"]:
            self.coord_chars += len(text)
            self.max_coord_result = max(self.max_coord_result, len(text))
            if tu["read"]:
                self.read_chars += len(text)
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
            return {k: int(r["usage"].get(src, 0) or 0) for k, src in USAGE_KEYS}
        return {k: sum(int(self.msgs[i]["usage"].get(src, 0) or 0) for i in self.order) for k, src in USAGE_KEYS}

    def _idle_turns(self):
        return sum(1 for i in self.order if self.msgs[i]["tools"] and all(
            t["name"] == "Bash" and self.idle_re.search(t["input"].get("command", "")) for t in self.msgs[i]["tools"]))

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
            "human_mentions": 1 if HUMAN_WORDS.search(final or "") else 0,
            "coord_context_chars": self.coord_chars,
            "read_context_chars": self.read_chars,
            "hook_context_chars": self.hook_chars,
            "max_coord_result_chars": self.max_coord_result,
            "final_text": final or "",
            "is_error": bool(r.get("is_error")),
            "result_seen": self.result is not None,
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
