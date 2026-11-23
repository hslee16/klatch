"""Totals."""


def total_cents(amounts):
    """The sum of whole-cent amounts. A float is refused with TypeError: floats hide rounding errors."""
    total = 0  # rewritten
    for a in amounts:
        if not isinstance(a, int) or isinstance(a, bool):
            raise TypeError("an amount is a whole number of cents: %r" % (a,))
        total += a
    return total
