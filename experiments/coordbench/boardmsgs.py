"""Which board edits are messages (SPEC 5, metric version 2, #1520 review). A board edit is a message only when it adds a line
that starts with one of the shared message prefixes (`seeking:`, `answer:`, `handoff:`, `need-info:`, `info:`); a claim or status
line (`- NAME | working | FILES | TITLE`) is a board edit and nothing more. A board answer that repeats text the same agent just
sent with SendMessage (the same text within DEDUPE_S seconds) is the same message, not a second one. Standard library only."""
import re

PREFIXES = ("seeking", "answer", "handoff", "need-info", "info")
DEDUPE_S = 30.0
_LINE = re.compile(r"^\s*(?:-\s+)?(%s):\s*(.*?)\s*$" % "|".join(PREFIXES), re.I)
_IN_COMMAND = re.compile(r"(?:^|[\n'\"])\s*(?:-\s+)?(?:%s):\s*[^'\"\n]*" % "|".join(PREFIXES), re.I)
_FROM = re.compile(r"\s*\(from [^)]*\)\s*$")


def _lines(text):
    return [ln for ln in str(text or "").split("\n")]


def _added_lines(name, inp):
    """The lines a tool call adds to the board. Edit and MultiEdit: the lines of the new text that the old text did not have;
    Write: every line of the content (it cannot say which are new); Bash: the quoted or line-start fragments of the command."""
    if name == "Edit":
        old = set(_lines(inp.get("old_string")))
        return [ln for ln in _lines(inp.get("new_string")) if ln not in old]
    if name == "MultiEdit":
        edits = [e for e in inp.get("edits", []) if isinstance(e, dict)] if isinstance(inp.get("edits"), list) else []
        return [ln for e in edits for ln in _added_lines("Edit", e)]
    if name == "Write":
        return _lines(inp.get("content"))
    if name == "Bash":
        return [m.group(0).lstrip("\n'\" \t") for m in _IN_COMMAND.finditer(str(inp.get("command", "")))]
    return []


def message_texts(name, inp):
    """[normalised text] of the message lines a board edit adds (its prefix and a trailing `(from NAME)` removed)."""
    out = []
    for ln in _added_lines(name, inp):
        m = _LINE.match(ln)
        if m:
            out.append(normalise(m.group(2)))
    return out


def normalise(text):
    """The comparable form of a message body: no leading prefix, no trailing `(from NAME)`, whitespace collapsed."""
    t = " ".join(str(text or "").split())
    m = _LINE.match(t)
    t = m.group(2) if m else t
    return _FROM.sub("", t).strip()


def same_recently(t, others):
    """True when an earlier or later entry of `others` ((time, text) pairs) is within DEDUPE_S of t (no times: always)."""
    return any(t is None or u is None or abs(t - u) <= DEDUPE_S for u in others)
