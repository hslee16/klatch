"""Bin codes for the inventory (step 2)."""
import collections
import re

Bin = collections.namedtuple("Bin", ["pfx", "num"])
_BIN = re.compile(r"([A-Z]{2})[-/]?([0-9]+)")


def parse_bin(raw):
    """Split a raw bin code (two capital letters, an optional - or /, digits) into Bin(pfx, num); num is an int."""
    m = _BIN.fullmatch(raw.strip())
    if m is None:
        raise ValueError("not a bin code: %r" % (raw,))
    return Bin(m.group(1), int(m.group(2)))


LEGACY = "MD"
MOVED_TO = "RW"
OFFSET = 176


def canonical_bin(raw):
    """The canonical code of a bin: PREFIX-NNNNN. A legacy bin moves to another prefix and number."""
    b = parse_bin(raw)
    if b.pfx == LEGACY:
        return "%s-%05d" % (MOVED_TO, b.num + OFFSET)
    return "%s-%05d" % (b.pfx, b.num)
