"""Walking a sequence in groups."""


def pairs(seq):
    items = list(seq)
    return [(items[i], items[i + 1]) for i in range(len(items) - 1)]


def chunk(seq, n):
    if n < 1:
        raise ValueError("n must be at least 1")
    items = list(seq)
    return [items[i:i + n] for i in range(0, len(items), n)]


def window(seq, n):
    if n < 1:
        raise ValueError("n must be at least 1")
    items = list(seq)
    return [items[i:i + n] for i in range(len(items) - n + 1)]
