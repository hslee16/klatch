"""Checksums."""


def adler32(data):
    """The Adler-32 checksum of bytes, as an int (the same value as zlib.adler32(data))."""
    raise NotImplementedError


def luhn_valid(number):
    """True when the digit string passes the Luhn check. Spaces are ignored; any other non-digit character, or no digits
    at all, gives False."""
    raise NotImplementedError
