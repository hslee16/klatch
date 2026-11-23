"""Roman numerals."""

_TABLE = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"),
          (9, "IX"), (5, "V"), (4, "IV"), (1, "I")]


def to_roman(n):
    """The canonical Roman numeral for an int from 1 to 3999. Anything else raises ValueError."""
    if not isinstance(n, int) or isinstance(n, bool) or not 1 <= n <= 3999:
        raise ValueError("n must be an int from 1 to 3999")
    out = []
    for value, symbol in _TABLE:
        while n >= value:
            out.append(symbol)
            n -= value
    return "".join(out)


def from_roman(text):
    """The int for a canonical upper-case Roman numeral. Anything that is not exactly what to_roman would produce
    ("IIII", "VX", "", "iv") raises ValueError."""
    pos, total = 0, 0
    for value, symbol in _TABLE:
        while text.startswith(symbol, pos):
            total += value
            pos += len(symbol)
    if pos != len(text) or total == 0 or to_roman(total) != text:
        raise ValueError("not a canonical Roman numeral")
    return total
