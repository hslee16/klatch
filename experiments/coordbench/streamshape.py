"""Safe reads of a stream-json event (#1519). Events come from a program outside the harness; an error or retry event can carry
"message" as a plain string, "content" as a string, blocks that are not objects, ids and inputs of the wrong type. These helpers
return an empty value for the shapes tests/test_stream_shapes.py tries (and the like); they cannot promise to cover every shape."""


def message(ev):
    """The "message" object of an event, {} when the event has none or it is not an object."""
    m = ev.get("message") if isinstance(ev, dict) else None
    return m if isinstance(m, dict) else {}


def content(ev):
    """The list of content blocks of an event's message, [] for any other shape (a string content, none)."""
    c = message(ev).get("content")
    return c if isinstance(c, list) else []


def blocks(ev):
    """The content blocks that are objects."""
    return [b for b in content(ev) if isinstance(b, dict)]


def text(v):
    """v when it is a string, else ""."""
    return v if isinstance(v, str) else ""


def ident(v):
    """An id usable as a dict key (a string or an integer), else None."""
    return v if isinstance(v, (str, int)) and not isinstance(v, bool) else None


def numbers(d):
    """The numeric fields of a usage object (booleans and text left out), {} for anything but an object."""
    if not isinstance(d, dict):
        return {}
    return {k: v for k, v in d.items() if isinstance(v, (int, float)) and not isinstance(v, bool)}


PATH_KEYS = ("command", "file_path", "path", "pattern", "glob", "notebook_path")


def tool_use(b):
    """A tool_use block with its fields in the types the readers rely on: id (str, int or None), name (str, "?" if missing), input
    (an object; the text fields a reader matches on, command and the paths, are strings: a list becomes its words, anything else
    is dropped)."""
    raw = b.get("input") if isinstance(b.get("input"), dict) else {}
    inp = dict(raw)
    for k in PATH_KEYS:
        v = inp.get(k)
        if k in inp and not isinstance(v, str):
            inp[k] = " ".join(str(x) for x in v) if isinstance(v, list) else ""
    return {"id": ident(b.get("id")), "name": text(b.get("name")) or "?", "input": inp}
