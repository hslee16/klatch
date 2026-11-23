"""Checksums."""


def adler32(data):
    """The Adler-32 checksum of bytes, as an int (the same value as zlib.adler32(data))."""
    a, b = 1, 0
    for byte in data:
        a = (a + byte) % 65521
        b = (b + a) % 65521
    return (b << 16) | a


def luhn_valid(number):
    """True when the digit string passes the Luhn check. Spaces are ignored; any other non-digit character, or no digits
    at all, gives False."""
    digits = number.replace(" ", "")
    if not digits or not digits.isascii() or not digits.isdigit():
        return False
    total = 0
    for i, ch in enumerate(reversed(digits)):
        d = int(ch)
        if i % 2 == 1:
            d *= 2
            if d > 9:
                d -= 9
        total += d
    return total % 10 == 0
