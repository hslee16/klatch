"""Run-length encoding."""


def rle_encode(text):
    """Encode runs as count then character: "aaabcc" gives "3a1b2c", "" gives "". The text must not contain digits
    (ValueError)."""
    raise NotImplementedError


def rle_decode(text):
    """Invert rle_encode: "3a1b2c" gives "aaabcc". A string that is not a sequence of (count, character) pairs, or has a
    count of 0, raises ValueError."""
    raise NotImplementedError
