"""Safe reads of a stream-json event (#1519). Events come from a program outside the harness; an error or retry event can carry
"message" as a plain string, "content" as a string, blocks that are not objects. These helpers return an empty value for any
shape they do not expect, so no reader of a stream can crash on it."""


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
