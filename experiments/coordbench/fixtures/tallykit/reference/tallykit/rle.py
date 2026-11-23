"""Run-length encoding."""
import re


def rle_encode(text):
    """Encode runs as count then character: "aaabcc" gives "3a1b2c", "" gives "". The text must not contain digits
    (ValueError)."""
    if any(ch.isdigit() for ch in text):
        raise ValueError("text must not contain digits")
    out = []
    i = 0
    while i < len(text):
        j = i
        while j < len(text) and text[j] == text[i]:
            j += 1
        out.append("%d%s" % (j - i, text[i]))
        i = j
    return "".join(out)


def rle_decode(text):
    """Invert rle_encode: "3a1b2c" gives "aaabcc". A string that is not a sequence of (count, character) pairs, or has a
    count of 0, raises ValueError."""
    if not re.fullmatch(r"(?:\d+\D)*", text, re.S):
        raise ValueError("malformed run-length text")
    out = []
    for count, ch in re.findall(r"(\d+)(\D)", text, re.S):
        if int(count) == 0:
            raise ValueError("zero count")
        out.append(ch * int(count))
    return "".join(out)
