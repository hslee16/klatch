"""Word wrapping."""


def wrap(text, width):
    if width < 1:
        raise ValueError("width must be at least 1")
    lines, cur = [], ""
    for word in text.split():
        if cur and len(cur) + 1 + len(word) <= width:
            cur += " " + word
        else:
            if cur:
                lines.append(cur)
            cur = word
    if cur:
        lines.append(cur)
    return lines


def unwrap(lines):
    return " ".join(" ".join(lines).split())
